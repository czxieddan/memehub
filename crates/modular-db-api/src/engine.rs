use crate::{
    config::Config,
    indexes::{self, IndexQuery, Work as IndexWork},
    model::{ApiError, ListQuery, Operation, Record},
};
use anyhow::{Context, anyhow};
use rnmdb_cli::{CommandOutput, LocalSession};
use rnmdb_storage::PageCryptoKey;
use rnmdb_types::SqlValue;
use serde_json::{Value, json};

pub struct Engine {
    session: LocalSession,
    healthy: bool,
}
struct Stored {
    key: String,
    version: u64,
    body: String,
}

pub fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
impl Engine {
    pub fn open(config: &Config) -> anyhow::Result<Self> {
        let bytes = hex::decode(std::fs::read_to_string(&config.page_key_file)?.trim())?;
        let key: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow!("page key requires 32 bytes"))?;
        std::fs::create_dir_all(config.path.parent().context("database parent")?)?;
        Self::new(LocalSession::single_file_with_key(
            &config.path,
            PageCryptoKey::from_bytes(key),
        )?)
    }
    pub fn new(mut session: LocalSession) -> anyhow::Result<Self> {
        if session
            .execute("SELECT id FROM api_records LIMIT 1")
            .is_err()
        {
            session.execute("CREATE TABLE api_records (id TEXT, namespace TEXT, record_key TEXT, body TEXT, version INT64)")?;
            session.execute("CREATE UNIQUE INDEX api_records_id ON api_records (id)")?;
            session.execute("CREATE INDEX api_records_namespace ON api_records (namespace)")?;
            session.checkpoint()?;
        }
        indexes::initialize(&mut session)?;
        Ok(Self {
            session,
            healthy: true,
        })
    }
    pub fn get(&mut self, namespace: &str, key: &str) -> Result<Option<Record>, ApiError> {
        self.ensure_healthy()?;
        self.stored(namespace, key)?
            .filter(|r| !r.body.is_empty())
            .map(decode)
            .transpose()
    }
    pub fn list(&mut self, namespace: &str, query: &ListQuery) -> Result<Value, ApiError> {
        self.ensure_healthy()?;
        let limit = query.limit.unwrap_or(100).clamp(1, 500);
        let cursor = literal(query.cursor.as_deref().unwrap_or(""));
        let prefix = literal(query.prefix.as_deref().unwrap_or(""));
        let sql = format!(
            "SELECT record_key, version, body FROM api_records WHERE namespace = {} AND record_key > {cursor} AND record_key >= {prefix} AND body <> '' ORDER BY record_key LIMIT {}",
            literal(namespace),
            limit + 1
        );
        let mut items = self
            .rows(&sql)?
            .into_iter()
            .filter(|r| r.key.starts_with(query.prefix.as_deref().unwrap_or("")))
            .map(decode)
            .collect::<Result<Vec<_>, _>>()?;
        let next = if items.len() > limit {
            items.truncate(limit);
            items.last().map(|r| r.key.clone())
        } else {
            None
        };
        Ok(json!({"items": items, "next_cursor": next}))
    }
    pub fn transaction(&mut self, operations: &[Operation]) -> Result<Value, ApiError> {
        self.ensure_healthy()?;
        self.session.execute("BEGIN").map_err(anyhow::Error::from)?;
        let result = self.apply_all(operations);
        self.finish(result)
    }
    fn finish(&mut self, result: Result<Value, ApiError>) -> Result<Value, ApiError> {
        match result {
            Ok(value) => self.commit(value),
            Err(error) => {
                self.rollback();
                Err(error)
            }
        }
    }
    fn apply_all(&mut self, operations: &[Operation]) -> Result<Value, ApiError> {
        let mut results = Vec::with_capacity(operations.len());
        for operation in operations {
            results.push(self.apply(operation)?);
        }
        Ok(json!({"results": results}))
    }
    fn commit(&mut self, value: Value) -> Result<Value, ApiError> {
        if let Err(error) = self.session.execute("COMMIT") {
            self.healthy = false;
            return Err(anyhow::Error::from(error).into());
        }
        Ok(value)
    }
    fn rollback(&mut self) {
        if self.session.execute("ROLLBACK").is_err() {
            self.healthy = false;
        }
    }
    fn apply(&mut self, operation: &Operation) -> Result<Value, ApiError> {
        let (namespace, key) = operation.location();
        let previous = self.stored(namespace, key)?;
        let visible = previous
            .as_ref()
            .filter(|r| !r.body.is_empty())
            .map_or(0, |r| r.version);
        let next = previous
            .as_ref()
            .map_or(0, |r| r.version)
            .checked_add(1)
            .ok_or_else(|| ApiError::bad("record version exhausted"))?;
        match operation {
            Operation::Put {
                value,
                expected_version,
                ..
            } => {
                check(*expected_version, visible)?;
                self.save(
                    namespace,
                    key,
                    next,
                    &serde_json::to_string(value).map_err(anyhow::Error::from)?,
                )?;
                Ok(json!({"key": key, "version": next, "value": value}))
            }
            Operation::Delete {
                expected_version, ..
            } => {
                check(*expected_version, visible)?;
                self.save(namespace, key, next, "")?;
                Ok(json!({"key": key, "deleted": true}))
            }
            Operation::Check {
                expected_version, ..
            } => {
                check(Some(*expected_version), visible)?;
                Ok(json!({"key": key, "version": visible}))
            }
        }
    }
    fn save(
        &mut self,
        namespace: &str,
        key: &str,
        version: u64,
        body: &str,
    ) -> Result<(), ApiError> {
        let id = literal(&format!("{namespace}/{key}"));
        self.session
            .execute(&format!("DELETE FROM api_records WHERE id = {id}"))
            .map_err(anyhow::Error::from)?;
        let sql = format!(
            "INSERT INTO api_records (id, namespace, record_key, body, version) VALUES ({id}, {}, {}, {}, {version})",
            literal(namespace),
            literal(key),
            literal(body)
        );
        self.session.execute(&sql).map_err(anyhow::Error::from)?;
        indexes::replace_record(&mut self.session, namespace, key, body)?;
        Ok(())
    }
    fn stored(&mut self, namespace: &str, key: &str) -> Result<Option<Stored>, ApiError> {
        let sql = format!(
            "SELECT record_key, version, body FROM api_records WHERE id = {}",
            literal(&format!("{namespace}/{key}"))
        );
        Ok(self.rows(&sql)?.into_iter().next())
    }
    fn rows(&mut self, sql: &str) -> Result<Vec<Stored>, ApiError> {
        let output = self.session.execute(sql).map_err(anyhow::Error::from)?;
        let CommandOutput::Rows(batch) = output else {
            return Err(anyhow!("expected rows").into());
        };
        batch
            .rows()
            .iter()
            .map(|row| stored_row(row.values()))
            .collect()
    }
    fn ensure_healthy(&self) -> Result<(), ApiError> {
        if !self.healthy {
            return Err(anyhow!("indeterminate commit; restart and verify storage").into());
        }
        Ok(())
    }
    pub fn index(&mut self, namespace: &str, work: IndexWork) -> Result<Value, ApiError> {
        self.ensure_healthy()?;
        match work {
            IndexWork::List => {
                Ok(json!({"items": indexes::definitions(&mut self.session, namespace)?}))
            }
            IndexWork::Query(name, query) => self.query_index(namespace, &name, &query),
            change => {
                self.session.execute("BEGIN").map_err(anyhow::Error::from)?;
                let result = indexes::change(&mut self.session, namespace, change);
                self.finish(result)
            }
        }
    }
    fn query_index(
        &mut self,
        namespace: &str,
        name: &str,
        query: &IndexQuery,
    ) -> Result<Value, ApiError> {
        let mut keys = indexes::query_keys(&mut self.session, namespace, name, query)?;
        let limit = query.limit.unwrap_or(100).clamp(1, 500);
        let next = if keys.len() > limit {
            keys.truncate(limit);
            keys.last().cloned()
        } else {
            None
        };
        let mut items = Vec::with_capacity(keys.len());
        for key in keys {
            items.push(
                self.get(namespace, &key)?
                    .context("indexed record missing")?,
            );
        }
        Ok(json!({"items": items, "next_cursor": next}))
    }
    pub fn backup(&mut self, config: &Config) -> Result<Value, ApiError> {
        self.ensure_healthy()?;
        std::fs::create_dir_all(&config.backup_dir).map_err(anyhow::Error::from)?;
        let name = format!("{}.rnmdb", uuid::Uuid::new_v4());
        rnmdb_storage::backup_single_file(&config.path, config.backup_dir.join(&name))
            .map_err(anyhow::Error::from)?;
        Ok(json!({"backup": name}))
    }
}
fn stored_row(values: &[SqlValue]) -> Result<Stored, ApiError> {
    match values {
        [
            SqlValue::Text(key),
            SqlValue::Int64(version),
            SqlValue::Text(body),
        ] => Ok(Stored {
            key: key.clone(),
            version: *version as u64,
            body: body.clone(),
        }),
        _ => Err(anyhow!("invalid stored record").into()),
    }
}
fn decode(row: Stored) -> Result<Record, ApiError> {
    Ok(Record {
        key: row.key,
        version: row.version,
        value: serde_json::from_str(&row.body).map_err(anyhow::Error::from)?,
    })
}
fn check(expected: Option<u64>, actual: u64) -> Result<(), ApiError> {
    if expected.is_some_and(|v| v != actual) {
        return Err(ApiError::conflict());
    }
    Ok(())
}
