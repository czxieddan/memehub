use crate::error::Error;
use anyhow::{Context, ensure};
use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone)]
pub struct Gateway {
    http: Client,
    base: String,
    token: String,
}
#[derive(Serialize, Deserialize)]
pub struct Identity {
    pub id: u64,
    pub login: String,
    pub email: String,
    pub contribution_email: String,
    pub csrf_token: String,
}
impl Gateway {
    pub fn new(http: Client, base: String, token: String) -> Self {
        Self { http, base, token }
    }
    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base.trim_end_matches('/'), path)
    }
    pub async fn identity(&self, headers: &HeaderMap) -> Result<Identity, Error> {
        let cookie = headers
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let response = self
            .http
            .get(self.endpoint("/v1/session"))
            .bearer_auth(&self.token)
            .header("cookie", cookie)
            .send()
            .await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(Error::unauthorized());
        }
        Ok(response.error_for_status()?.json().await?)
    }
    pub async fn change(&self, body: &Value) -> anyhow::Result<Value> {
        Ok(self
            .http
            .post(self.endpoint("/v1/changes"))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    pub async fn pull(&self, repository: &str, number: u64) -> anyhow::Result<Value> {
        Ok(self
            .http
            .get(self.endpoint(&format!("/v1/pulls/{number}")))
            .bearer_auth(&self.token)
            .query(&[("repository", repository)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    pub async fn pulls(&self, repository: &str, page: u32) -> anyhow::Result<Value> {
        let response = self
            .http
            .get(self.endpoint("/v1/pulls"))
            .bearer_auth(&self.token)
            .query(&[
                ("repository", repository.to_string()),
                ("page", page.to_string()),
            ])
            .send()
            .await?
            .error_for_status()?;
        Ok(serde_json::from_slice(
            &bounded(response, 1024 * 1024).await?,
        )?)
    }
    pub async fn content(
        &self,
        repository: &str,
        path: &str,
        reference: &str,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let response = self
            .http
            .get(self.endpoint("/v1/content"))
            .bearer_auth(&self.token)
            .query(&[
                ("repository", repository),
                ("path", path),
                ("reference", reference),
            ])
            .send()
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let data: Value =
            serde_json::from_slice(&bounded(response.error_for_status()?, 9 * 1024 * 1024).await?)?;
        let bytes = STANDARD.decode(data["content_base64"].as_str().context("missing content")?)?;
        ensure!(
            bytes.len() <= 6 * 1024 * 1024,
            "repository file exceeds limit"
        );
        Ok(Some(bytes))
    }
    pub async fn events(&self, cursor: &str) -> anyhow::Result<Value> {
        Ok(self
            .http
            .get(self.endpoint("/v1/events"))
            .bearer_auth(&self.token)
            .query(&[("cursor", cursor)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
}
async fn bounded(mut response: reqwest::Response, max: usize) -> anyhow::Result<Vec<u8>> {
    ensure!(
        response.content_length().unwrap_or(0) <= max as u64,
        "response too large"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(bytes.len() + chunk.len() <= max, "response too large");
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
