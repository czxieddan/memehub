use crate::{App, config::now, model::Submission, publication};
use anyhow::{Context, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::watch;

pub async fn run(app: App, mut shutdown: watch::Receiver<bool>) {
    let mut timer = tokio::time::interval(Duration::from_secs(5));
    let mut last_sync = 0;
    loop {
        tokio::select! {
            _ = shutdown.changed()=>break,
            _ = timer.tick()=> {
                if let Err(error)=tick(&app,&mut last_sync).await { tracing::warn!(error=%error,"background work delayed"); }
            }
        }
    }
}
async fn tick(app: &App, last_sync: &mut u64) -> anyhow::Result<()> {
    if let Err(error) = process_jobs(app).await {
        tracing::warn!(%error, "submission processing delayed");
    }
    let changed = match consume_events(app).await {
        Ok(changed) => changed,
        Err(error) => {
            tracing::warn!(%error, "event consumption delayed");
            false
        }
    };
    if changed || now().saturating_sub(*last_sync) >= app.config.sync_seconds {
        let publication = publication::synchronize(app).await;
        let reconciliation = reconcile_pulls(app).await;
        let cleanup = cleanup_orphans(app).await;
        publication?;
        reconciliation?;
        cleanup?;
        *last_sync = now();
    }
    Ok(())
}
async fn consume_events(app: &App) -> anyhow::Result<bool> {
    let cursor: Option<String> = app.db.value("corestate", "event_cursor").await?;
    let events = app.github.events(cursor.as_deref().unwrap_or("")).await?;
    let items = events["items"].as_array().context("event page")?;
    for item in items {
        if item.pointer("/event/event").and_then(Value::as_str) == Some("pull_request") {
            import_pull(app, &item["event"]).await?;
        }
    }
    if let Some(last) = items.last() {
        app.db
            .put("corestate", "event_cursor", &last["cursor"], None)
            .await?;
    }
    Ok(!items.is_empty())
}
async fn import_pull(app: &App, event: &Value) -> anyhow::Result<()> {
    if event["repository"] != app.config.repository {
        return Ok(());
    }
    let number = event["number"].as_u64().context("PR number")?;
    let pull = app.github.pull(&app.config.repository, number).await?;
    app.db
        .put("corepulls", &number.to_string(), &pull, None)
        .await
}
async fn reconcile_pulls(app: &App) -> anyhow::Result<()> {
    let mut page: u32 = app.db.value("corestate", "pull_page").await?.unwrap_or(1);
    for _ in 0..5 {
        let result = app.github.pulls(&app.config.repository, page).await?;
        for pull in result["items"]
            .as_array()
            .context("PR reconciliation page")?
        {
            let number = pull["number"].as_u64().context("PR number")?;
            app.db
                .put("corepulls", &number.to_string(), pull, None)
                .await?;
        }
        let next = result["next_page"].as_u64();
        page = next.unwrap_or(1).try_into()?;
        app.db.put("corestate", "pull_page", &page, None).await?;
        if next.is_none() {
            break;
        }
    }
    Ok(())
}
async fn process_jobs(app: &App) -> anyhow::Result<()> {
    let _lock = app.jobs.lock().await;
    for record in app.db.all("coresubmissions").await? {
        let mut task: Submission = serde_json::from_value(record.value)?;
        if task.next_attempt_at > now() {
            continue;
        }
        let result = advance(app, &mut task).await;
        if let Err(error) = result {
            tracing::warn!(submission=%task.id,error=%error,"submission operation delayed");
            failed_attempt(&mut task);
        }
        app.db
            .put("coresubmissions", &task.id, &task, Some(record.version))
            .await?;
        cleanup_task(app, &task).await?;
    }
    Ok(())
}
async fn advance(app: &App, task: &mut Submission) -> anyhow::Result<()> {
    match task.status.as_str() {
        "pending" | "retrying" => create_pr(app, task).await,
        "pr_open" | "awaiting_publish" => refresh_pr(app, task).await,
        _ => Ok(()),
    }
}
async fn create_pr(app: &App, task: &mut Submission) -> anyhow::Result<()> {
    uuid::Uuid::parse_str(&task.id)?;
    let bytes = tokio::fs::read(app.config.staging_dir.join(&task.id)).await?;
    ensure!(
        bytes.len() <= app.config.max_upload_bytes,
        "staged image too large"
    );
    #[derive(Serialize)]
    struct Descriptor<'a> {
        file: &'a str,
        display_name: &'a str,
    }
    let metadata = toml::to_string(&Descriptor {
        file: &task.filename,
        display_name: &task.display_name,
    })?;
    let directory = format!("data/{}/{}", task.uploader_id, task.name);
    let body = json!({"operation_id":task.id,"repository":app.config.repository,"base_branch":app.config.branch,"coauthor_user_id":task.uploader_id,
        "title":format!("feat: add {}",task.name),"message":format!("feat: add {}",task.name),
        "body":format!("The uploader consented to a public submission and GitHub coauthor attribution.\n\n<!-- memehub-uploader:{} -->",task.uploader_id),
        "files":[{"path":format!("{directory}/{}",task.filename),"content_base64":STANDARD.encode(bytes)},{"path":format!("{directory}/meme.toml"),"content_base64":STANDARD.encode(metadata)}]});
    let result = app.github.change(&body).await?;
    task.pr_number = Some(result["number"].as_u64().context("created PR number")?);
    task.pr_url = Some(
        result["url"]
            .as_str()
            .context("created PR URL")?
            .to_string(),
    );
    task.status = "pr_open".into();
    task.error = None;
    task.attempts = 0;
    task.next_attempt_at = now() + 30;
    Ok(())
}
async fn refresh_pr(app: &App, task: &mut Submission) -> anyhow::Result<()> {
    let number = task.pr_number.context("submission missing PR number")?;
    let pull = app.github.pull(&app.config.repository, number).await?;
    task.status = pull_status(&pull).into();
    if task.status == "awaiting_publish" && published(app, task).await? {
        task.status = "published".into();
    }
    if task.status == "closed" {
        task.error = Some(
            "This PR is closed. See its GitHub Actions report for duplicate images or other issues."
                .into(),
        );
    }
    task.next_attempt_at = now() + 30;
    task.attempts = 0;
    Ok(())
}
fn pull_status(pull: &Value) -> &'static str {
    if pull["merged"].as_bool() == Some(true) {
        return "awaiting_publish";
    }
    if pull["state"] == "closed" {
        return "closed";
    }
    "pr_open"
}
async fn published(app: &App, task: &Submission) -> anyhow::Result<bool> {
    let items = app.db.all("corememes").await?;
    Ok(items
        .iter()
        .any(|r| r.value["uploader_id"] == task.uploader_id && r.value["name"] == task.name))
}
fn failed_attempt(task: &mut Submission) {
    task.attempts = task.attempts.saturating_add(1);
    task.next_attempt_at = now() + 5 * 2_u64.pow(task.attempts.min(7));
    if task.pr_number.is_some() {
        return;
    }
    task.status = if task.attempts >= 5 {
        "failed"
    } else {
        "retrying"
    }
    .into();
    task.error = Some(
        "PR creation failed temporarily. Automatic retries are limited; an administrator can check the GitHub configuration and retry."
            .into(),
    );
}
async fn cleanup_task(app: &App, task: &Submission) -> anyhow::Result<()> {
    if task.pr_number.is_none() {
        return Ok(());
    }
    uuid::Uuid::parse_str(&task.id)?;
    match tokio::fs::remove_file(app.config.staging_dir.join(&task.id)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
async fn cleanup_orphans(app: &App) -> anyhow::Result<()> {
    let entries = match tokio::fs::read_dir(&app.config.staging_dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    remove_orphans(app, entries).await
}
async fn remove_orphans(app: &App, mut entries: tokio::fs::ReadDir) -> anyhow::Result<()> {
    while let Some(entry) = entries.next_entry().await? {
        let id = entry.file_name().to_string_lossy().into_owned();
        if !old_staged_file(&entry).await? {
            continue;
        }
        if uuid::Uuid::parse_str(&id).is_err() {
            continue;
        }
        if app.db.get("coresubmissions", &id).await?.is_none() {
            tokio::fs::remove_file(entry.path()).await?;
        }
    }
    Ok(())
}
async fn old_staged_file(entry: &tokio::fs::DirEntry) -> anyhow::Result<bool> {
    let metadata = entry.metadata().await?;
    let elapsed = metadata.modified()?.elapsed().unwrap_or_default();
    Ok(metadata.is_file() && elapsed > Duration::from_secs(86400))
}
