use crate::{App, config::now, model::valid_hash};
use anyhow::ensure;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const MAX_PUBLISHED: usize = 500;
pub const VIEW_COOLDOWN_SECONDS: u64 = 30;

// Tuple storage keeps all 500 entries below the database's 64 KiB record limit.
// Fields are first_seen, aggregate views (saturating), and last counted view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metrics(pub u64, pub u32, pub u64);
impl Metrics {
    pub fn record_view(&mut self, timestamp: u64) -> bool {
        if self.1 == u32::MAX {
            return false;
        }
        if self.1 > 0 && timestamp.saturating_sub(self.2) < VIEW_COOLDOWN_SECONDS {
            return false;
        }
        self.1 += 1;
        self.2 = timestamp;
        true
    }
}
pub type Catalog = BTreeMap<String, Metrics>;
pub struct Snapshot {
    pub version: u64,
    pub entries: Catalog,
}
impl Snapshot {
    pub fn operation(&self, entries: &Catalog) -> Value {
        json!({"op":"put","namespace":"corestate","key":"gallery_catalog",
            "expected_version":self.version,"value":entries})
    }
}
pub async fn load(app: &App) -> anyhow::Result<Snapshot> {
    let record = app.db.get("corestate", "gallery_catalog").await?;
    let version = record.as_ref().map_or(0, |r| r.version);
    let entries: Catalog = record
        .map(|r| serde_json::from_value(r.value))
        .transpose()?
        .unwrap_or_default();
    ensure!(
        entries.len() <= MAX_PUBLISHED,
        "catalog exceeds publication limit"
    );
    ensure!(
        entries.keys().all(|hash| valid_hash(hash)),
        "invalid catalog hash"
    );
    Ok(Snapshot { version, entries })
}
pub async fn prepare(app: &App, hashes: &[String]) -> anyhow::Result<(Value, bool)> {
    ensure!(hashes.len() <= MAX_PUBLISHED, "too many published entries");
    let snapshot = load(app).await?;
    let timestamp = now();
    let entries: Catalog = hashes
        .iter()
        .map(|hash| {
            let metrics = snapshot
                .entries
                .get(hash)
                .cloned()
                .unwrap_or(Metrics(timestamp, 0, 0));
            (hash.clone(), metrics)
        })
        .collect();
    ensure!(
        serde_json::to_vec(&entries)?.len() <= 65536,
        "catalog too large"
    );
    let changed = snapshot.entries != entries;
    let operation = if changed || snapshot.version == 0 {
        snapshot.operation(&entries)
    } else {
        json!({"op":"check","namespace":"corestate","key":"gallery_catalog",
            "expected_version":snapshot.version})
    };
    Ok((operation, changed || snapshot.version == 0))
}
