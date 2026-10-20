use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{mpsc, oneshot, watch};
mod config;
mod engine;
mod index_routes;
mod indexes;
mod model;
use config::{Config, Principal, allow, authorize};
use engine::Engine;
use model::{ApiError, ListQuery, Operation, Write, validate_location};

#[derive(Clone)]
struct App {
    actor: mpsc::Sender<Message>,
    principals: Arc<Vec<Principal>>,
}
struct Message {
    work: Work,
    reply: oneshot::Sender<Result<Value, ApiError>>,
}
enum Work {
    Get(String, String),
    List(String, ListQuery),
    Transaction(Vec<Operation>),
    Index(String, indexes::Work),
    Backup,
}
impl App {
    async fn call(&self, work: Work) -> Result<Value, ApiError> {
        let (reply, result) = oneshot::channel();
        self.actor.try_send(Message { work, reply }).map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "database busy or stopped".into(),
            )
        })?;
        result
            .await
            .map_err(|_| anyhow::anyhow!("database worker stopped"))?
    }
}
pub async fn run(path: PathBuf, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
    let config = Config::load(&path)?;
    let principals = Arc::new(config.principals()?);
    let (actor, receiver) = mpsc::channel(32);
    let (ready, wait) = oneshot::channel();
    let worker_config = config.clone();
    let worker = std::thread::spawn(move || worker(worker_config, receiver, ready));
    wait.await??;
    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    tracing::info!(listen = %config.listen, "database listening");
    let app = router(App { actor, principals });
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.changed().await;
        })
        .await?;
    tokio::task::spawn_blocking(move || {
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("database worker panicked"))
    })
    .await??;
    Ok(())
}
fn worker(
    config: Config,
    mut receiver: mpsc::Receiver<Message>,
    ready: oneshot::Sender<anyhow::Result<()>>,
) {
    let mut engine = match Engine::open(&config) {
        Ok(value) => value,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    while let Some(message) = receiver.blocking_recv() {
        let result = perform(&mut engine, &config, message.work);
        let _ = message.reply.send(result);
    }
}
fn perform(engine: &mut Engine, config: &Config, work: Work) -> Result<Value, ApiError> {
    match work {
        Work::Get(namespace, key) => engine
            .get(&namespace, &key)?
            .map(|r| json!(r))
            .ok_or(ApiError(StatusCode::NOT_FOUND, "record not found".into())),
        Work::List(namespace, query) => engine.list(&namespace, &query),
        Work::Transaction(operations) => engine.transaction(&operations),
        Work::Index(namespace, work) => engine.index(&namespace, work),
        Work::Backup => engine.backup(config),
    }
}
fn router(state: App) -> Router {
    Router::new()
        .route(
            "/health/live",
            get(|| async { Json(json!({"status":"ok"})) }),
        )
        .route("/v1/namespaces/{namespace}/records", get(list))
        .route(
            "/v1/namespaces/{namespace}/records/{key}",
            get(read).put(write).delete(remove),
        )
        .route(
            "/v1/namespaces/{namespace}/indexes",
            get(index_routes::list),
        )
        .route(
            "/v1/namespaces/{namespace}/indexes/{name}",
            put(index_routes::declare).delete(index_routes::remove),
        )
        .route(
            "/v1/namespaces/{namespace}/indexes/{name}/query",
            post(index_routes::query),
        )
        .route("/v1/transactions", post(transaction))
        .route("/v1/admin/backups", post(backup))
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
        .with_state(state)
}
async fn read(
    State(app): State<App>,
    headers: HeaderMap,
    Path((namespace, key)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    allow(&authorize(&app.principals, &headers)?, &namespace)?;
    validate_location(&namespace, &key)?;
    Ok(Json(app.call(Work::Get(namespace, key)).await?))
}
async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    Path(namespace): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    allow(&authorize(&app.principals, &headers)?, &namespace)?;
    validate_location(&namespace, "_")?;
    Ok(Json(app.call(Work::List(namespace, query)).await?))
}
async fn write(
    State(app): State<App>,
    headers: HeaderMap,
    Path((namespace, key)): Path<(String, String)>,
    Json(body): Json<Write>,
) -> Result<Json<Value>, ApiError> {
    transact(
        app,
        headers,
        vec![Operation::Put {
            namespace,
            key,
            value: body.value,
            expected_version: body.expected_version,
        }],
    )
    .await
}
#[derive(Deserialize)]
struct DeleteQuery {
    expected_version: Option<u64>,
}
async fn remove(
    State(app): State<App>,
    headers: HeaderMap,
    Path((namespace, key)): Path<(String, String)>,
    Query(query): Query<DeleteQuery>,
) -> Result<Json<Value>, ApiError> {
    transact(
        app,
        headers,
        vec![Operation::Delete {
            namespace,
            key,
            expected_version: query.expected_version,
        }],
    )
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    operations: Vec<Operation>,
}
async fn transaction(
    State(app): State<App>,
    headers: HeaderMap,
    Json(batch): Json<Batch>,
) -> Result<Json<Value>, ApiError> {
    transact(app, headers, batch.operations).await
}
async fn transact(
    app: App,
    headers: HeaderMap,
    operations: Vec<Operation>,
) -> Result<Json<Value>, ApiError> {
    let principal = authorize(&app.principals, &headers)?;
    if operations.is_empty() || operations.len() > 10000 {
        return Err(ApiError::bad("transaction requires 1..10000 operations"));
    }
    for operation in &operations {
        validate_operation(&principal, operation)?;
    }
    Ok(Json(app.call(Work::Transaction(operations)).await?))
}
fn validate_operation(principal: &Principal, operation: &Operation) -> Result<(), ApiError> {
    let (namespace, key) = operation.location();
    allow(principal, namespace)?;
    validate_location(namespace, key)?;
    if let Operation::Put { value, .. } = operation {
        if value.to_string().len() > 65536 {
            return Err(ApiError::bad("record exceeds 64 KiB"));
        }
    }
    Ok(())
}
async fn backup(State(app): State<App>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    if !authorize(&app.principals, &headers)?.admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "administrator required".into(),
        ));
    }
    Ok(Json(app.call(Work::Backup).await?))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod index_tests;
