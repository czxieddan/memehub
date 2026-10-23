use crate::{
    config::{Config, now},
    error::Error,
    model::valid_name,
};
use anyhow::ensure;
use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

pub const MAX_CLAIMS: usize = 8;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Acceptance {
    pub version: String,
    pub claims: Vec<String>,
    pub accepted_at: u64,
}
pub fn validate(version: &str, claims: &[String]) -> anyhow::Result<()> {
    if version.is_empty() {
        ensure!(claims.is_empty(), "upload policy claims require a version");
        return Ok(());
    }
    ensure!(
        valid_name(version),
        "upload policy version must be a safe 1-64 character ID"
    );
    ensure!(!claims.is_empty(), "upload policy version requires claims");
    validate_claim_ids(claims)
}
fn validate_claim_ids(claims: &[String]) -> anyhow::Result<()> {
    ensure!(
        claims.len() <= MAX_CLAIMS,
        "upload policy supports at most eight claims"
    );
    ensure!(
        claims.iter().all(|claim| valid_name(claim)),
        "invalid upload policy claim ID"
    );
    let unique: HashSet<_> = claims.iter().collect();
    ensure!(
        unique.len() == claims.len(),
        "duplicate upload policy claim"
    );
    Ok(())
}
pub async fn show(State(app): State<crate::App>) -> Json<Value> {
    Json(json!({"version":app.config.upload_policy_version,
        "required_claims":app.config.upload_policy_claims}))
}
pub fn accept(
    config: &Config,
    version: Option<&str>,
    claims: Option<&str>,
) -> Result<Option<Acceptance>, Error> {
    if version.unwrap_or_default() != config.upload_policy_version {
        return Err(Error::bad("Upload policy version is missing or stale"));
    }
    let claims: Vec<String> = claims
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| Error::bad("Policy claims must be a JSON array of strings"))?
        .unwrap_or_default();
    check_claims(&config.upload_policy_claims, &claims)?;
    if config.upload_policy_version.is_empty() {
        return Ok(None);
    }
    Ok(Some(Acceptance {
        version: config.upload_policy_version.clone(),
        claims: config.upload_policy_claims.clone(),
        accepted_at: now(),
    }))
}
fn check_claims(required: &[String], actual: &[String]) -> Result<(), Error> {
    if actual.len() != required.len() {
        return Err(Error::bad(
            "Policy claims must exactly match the required claims",
        ));
    }
    let unique: HashSet<_> = actual.iter().collect();
    if unique.len() != actual.len() {
        return Err(Error::bad("Duplicate policy claim"));
    }
    if !required.iter().all(|claim| unique.contains(claim)) {
        return Err(Error::bad("Unknown or missing policy claim"));
    }
    Ok(())
}
pub fn verify_receipt(config: &Config, receipt: Option<&Acceptance>) -> Result<(), Error> {
    if config.upload_policy_version.is_empty() {
        return Ok(());
    }
    let receipt = receipt.ok_or_else(|| Error::bad("Upload policy acceptance is required"))?;
    if receipt.version != config.upload_policy_version {
        return Err(Error::bad("Upload policy acceptance is stale"));
    }
    check_claims(&config.upload_policy_claims, &receipt.claims)
}
