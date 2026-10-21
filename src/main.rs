use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use std::{path::PathBuf, time::Duration};
use tokio::{sync::watch, task::JoinSet};

#[derive(Parser)]
#[command(version, about = "A GitHub-powered meme sharing site")]
struct Arguments {
    #[arg(long, global = true, default_value = "config/local.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long, value_enum, default_value = "all")]
        service: Service,
    },
    Admin(meme_cli::Admin),
}
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Service {
    All,
    Database,
    Github,
    Core,
    Web,
}
#[derive(Deserialize)]
struct Endpoint {
    listen: String,
}
#[derive(Deserialize)]
struct Configuration {
    database: Endpoint,
    github: Endpoint,
    core: Endpoint,
    web: Endpoint,
}
impl Configuration {
    fn address(&self, service: Service) -> &str {
        match service {
            Service::Database => &self.database.listen,
            Service::Github => &self.github.listen,
            Service::Core => &self.core.listen,
            Service::Web => &self.web.listen,
            Service::All => unreachable!("all is expanded before startup"),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let arguments = Arguments::parse();
    match arguments.command {
        Command::Admin(admin) => meme_cli::run(arguments.config, admin).await,
        Command::Serve { service } => serve(arguments.config, service).await,
    }
}
async fn serve(path: PathBuf, service: Service) -> anyhow::Result<()> {
    let config: Configuration = toml::from_str(&std::fs::read_to_string(&path)?)?;
    let (shutdown, receiver) = watch::channel(false);
    let mut tasks = JoinSet::new();
    let startup = start_services(&path, service, &config, receiver, &mut tasks).await;
    let outcome = match startup {
        Err(error) => Err(error),
        Ok(()) => tokio::select! {
            signal = shutdown_signal() => signal,
            result = tasks.join_next() => unexpected_exit(result),
        },
    };
    let _ = shutdown.send(true);
    if tokio::time::timeout(Duration::from_secs(15), drain(&mut tasks))
        .await
        .is_err()
    {
        tasks.abort_all();
        tracing::warn!("service shutdown exceeded 15 seconds");
    }
    outcome
}
async fn start_services(
    path: &std::path::Path,
    service: Service,
    config: &Configuration,
    shutdown: watch::Receiver<bool>,
    tasks: &mut JoinSet<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let services = match service {
        Service::All => vec![
            Service::Database,
            Service::Github,
            Service::Core,
            Service::Web,
        ],
        single => vec![single],
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()?;
    for service in services {
        spawn(tasks, service, path.to_path_buf(), shutdown.clone());
        let address: std::net::SocketAddr = config.address(service).parse()?;
        let host = if address.ip().is_unspecified() {
            std::net::IpAddr::from([127, 0, 0, 1])
        } else {
            address.ip()
        };
        let url = format!(
            "http://{}/health/live",
            std::net::SocketAddr::new(host, address.port())
        );
        wait_ready(&http, &url, tasks)
            .await
            .with_context(|| format!("{service:?} startup"))?;
    }
    Ok(())
}
fn spawn(
    tasks: &mut JoinSet<anyhow::Result<()>>,
    service: Service,
    path: PathBuf,
    shutdown: watch::Receiver<bool>,
) {
    tasks.spawn(async move {
        let result = match service {
            Service::Database => modular_db_api::run(path, shutdown).await,
            Service::Github => meme_github::run(path, shutdown).await,
            Service::Core => meme_core::run(path, shutdown).await,
            Service::Web => meme_web::run(path, shutdown).await,
            Service::All => unreachable!(),
        };
        result.with_context(|| format!("{service:?} service stopped"))
    });
}
async fn wait_ready(
    http: &reqwest::Client,
    url: &str,
    tasks: &mut JoinSet<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(result) = tasks.try_join_next() {
            return unexpected_exit(Some(result));
        }
        if http
            .get(url)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("readiness timed out: {url}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
fn unexpected_exit(
    result: Option<Result<anyhow::Result<()>, tokio::task::JoinError>>,
) -> anyhow::Result<()> {
    result.context("no running services")???;
    bail!("a service exited before shutdown")
}
async fn drain(tasks: &mut JoinSet<anyhow::Result<()>>) {
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(())) => {}
            other => tracing::warn!(?other, "service shutdown failed"),
        }
    }
}
#[cfg(unix)]
async fn shutdown_signal() -> anyhow::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! { signal = tokio::signal::ctrl_c() => signal?, _ = terminate.recv() => {} }
    Ok(())
}
#[cfg(not(unix))]
async fn shutdown_signal() -> anyhow::Result<()> {
    Ok(tokio::signal::ctrl_c().await?)
}
