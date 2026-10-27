use std::net::IpAddr;
use std::path::{Path, PathBuf};

use nekochat_server::config::{Config, ProviderEntry, load, resolve_api_keys, startup_warnings};

fn cfg(listen: &str, access_password: Option<&str>, trusted_proxies: Vec<IpAddr>) -> Config {
    Config {
        listen: listen.parse().unwrap(),
        data_dir: PathBuf::from("./data"),
        access_password: access_password.map(str::to_owned),
        admin_password: None,
        trusted_proxies,
        allowed_hosts: vec![],
        providers: vec![],
    }
}

fn cfg_with_one_provider() -> Config {
    let mut c = cfg("127.0.0.1:8787", None, vec![]);
    c.providers.push(ProviderEntry {
        key: "deepseek".into(),
        label: "DeepSeek 官方".into(),
        base_url: "https://api.deepseek.com".into(),
        models_url: None,
        api_key_env: "DEEPSEEK_API_KEY".into(),
        session_header: None,
        monitoring: None,
    });
    c
}

#[test]
fn example_config_parses() {
    let cfg = load(Path::new("../../config.example.toml")).unwrap();
    assert_eq!(cfg.listen, "127.0.0.1:10000".parse().unwrap());
    assert_eq!(
        cfg.providers
            .iter()
            .map(|p| p.key.as_str())
            .collect::<Vec<_>>(),
        ["deepseek", "opencode-go"]
    );
    assert_eq!(
        cfg.providers[1].session_header.as_deref(),
        Some("x-opencode-session")
    );
}

#[test]
fn missing_api_key_names_provider_and_var() {
    let err = resolve_api_keys(&cfg_with_one_provider(), |_| None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("deepseek") && err.contains("DEEPSEEK_API_KEY"));
}

#[test]
fn loopback_without_password_has_no_warning() {
    assert!(startup_warnings(&cfg("127.0.0.1:8787", None, vec![])).is_empty());
}

#[test]
fn public_without_password_warns_open_access() {
    let w = startup_warnings(&cfg("0.0.0.0:8787", None, vec![]));
    assert!(w.iter().any(|s| s.contains("未设置访问密码")));
}

#[test]
fn public_with_password_without_proxy_warns_plaintext() {
    let w = startup_warnings(&cfg("0.0.0.0:8787", Some("pw"), vec![]));
    assert!(w.iter().any(|s| s.contains("明文")));
}

#[test]
fn public_with_password_proxy_and_hosts_is_quiet() {
    let mut c = cfg(
        "0.0.0.0:8787",
        Some("pw"),
        vec!["127.0.0.1".parse().unwrap()],
    );
    c.allowed_hosts = vec!["chat.example.com".into()];
    assert!(startup_warnings(&c).is_empty());
}

#[test]
fn unspecified_listen_without_allowed_hosts_warns() {
    for listen in ["0.0.0.0:8787", "[::]:8787"] {
        let w = startup_warnings(&cfg(listen, Some("pw"), vec!["127.0.0.1".parse().unwrap()]));
        assert!(w.iter().any(|s| s.contains("allowed_hosts")), "{listen}");
    }
    let w = startup_warnings(&cfg("192.168.1.5:8787", Some("pw"), vec![]));
    assert!(!w.iter().any(|s| s.contains("allowed_hosts")));
}
