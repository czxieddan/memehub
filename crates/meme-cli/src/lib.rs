use anyhow::{Context, ensure};
use clap::{Args, Subcommand};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
mod configuration;
mod secrets;

#[derive(Args)]
pub struct Admin {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Subcommand)]
pub enum Command {
    Init,
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Secret {
        name: String,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
    Status,
    Sync,
    Submissions,
    Pulls,
    Retry {
        id: String,
    },
    Backup,
    Data {
        #[command(subcommand)]
        command: DataCommand,
    },
}
#[derive(Subcommand)]
pub enum ConfigCommand {
    Show,
    Set { key: String, value: String },
}
#[derive(Subcommand)]
pub enum CacheCommand {
    Show,
    Set {
        #[arg(long)]
        max_bytes: usize,
        #[arg(long)]
        max_entries: usize,
        #[arg(long)]
        ttl_seconds: u64,
    },
}
#[derive(Subcommand)]
pub enum DataCommand {
    List {
        namespace: String,
    },
    Get {
        namespace: String,
        key: String,
    },
    Put {
        namespace: String,
        key: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        expected_version: u64,
    },
    Delete {
        namespace: String,
        key: String,
        #[arg(long)]
        expected_version: u64,
    },
}
#[derive(Deserialize)]
struct Config {
    core_url: String,
    admin_token_file: PathBuf,
    database_url: String,
    database_token_file: PathBuf,
}

pub async fn run(path: PathBuf, admin: Admin) -> anyhow::Result<()> {
    match admin.command {
        Command::Init => secrets::initialize(&path),
        Command::Config { command } => match command {
            ConfigCommand::Show => configuration::show(&path),
            ConfigCommand::Set { key, value } => configuration::set(&path, &key, &value),
        },
        Command::Secret { name, file } => secrets::import(&path, &name, file),
        command => remote(&path, command).await,
    }
}
fn load(path: &Path) -> anyhow::Result<Config> {
    #[derive(Deserialize)]
    struct File {
        cli: Config,
    }
    let mut config = toml::from_str::<File>(&std::fs::read_to_string(path)?)?.cli;
    let root = path.parent().context("configuration parent")?;
    config.admin_token_file = root.join(config.admin_token_file);
    config.database_token_file = root.join(config.database_token_file);
    Ok(config)
}
async fn remote(path: &Path, command: Command) -> anyhow::Result<()> {
    let config = load(path)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let token = std::fs::read_to_string(&config.admin_token_file)?
        .trim()
        .to_string();
    let (method, endpoint, body) = request(command)?;
    let (base, token) =
        if endpoint.starts_with("/v1/namespaces/") || endpoint == "/v1/admin/backups" {
            (
                config.database_url,
                std::fs::read_to_string(config.database_token_file)?
                    .trim()
                    .to_string(),
            )
        } else {
            (config.core_url, token)
        };
    let mut request = client
        .request(method, format!("{}{endpoint}", base.trim_end_matches('/')))
        .bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await?;
    ensure!(
        response.status().is_success(),
        "management API returned {}",
        response.status()
    );
    let value: Value = response.json().await?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
fn request(command: Command) -> anyhow::Result<(Method, String, Option<Value>)> {
    let result = match command {
        Command::Cache { command } => cache_request(command),
        Command::Sync => (Method::POST, "/v1/admin/sync".into(), None),
        Command::Status => (Method::GET, "/health/live".into(), None),
        Command::Submissions => (Method::GET, "/v1/admin/submissions".into(), None),
        Command::Pulls => (Method::GET, "/v1/admin/pulls".into(), None),
        Command::Retry { id } => {
            uuid::Uuid::parse_str(&id)?;
            (
                Method::POST,
                format!("/v1/admin/submissions/{id}/retry"),
                None,
            )
        }
        Command::Backup => (Method::POST, "/v1/admin/backups".into(), None),
        Command::Data { command } => return data_request(command),
        _ => anyhow::bail!("local command cannot use remote request"),
    };
    Ok(result)
}
fn cache_request(command: CacheCommand) -> (Method, String, Option<Value>) {
    match command {
        CacheCommand::Show => (Method::GET, "/v1/admin/cache".into(), None),
        CacheCommand::Set {
            max_bytes,
            max_entries,
            ttl_seconds,
        } => (
            Method::PUT,
            "/v1/admin/cache".into(),
            Some(
                json!({"max_bytes":max_bytes,"max_entries":max_entries,"ttl_seconds":ttl_seconds}),
            ),
        ),
    }
}
fn data_request(command: DataCommand) -> anyhow::Result<(Method, String, Option<Value>)> {
    match command {
        DataCommand::List { namespace } => Ok((Method::GET, data_path(&namespace, None)?, None)),
        DataCommand::Get { namespace, key } => {
            Ok((Method::GET, data_path(&namespace, Some(&key))?, None))
        }
        DataCommand::Delete {
            namespace,
            key,
            expected_version,
        } => Ok((
            Method::DELETE,
            format!(
                "{}?expected_version={expected_version}",
                data_path(&namespace, Some(&key))?
            ),
            None,
        )),
        DataCommand::Put {
            namespace,
            key,
            file,
            expected_version,
        } => {
            let value: Value = serde_json::from_slice(&std::fs::read(file)?)?;
            Ok((
                Method::PUT,
                data_path(&namespace, Some(&key))?,
                Some(json!({"value":value,"expected_version":expected_version})),
            ))
        }
    }
}
fn data_path(namespace: &str, key: Option<&str>) -> anyhow::Result<String> {
    let mut url = url::Url::parse("http://local.invalid")?;
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| anyhow::anyhow!("URL"))?;
    segments.extend(["v1", "namespaces", namespace, "records"]);
    if let Some(key) = key {
        segments.push(key);
    }
    drop(segments);
    Ok(url.path().to_string())
}
