#!/usr/bin/env python3
"""Deployment boundary regressions; run with python3 -m unittest discover -s scripts -p 'test_*.py'."""
import importlib.util
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import unittest
from dataclasses import replace
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("deploy.py")


class DeploymentTests(unittest.TestCase):
    def setUp(self):
        if "nekochat_deploy" not in sys.modules:
            spec = importlib.util.spec_from_file_location("nekochat_deploy", SCRIPT)
            module = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = module
            spec.loader.exec_module(module)
        self.deploy = sys.modules["nekochat_deploy"]
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.runtime = self.root / "runtime"

    def binaries(self, directory, names=("nekochat", "nekochat-search")):
        directory.mkdir(parents=True, exist_ok=True)
        for name in names:
            shutil.copy2("/bin/true", directory / name)
        return directory

    def test_reuses_complete_build_before_deployed_pair(self):
        built = self.binaries(self.root / "target/release")
        self.binaries(self.runtime / "bin")
        self.assertEqual(self.deploy.existing_binaries(self.root, self.runtime), built)

    def test_partial_build_never_mixes_with_deployed_programs(self):
        self.binaries(self.root / "target/release", ("nekochat",))
        deployed = self.binaries(self.runtime / "bin")
        self.assertEqual(self.deploy.existing_binaries(self.root, self.runtime), deployed)
        (deployed / "nekochat").unlink()
        self.assertIsNone(self.deploy.existing_binaries(self.root, self.runtime))

    def test_deployment_cannot_be_cleaned_with_target(self):
        for path in (self.root, self.root / "target/runtime", self.root / "web"):
            with self.subTest(path=path), self.assertRaises(RuntimeError):
                self.deploy.validate_runtime(self.root, path)
        (self.root / "alias").symlink_to(self.root / "target", target_is_directory=True)
        with self.assertRaises(RuntimeError):
            self.deploy.validate_runtime(self.root, self.root / "alias/runtime")

    def test_stop_matches_configuration_and_not_another_instance(self):
        config = self.root / "config.toml"
        matches = self.deploy.matches_service
        self.assertTrue(matches("/old/nekochat-0.13.0", ["nekochat", "serve", "--config", "config.toml"], self.root, self.root, config, "web"))
        self.assertFalse(matches("/old/nekochat", ["nekochat", "serve", "--config", "/other/config.toml"], self.root, self.root, config, "web"))
        self.assertFalse(matches("/old/another-program", ["x", "serve", "--config", "config.toml"], self.root, self.root, config, "web"))
        self.assertFalse(matches("/old/nekochat-search", ["x", "--config", "config.toml"], self.root, self.root, config, "web"))

    def test_pid_identity_mismatch_does_not_signal_live_process(self):
        with subprocess.Popen(["/bin/sleep", "60"]) as child:
            try:
                identity = self.deploy.Process.read(child.pid)
                self.deploy.stop_processes([replace(identity, started=identity.started + 1)])
                self.assertIsNone(child.poll())
                self.deploy.stop_processes([identity])
                self.assertEqual(child.wait(timeout=3), -15)
            finally:
                if child.poll() is None:
                    child.terminate()

    def test_saved_environment_is_private_and_omits_unrelated_variables(self):
        required = {"DEPLOY_TEST_TOKEN"}
        self.deploy.save_environment(self.runtime, {"DEPLOY_TEST_TOKEN": "synthetic", "UNRELATED": "omit"}, required)
        path = self.runtime / "environment.json"
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.deploy.load_saved_environment(self.runtime), {"DEPLOY_TEST_TOKEN": "synthetic"})

    def test_refuses_unmanaged_runtime_bin_directory(self):
        self.binaries(self.runtime / "bin", ("unrelated-command",))
        with self.assertRaises(RuntimeError):
            self.deploy.validate_runtime(self.root, self.runtime)

    def test_stop_recognizes_clap_equals_config_argument(self):
        self.assertTrue(self.deploy.matches_service("/old/nekochat", ["nekochat", "serve", "--config=config.toml"], self.root, self.root, self.root / "config.toml", "web"))

    def test_runtime_recovery_preserves_proxy_without_unrelated_environment(self):
        with subprocess.Popen(["/bin/sleep", "60"], env={"HTTPS_PROXY": "http://proxy.example:8080", "UNRELATED": "omit"}) as child:
            try:
                with patch.dict(os.environ, {}, clear=True):
                    env = self.deploy.runtime_environment(self.root, self.runtime, set(), [self.deploy.Process.read(child.pid)])
                self.assertEqual(env["HTTPS_PROXY"], "http://proxy.example:8080")
                self.deploy.save_environment(self.runtime, env, set())
                self.assertEqual(self.deploy.load_saved_environment(self.runtime)["HTTPS_PROXY"], "http://proxy.example:8080")
                self.assertNotIn("UNRELATED", self.deploy.load_saved_environment(self.runtime))
            finally:
                child.terminate()

    def test_other_runtime_destination_cannot_bypass_repository_lock(self):
        with self.deploy.deployment_lock(self.root, self.runtime):
            with self.assertRaises(RuntimeError):
                with self.deploy.deployment_lock(self.root, self.root / "another-runtime"):
                    self.fail("shared repository lock was bypassed")

    def test_clean_refuses_a_live_executable_inside_target(self):
        directory = self.root / "target/release"
        directory.mkdir(parents=True)
        shutil.copy2("/bin/sleep", directory / "nekochat")
        with subprocess.Popen([str(directory / "nekochat"), "60"]) as child:
            try:
                with self.assertRaises(RuntimeError):
                    self.deploy.clean_target(self.root)
                self.assertTrue((directory / "nekochat").exists())
            finally:
                child.terminate()

    def test_search_child_must_belong_to_selected_web_parent(self):
        with subprocess.Popen(["/bin/sleep", "60"]) as child:
            try:
                parent = self.deploy.Process.read(os.getpid())
                search = self.deploy.Process.read(child.pid)
                self.assertEqual(self.deploy.owned_search([parent], [search]), [search])
                self.assertEqual(self.deploy.owned_search([search], [search]), [])
                self.assertEqual(self.deploy.owned_search([], [search]), [])
            finally:
                child.terminate()

    def test_health_listener_must_be_owned_by_started_process(self):
        with socket.socket() as listener, subprocess.Popen(["/bin/sleep", "60"]) as child:
            try:
                listener.bind(("127.0.0.1", 0))
                listener.listen()
                config = {"listen": f"127.0.0.1:{listener.getsockname()[1]}"}
                self.assertTrue(self.deploy.owns_listener(os.getpid(), config, ""))
                self.assertFalse(self.deploy.owns_listener(child.pid, config, ""))
            finally:
                child.terminate()


if __name__ == "__main__":
    unittest.main()
