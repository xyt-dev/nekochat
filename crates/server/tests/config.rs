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
    });
    c
}

#[test]
fn example_config_parses() {
    let cfg = load(Path::new("../../config.example.toml")).unwrap();
    assert_eq!(cfg.listen, "127.0.0.1:8787".parse().unwrap());
    assert_eq!(cfg.providers.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(), ["deepseek", "opencode-go"]);
    assert_eq!(cfg.providers[1].session_header.as_deref(), Some("x-opencode-session"));
}

#[test]
fn missing_api_key_names_provider_and_var() {
    let err = resolve_api_keys(&cfg_with_one_provider(), |_| None).unwrap_err().to_string();
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
fn public_with_password_and_proxy_is_quiet() {
    assert!(startup_warnings(&cfg("0.0.0.0:8787", Some("pw"), vec!["127.0.0.1".parse().unwrap()])).is_empty());
}
