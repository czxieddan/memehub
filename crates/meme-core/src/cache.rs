use anyhow::ensure;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub max_bytes: usize,
    pub max_entries: usize,
    pub ttl_seconds: u64,
}
impl Policy {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.max_bytes <= 2 * 1024 * 1024 * 1024,
            "cache limit exceeds 2 GiB"
        );
        ensure!(
            self.max_entries <= 100000,
            "cache entry limit exceeds 100000"
        );
        ensure!(self.ttl_seconds <= 604800, "cache TTL exceeds 7 days");
        Ok(())
    }
}
struct Entry {
    bytes: Arc<Vec<u8>>,
    inserted: Instant,
    touched: Instant,
    heat: f64,
}
pub struct Cache {
    pub policy: Policy,
    entries: HashMap<String, Entry>,
    used: usize,
}
impl Cache {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            entries: HashMap::new(),
            used: 0,
        }
    }
    pub fn get(&mut self, key: &str) -> Option<Arc<Vec<u8>>> {
        self.expire(Instant::now());
        let entry = self.entries.get_mut(key)?;
        entry.heat = score(entry, Instant::now()) + 1.0;
        entry.touched = Instant::now();
        Some(entry.bytes.clone())
    }
    pub fn insert(&mut self, key: String, bytes: Vec<u8>) {
        self.remove(&key);
        if !self.can_store(bytes.len()) {
            return;
        }
        let now = Instant::now();
        self.used += bytes.len();
        self.entries.insert(
            key,
            Entry {
                bytes: Arc::new(bytes),
                inserted: now,
                touched: now,
                heat: 1.0,
            },
        );
        self.trim(now);
    }
    fn can_store(&self, size: usize) -> bool {
        size <= self.policy.max_bytes && self.policy.max_entries > 0 && self.policy.ttl_seconds > 0
    }
    pub fn reconfigure(&mut self, policy: Policy) {
        self.policy = policy;
        self.trim(Instant::now());
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.used = 0;
    }
    pub fn stats(&mut self) -> serde_json::Value {
        self.expire(Instant::now());
        serde_json::json!({"policy":self.policy,"used_bytes":self.used,"entries":self.entries.len()})
    }
    fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key) {
            self.used -= entry.bytes.len();
        }
    }
    fn expire(&mut self, now: Instant) {
        let ttl = Duration::from_secs(self.policy.ttl_seconds);
        self.entries.retain(|_, entry| {
            let keep = now.duration_since(entry.inserted) < ttl;
            if !keep {
                self.used -= entry.bytes.len();
            }
            keep
        });
    }
    fn trim(&mut self, now: Instant) {
        self.expire(now);
        while self.used > self.policy.max_bytes || self.entries.len() > self.policy.max_entries {
            let key = self
                .entries
                .iter()
                .min_by(|(_, a), (_, b)| score(a, now).total_cmp(&score(b, now)))
                .map(|(key, _)| key.clone());
            let Some(key) = key else {
                break;
            };
            self.remove(&key);
        }
    }
}
fn score(entry: &Entry, now: Instant) -> f64 {
    entry.heat * 0.5_f64.powf(now.duration_since(entry.touched).as_secs_f64() / 300.0)
}
