use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);
impl ApiError {
    pub fn bad(message: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
    pub fn conflict() -> Self {
        Self(StatusCode::CONFLICT, "record version conflict".into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        tracing::error!(error = %error, "database operation failed");
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            "database operation unavailable".into(),
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub key: String,
    pub version: u64,
    pub value: Value,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    pub value: Value,
    pub expected_version: Option<u64>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Put {
        namespace: String,
        key: String,
        value: Value,
        expected_version: Option<u64>,
    },
    Delete {
        namespace: String,
        key: String,
        expected_version: Option<u64>,
    },
    Check {
        namespace: String,
        key: String,
        expected_version: u64,
    },
}
impl Operation {
    pub fn location(&self) -> (&str, &str) {
        match self {
            Self::Put { namespace, key, .. } => (namespace, key),
            Self::Delete { namespace, key, .. } => (namespace, key),
            Self::Check { namespace, key, .. } => (namespace, key),
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub prefix: Option<String>,
}

pub fn validate_location(namespace: &str, key: &str) -> Result<(), ApiError> {
    if !valid_namespace(namespace) {
        return Err(ApiError::bad("invalid namespace"));
    }
    if key.is_empty() || key.len() > 200 {
        return Err(ApiError::bad("invalid key length"));
    }
    if key.chars().any(char::is_control) {
        return Err(ApiError::bad("control character in key"));
    }
    Ok(())
}
fn valid_namespace(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
