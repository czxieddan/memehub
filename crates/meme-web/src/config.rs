use std::{net::SocketAddr, path::Path};

use anyhow::{Context, bail};
use serde::Deserialize;
use url::Url;

#[derive(Clone, Debug)]
pub(crate) struct WebConfig {
    pub listen: SocketAddr,
    pub core_url: Url,
    pub github_url: Url,
    pub source_url: Option<Url>,
}

#[derive(Deserialize)]
struct Document {
    web: RawWeb,
}

#[derive(Deserialize)]
struct RawWeb {
    listen: SocketAddr,
    core_url: String,
    github_url: String,
    #[serde(default)]
    source_url: String,
}

pub(crate) async fn load(path: &Path) -> anyhow::Result<WebConfig> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .context("read web configuration")?;
    parse(&contents)
}

pub(crate) fn parse(contents: &str) -> anyhow::Result<WebConfig> {
    let document: Document = toml::from_str(contents).context("parse [web] configuration")?;
    let raw = document.web;
    let source_url = match raw.source_url.trim() {
        "" => None,
        value => Some(http_url(value).context("invalid web.source_url")?),
    };
    Ok(WebConfig {
        listen: raw.listen,
        core_url: upstream_url(&raw.core_url).context("invalid web.core_url")?,
        github_url: upstream_url(&raw.github_url).context("invalid web.github_url")?,
        source_url,
    })
}

fn upstream_url(value: &str) -> anyhow::Result<Url> {
    let url = http_url(value)?;
    if url.query().is_some() || url.fragment().is_some() {
        bail!("upstream URL must not contain a query or fragment");
    }
    Ok(url)
}

fn http_url(value: &str) -> anyhow::Result<Url> {
    let url = Url::parse(value)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("URL must use http or https and have a host");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("URL must not contain credentials");
    }
    Ok(url)
}
