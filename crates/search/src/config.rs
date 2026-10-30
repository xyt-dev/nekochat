//! Configuration shared by the standalone engine and its local process owner.
use serde::Deserialize;
use std::{net::SocketAddr, path::{Path, PathBuf}};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    #[serde(default = "default_data")]
    pub data_dir: PathBuf,
    pub access_token_env: String,
    pub admin_token_env: String,
}

fn default_listen() -> SocketAddr { SocketAddr::from(([127, 0, 0, 1], 10011)) }
fn default_data() -> PathBuf { PathBuf::from("./search-data") }

fn secret(name: &str) -> Result<String, &'static str> {
    if name.is_empty() || name.len() > 128 || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
        return Err("invalid inbound token environment variable name");
    }
    std::env::var(name).ok().filter(|token| !token.trim().is_empty())
        .ok_or("required inbound token environment variable is missing or empty")
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, &'static str> {
        let metadata = std::fs::metadata(path).map_err(|_| "cannot read search configuration")?;
        if !metadata.is_file() || metadata.len() > 65536 { return Err("invalid search configuration file"); }
        let text = std::fs::read_to_string(path).map_err(|_| "cannot read search configuration")?;
        toml::from_str(&text).map_err(|_| "invalid search TOML configuration")
    }

    pub fn tokens(&self) -> Result<(String, String), &'static str> {
        Ok((secret(&self.access_token_env)?, secret(&self.admin_token_env)?))
    }
}
