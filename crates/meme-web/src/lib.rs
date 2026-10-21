//! Embedded public web interface and a deliberately narrow HTTP proxy.

mod assets;
mod config;
mod locales;
mod proxy;
mod routes;

use std::path::PathBuf;

use anyhow::Context;
use tokio::sync::watch;

/// Start the public web listener using the independent `[web]` configuration.
pub async fn run(config_path: PathBuf, shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
    let config = config::load(&config_path).await?;
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .context("bind web listener")?;
    let address = listener.local_addr()?;
    let app = routes::router(proxy::AppState::new(config)?);
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
