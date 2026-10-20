use super::*;
use crate::{
    cache::{Cache, Policy},
    config::Config,
    database::Database,
    github::Gateway,
};
use axum::{Router, extract::State, routing::put};
use std::sync::Arc;
use tokio::sync::Mutex;

fn submission() -> Submission {
    Submission {
        id: uuid::Uuid::new_v4().to_string(),
        uploader_id: 42,
        name: "sample".into(),
        display_name: "示例".into(),
        filename: "image.png".into(),
        status: "pending".into(),
        created_at: 1,
        attempts: 0,
        next_attempt_at: 0,
        pr_number: None,
        pr_url: None,
        error: None,
    }
}
#[derive(Clone)]
struct Mock {
    write_status: StatusCode,
    read_status: StatusCode,
    record: Value,
}
async fn mock_write(State(mock): State<Mock>) -> StatusCode {
    mock.write_status
}
async fn mock_read(State(mock): State<Mock>) -> (StatusCode, Json<Value>) {
    (mock.read_status, Json(mock.record))
}
async fn fixture(mock: Mock) -> (tempfile::TempDir, App, tokio::task::JoinHandle<()>) {
    let temp = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .route(
            "/v1/namespaces/coresubmissions/records/{id}",
            put(mock_write).get(mock_read),
        )
        .with_state(mock);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    #[derive(serde::Deserialize)]
    struct File {
        core: Config,
    }
    let mut config = toml::from_str::<File>(include_str!("../../../config/example.toml"))
        .unwrap()
        .core;
    config.staging_dir = temp.path().join("staging");
    let http = reqwest::Client::new();
    let app = App {
        config: Arc::new(config),
        db: Database::new(http.clone(), base.clone(), "a".repeat(48)),
        github: Gateway::new(http, base, "b".repeat(48)),
        cache: Arc::new(Mutex::new(Cache::new(Policy {
            max_bytes: 1024,
            max_entries: 10,
            ttl_seconds: 60,
        }))),
        admin_token: "c".repeat(48),
        publication: Arc::new(Mutex::new(())),
        jobs: Arc::new(Mutex::new(())),
        settings: Arc::new(Mutex::new(())),
        download: Arc::new(Mutex::new(())),
    };
    (temp, app, server)
}
async fn check(
    write_status: StatusCode,
    read_status: StatusCode,
    expected_success: bool,
    expected_file: bool,
) {
    let task = submission();
    let mock = Mock {
        write_status,
        read_status,
        record: json!({"key":task.id,"version":1,"value":task}),
    };
    let (_temp, app, server) = fixture(mock).await;
    save_file(&app, &task.id, b"image data").await.unwrap();
    assert_eq!(persist(&app, &task).await.is_ok(), expected_success);
    assert_eq!(
        app.config.staging_dir.join(&task.id).exists(),
        expected_file
    );
    server.abort();
}
#[tokio::test]
async fn successful_insert_keeps_staged_image() {
    check(StatusCode::OK, StatusCode::NOT_FOUND, true, true).await;
}
#[tokio::test]
async fn ambiguous_insert_recovers_the_existing_identical_job() {
    check(StatusCode::SERVICE_UNAVAILABLE, StatusCode::OK, true, true).await;
}
#[tokio::test]
async fn confirmed_missing_job_removes_staged_image() {
    check(
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::NOT_FOUND,
        false,
        false,
    )
    .await;
}
#[tokio::test]
async fn unavailable_database_keeps_staged_image_for_recovery() {
    check(
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::SERVICE_UNAVAILABLE,
        false,
        true,
    )
    .await;
}
#[tokio::test]
async fn staged_files_use_create_new_and_preserve_existing_bytes() {
    let mock = Mock {
        write_status: StatusCode::OK,
        read_status: StatusCode::NOT_FOUND,
        record: Value::Null,
    };
    let (_temp, app, server) = fixture(mock).await;
    let id = uuid::Uuid::new_v4().to_string();
    save_file(&app, &id, b"original").await.unwrap();
    assert!(save_file(&app, &id, b"replacement").await.is_err());
    assert_eq!(
        tokio::fs::read(app.config.staging_dir.join(id))
            .await
            .unwrap(),
        b"original"
    );
    server.abort();
}
