use crate::{
    App, Work,
    config::{allow, authorize},
    indexes::{Declaration, IndexQuery, Work as IndexWork},
    model::{ApiError, validate_location},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde_json::Value;

fn access(app: &App, headers: &HeaderMap, namespace: &str, name: &str) -> Result<(), ApiError> {
    allow(&authorize(&app.principals, headers)?, namespace)?;
    validate_location(namespace, "_")?;
    validate_location(name, "_")
}
pub async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    Path(namespace): Path<String>,
) -> Result<Json<Value>, ApiError> {
    access(&app, &headers, &namespace, "_")?;
    Ok(Json(
        app.call(Work::Index(namespace, IndexWork::List)).await?,
    ))
}
pub async fn declare(
    State(app): State<App>,
    headers: HeaderMap,
    Path((namespace, name)): Path<(String, String)>,
    Json(declaration): Json<Declaration>,
) -> Result<Json<Value>, ApiError> {
    access(&app, &headers, &namespace, &name)?;
    Ok(Json(
        app.call(Work::Index(
            namespace,
            IndexWork::Declare(name, declaration),
        ))
        .await?,
    ))
}
pub async fn remove(
    State(app): State<App>,
    headers: HeaderMap,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    access(&app, &headers, &namespace, &name)?;
    Ok(Json(
        app.call(Work::Index(namespace, IndexWork::Remove(name)))
            .await?,
    ))
}
pub async fn query(
    State(app): State<App>,
    headers: HeaderMap,
    Path((namespace, name)): Path<(String, String)>,
    Json(query): Json<IndexQuery>,
) -> Result<Json<Value>, ApiError> {
    access(&app, &headers, &namespace, &name)?;
    Ok(Json(
        app.call(Work::Index(namespace, IndexWork::Query(name, query)))
            .await?,
    ))
}
