use crate::{
    App,
    cache::Policy,
    error::Error,
    model::{Meme, Submission, mime, valid_hash},
    publication,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

#[derive(Deserialize)]
pub struct GalleryQuery {
    query: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}
pub async fn gallery(
    State(app): State<App>,
    Query(query): Query<GalleryQuery>,
) -> Result<Json<Value>, Error> {
    let _lock = app.publication.lock().await;
    let records = app.db.all("corememes").await?;
    let mut items = records
        .into_iter()
        .map(|r| serde_json::from_value::<Meme>(r.value))
        .collect::<Result<Vec<_>, _>>()?;
    let term = query.query.unwrap_or_default().to_lowercase();
    let cursor = query.cursor.unwrap_or_default();
    items.retain(|item| gallery_match(item, &term, &cursor));
    items.sort_by(|a, b| a.manifest_hash.cmp(&b.manifest_hash));
    let limit = query.limit.unwrap_or(40).clamp(1, 100);
    let more = items.len() > limit;
    items.truncate(limit);
    let next = if more {
        items.last().map(|item| item.manifest_hash.clone())
    } else {
        None
    };
    Ok(Json(json!({"items":items,"next_cursor":next})))
}
fn gallery_match(item: &Meme, term: &str, cursor: &str) -> bool {
    item.manifest_hash.as_str() > cursor
        && (item.name.to_lowercase().contains(term)
            || item.display_name.to_lowercase().contains(term))
}
pub async fn image(State(app): State<App>, Path(hash): Path<String>) -> Result<Response, Error> {
    if !valid_hash(&hash) {
        return Err(Error::missing());
    }
    let _lock = app.publication.lock().await;
    let _: Meme = app
        .db
        .value("corememes", &hash)
        .await?
        .ok_or_else(Error::missing)?;
    let state: Value = app
        .db
        .value("corestate", "publication")
        .await?
        .ok_or_else(Error::missing)?;
    let commit = state["source_commit"].as_str().ok_or_else(Error::missing)?;
    let path = format!("manifest/{hash}/manifest.toml");
    let bytes = publication::cached_content(&app, &path, commit).await?;
    let manifest = publication::parse_manifest(&bytes, &hash)?;
    let bytes =
        publication::cached_content(&app, manifest.path.trim_start_matches('/'), commit).await?;
    let mut response = bytes.as_ref().clone().into_response();
    response.headers_mut().insert(
        "content-type",
        mime(&manifest.path)
            .ok_or_else(Error::missing)?
            .parse()
            .unwrap(),
    );
    response
        .headers_mut()
        .insert("cache-control", "public, max-age=60".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    Ok(response)
}
pub fn admin(app: &App, headers: &HeaderMap) -> Result<(), Error> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !bool::from(token.as_bytes().ct_eq(app.admin_token.as_bytes())) {
        return Err(Error::unauthorized());
    }
    Ok(())
}
pub async fn cache_show(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>, Error> {
    admin(&app, &headers)?;
    Ok(Json(app.cache.lock().await.stats()))
}
pub async fn cache_set(
    State(app): State<App>,
    headers: HeaderMap,
    Json(policy): Json<Policy>,
) -> Result<Json<Value>, Error> {
    admin(&app, &headers)?;
    policy
        .validate()
        .map_err(|error| Error::bad(&error.to_string()))?;
    let _lock = app.settings.lock().await;
    app.db.put("coreconfig", "cache", &policy, None).await?;
    app.cache.lock().await.reconfigure(policy);
    Ok(Json(app.cache.lock().await.stats()))
}
pub async fn sync(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>, Error> {
    admin(&app, &headers)?;
    Ok(Json(publication::synchronize(&app).await?))
}
pub async fn tasks(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>, Error> {
    admin(&app, &headers)?;
    Ok(Json(json!({"items":app.db.all("coresubmissions").await?})))
}
pub async fn pulls(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>, Error> {
    admin(&app, &headers)?;
    Ok(Json(json!({"items":app.db.all("corepulls").await?})))
}
pub async fn retry(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    admin(&app, &headers)?;
    uuid::Uuid::parse_str(&id).map_err(|_| Error::bad("投稿编号无效"))?;
    let _lock = app.jobs.lock().await;
    let record = app
        .db
        .get("coresubmissions", &id)
        .await?
        .ok_or_else(Error::missing)?;
    let mut task: Submission = serde_json::from_value(record.value)?;
    if task.status != "failed" {
        return Err(Error(
            StatusCode::CONFLICT,
            "仅能重试创建失败的 PR，已关闭 PR 请在 GitHub 修正".into(),
        ));
    }
    task.status = "pending".into();
    task.attempts = 0;
    task.error = None;
    task.next_attempt_at = 0;
    app.db
        .put("coresubmissions", &id, &task, Some(record.version))
        .await?;
    Ok(Json(json!(task)))
}
