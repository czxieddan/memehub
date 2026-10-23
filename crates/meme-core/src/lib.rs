use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, watch};
mod cache;
mod catalog;
mod config;
mod database;
mod error;
mod gallery;
mod gallery_page;
mod github;
mod model;
mod policy;
mod publication;
mod routes;
mod submissions;
mod upload;
mod worker;
use cache::{Cache, Policy};
use config::{Config, secret};
use database::Database;
use github::Gateway;
#[derive(Clone)]
struct App {
    config: Arc<Config>,
    db: Database,
    github: Gateway,
    cache: Arc<Mutex<Cache>>,
    admin_token: String,
    publication: Arc<Mutex<()>>,
    jobs: Arc<Mutex<()>>,
    settings: Arc<Mutex<()>>,
    download: Arc<Mutex<()>>,
}
pub async fn run(path: PathBuf, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
    let config = Arc::new(Config::load(&path)?);
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let db = Database::new(
        http.clone(),
        config.database_url.clone(),
        secret(&config.database_token_file)?,
    );
    let policy: Policy = db.value("coreconfig", "cache").await?.unwrap_or(Policy {
        max_bytes: config.cache_max_bytes,
        max_entries: config.cache_max_entries,
        ttl_seconds: config.cache_ttl_seconds,
    });
    policy.validate()?;
    let app = App {
        config: config.clone(),
        db,
        github: Gateway::new(
            http,
            config.github_url.clone(),
            secret(&config.github_token_file)?,
        ),
        cache: Arc::new(Mutex::new(Cache::new(policy))),
        admin_token: secret(&config.admin_token_file)?,
        publication: Arc::new(Mutex::new(())),
        jobs: Arc::new(Mutex::new(())),
        settings: Arc::new(Mutex::new(())),
        download: Arc::new(Mutex::new(())),
    };
    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    let worker = tokio::spawn(worker::run(app.clone(), shutdown.clone()));
    tracing::info!(listen=%config.listen,"core listening");
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async move {
            let _ = shutdown.changed().await;
        })
        .await?;
    worker.abort();
    Ok(())
}
fn router(app: App) -> Router {
    Router::new()
        .route(
            "/health/live",
            get(|| async { Json(json!({"status":"ok"})) }),
        )
        .route("/v1/gallery", get(gallery::list))
        .route("/v1/memes/{hash}", get(gallery::detail))
        .route("/v1/memes/{hash}/view", post(gallery::view))
        .route("/v1/policy", get(policy::show))
        .route("/v1/images/{hash}", get(routes::image))
        .route("/v1/me", get(submissions::me))
        .route(
            "/v1/submissions",
            get(submissions::list).post(submissions::create),
        )
        .route(
            "/v1/admin/cache",
            get(routes::cache_show).put(routes::cache_set),
        )
        .route(
            "/v1/admin/gallery",
            get(gallery::show)
                .put(gallery::set)
                .layer(DefaultBodyLimit::max(40 * 1024)),
        )
        .route("/v1/admin/sync", post(routes::sync))
        .route("/v1/admin/submissions", get(routes::tasks))
        .route("/v1/admin/pulls", get(routes::pulls))
        .route("/v1/admin/submissions/{id}/retry", post(routes::retry))
        .layer(DefaultBodyLimit::max(6 * 1024 * 1024))
        .with_state(app)
}
