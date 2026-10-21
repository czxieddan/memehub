use crate::{
    App,
    config::{now, secret},
    error::Error,
    remote::text,
    session::{self, Session, User},
};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, header::SET_COOKIE},
    response::{IntoResponse, Redirect, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

#[derive(Serialize, Deserialize)]
struct Pending {
    verifier: String,
    expires_at: u64,
}
#[derive(Deserialize)]
pub struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

pub async fn start(State(app): State<App>) -> Result<Response, Error> {
    if app.config.client_id.is_empty() {
        return Err(Error::bad("An administrator must configure the GitHub App"));
    }
    let state = session::random();
    let verifier = session::random();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    app.db
        .put(
            "ghoauth",
            &state,
            &Pending {
                verifier,
                expires_at: now() + 600,
            },
            Some(0),
        )
        .await?;
    let mut url = url::Url::parse(&format!(
        "{}/login/oauth/authorize",
        app.config.oauth_base.trim_end_matches('/')
    ))
    .map_err(anyhow::Error::from)?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", app.config.client_id.as_str()),
        (
            "redirect_uri",
            &format!(
                "{}/auth/callback",
                app.config.public_base.trim_end_matches('/')
            ),
        ),
        ("state", &state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    let mut response = Redirect::temporary(url.as_str()).into_response();
    response.headers_mut().append(
        SET_COOKIE,
        session::cookie_header(&app, "memehub_oauth", &state, 600)
            .parse()
            .map_err(anyhow::Error::from)?,
    );
    Ok(response)
}
pub async fn callback(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> Result<Response, Error> {
    if query.error.is_some() {
        return Err(Error::bad("GitHub authorization was not completed"));
    }
    let state = query
        .state
        .as_deref()
        .ok_or_else(|| Error::bad("OAuth state is required"))?;
    let browser_state =
        session::cookie(&headers, "memehub_oauth").ok_or_else(Error::unauthorized)?;
    validate_state(state, &browser_state)?;
    let record = app
        .db
        .get("ghoauth", state)
        .await?
        .ok_or_else(Error::unauthorized)?;
    let pending: Pending = serde_json::from_value(record.value)?;
    if pending.expires_at <= now() {
        return Err(Error::bad(
            "Authorization request expired. Please sign in again.",
        ));
    }
    app.db.delete("ghoauth", state, record.version).await?;
    let code = query
        .code
        .ok_or_else(|| Error::bad("Authorization code is required"))?;
    let token = exchange(&app, &code, &pending.verifier).await?;
    let user = fetch_user(&app, &token).await?;
    let cookie = persist_identity(&app, user).await?;
    let mut response = Redirect::to(&app.config.public_base).into_response();
    response.headers_mut().append(
        SET_COOKIE,
        session::cookie_header(
            &app,
            "memehub_session",
            &cookie,
            app.config.session_ttl_seconds,
        )
        .parse()
        .map_err(anyhow::Error::from)?,
    );
    response.headers_mut().append(
        SET_COOKIE,
        session::cookie_header(&app, "memehub_oauth", "", 0)
            .parse()
            .map_err(anyhow::Error::from)?,
    );
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    Ok(response)
}
async fn persist_identity(app: &App, mut user: User) -> Result<String, Error> {
    let id = user.id.to_string();
    let previous = app.db.get("ghusers", &id).await?;
    let version = previous.as_ref().map_or(0, |record| record.version);
    let previous = previous
        .map(|record| serde_json::from_value::<User>(record.value))
        .transpose()?;
    user.authorize(previous.as_ref());
    let cookie = session::random();
    let session = Session {
        user_id: user.id,
        authorization_id: user.authorization_id.clone(),
        csrf_token: session::random(),
        expires_at: now() + app.config.session_ttl_seconds,
    };
    app.db.transaction(vec![
        json!({"op":"put","namespace":"ghusers","key":id,"value":user,"expected_version":version}),
        json!({"op":"put","namespace":"ghsessions","key":cookie,"value":session,"expected_version":0}),
    ]).await?;
    Ok(cookie)
}
pub fn validate_state(state: &str, browser: &str) -> Result<(), Error> {
    if state.len() != 64 {
        return Err(Error::unauthorized());
    }
    if !bool::from(state.as_bytes().ct_eq(browser.as_bytes())) {
        return Err(Error::unauthorized());
    }
    Ok(())
}
async fn exchange(app: &App, code: &str, verifier: &str) -> Result<String, Error> {
    let body = json!({"client_id":app.config.client_id,"client_secret":secret(&app.config.client_secret_file)?,"code":code,"code_verifier":verifier,"redirect_uri":format!("{}/auth/callback",app.config.public_base.trim_end_matches('/'))});
    let response = app
        .remote
        .http
        .post(format!(
            "{}/login/oauth/access_token",
            app.config.oauth_base.trim_end_matches('/')
        ))
        .header("accept", "application/json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    let data: Value = serde_json::from_slice(&crate::remote::bounded(response, 65536).await?)?;
    Ok(text(&data, "access_token")?.to_string())
}
async fn fetch_user(app: &App, token: &str) -> Result<User, Error> {
    let remote = &app.remote;
    let response = remote
        .http
        .get(remote.url("user")?)
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?;
    let profile: Value = serde_json::from_slice(&crate::remote::bounded(response, 65536).await?)?;
    let response = remote
        .http
        .get(remote.url("user/emails")?)
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?;
    let emails: Vec<Value> =
        serde_json::from_slice(&crate::remote::bounded(response, 65536).await?)?;
    let email = verified_email(&emails).ok_or_else(|| {
        Error::bad("Access to at least one verified GitHub email address is required")
    })?;
    let id = profile["id"]
        .as_u64()
        .ok_or_else(|| Error::bad("GitHub returned an invalid identity"))?;
    let login = text(&profile, "login")?.to_string();
    Ok(User {
        id,
        contribution_email: format!("{id}+{login}@users.noreply.github.com"),
        login,
        email,
        last_login: now(),
        revoked: false,
        authorization_id: String::new(),
    })
}
fn verified_email(emails: &[Value]) -> Option<String> {
    let verified = |value: &&Value| value["verified"].as_bool() == Some(true);
    let email = emails
        .iter()
        .filter(verified)
        .find(|v| v["primary"].as_bool() == Some(true))
        .or_else(|| emails.iter().find(verified))?;
    email["email"].as_str().map(str::to_string)
}
pub async fn logout(State(app): State<App>, headers: HeaderMap) -> Result<Response, Error> {
    let identity = session::identity(&app, &headers).await?;
    let origin = headers.get("origin").and_then(|value| value.to_str().ok());
    if origin != Some(app.config.public_base.trim_end_matches('/')) {
        return Err(Error::bad("Request origin validation failed"));
    }
    session::csrf(&headers, &identity.csrf_token)?;
    let token = session::cookie(&headers, "memehub_session").ok_or_else(Error::unauthorized)?;
    if let Some(record) = app.db.get("ghsessions", &token).await? {
        app.db.delete("ghsessions", &token, record.version).await?;
    }
    let mut response = axum::http::StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        session::cookie_header(&app, "memehub_session", "", 0)
            .parse()
            .map_err(anyhow::Error::from)?,
    );
    Ok(response)
}
