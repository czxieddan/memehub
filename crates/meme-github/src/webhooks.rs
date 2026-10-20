use crate::{
    App,
    config::now,
    error::Error,
    session::{User, credential},
};
use axum::{
    Json,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;

pub fn verify(secret: &[u8], signature: &str, body: &[u8]) -> Result<(), Error> {
    let hex = signature
        .strip_prefix("sha256=")
        .ok_or_else(Error::unauthorized)?;
    let signature = hex::decode(hex).map_err(|_| Error::unauthorized())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(anyhow::Error::from)?;
    mac.update(body);
    mac.verify_slice(&signature)
        .map_err(|_| Error::unauthorized())
}
pub async fn receive(
    State(app): State<App>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, Error> {
    let signature = header(&headers, "x-hub-signature-256")?;
    verify(app.webhook_secret.as_bytes(), signature, &body)?;
    let delivery = header(&headers, "x-github-delivery")?;
    uuid::Uuid::parse_str(delivery).map_err(|_| Error::bad("invalid delivery ID"))?;
    if app.db.get("ghdeliveries", delivery).await?.is_some() {
        return Ok(Json(json!({"duplicate":true})));
    }
    let event = header(&headers, "x-github-event")?;
    let payload: Value = serde_json::from_slice(&body)?;
    if event == "github_app_authorization" {
        revoke(&app, &payload).await?;
    }
    let repository = payload
        .pointer("/repository/full_name")
        .and_then(Value::as_str);
    if let Some(repo) = repository {
        app.config.repository(repo)?;
    }
    let record = json!({"delivery_id":delivery,"event":event,"repository":repository,"action":payload["action"],"number":payload["number"],"ref":payload["ref"],"received_at":now()});
    let key = format!("{:020}-{delivery}", now());
    let operations = vec![
        json!({"op":"put","namespace":"ghdeliveries","key":delivery,"value":{"received_at":now()},"expected_version":0}),
        json!({"op":"put","namespace":"ghevents","key":key,"value":record,"expected_version":0}),
    ];
    if let Err(error) = app.db.transaction(operations).await
        && app.db.get("ghdeliveries", delivery).await?.is_none()
    {
        return Err(error.into());
    }
    Ok(Json(json!({"accepted":true})))
}
fn header<'a>(headers: &'a HeaderMap, key: &str) -> Result<&'a str, Error> {
    headers
        .get(key)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| Error(StatusCode::BAD_REQUEST, format!("missing {key}")))
}
async fn revoke(app: &App, payload: &Value) -> Result<(), Error> {
    let Some(id) = payload.pointer("/sender/id").and_then(Value::as_u64) else {
        return Ok(());
    };
    if let Some(record) = app.db.get("ghusers", &id.to_string()).await? {
        let mut user: User = serde_json::from_value(record.value)?;
        user.revoked = true;
        app.db
            .put("ghusers", &id.to_string(), &user, Some(record.version))
            .await?;
    }
    Ok(())
}
#[derive(Deserialize)]
pub struct EventsQuery {
    cursor: Option<String>,
}
pub async fn events(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Value>, Error> {
    credential(&headers, &app.service_token)?;
    let (items, next) = app.db.page("ghevents", query.cursor.as_deref()).await?;
    Ok(Json(
        json!({"items":items.iter().map(|r| json!({"cursor":r.key,"event":r.value})).collect::<Vec<_>>(),"next_cursor":next}),
    ))
}
