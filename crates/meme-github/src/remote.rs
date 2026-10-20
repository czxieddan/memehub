use crate::config::{Config, now};
use anyhow::{Context, ensure};
use reqwest::{Client, Method, StatusCode};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Remote {
    pub http: Client,
    pub config: Arc<Config>,
    token: Arc<Mutex<Option<(String, u64)>>>,
}
impl Remote {
    pub fn new(http: Client, config: Arc<Config>) -> Self {
        Self {
            http,
            config,
            token: Arc::new(Mutex::new(None)),
        }
    }
    pub fn url(&self, path: &str) -> anyhow::Result<url::Url> {
        let mut url = url::Url::parse(&self.config.api_base)?;
        let (path, query) = path
            .split_once('?')
            .map_or((path, None), |(path, query)| (path, Some(query)));
        url.set_path(&format!(
            "{}/{}",
            url.path().trim_end_matches('/'),
            path.trim_start_matches('/')
        ));
        url.set_query(query);
        Ok(url)
    }
    pub async fn installation_token(&self) -> anyhow::Result<String> {
        let mut cached = self.token.lock().await;
        if let Some((token, expires)) = cached.as_ref().filter(|(_, expires)| *expires > now() + 60)
        {
            let _ = expires;
            return Ok(token.clone());
        }
        #[derive(Serialize)]
        struct Claims<'a> {
            iat: u64,
            exp: u64,
            iss: &'a str,
        }
        let pem = std::fs::read(&self.config.private_key_file).context("GitHub App private key")?;
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(&pem)?;
        let jwt = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &Claims {
                iat: now().saturating_sub(60),
                exp: now() + 480,
                iss: &self.config.app_id,
            },
            &key,
        )?;
        let url = self.url(&format!(
            "app/installations/{}/access_tokens",
            self.config.installation_id
        ))?;
        let response = self
            .http
            .post(url)
            .bearer_auth(jwt)
            .json(&json!({}))
            .send()
            .await?
            .error_for_status()?;
        let data: Value = response.json().await?;
        let token = text(&data, "token")?.to_string();
        *cached = Some((token.clone(), now() + 3000));
        Ok(token)
    }
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<Option<Value>> {
        let token = self.installation_token().await?;
        let mut request = self
            .http
            .request(method, self.url(path)?)
            .bearer_auth(token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        ensure!(
            response.status().is_success(),
            "GitHub returned {}",
            response.status()
        );
        Ok(Some(serde_json::from_slice(
            &bounded(response, 8 * 1024 * 1024).await?,
        )?))
    }
    pub async fn required(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<Value> {
        self.request(method, path, body)
            .await?
            .context("GitHub resource not found")
    }
    pub async fn content(
        &self,
        repository: &str,
        path: &str,
        reference: &str,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        self.config.repository(repository)?;
        validate_path(path)?;
        let response = self
            .http
            .get(self.url(&format!("repos/{repository}/contents/{path}"))?)
            .query(&[("ref", reference)])
            .header("accept", "application/vnd.github.raw+json")
            .bearer_auth(self.installation_token().await?)
            .send()
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        ensure!(
            response.status().is_success(),
            "GitHub content returned {}",
            response.status()
        );
        Ok(Some(bounded(response, 6 * 1024 * 1024).await?))
    }
}
pub async fn bounded(mut response: reqwest::Response, max: usize) -> anyhow::Result<Vec<u8>> {
    ensure!(
        response.content_length().unwrap_or(0) <= max as u64,
        "upstream response too large"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= max,
            "upstream response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub fn text<'a>(value: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("missing field {key}"))
}
pub fn validate_path(path: &str) -> anyhow::Result<()> {
    ensure!(
        !path.starts_with('/') && path.len() <= 240,
        "invalid repository path"
    );
    ensure!(
        !path.contains('\\') && !path.chars().any(char::is_control),
        "invalid repository path"
    );
    ensure!(
        path.split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".." && s != ".git"),
        "invalid repository path"
    );
    Ok(())
}
