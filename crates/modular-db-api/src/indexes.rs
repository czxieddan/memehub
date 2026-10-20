use crate::{engine::literal, model::ApiError};
use anyhow::anyhow;
use rnmdb_cli::{CommandOutput, LocalSession};
use rnmdb_types::SqlValue;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub pointer: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Definition {
    pub name: String,
    pub pointer: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexQuery {
    pub value: Value,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}
pub enum Work {
    List,
    Declare(String, Declaration),
    Remove(String),
    Query(String, IndexQuery),
}

pub fn initialize(session: &mut LocalSession) -> anyhow::Result<()> {
    if session
        .execute("SELECT id FROM api_scalar_indexes LIMIT 1")
        .is_err()
    {
        session.execute("CREATE TABLE api_scalar_indexes (id TEXT, namespace TEXT, index_name TEXT, pointer TEXT)")?;
        session.execute("CREATE UNIQUE INDEX api_scalar_indexes_id ON api_scalar_indexes (id)")?;
    }
    if session
        .execute("SELECT id FROM api_scalar_values LIMIT 1")
        .is_err()
    {
        session.execute("CREATE TABLE api_scalar_values (id TEXT, namespace TEXT, index_name TEXT, scalar_value TEXT, record_key TEXT)")?;
        session.execute("CREATE UNIQUE INDEX api_scalar_values_id ON api_scalar_values (id)")?;
        session.execute("CREATE INDEX api_scalar_values_lookup ON api_scalar_values (namespace, index_name, scalar_value, record_key)")?;
    }
    session.checkpoint()?;
    Ok(())
}

pub fn definitions(
    session: &mut LocalSession,
    namespace: &str,
) -> Result<Vec<Definition>, ApiError> {
    let sql = format!(
        "SELECT index_name, pointer FROM api_scalar_indexes WHERE namespace = {} ORDER BY index_name",
        literal(namespace)
    );
    rows(session, &sql)?.into_iter().map(definition).collect()
}
fn definition(row: Vec<SqlValue>) -> Result<Definition, ApiError> {
    match row.as_slice() {
        [SqlValue::Text(name), SqlValue::Text(pointer)] => Ok(Definition {
            name: name.clone(),
            pointer: pointer.clone(),
        }),
        _ => Err(anyhow!("invalid scalar index definition").into()),
    }
}
pub fn change(session: &mut LocalSession, namespace: &str, work: Work) -> Result<Value, ApiError> {
    match work {
        Work::Declare(name, declaration) => declare(session, namespace, &name, declaration),
        Work::Remove(name) => {
            remove(session, namespace, &name)?;
            Ok(json!({"deleted": true}))
        }
        _ => Err(anyhow!("invalid scalar index change").into()),
    }
}
fn declare(
    session: &mut LocalSession,
    namespace: &str,
    name: &str,
    declaration: Declaration,
) -> Result<Value, ApiError> {
    validate_pointer(&declaration.pointer)?;
    let current = definitions(session, namespace)?;
    if current.len() >= 32 && !current.iter().any(|item| item.name == name) {
        return Err(ApiError::bad("namespace allows at most 32 scalar indexes"));
    }
    let sql = format!(
        "SELECT record_key, body FROM api_records WHERE namespace = {} AND body <> '' LIMIT 100001",
        literal(namespace)
    );
    let records = rows(session, &sql)?;
    if records.len() > 100000 {
        return Err(ApiError::bad("index backfill exceeds 100000 records"));
    }
    remove(session, namespace, name)?;
    let definition = Definition {
        name: name.into(),
        pointer: declaration.pointer,
    };
    let sql = format!(
        "INSERT INTO api_scalar_indexes (id, namespace, index_name, pointer) VALUES ({}, {}, {}, {})",
        literal(&format!("{namespace}/{name}")),
        literal(namespace),
        literal(name),
        literal(&definition.pointer)
    );
    session.execute(&sql).map_err(anyhow::Error::from)?;
    for row in records {
        backfill(session, namespace, &definition, &row)?;
    }
    Ok(json!(definition))
}
fn backfill(
    session: &mut LocalSession,
    namespace: &str,
    definition: &Definition,
    row: &[SqlValue],
) -> Result<(), ApiError> {
    let [SqlValue::Text(key), SqlValue::Text(body)] = row else {
        return Err(anyhow!("invalid indexed record").into());
    };
    let value: Value = serde_json::from_str(body).map_err(anyhow::Error::from)?;
    insert_value(session, namespace, key, definition, &value)
}
fn validate_pointer(pointer: &str) -> Result<(), ApiError> {
    let valid_escapes = pointer
        .split('~')
        .skip(1)
        .all(|part| part.starts_with('0') || part.starts_with('1'));
    if !pointer.starts_with('/')
        || pointer.len() > 256
        || pointer.chars().any(char::is_control)
        || !valid_escapes
    {
        return Err(ApiError::bad(
            "index pointer must be a valid nonempty JSON pointer within 256 bytes",
        ));
    }
    Ok(())
}
fn remove(session: &mut LocalSession, namespace: &str, name: &str) -> Result<(), ApiError> {
    let condition = format!(
        "namespace = {} AND index_name = {}",
        literal(namespace),
        literal(name)
    );
    session
        .execute(&format!("DELETE FROM api_scalar_values WHERE {condition}"))
        .map_err(anyhow::Error::from)?;
    session
        .execute(&format!("DELETE FROM api_scalar_indexes WHERE {condition}"))
        .map_err(anyhow::Error::from)?;
    Ok(())
}
pub fn replace_record(
    session: &mut LocalSession,
    namespace: &str,
    key: &str,
    body: &str,
) -> Result<(), ApiError> {
    let sql = format!(
        "DELETE FROM api_scalar_values WHERE namespace = {} AND record_key = {}",
        literal(namespace),
        literal(key)
    );
    session.execute(&sql).map_err(anyhow::Error::from)?;
    if body.is_empty() {
        return Ok(());
    }
    let value = serde_json::from_str(body).map_err(anyhow::Error::from)?;
    for definition in definitions(session, namespace)? {
        insert_value(session, namespace, key, &definition, &value)?;
    }
    Ok(())
}
fn insert_value(
    session: &mut LocalSession,
    namespace: &str,
    key: &str,
    definition: &Definition,
    value: &Value,
) -> Result<(), ApiError> {
    let Some(scalar) = value.pointer(&definition.pointer).filter(|value| {
        value.is_null() || value.is_boolean() || value.is_number() || value.is_string()
    }) else {
        return Ok(());
    };
    let sql = format!(
        "INSERT INTO api_scalar_values (id, namespace, index_name, scalar_value, record_key) VALUES ({}, {}, {}, {}, {})",
        literal(&format!("{namespace}/{}/{key}", definition.name)),
        literal(namespace),
        literal(&definition.name),
        literal(&scalar.to_string()),
        literal(key)
    );
    session.execute(&sql).map_err(anyhow::Error::from)?;
    Ok(())
}
pub fn query_keys(
    session: &mut LocalSession,
    namespace: &str,
    name: &str,
    query: &IndexQuery,
) -> Result<Vec<String>, ApiError> {
    if query.value.is_array() || query.value.is_object() {
        return Err(ApiError::bad("index query requires a JSON scalar"));
    }
    if !definitions(session, namespace)?
        .iter()
        .any(|item| item.name == name)
    {
        return Err(ApiError(
            axum::http::StatusCode::NOT_FOUND,
            "index not found".into(),
        ));
    }
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let sql = format!(
        "SELECT record_key FROM api_scalar_values WHERE namespace = {} AND index_name = {} AND scalar_value = {} AND record_key > {} ORDER BY record_key LIMIT {}",
        literal(namespace),
        literal(name),
        literal(&query.value.to_string()),
        literal(query.cursor.as_deref().unwrap_or("")),
        limit + 1
    );
    rows(session, &sql)?.into_iter().map(record_key).collect()
}
fn record_key(row: Vec<SqlValue>) -> Result<String, ApiError> {
    match row.as_slice() {
        [SqlValue::Text(key)] => Ok(key.clone()),
        _ => Err(anyhow!("invalid scalar index record key").into()),
    }
}
fn rows(session: &mut LocalSession, sql: &str) -> Result<Vec<Vec<SqlValue>>, ApiError> {
    let CommandOutput::Rows(batch) = session.execute(sql).map_err(anyhow::Error::from)? else {
        return Err(anyhow!("expected scalar index rows").into());
    };
    Ok(batch
        .rows()
        .iter()
        .map(|row| row.values().to_vec())
        .collect())
}
