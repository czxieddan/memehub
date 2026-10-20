use crate::{App, config::now, error::Error};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub login: String,
    pub email: String,
    pub contribution_email: String,
    pub last_login: u64,
    #[serde(default)]
    pub revoked: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub user_id: u64,
    pub csrf_token: String,
    pub expires_at: u64,
}
#[derive(Serialize)]
pub struct Identity {
    #[serde(flatten)]
    pub user: User,
    pub csrf_token: String,
}

pub fn random() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == name).then(|| value.to_string())
        })
}
pub fn cookie_header(app: &App, name: &str, value: &str, seconds: u64) -> String {
    let secure = if app.config.public_base.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={seconds}{secure}")
}
pub fn credential(headers: &HeaderMap, expected: &str) -> Result<(), Error> {
    let actual = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !bool::from(expected.as_bytes().ct_eq(actual.as_bytes())) {
        return Err(Error::unauthorized());
    }
    Ok(())
}
pub async fn identity(app: &App, headers: &HeaderMap) -> Result<Identity, Error> {
    let token = cookie(headers, "memehub_session").ok_or_else(Error::unauthorized)?;
    if token.len() != 64 {
        return Err(Error::unauthorized());
    }
    let session: Session = app
        .db
        .value("ghsessions", &token)
        .await?
        .ok_or_else(Error::unauthorized)?;
    if session.expires_at <= now() {
        return Err(Error::unauthorized());
    }
    let user: User = app
        .db
        .value("ghusers", &session.user_id.to_string())
        .await?
        .ok_or_else(Error::unauthorized)?;
    if user.revoked {
        return Err(Error::unauthorized());
    }
    Ok(Identity {
        user,
        csrf_token: session.csrf_token,
    })
}
pub async fn introspect(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<Identity>, Error> {
    credential(&headers, &app.service_token)?;
    Ok(Json(identity(&app, &headers).await?))
}
pub fn csrf(headers: &HeaderMap, expected: &str) -> Result<(), Error> {
    let actual = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !bool::from(expected.as_bytes().ct_eq(actual.as_bytes())) {
        return Err(Error::bad("CSRF 校验失败，请刷新后重试"));
    }
    Ok(())
}
