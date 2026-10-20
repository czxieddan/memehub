use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
#[derive(Debug)]
pub struct Error(pub StatusCode, pub String);
impl Error {
    pub fn bad(text: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, text.into())
    }
    pub fn unauthorized() -> Self {
        Self(StatusCode::UNAUTHORIZED, "请先通过 GitHub 登录".into())
    }
    pub fn missing() -> Self {
        Self(StatusCode::NOT_FOUND, "记录不存在".into())
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        tracing::warn!(error=%error,"core operation unavailable");
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            "服务暂时不可用，请稍后重试".into(),
        )
    }
}
impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        anyhow::Error::from(error.without_url()).into()
    }
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        anyhow::Error::from(error).into()
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        anyhow::Error::from(error).into()
    }
}
