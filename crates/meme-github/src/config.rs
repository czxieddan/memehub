use anyhow::{Context, ensure};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Deserialize)]
pub struct Config {
    pub listen: String,
    pub api_base: String,
    pub oauth_base: String,
    pub public_base: String,
    pub database_url: String,
    pub database_token_file: PathBuf,
    pub service_token_file: PathBuf,
    pub webhook_secret_file: PathBuf,
    pub private_key_file: PathBuf,
    pub client_secret_file: PathBuf,
    pub app_id: String,
    pub client_id: String,
    pub installation_id: u64,
    pub allowed_repositories: Vec<String>,
    pub allowed_path_prefixes: Vec<String>,
    pub session_ttl_seconds: u64,
}
impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct File {
            github: Config,
        }
        let mut config = toml::from_str::<File>(&std::fs::read_to_string(path)?)?.github;
        let root = path.parent().context("configuration parent")?;
        config.database_token_file = root.join(&config.database_token_file);
        config.service_token_file = root.join(&config.service_token_file);
        config.webhook_secret_file = root.join(&config.webhook_secret_file);
        config.private_key_file = root.join(&config.private_key_file);
        config.client_secret_file = root.join(&config.client_secret_file);
        ensure!(
            (300..=2592000).contains(&config.session_ttl_seconds),
            "session TTL outside bounds"
        );
        Ok(config)
    }
    pub fn repository(&self, repo: &str) -> anyhow::Result<()> {
        ensure!(
            repo.split('/').count() == 2,
            "repository must be owner/name"
        );
        ensure!(
            repo.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b)),
            "invalid repository"
        );
        ensure!(
            self.allowed_repositories.iter().any(|r| r == repo),
            "repository not allowed"
        );
        Ok(())
    }
}
pub fn secret(path: &Path) -> anyhow::Result<String> {
    let value = std::fs::read_to_string(path)?.trim().to_string();
    ensure!(
        value.len() >= 16,
        "secret missing or too short: {}",
        path.display()
    );
    Ok(value)
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
