use axum::{
    Json, Router,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};

use crate::{
    assets,
    proxy::{self, AppState, IMAGE_LIMIT, ProxySpec, QueryPolicy},
};

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(assets::index))
        .route("/assets/app.css", get(assets::css))
        .route("/assets/app.js", get(assets::js))
        .route("/assets/i18n.js", get(assets::i18n))
        .route("/assets/locales/{name}", get(assets::locale))
        .route("/assets/icon.svg", get(assets::icon))
        .route("/source", get(assets::source))
        .route("/health/live", get(health))
        .route("/api/gallery", get(gallery).head(unsupported_method))
        .route("/api/images/{hash}", get(image).head(unsupported_method))
        .route("/api/me", get(me).head(unsupported_method))
        .route(
            "/api/submissions",
            get(submissions).post(submissions).head(unsupported_method),
        )
        .route("/auth/github", get(login).head(unsupported_method))
        .route("/auth/callback", get(callback).head(unsupported_method))
        .route("/auth/logout", post(logout))
        .route("/webhooks/github", post(webhook))
        .fallback(not_found)
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

async fn gallery(State(state): State<AppState>, request: Request) -> Response {
    let mut spec = ProxySpec::core("/v1/gallery");
    spec.query = QueryPolicy::Gallery;
    proxy::forward(state, request, spec).await
}

async fn image(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    request: Request,
) -> Response {
    if !valid_hash(&hash) {
        return not_found().await;
    }
    let mut spec = ProxySpec::core(format!("/v1/images/{hash}"));
    spec.response_limit = IMAGE_LIMIT;
    proxy::forward(state, request, spec).await
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

async fn me(State(state): State<AppState>, request: Request) -> Response {
    proxy::forward(state, request, ProxySpec::core("/v1/me")).await
}

async fn submissions(State(state): State<AppState>, request: Request) -> Response {
    proxy::forward(state, request, ProxySpec::core("/v1/submissions")).await
}

async fn login(State(state): State<AppState>, request: Request) -> Response {
    proxy::forward(state, request, ProxySpec::auth("/auth/github")).await
}

async fn callback(State(state): State<AppState>, request: Request) -> Response {
    let mut spec = ProxySpec::auth("/auth/callback");
    spec.query = QueryPolicy::Callback;
    proxy::forward(state, request, spec).await
}

async fn logout(State(state): State<AppState>, request: Request) -> Response {
    proxy::forward(state, request, ProxySpec::auth("/auth/logout")).await
}

async fn webhook(State(state): State<AppState>, request: Request) -> Response {
    proxy::forward(state, request, ProxySpec::webhook()).await
}

async fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": {"code": "not_found", "message": crate::locales::english("error.not_found")}})),
    )
        .into_response()
}

async fn unsupported_method() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}
