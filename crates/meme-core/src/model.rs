use crate::error::Error;
use anyhow::ensure;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meme {
    pub manifest_hash: String,
    pub uploader_id: u64,
    pub name: String,
    pub display_name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub hash: String,
    pub uploader_id: u64,
    pub name: String,
    pub display_name: String,
    pub path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub schema_version: u32,
    pub source_commit: String,
    pub hashes: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Submission {
    pub id: String,
    pub uploader_id: u64,
    pub name: String,
    pub display_name: String,
    pub filename: String,
    pub status: String,
    pub created_at: u64,
    pub attempts: u32,
    pub next_attempt_at: u64,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
    pub error: Option<String>,
}
pub fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
pub fn valid_filename(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && !value.starts_with('.')
}
pub fn names(name: &str, display: &str) -> Result<(), Error> {
    if !valid_name(name) {
        return Err(Error::bad(
            "Name 需为 1–64 位英文字母、数字、下划线或连字符",
        ));
    }
    if display.trim().is_empty()
        || display.chars().count() > 120
        || display.chars().any(char::is_control)
    {
        return Err(Error::bad("外显名称需为 1–120 字且不含控制字符"));
    }
    Ok(())
}
impl Index {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(self.schema_version == 1, "unsupported index schema");
        ensure!(
            self.source_commit.len() == 40
                && self.source_commit.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid source commit"
        );
        ensure!(
            self.hashes.len() <= 4000,
            "initial release supports up to 4000 memes"
        );
        let unique: std::collections::HashSet<_> = self.hashes.iter().collect();
        ensure!(unique.len() == self.hashes.len(), "duplicate index hashes");
        ensure!(
            self.hashes.iter().all(|h| valid_hash(h)),
            "invalid index hash"
        );
        Ok(())
    }
}
impl Manifest {
    pub fn validate(&self, hash: &str) -> anyhow::Result<()> {
        ensure!(
            self.schema_version == 1 && self.hash == hash,
            "invalid manifest identity"
        );
        ensure!(
            self.uploader_id > 0 && valid_name(&self.name),
            "invalid manifest owner or Name"
        );
        names(&self.name, &self.display_name).map_err(|e| anyhow::anyhow!(e.1))?;
        let prefix = format!("/data/{}/{}/", self.uploader_id, self.name);
        let filename = self
            .path
            .strip_prefix(&prefix)
            .ok_or_else(|| anyhow::anyhow!("manifest path mismatch"))?;
        ensure!(valid_filename(filename), "invalid manifest filename");
        ensure!(mime(filename).is_some(), "unsupported image extension");
        Ok(())
    }
    pub fn meme(&self) -> Meme {
        Meme {
            manifest_hash: self.hash.clone(),
            uploader_id: self.uploader_id,
            name: self.name.clone(),
            display_name: self.display_name.clone(),
        }
    }
}
pub fn mime(filename: &str) -> Option<&'static str> {
    match filename.rsplit('.').next()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}
