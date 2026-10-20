use anyhow::{Context, ensure};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Deserialize)]
pub struct Config {
    pub listen: String,
    pub public_base: String,
    pub database_url: String,
    pub database_token_file: PathBuf,
    pub github_url: String,
    pub github_token_file: PathBuf,
    pub admin_token_file: PathBuf,
    pub staging_dir: PathBuf,
    pub repository: String,
    pub branch: String,
    pub sync_seconds: u64,
    pub max_upload_bytes: usize,
    pub max_pixels: u64,
    pub cache_max_bytes: usize,
    pub cache_max_entries: usize,
    pub cache_ttl_seconds: u64,
}
impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct File {
            core: Config,
        }
        let mut config = toml::from_str::<File>(&std::fs::read_to_string(path)?)?.core;
        let root = path.parent().context("configuration parent")?;
        config.database_token_file = root.join(&config.database_token_file);
        config.github_token_file = root.join(&config.github_token_file);
        config.admin_token_file = root.join(&config.admin_token_file);
        config.staging_dir = root.join(&config.staging_dir);
        ensure!(
            (5..=3600).contains(&config.sync_seconds),
            "sync interval outside 5..3600 seconds"
        );
        ensure!(
            (1..=5242880).contains(&config.max_upload_bytes),
            "upload limit outside 1..5 MiB"
        );
        ensure!(
            (1..=16777216).contains(&config.max_pixels),
            "image pixel limit outside bounds"
        );
        Ok(config)
    }
}
pub fn secret(path: &Path) -> anyhow::Result<String> {
    let value = std::fs::read_to_string(path)?.trim().to_string();
    ensure!(
        value.len() >= 32,
        "service secret too short: {}",
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
