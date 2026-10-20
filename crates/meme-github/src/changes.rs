use crate::{
    App,
    error::Error,
    remote::{text, validate_path},
    session::{User, credential},
};
use anyhow::{Context, ensure};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub operation_id: String,
    pub repository: String,
    pub base_branch: String,
    pub title: String,
    pub body: String,
    pub message: String,
    pub coauthor_user_id: Option<u64>,
    pub files: Vec<FileChange>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileChange {
    pub path: String,
    pub content_base64: String,
}

pub async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Json(change): Json<Change>,
) -> Result<Json<Value>, Error> {
    credential(&headers, &app.service_token)?;
    validate(&app, &change)?;
    let _lock = app.mutation.lock().await;
    let branch = format!("changes/{}", change.operation_id);
    if let Some(existing) = app.db.get("ghoperations", &change.operation_id).await? {
        if existing.value["repository"] != change.repository {
            return Err(Error::bad("operation belongs to another repository"));
        }
        return Ok(Json(existing.value));
    }
    let pull = match find_pull(&app, &change.repository, &branch).await? {
        Some(pull) => pull,
        None => create_pull(&app, &change, &branch).await?,
    };
    let result = json!({"operation_id":change.operation_id,"repository":change.repository,"number":pull["number"],"url":pull["html_url"],"branch":branch});
    app.db
        .put("ghoperations", &change.operation_id, &result, Some(0))
        .await?;
    Ok(Json(result))
}
fn validate(app: &App, change: &Change) -> Result<(), Error> {
    app.config.repository(&change.repository)?;
    uuid::Uuid::parse_str(&change.operation_id)
        .map_err(|_| Error::bad("operation ID must be UUID"))?;
    if change.files.is_empty() || change.files.len() > 16 {
        return Err(Error::bad("changes require 1..16 files"));
    }
    if change.title.is_empty() || change.title.len() > 256 {
        return Err(Error::bad("invalid title"));
    }
    let mut size = 0;
    for file in &change.files {
        validate_path(&file.path)?;
        if !app
            .config
            .allowed_path_prefixes
            .iter()
            .any(|prefix| file.path.starts_with(prefix))
        {
            return Err(Error(StatusCode::FORBIDDEN, "path not authorized".into()));
        }
        size += file.content_base64.len();
    }
    if size > 28 * 1024 * 1024 {
        return Err(Error::bad("change exceeds size limit"));
    }
    Ok(())
}
async fn find_pull(app: &App, repo: &str, branch: &str) -> anyhow::Result<Option<Value>> {
    let owner = repo.split('/').next().context("repository owner")?;
    let data = app
        .remote
        .required(
            Method::GET,
            &format!("repos/{repo}/pulls?state=all&head={owner}:{branch}&per_page=100"),
            None,
        )
        .await?;
    Ok(data
        .as_array()
        .context("pull list")?
        .iter()
        .find(|v| v.pointer("/head/ref").and_then(Value::as_str) == Some(branch))
        .cloned())
}
async fn create_pull(app: &App, change: &Change, branch: &str) -> anyhow::Result<Value> {
    let repo = &change.repository;
    let existing = app
        .remote
        .request(
            Method::GET,
            &format!("repos/{repo}/git/ref/heads/{branch}"),
            None,
        )
        .await?;
    if existing.is_none() {
        create_branch(app, change, branch).await?;
    }
    let body = json!({"title":change.title,"body":change.body,"head":branch,"base":change.base_branch,"maintainer_can_modify":true});
    app.remote
        .required(Method::POST, &format!("repos/{repo}/pulls"), Some(&body))
        .await
}
async fn create_branch(app: &App, change: &Change, branch: &str) -> anyhow::Result<()> {
    let repo = &change.repository;
    let mut branch_url = app.remote.url(&format!(
        "repos/{repo}/git/ref/heads/{}",
        change.base_branch
    ))?;
    branch_url.set_query(None);
    let base = app
        .remote
        .required(Method::GET, branch_url.path().trim_start_matches('/'), None)
        .await?;
    let sha = base
        .pointer("/object/sha")
        .and_then(Value::as_str)
        .context("base commit")?;
    let commit = app
        .remote
        .required(
            Method::GET,
            &format!("repos/{repo}/git/commits/{sha}"),
            None,
        )
        .await?;
    let tree = commit
        .pointer("/tree/sha")
        .and_then(Value::as_str)
        .context("base tree")?;
    let files = make_blobs(app, change).await?;
    let tree = app
        .remote
        .required(
            Method::POST,
            &format!("repos/{repo}/git/trees"),
            Some(&json!({"base_tree":tree,"tree":files})),
        )
        .await?;
    let message = commit_message(app, change).await?;
    let commit = app
        .remote
        .required(
            Method::POST,
            &format!("repos/{repo}/git/commits"),
            Some(&json!({"message":message,"tree":text(&tree,"sha")?,"parents":[sha]})),
        )
        .await?;
    app.remote
        .required(
            Method::POST,
            &format!("repos/{repo}/git/refs"),
            Some(&json!({"ref":format!("refs/heads/{branch}"),"sha":text(&commit,"sha")?})),
        )
        .await?;
    Ok(())
}
async fn make_blobs(app: &App, change: &Change) -> anyhow::Result<Vec<Value>> {
    let mut entries = Vec::new();
    for file in &change.files {
        let bytes = STANDARD.decode(&file.content_base64)?;
        ensure!(bytes.len() <= 6 * 1024 * 1024, "file too large");
        let blob = app
            .remote
            .required(
                Method::POST,
                &format!("repos/{}/git/blobs", change.repository),
                Some(&json!({"content":file.content_base64,"encoding":"base64"})),
            )
            .await?;
        entries
            .push(json!({"path":file.path,"mode":"100644","type":"blob","sha":text(&blob,"sha")?}));
    }
    Ok(entries)
}
async fn commit_message(app: &App, change: &Change) -> anyhow::Result<String> {
    let mut message = format!(
        "{}\n\nMemehub-Operation: {}",
        change.message, change.operation_id
    );
    if let Some(id) = change.coauthor_user_id {
        let user: User = app
            .db
            .value("ghusers", &id.to_string())
            .await?
            .context("coauthor identity not found")?;
        ensure!(!user.revoked, "coauthor authorization revoked");
        ensure!(
            !user.contribution_email.contains(['\r', '\n', '<', '>']),
            "invalid coauthor email"
        );
        message.push_str(&format!(
            "\n\nCo-authored-by: {} <{}>",
            user.login, user.contribution_email
        ));
    }
    Ok(message)
}
#[derive(Deserialize)]
pub struct RepoQuery {
    pub repository: String,
}
pub async fn pull(
    State(app): State<App>,
    headers: HeaderMap,
    Path(number): Path<u64>,
    Query(query): Query<RepoQuery>,
) -> Result<Json<Value>, Error> {
    credential(&headers, &app.service_token)?;
    app.config.repository(&query.repository)?;
    let data = app
        .remote
        .required(
            Method::GET,
            &format!("repos/{}/pulls/{number}", query.repository),
            None,
        )
        .await?;
    Ok(Json(
        json!({"number":data["number"],"state":data["state"],"merged":data["merged"],"url":data["html_url"],"title":data["title"],"body":data["body"],"user_id":data.pointer("/user/id")}),
    ))
}
#[derive(Deserialize)]
pub struct PullsQuery {
    pub repository: String,
    pub page: Option<u32>,
}
pub async fn pulls(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<PullsQuery>,
) -> Result<Json<Value>, Error> {
    credential(&headers, &app.service_token)?;
    app.config.repository(&query.repository)?;
    let page = query.page.unwrap_or(1).clamp(1, 10000);
    let values = app
        .remote
        .required(
            Method::GET,
            &format!(
                "repos/{}/pulls?state=all&sort=updated&direction=desc&per_page=100&page={page}",
                query.repository
            ),
            None,
        )
        .await?;
    let values = values.as_array().context("pull list")?;
    let items: Vec<Value> = values.iter().map(|data| json!({"number":data["number"],"state":data["state"],"merged":!data["merged_at"].is_null(),"url":data["html_url"],"title":data["title"],"user_id":data.pointer("/user/id")})).collect();
    Ok(Json(
        json!({"items":items,"next_page":if values.len()==100 { Some(page+1) } else { None }}),
    ))
}
#[derive(Deserialize)]
pub struct ContentQuery {
    pub repository: String,
    pub path: String,
    pub reference: String,
}
pub async fn content(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<ContentQuery>,
) -> Result<Json<Value>, Error> {
    credential(&headers, &app.service_token)?;
    let bytes = app
        .remote
        .content(&query.repository, &query.path, &query.reference)
        .await?
        .ok_or(Error(
            StatusCode::NOT_FOUND,
            "repository file not found".into(),
        ))?;
    Ok(Json(json!({"content_base64":STANDARD.encode(bytes)})))
}
