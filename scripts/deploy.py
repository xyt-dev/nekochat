#!/usr/bin/env python3
"""Build/reuse, deploy outside target, start/stop, and clean Nekochat (Linux, Python 3.11+)."""
import argparse
from contextlib import contextmanager, ExitStack
from dataclasses import dataclass
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.error
import urllib.request

PROGRAMS = ("nekochat", "nekochat-search")
PROXY_ENV = {name for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY") for name in (key, key.lower())}


def existing_binaries(root, runtime):
    for directory in (root / "target/release", runtime / "bin"):
        if all((directory / name).is_file() and os.access(directory / name, os.X_OK) for name in PROGRAMS):
            return directory
    return None


def validate_runtime(root, runtime):
    root, runtime = root.resolve(), runtime.resolve()
    # A dedicated directory must never contain source, configuration, or Cargo output.
    protected = ("target", "data", "web", "crates", "scripts", "docs", "resource-packs", ".git", ".superpowers")
    if runtime == root or runtime in root.parents or any(runtime.is_relative_to(root / name) for name in protected):
        raise RuntimeError("runtime 必须是独立目录，不能使用仓库本身、源码、数据或 target 目录")
    marker = runtime / ".nekochat-runtime"
    installed = runtime / "bin"
    if marker.exists() and (marker.is_symlink() or marker.read_text() != str(root)):
        raise RuntimeError("独立目录属于另一个仓库，拒绝覆盖")
    if installed.exists() or installed.is_symlink():
        if not marker.is_file() or installed.is_symlink() or not installed.is_dir():
            raise RuntimeError("现有 bin/ 未由本站部署脚本管理，拒绝覆盖")
        if any(path.name not in PROGRAMS or not path.is_file() or path.is_symlink() for path in installed.iterdir()):
            raise RuntimeError("bin/ 包含非本站程序，拒绝覆盖或删除")


@dataclass(frozen=True)
class Process:
    pid: int
    started: int

    @classmethod
    def read(cls, pid):
        stat = (Path("/proc") / str(pid) / "stat").read_text()
        fields = stat[stat.rfind(")") + 2:].split()
        if fields[0] == "Z":
            raise ProcessLookupError(pid)
        return cls(pid, int(fields[19]))

    def alive(self):
        try:
            return Process.read(self.pid) == self
        except (OSError, ValueError):
            return False

    def terminate(self):
        # pidfd binds the signal to one process, including if the numeric PID is reused.
        try:
            fd = os.pidfd_open(self.pid)
            try:
                if self.alive():
                    signal.pidfd_send_signal(fd, signal.SIGTERM)
            finally:
                os.close(fd)
        except ProcessLookupError:
            pass

    def parent(self):
        try:
            stat = (Path("/proc") / str(self.pid) / "stat").read_text()
            return int(stat[stat.rfind(")") + 2:].split()[1]) if self.alive() else None
        except OSError:
            return None


def matches_service(exe, args, cwd, root, config, role):
    name = Path(exe.removesuffix(" (deleted)")).name
    expected = r"nekochat(?:-\d+\.\d+\.\d+)?" if role == "web" else r"nekochat-search"
    if not re.fullmatch(expected, name) or cwd != root or (role == "web" and "serve" not in args):
        return False
    configured = None
    for index, arg in enumerate(args):
        if arg.startswith("--config="):
            configured = arg.split("=", 1)[1]
        elif arg == "--config" and index + 1 < len(args):
            configured = args[index + 1]
    return configured is not None and (cwd / configured).resolve() == config


def services(root, config, role):
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            process = Process.read(int(entry.name))
            exe = os.readlink(entry / "exe")
            cwd = Path(os.readlink(entry / "cwd"))
            args = [part.decode() for part in (entry / "cmdline").read_bytes().split(b"\0") if part]
            if matches_service(exe, args, cwd, root, config, role) and process.alive():
                found.append(process)
        except (OSError, ValueError, UnicodeError):
            continue
    return found


def owned_search(web, search):
    parents = {process.pid for process in web}
    return [process for process in search if process.parent() in parents]


def managed_services(root, config, search_config):
    web = services(root, config, "web")
    return web, owned_search(web, services(root, search_config, "search"))


def stop_processes(processes):
    for process in processes:
        process.terminate()
    deadline = time.monotonic() + 20
    while any(process.alive() for process in processes):
        if time.monotonic() >= deadline:
            raise RuntimeError("进程未在 20 秒内退出；保留文件，不强制杀进程，请检查服务状态")
        time.sleep(0.1)


def load_saved_environment(runtime):
    path = runtime / "environment.json"
    if not path.exists():
        return {}
    if path.is_symlink() or path.stat().st_mode & 0o077:
        raise RuntimeError("runtime/environment.json 必须是权限 0600 的普通文件")
    values = json.loads(path.read_text())
    if not isinstance(values, dict) or not all(isinstance(k, str) and isinstance(v, str) for k, v in values.items()):
        raise RuntimeError("保存的环境变量格式无效")
    return values


def save_environment(runtime, env, required):
    runtime.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".environment-", dir=runtime)
    try:
        with os.fdopen(fd, "w") as output:
            json.dump({key: env[key] for key in sorted(required | PROXY_ENV) if key in env}, output)
        os.replace(temporary, runtime / "environment.json")
    finally:
        Path(temporary).unlink(missing_ok=True)


