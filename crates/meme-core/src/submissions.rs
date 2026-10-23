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
    let upload = upload::read(form, &app.config).await?;
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
        policy_acceptance: upload.policy_acceptance,
    };
    persist(&app, &submission).await?;
    Ok((StatusCode::ACCEPTED, Json(submission)))
}
async fn persist(app: &App, submission: &Submission) -> Result<(), Error> {
    let result = app
        .db
        .put("coresubmissions", &submission.id, submission, Some(0))
        .await;
    let Err(error) = result else {
        return Ok(());
    };
    match app.db.get("coresubmissions", &submission.id).await {
        Ok(Some(record)) if record.value == serde_json::to_value(submission)? => Ok(()),
        Ok(None) => {
            remove_staged(app, &submission.id).await;
            Err(error.into())
        }
        _ => Err(error.into()),
    }
}
async fn remove_staged(app: &App, id: &str) {
    if let Err(error) = tokio::fs::remove_file(app.config.staging_dir.join(id)).await {
        tracing::warn!(%error, submission = id, "staged file cleanup delayed");
    }
}
pub fn verify_origin(headers: &HeaderMap, public_base: &str) -> Result<(), Error> {
    if headers.get_all("origin").iter().count() != 1 {
        return Err(Error::bad("Request origin validation failed"));
    }
    let origin = headers.get("origin").and_then(|value| value.to_str().ok());
    if origin != Some(public_base.trim_end_matches('/')) {
        return Err(Error::bad("Request origin validation failed"));
    }
    Ok(())
}
pub fn verify_csrf(headers: &HeaderMap, expected: &str) -> Result<(), Error> {
    let actual = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !bool::from(expected.as_bytes().ct_eq(actual.as_bytes())) {
        return Err(Error::bad(
            "CSRF validation failed. Please refresh the page.",
        ));
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
    let result = async {
        file.write_all(bytes).await?;
        file.sync_all().await
    }
    .await;
    drop(file);
    if let Err(error) = result {
        remove_staged(app, id).await;
        return Err(error.into());
    }
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
            "You can submit up to three times per minute".into(),
        ));
    }
    if mine
        .iter()
        .any(|s| s.name == name && !matches!(s.status.as_str(), "closed" | "failed"))
    {
        return Err(Error(
            StatusCode::CONFLICT,
            "A submission already exists for this Name".into(),
        ));
    }
    check_published_name(app, id, name).await
}
async fn check_published_name(app: &App, id: u64, name: &str) -> Result<(), Error> {
    for record in app.db.all("corememes").await? {
        let meme: Meme = serde_json::from_value(record.value)?;
        if meme.uploader_id == id && meme.name == name {
            return Err(Error(
                StatusCode::CONFLICT,
                "This Name is already published. Submit updates through a repository PR.".into(),
            ));
        }
    }
    Ok(())
}
