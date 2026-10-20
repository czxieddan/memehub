use crate::{
    App,
    config::now,
    error::Error,
    model::{Meme, Submission},
    upload,
};
use axum::{
    Json,
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use tokio::io::AsyncWriteExt;

pub async fn me(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<crate::github::Identity>, Error> {
    Ok(Json(app.github.identity(&headers).await?))
}
pub async fn list(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>, Error> {
    let user = app.github.identity(&headers).await?;
    let items = app
        .db
        .all("coresubmissions")
        .await?
        .into_iter()
        .map(|r| serde_json::from_value::<Submission>(r.value))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(
        json!({"items":items.into_iter().filter(|s| s.uploader_id==user.id).collect::<Vec<_>>()}),
    ))
}
pub async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    form: Multipart,
) -> Result<(StatusCode, Json<Submission>), Error> {
    let user = app.github.identity(&headers).await?;
    verify_origin(&headers, &app.config.public_base)?;
    verify_csrf(&headers, &user.csrf_token)?;
    let upload = upload::read(form, app.config.max_upload_bytes).await?;
    let pixels = app.config.max_pixels;
    let upload = tokio::task::spawn_blocking(move || {
        upload::validate_image(&upload.bytes, &upload.filename, pixels)?;
        Ok::<_, Error>(upload)
    })
    .await
    .map_err(anyhow::Error::from)??;
    let _lock = app.jobs.lock().await;
    check_limits(&app, user.id, &upload.name).await?;
    let id = uuid::Uuid::new_v4().to_string();
    save_file(&app, &id, &upload.bytes).await?;
    let submission = Submission {
        id: id.clone(),
        uploader_id: user.id,
        name: upload.name,
        display_name: upload.display_name,
        filename: upload.filename,
        status: "pending".into(),
        created_at: now(),
        attempts: 0,
        next_attempt_at: 0,
        pr_number: None,
        pr_url: None,
        error: None,
    };
    app.db
        .put("coresubmissions", &id, &submission, Some(0))
        .await?;
    Ok((StatusCode::ACCEPTED, Json(submission)))
}
fn verify_origin(headers: &HeaderMap, public_base: &str) -> Result<(), Error> {
    let origin = headers.get("origin").and_then(|value| value.to_str().ok());
    if origin != Some(public_base.trim_end_matches('/')) {
        return Err(Error::bad("请求来源校验失败"));
    }
    Ok(())
}
pub fn verify_csrf(headers: &HeaderMap, expected: &str) -> Result<(), Error> {
    let actual = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !bool::from(expected.as_bytes().ct_eq(actual.as_bytes())) {
        return Err(Error::bad("CSRF 校验失败，请刷新页面"));
    }
    Ok(())
}
async fn save_file(app: &App, id: &str, bytes: &[u8]) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(&app.config.staging_dir).await?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(app.config.staging_dir.join(id))
        .await?;
    file.write_all(bytes).await?;
    file.sync_all().await?;
    Ok(())
}
async fn check_limits(app: &App, id: u64, name: &str) -> Result<(), Error> {
    let records = app.db.all("coresubmissions").await?;
    let tasks = records
        .into_iter()
        .map(|r| serde_json::from_value::<Submission>(r.value))
        .collect::<Result<Vec<_>, _>>()?;
    let mine: Vec<_> = tasks.iter().filter(|s| s.uploader_id == id).collect();
    if mine.iter().filter(|s| s.created_at + 60 > now()).count() >= 3 {
        return Err(Error(
            StatusCode::TOO_MANY_REQUESTS,
            "每分钟最多投稿三次".into(),
        ));
    }
    if mine
        .iter()
        .any(|s| s.name == name && !matches!(s.status.as_str(), "closed" | "failed"))
    {
        return Err(Error(StatusCode::CONFLICT, "这个 Name 已有投稿".into()));
    }
    check_published_name(app, id, name).await
}
async fn check_published_name(app: &App, id: u64, name: &str) -> Result<(), Error> {
    for record in app.db.all("corememes").await? {
        let meme: Meme = serde_json::from_value(record.value)?;
        if meme.uploader_id == id && meme.name == name {
            return Err(Error(
                StatusCode::CONFLICT,
                "这个 Name 已发布，更新请通过仓库 PR 提交".into(),
            ));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csrf_requires_matching_origin_and_token() {
        let mut headers = HeaderMap::new();
        assert!(verify_origin(&headers, "https://meme.example").is_err());
        headers.insert("origin", "https://evil.example".parse().unwrap());
        assert!(verify_origin(&headers, "https://meme.example").is_err());
        headers.insert("origin", "https://meme.example".parse().unwrap());
        assert!(verify_origin(&headers, "https://meme.example/").is_ok());
        assert!(verify_csrf(&headers, "a-secret-token").is_err());
        headers.insert("x-csrf-token", "a-secret-token".parse().unwrap());
        assert!(verify_csrf(&headers, "a-secret-token").is_ok());
    }
}
