use anyhow::{Context, ensure};
use serde_json::json;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub fn initialize(config: &Path) -> anyhow::Result<()> {
    ensure!(!config.exists(), "configuration already exists");
    let root = config.parent().context("configuration parent")?;
    let secrets = root.join("../.local/secrets");
    std::fs::create_dir_all(&secrets)?;
    restrict_directory(&secrets)?;
    for name in [
        "database-key",
        "github-db-token",
        "core-db-token",
        "admin-db-token",
        "github-service-token",
        "admin-token",
        "webhook-secret",
    ] {
        create_secret(
            &secrets.join(name),
            format!("{}{}\n", Uuid::new_v4().simple(), Uuid::new_v4().simple()).as_bytes(),
        )?;
    }
    let read = |name: &str| -> anyhow::Result<String> {
        Ok(std::fs::read_to_string(secrets.join(name))?.trim().into())
    };
    let principals = json!([
        {"token":read("github-db-token")?,"namespaces":["ghusers","ghsessions","ghoauth","ghoperations","ghdeliveries","ghevents"],"admin":false},
        {"token":read("core-db-token")?,"namespaces":["corememes","corestate","coresubmissions","corepulls","coreconfig"],"admin":false},
        {"token":read("admin-db-token")?,"namespaces":[],"admin":true}
    ]);
    create_secret(
        &secrets.join("db-principals.json"),
        serde_json::to_string_pretty(&principals)?.as_bytes(),
    )?;
    create_secret(
        &secrets.join("github-client-secret"),
        b"UNCONFIGURED-client-secret\n",
    )?;
    create_secret(
        &secrets.join("github-app.pem"),
        b"UNCONFIGURED: import a GitHub App RSA PEM private key\n",
    )?;
    std::fs::create_dir_all(root)?;
    create_secret(config, include_bytes!("../../../config/example.toml"))?;
    println!(
        "已创建配置 {} 和 .local/secrets；现有文件不会被覆盖",
        config.display()
    );
    Ok(())
}
pub fn create_secret(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    set_mode(&mut options);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
#[cfg(unix)]
fn set_mode(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}
#[cfg(not(unix))]
fn set_mode(_: &mut std::fs::OpenOptions) {}
#[cfg(unix)]
fn restrict_directory(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}
#[cfg(not(unix))]
fn restrict_directory(_: &Path) -> anyhow::Result<()> {
    Ok(())
}

pub fn import(config: &Path, name: &str, input: Option<PathBuf>) -> anyhow::Result<()> {
    let field = match name {
        "github-app" => "private_key_file",
        "github-client" => "client_secret_file",
        "webhook" => "webhook_secret_file",
        _ => anyhow::bail!("supported secrets: github-app, github-client, webhook"),
    };
    let document: toml::Value = toml::from_str(&std::fs::read_to_string(config)?)?;
    let relative = document
        .get("github")
        .and_then(|v| v.get(field))
        .and_then(toml::Value::as_str)
        .context("missing secret path")?;
    let destination = config
        .parent()
        .context("configuration parent")?
        .join(relative);
    let bytes = read_input(input)?;
    ensure!(bytes.len() >= 16, "secret input is too short");
    replace(&destination, &bytes)?;
    println!("已更新 {name}；请重启 GitHub 服务加载密钥");
    Ok(())
}
fn read_input(input: Option<PathBuf>) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    match input {
        Some(path) => std::fs::File::open(path)?
            .take(65537)
            .read_to_end(&mut bytes)?,
        None => std::io::stdin().take(65537).read_to_end(&mut bytes)?,
    };
    ensure!(bytes.len() <= 65536, "input exceeds 64 KiB");
    Ok(bytes)
}
pub fn replace(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.file_type().is_file(),
        "refusing non-regular target"
    );
    let temp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    create_secret(&temp, bytes)?;
    let result = std::fs::rename(&temp, path);
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.context("replace configuration or secret")
}
