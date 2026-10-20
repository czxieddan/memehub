use crate::model::ApiError;
use anyhow::{Context, ensure};
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;

#[derive(Clone, Deserialize)]
pub struct Config {
    pub listen: String,
    pub path: PathBuf,
    pub page_key_file: PathBuf,
    pub principals_file: PathBuf,
    pub backup_dir: PathBuf,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub token: String,
    pub namespaces: Vec<String>,
    #[serde(default)]
    pub admin: bool,
}
impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct File {
            database: Config,
        }
        let mut config: Config = toml::from_str::<File>(&std::fs::read_to_string(path)?)?.database;
        let root = path.parent().context("configuration has no parent")?;
        config.path = root.join(&config.path);
        config.page_key_file = root.join(&config.page_key_file);
        config.principals_file = root.join(&config.principals_file);
        config.backup_dir = root.join(&config.backup_dir);
        Ok(config)
    }
    pub fn principals(&self) -> anyhow::Result<Vec<Principal>> {
        let values: Vec<Principal> =
            serde_json::from_slice(&std::fs::read(&self.principals_file)?)?;
        ensure!(!values.is_empty(), "database principals are required");
        for item in &values {
            ensure!(item.token.len() >= 32, "service tokens require 32 bytes");
        }
        Ok(values)
    }
}
pub fn authorize(principals: &[Principal], headers: &HeaderMap) -> Result<Principal, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    principals
        .iter()
        .find(|p| bool::from(p.token.as_bytes().ct_eq(token.as_bytes())))
        .cloned()
        .ok_or(ApiError(
            StatusCode::UNAUTHORIZED,
            "invalid service credential".into(),
        ))
}
pub fn allow(principal: &Principal, namespace: &str) -> Result<(), ApiError> {
    if principal.admin || principal.namespaces.iter().any(|n| n == namespace) {
        return Ok(());
    }
    Err(ApiError(
        StatusCode::FORBIDDEN,
        "namespace access denied".into(),
    ))
}
