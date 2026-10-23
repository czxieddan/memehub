use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};

use crate::proxy::{self, AppState, IMAGE_LIMIT, ProxySpec, QueryPolicy};

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(health))
        .route("/api/gallery", get(gallery).head(unsupported_method))
        .route("/api/policy", get(policy).head(unsupported_method))
        .route("/api/memes/{hash}", get(meme).head(unsupported_method))
        .route("/api/memes/{hash}/view", post(view))
        .route("/api/images/{hash}", get(image).head(unsupported_method))
        .route("/api/me", get(me).head(unsupported_method))
        .route("/api/avatar", get(avatar).head(avatar_head))
        .route(
            "/api/submissions",
            get(submissions).post(submissions).head(unsupported_method),
        )
        .route("/auth/github", get(login).head(unsupported_method))
        .route("/auth/callback", get(callback).head(unsupported_method))
        .route("/auth/logout", post(logout))
        .route("/webhooks/github", post(webhook))
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

async fn policy(State(state): State<AppState>, request: Request) -> Response {
    proxy::forward(state, request, ProxySpec::core("/v1/policy")).await
}

async fn meme(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    request: Request,
) -> Response {
    if !valid_hash(&hash) {
        return not_found().await;
    }
    proxy::forward(state, request, ProxySpec::core(format!("/v1/memes/{hash}"))).await
}

async fn view(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    request: Request,
) -> Response {
    if !valid_hash(&hash) {
        return not_found().await;
    }
    proxy::forward(
        state,
        request,
        ProxySpec::core(format!("/v1/memes/{hash}/view")),
    )
    .await
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

async fn avatar(State(state): State<AppState>, request: Request) -> Response {
    let response = if request.uri().query().is_some() {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": {"code": "avatar_query", "message": "Avatar query parameters are not supported"}})),
        )
            .into_response()
    } else {
        let (parts, _) = request.into_parts();
        let request = Request::from_parts(parts, Body::empty());
        proxy::forward(state, request, ProxySpec::avatar()).await
    };
    avatar_response(response)
}

async fn avatar_head() -> Response {
    avatar_response(StatusCode::METHOD_NOT_ALLOWED.into_response())
}

fn avatar_response(mut response: Response) -> Response {
    response.headers_mut().insert(
        "cache-control",
        "private, no-store".parse().expect("static header"),
    );
    response
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

pub(crate) async fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": {"code": "not_found", "message": crate::locales::english("error.not_found")}})),
    )
        .into_response()
}

async fn unsupported_method() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}
