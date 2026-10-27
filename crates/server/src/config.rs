//! Server configuration file loading and startup checks.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Deserializer};

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct ProviderEntry {
    pub key: String,
    pub label: String,
    pub base_url: String,
    #[serde(default)]
    pub models_url: Option<String>,
    pub api_key_env: String,
    #[serde(default)]
    pub session_header: Option<String>,
    #[serde(default)]
    pub monitoring: Option<Monitoring>,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MonitoringKind {
    Deepseek,
    OpencodeGo,
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoPlan {
    #[default]
    Go,
    GoPlus,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Monitoring {
    pub kind: MonitoringKind,
    #[serde(default)]
    pub balance_url: Option<String>,
    #[serde(default)]
    pub plan: GoPlan,
    #[serde(default, deserialize_with = "monthly_limits")]
    pub monthly_limits: HashMap<String, f64>,
}

fn monthly_limits<'de, D: Deserializer<'de>>(d: D) -> Result<HashMap<String, f64>, D::Error> {
    let limits = HashMap::<String, f64>::deserialize(d)?;
    if limits.values().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(serde::de::Error::custom(
            "monthly_limits must be finite and nonnegative",
        ));
    }
    Ok(limits)
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub access_password: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub admin_password: Option<String>,
    #[serde(default)]
    pub trusted_proxies: Vec<IpAddr>,
    /// Extra hostnames accepted in the `Host` header (reverse proxy / Tailscale names).
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub providers: Vec<ProviderEntry>,
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 10000))
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("./data")
}

fn empty_as_none<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.filter(|s| !s.is_empty()))
}

pub fn load(path: &Path) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("读取配置文件 {} 失败", path.display()))?;
    toml::from_str(&text).with_context(|| format!("解析配置文件 {} 失败", path.display()))
}

/// Pairs each provider with its API key read through `env`.
pub fn resolve_api_keys(
    cfg: &Config,
    env: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Vec<(ProviderEntry, String)>> {
    cfg.providers
        .iter()
        .map(|p| match env(&p.api_key_env).filter(|k| !k.is_empty()) {
            Some(key) => Ok((p.clone(), key)),
            None => bail!(
                "提供商 {} 缺少 API 密钥：环境变量 {} 未设置",
                p.key,
                p.api_key_env
            ),
        })
        .collect()
}

pub fn startup_warnings(cfg: &Config) -> Vec<String> {
    let mut warnings = Vec::new();
    if cfg.listen.ip().is_loopback() {
        return warnings;
    }
    if cfg.access_password.is_none() {
        warnings.push(format!(
            "监听 {} 不是环回地址，且未设置访问密码：任何能连到此端口的人都可以使用",
            cfg.listen
        ));
    } else if cfg.trusted_proxies.is_empty() {
        warnings.push(format!(
            "监听 {} 不是环回地址，且未配置 trusted_proxies：访问密码可能以明文 HTTP 传输，建议放在 HTTPS 反向代理之后",
            cfg.listen
        ));
    }
    if cfg.listen.ip().is_unspecified() && cfg.allowed_hosts.is_empty() {
        warnings.push(format!(
            "监听 {} 是通配地址，且未配置 allowed_hosts：只接受以 localhost / 127.0.0.1 / ::1 访问的请求，其他主机名（反向代理、Tailscale 等）会被拒绝（421）",
            cfg.listen
        ));
    }
    warnings
}
