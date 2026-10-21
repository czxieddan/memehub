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
    #[serde(default)]
    pub authorization_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub user_id: u64,
    pub csrf_token: String,
    pub expires_at: u64,
    #[serde(default)]
    pub authorization_id: String,
}
#[derive(Serialize)]
pub struct Identity {
    #[serde(flatten, serialize_with = "public_user")]
    pub user: User,
    pub csrf_token: String,
}
fn public_user<S: serde::Serializer>(user: &User, serializer: S) -> Result<S::Ok, S::Error> {
    serde_json::json!({
        "id":user.id,"login":user.login,"email":user.email,
        "contribution_email":user.contribution_email,
        "last_login":user.last_login,"revoked":user.revoked,
    })
    .serialize(serializer)
}
impl User {
    pub fn authorize(&mut self, previous: Option<&Self>) {
        self.authorization_id = previous
            .filter(|user| !user.revoked && user.authorization_id.len() == 64)
            .map(|user| user.authorization_id.clone())
            .unwrap_or_else(random);
    }
}
impl Session {
    pub fn valid_for(&self, user: &User) -> bool {
        self.expires_at > now()
            && !user.revoked
            && self.user_id == user.id
            && self.authorization_id.len() == 64
            && self.authorization_id == user.authorization_id
    }
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
    let user: User = app
        .db
        .value("ghusers", &session.user_id.to_string())
        .await?
        .ok_or_else(Error::unauthorized)?;
    if !session.valid_for(&user) {
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
        return Err(Error::bad(
            "CSRF validation failed. Please refresh and try again.",
        ));
    }
    Ok(())
}
