use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, watch};
mod changes;
mod config;
mod database;
mod error;
mod oauth;
mod remote;
mod session;
mod webhooks;
use config::{Config, secret};
use database::Database;
use remote::Remote;

#[derive(Clone)]
struct App {
    config: Arc<Config>,
    db: Database,
    remote: Remote,
    service_token: String,
    webhook_secret: String,
    mutation: Arc<Mutex<()>>,
}
pub async fn run(path: PathBuf, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
    let config = Arc::new(Config::load(&path)?);
    let http = reqwest::Client::builder()
        .user_agent("memehub/0.1")
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(45))
        .build()?;
    let db = Database::new(
        http.clone(),
        config.database_url.clone(),
        secret(&config.database_token_file)?,
    );
    let app = App {
        config: config.clone(),
        db,
        remote: Remote::new(http, config.clone()),
        service_token: secret(&config.service_token_file)?,
        webhook_secret: secret(&config.webhook_secret_file)?,
        mutation: Arc::new(Mutex::new(())),
    };
    let cleanup = tokio::spawn(cleanup(app.clone(), shutdown.clone()));
    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    tracing::info!(listen = %config.listen,"GitHub service listening");
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async move {
            let _ = shutdown.changed().await;
        })
        .await?;
    cleanup.abort();
    Ok(())
}
fn router(app: App) -> Router {
    Router::new()
        .route(
            "/health/live",
            get(|| async { Json(json!({"status":"ok"})) }),
        )
        .route("/auth/github", get(oauth::start))
        .route("/auth/callback", get(oauth::callback))
        .route("/auth/logout", post(oauth::logout))
        .route("/v1/session", get(session::introspect))
        .route("/v1/changes", post(changes::create))
        .route("/v1/pulls", get(changes::pulls))
        .route("/v1/pulls/{number}", get(changes::pull))
        .route("/v1/content", get(changes::content))
        .route("/v1/events", get(webhooks::events))
        .route(
            "/webhooks/github",
            post(webhooks::receive).layer(DefaultBodyLimit::max(1024 * 1024)),
        )
        .layer(DefaultBodyLimit::max(30 * 1024 * 1024))
        .with_state(app)
}
async fn cleanup(app: App, mut shutdown: watch::Receiver<bool>) {
    let mut timer = tokio::time::interval(Duration::from_secs(300));
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = timer.tick() => if let Err(error) = expire(&app).await { tracing::warn!(error=%error,"GitHub retention cleanup delayed"); }
        }
    }
}
async fn expire(app: &App) -> anyhow::Result<()> {
    for namespace in ["ghsessions", "ghoauth", "ghdeliveries", "ghevents"] {
        for record in app.db.all(namespace).await? {
            if expired(&record.value) {
                app.db
                    .delete(namespace, &record.key, record.version)
                    .await?;
            }
        }
    }
    Ok(())
}
fn expired(value: &serde_json::Value) -> bool {
    if let Some(expiry) = value["expires_at"].as_u64() {
        return expiry <= config::now();
    }
    value["received_at"]
        .as_u64()
        .is_some_and(|at| at.saturating_add(604800) < config::now())
}
