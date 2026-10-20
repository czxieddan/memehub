use anyhow::{Context, ensure};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Database {
    http: Client,
    base: String,
    token: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Record {
    pub key: String,
    pub version: u64,
    pub value: Value,
}
#[derive(Deserialize)]
struct Page {
    items: Vec<Record>,
    next_cursor: Option<String>,
}
impl Database {
    pub fn new(http: Client, base: String, token: String) -> Self {
        Self { http, base, token }
    }
    fn url(&self, namespace: &str, key: Option<&str>) -> anyhow::Result<url::Url> {
        let mut url = url::Url::parse(&self.base)?;
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid database URL"))?;
        segments
            .pop_if_empty()
            .extend(["v1", "namespaces", namespace, "records"]);
        if let Some(key) = key {
            segments.push(key);
        }
        drop(segments);
        Ok(url)
    }
    pub async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<Record>> {
        let response = self
            .http
            .get(self.url(namespace, Some(key))?)
            .bearer_auth(&self.token)
            .send()
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(response.error_for_status()?.json().await?))
    }
    pub async fn value<T: DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
    ) -> anyhow::Result<Option<T>> {
        self.get(namespace, key)
            .await?
            .map(|r| serde_json::from_value(r.value).map_err(Into::into))
            .transpose()
    }
    pub async fn put<T: Serialize>(
        &self,
        namespace: &str,
        key: &str,
        value: &T,
        expected: Option<u64>,
    ) -> anyhow::Result<()> {
        let body = json!({"value":value,"expected_version":expected});
        self.http
            .put(self.url(namespace, Some(key))?)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    pub async fn transaction(&self, operations: Vec<Value>) -> anyhow::Result<()> {
        self.http
            .post(format!(
                "{}/v1/transactions",
                self.base.trim_end_matches('/')
            ))
            .bearer_auth(&self.token)
            .json(&json!({"operations":operations}))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    pub async fn page(
        &self,
        namespace: &str,
        cursor: Option<&str>,
    ) -> anyhow::Result<(Vec<Record>, Option<String>)> {
        let response = self
            .http
            .get(self.url(namespace, None)?)
            .bearer_auth(&self.token)
            .query(&[("limit", "500"), ("cursor", cursor.unwrap_or(""))])
            .send()
            .await?
            .error_for_status()?;
        let page: Page = response.json().await?;
        Ok((page.items, page.next_cursor))
    }
    pub async fn all(&self, namespace: &str) -> anyhow::Result<Vec<Record>> {
        let mut items = Vec::new();
        let mut cursor = None;
        loop {
            let (page, next) = self.page(namespace, cursor.as_deref()).await?;
            items.extend(page);
            ensure!(
                items.len() <= 10000,
                "namespace scan exceeds initial deployment limit"
            );
            if next.is_none() {
                return Ok(items);
            }
            ensure!(next != cursor, "database cursor did not advance");
            cursor = next;
        }
    }
    pub async fn delete(&self, namespace: &str, key: &str, version: u64) -> anyhow::Result<()> {
        self.transaction(vec![
            json!({"op":"delete","namespace":namespace,"key":key,"expected_version":version}),
        ])
        .await
        .context("delete record")
    }
}