def runtime_environment(root, runtime, required, running):
    env = dict(os.environ)
    saved = load_saved_environment(runtime)
    preserved = required | PROXY_ENV
    # Existing deployments may have been launched without an environment file.
    for process in running:
        try:
            entries = (Path("/proc") / str(process.pid) / "environ").read_bytes().split(b"\0")
            if process.alive():
                for item in entries:
                    key, separator, value = item.partition(b"=")
                    if separator and key.decode(errors="replace") in preserved:
                        saved[key.decode()] = value.decode()
        except OSError:
            pass
    for key in preserved:
        if key not in env and key in saved:
            env[key] = saved[key]
    env_file = root / ".env.local"
    if env_file.is_file():
        result = subprocess.run(["bash", "-c", 'set -ae; source "$1" >/dev/null; exec env -0', "bash", str(env_file)],
                                env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if result.returncode:
            raise RuntimeError("加载 .env.local 失败；未输出文件内容，请自行检查")
        env = dict(item.decode().split("=", 1) for item in result.stdout.split(b"\0") if b"=" in item)
    missing = sorted(key for key in required if not env.get(key, "").strip())
    if missing:
        raise RuntimeError("缺少环境变量：" + ", ".join(missing))
    return env


def configurations(config, search_config):
    web = tomllib.loads(config.read_text())
    search = tomllib.loads(search_config.read_text())
    required = {item["api_key_env"] for item in web.get("providers", [])}
    required.update((search["access_token_env"], search["admin_token_env"]))
    return web, search, required


def listen_address(config, default):
    host, port = config.get("listen", default).rsplit(":", 1)
    return host.strip("[]"), int(port)


def ensure_ports_free(web, search):
    for config, default in ((web, "0.0.0.0:10000"), (search, "127.0.0.1:10011")):
        host, port = listen_address(config, default)
        family = socket.AF_INET6 if ":" in host else socket.AF_INET
        with socket.socket(family, socket.SOCK_STREAM) as listener:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            try:
                listener.bind((host, port))
            except OSError:
                raise RuntimeError(f"监听端口 {port} 被占用；不会停止未匹配的进程") from None


def stop(root, config, search_config):
    web, search = managed_services(root, config, search_config)
    stop_processes(web)
    stop_processes(search)
    # Catch a supervisor that restarted the service; never clean/start over it.
    if services(root, config, "web"):
        raise RuntimeError("检测到服务重新启动，请先停止外部进程管理器")
    print("本站主进程与搜索子进程已停止。", flush=True)


def owns_listener(pid, config, default):
    host, port = listen_address(config, default)
    family = socket.AF_INET6 if ":" in host else socket.AF_INET
    packed = socket.inet_pton(family, host)
    address = b"".join(packed[n:n + 4][::-1] if sys.byteorder == "little" else packed[n:n + 4] for n in range(0, len(packed), 4))
    expected = f"{address.hex().upper()}:{port:04X}"
    directory = Path("/proc") / str(pid)
    try:
        inodes = set()
        for fd in (directory / "fd").iterdir():
            try:
                link = os.readlink(fd)
                if link.startswith("socket:["):
                    inodes.add(link[8:-1])
            except OSError:
                pass
        table = "tcp6" if family == socket.AF_INET6 else "tcp"
        for row in (directory / "net" / table).read_text().splitlines()[1:]:
            fields = row.split()
            if fields[1] == expected and fields[3] == "0A" and fields[9] in inodes:
                return True
    except OSError:
        pass
    return False


def start(root, runtime, config, search_config, web, search, env):
    if services(root, config, "web") or services(root, search_config, "search"):
        raise RuntimeError("本站进程仍在运行；需要替换时请执行 deploy 或先 stop")
    ensure_ports_free(web, search)
    log_path = runtime / "service.log"
    fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, "ab") as log:
        child = subprocess.Popen([str(runtime / "bin/nekochat"), "serve", "--config", str(config),
                                  "--search-config", str(search_config)], cwd=root, env=env,
                                 stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
    host, port = listen_address(web, "0.0.0.0:10000")
    host = {"0.0.0.0": "127.0.0.1", "::": "::1"}.get(host, host)
    authority = f"[{host}]" if ":" in host else host
    request = urllib.request.Request(f"http://{authority}:{port}/api/health", headers={"Host": "localhost"})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    deadline = time.monotonic() + 30
    try:
        while child.poll() is None:
            try:
                if owns_listener(child.pid, web, "0.0.0.0:10000"):
                    with opener.open(request, timeout=1) as response:
                        healthy = response.status == 200 and response.read() == b"ok"
                    homepage = urllib.request.Request(f"http://{authority}:{port}/", headers={"Host": "localhost"})
                    with opener.open(homepage, timeout=1) as response:
                        frontend = response.status == 200 and response.headers.get_content_type() == "text/html"
                    if healthy and frontend and child.poll() is None:
                        save_environment(runtime, env, configurations(config, search_config)[2])
                        print(f"部署程序已启动，端口归属、首页及健康检查通过（PID {child.pid}）；日志：{log_path}", flush=True)
                        return
            except (OSError, urllib.error.URLError):
                pass
            if time.monotonic() >= deadline:
                break
            time.sleep(0.2)
        raise RuntimeError(f"启动失败或健康检查超时；请检查 {log_path}，target 尚未清理")
    except BaseException:
        if child.poll() is None:
            child.terminate()
            child.wait(timeout=25)
        raise


def build(root):
    print("未找到完整程序或指定了 --rebuild，构建前端和两个 release 程序。", flush=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(root / "target"))
    subprocess.run(["bun", "install", "--frozen-lockfile"], cwd=root / "web", env=env, check=True)
    subprocess.run(["bun", "run", "build"], cwd=root / "web", env=env, check=True)
    subprocess.run(["cargo", "build", "--release", "--locked", "-p", "nekochat-server", "-p", "nekochat-search"], cwd=root, env=env, check=True)
    return root / "target/release"


def clean_target(root):
    target = root / "target"
    if not target.exists():
        return
    if target.is_symlink():
        raise RuntimeError("target 是符号链接，拒绝自动清理")
    for entry in Path("/proc").iterdir():
        if entry.name.isdigit():
            try:
                exe = os.readlink(entry / "exe")
                if not exe.endswith(" (deleted)") and Path(exe).is_relative_to(target):
                    raise RuntimeError("仍有进程使用 target 内的程序，拒绝清理")
            except OSError:
                pass
    # Only this repository's target; never a configured shared Cargo target directory.
    shutil.rmtree(target)
    print("已清理仓库 target/，独立程序、配置与数据保留。", flush=True)


@contextmanager
def deployment_lock(root, runtime):
    runtime.mkdir(parents=True, exist_ok=True)
    with ExitStack() as stack:
        for path in (root / ".deploy.lock", runtime / ".deploy.lock"):
            fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
            lock = stack.enter_context(os.fdopen(fd, "w"))
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise RuntimeError("另一个部署操作正在运行") from None
        validate_runtime(root, runtime)
        marker = runtime / ".nekochat-runtime"
        if not marker.exists():
            fd = os.open(marker, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(fd, "w") as output:
                output.write(str(root))
        yield


def snapshot_running(root, config, search_config, destination):
    web, search = managed_services(root, config, search_config)
    if not web:
        return None
    if len(web) != 1 or len(search) != 1:
        raise RuntimeError("现有实例不是单一 Web 与所属搜索子进程，拒绝自动替换")
    destination.mkdir()
    for name, process in zip(PROGRAMS, (web[0], search[0])):
        shutil.copy2(Path("/proc") / str(process.pid) / "exe", destination / name)
        if not process.alive():
            raise RuntimeError("原进程在准备部署期间已变化，未执行停止")
    entries = (Path("/proc") / str(web[0].pid) / "environ").read_bytes().split(b"\0")
    if not web[0].alive():
        raise RuntimeError("原进程在准备部署期间已变化，未执行停止")
    return dict(item.decode().split("=", 1) for item in entries if b"=" in item)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", nargs="?", choices=("deploy", "start", "stop", "status"), default="deploy")
    parser.add_argument("--runtime-dir", type=Path, help="独立程序目录，默认仓库 runtime/")
    parser.add_argument("--config", type=Path, help="网站配置，默认仓库 config.toml")
    parser.add_argument("--search-config", type=Path, help="搜索配置，默认仓库 crates/search/config.toml")
    parser.add_argument("--rebuild", action="store_true", help="deploy 时强制重新构建")
    parser.add_argument("--no-clean", action="store_true", help="deploy 后保留 target")
    args = parser.parse_args()
    if args.action != "deploy" and (args.rebuild or args.no_clean):
        parser.error("--rebuild 和 --no-clean 仅用于 deploy")
    root = Path(__file__).resolve().parents[1]
    runtime = (args.runtime_dir or root / "runtime").resolve()
    config = (args.config or root / "config.toml").resolve()
    search_config = (args.search_config or root / "crates/search/config.toml").resolve()
    validate_runtime(root, runtime)
    if os.name != "posix" or not Path("/proc").is_dir() or not hasattr(os, "pidfd_open"):
        raise RuntimeError("需要支持 pidfd 的 Linux 和 Python 3.11+")
    with deployment_lock(root, runtime):
        if args.action == "status":
            web, search = managed_services(root, config, search_config)
            print("Web PID：" + (", ".join(str(p.pid) for p in web) or "未运行"))
            print("搜索 PID：" + (", ".join(str(p.pid) for p in search) or "未运行"))
            return
        if args.action == "stop":
            stop(root, config, search_config)
            return
        web, search, required = configurations(config, search_config)
        env = runtime_environment(root, runtime, required, services(root, config, "web"))
        if args.action == "start":
            if not all((runtime / "bin" / name).is_file() for name in PROGRAMS):
                raise RuntimeError("尚无独立程序，请先执行 deploy")
            start(root, runtime, config, search_config, web, search, env)
            return
        source = None if args.rebuild else existing_binaries(root, runtime)
        if source is None:
            source = build(root)
        else:
            print(f"复用已有程序，不重新构建：{source}", flush=True)
        # Copy and execute the staged programs before stopping the live service.
        with tempfile.TemporaryDirectory(prefix=".stage-", dir=runtime) as temporary:
            stage = Path(temporary) / "bin"
            stage.mkdir()
            for name in PROGRAMS:
                shutil.copy2(source / name, stage / name)
                subprocess.run([str(stage / name), "--help"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
            previous = runtime / ".previous-bin"
            if previous.exists():
                raise RuntimeError("存在上次未完成部署的 .previous-bin；请先检查，不覆盖旧程序")
            rollback = Path(temporary) / "rollback-bin"
            rollback_env = snapshot_running(root, config, search_config, rollback)
            stop(root, config, search_config)
            ensure_ports_free(web, search)
            installed = runtime / "bin"
            if installed.exists():
                installed.rename(previous)
            try:
                stage.rename(installed)
                start(root, runtime, config, search_config, web, search, env)
            except BaseException:
                if installed.exists():
                    shutil.rmtree(installed)
                if rollback_env is not None:
                    rollback.rename(installed)
                    if previous.exists():
                        shutil.rmtree(previous)
                    try:
                        start(root, runtime, config, search_config, web, search, rollback_env)
                    except Exception:
                        raise RuntimeError(f"新程序启动失败，旧程序已恢复但重启也失败；target 保留，请检查 {runtime / 'service.log'}") from None
                    print("新程序启动失败，已恢复并重启原服务；target 保留。", flush=True)
                elif previous.exists():
                    previous.rename(installed)
                raise
            if previous.exists():
                shutil.rmtree(previous)
        if not args.no_clean:
            clean_target(root)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        # Do not include subprocess environments or configuration contents in errors.
        if isinstance(error, (ValueError, KeyError)):
            print("失败：配置或保存的环境文件格式无效，请检查本地文件。", file=sys.stderr)
        else:
            print(f"失败：{error}", file=sys.stderr)
        sys.exit(1)
