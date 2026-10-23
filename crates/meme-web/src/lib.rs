//! Embedded default frontend, replaceable page router and narrow public proxy.

mod assets;
mod config;
mod locales;
mod proxy;
mod routes;
mod security;

use std::path::PathBuf;

use anyhow::Context;
use axum::{Extension, Router, middleware, routing::get};
use tokio::sync::watch;

/// Page/assets routes supplied by the ordinary distribution.
pub fn frontend() -> Router {
    Router::new()
        .route("/", get(assets::index))
        .route("/assets/app.css", get(assets::css))
        .route("/assets/app.js", get(assets::js))
        .route("/assets/i18n.js", get(assets::i18n))
        .route("/assets/locales/{name}", get(assets::locale))
        .route("/assets/icon.svg", get(assets::icon))
        .route("/source", get(assets::source))
}

/// Start the public listener with the built-in frontend.
pub async fn run(config_path: PathBuf, shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
    run_with_frontend(config_path, shutdown, frontend()).await
}

/// Serve compiled page routes with the shared API, OAuth, Webhooks and headers.
///
/// The external router must not register reserved paths under `/api`, `/auth`,
/// `/webhooks` or `/health`. It receives no secrets or private proxy state.
pub async fn run_with_frontend(
    config_path: PathBuf,
    shutdown: watch::Receiver<bool>,
    frontend: Router,
) -> anyhow::Result<()> {
    let config = config::load(&config_path).await?;
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .context("bind web listener")?;
    let address = listener.local_addr()?;
    let source = config.source_url.clone();
    let app = routes::router(proxy::AppState::new(config)?)
        .merge(frontend)
        .fallback(routes::not_found)
        .layer(Extension(source))
        .layer(middleware::from_fn(security::headers));
    tracing::info!(%address, "web listener ready");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown))
        .await
        .context("serve web interface")
}

async fn shutdown_signal(mut receiver: watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    while receiver.changed().await.is_ok() {
        if *receiver.borrow() {
            break;
        }
    }
}
