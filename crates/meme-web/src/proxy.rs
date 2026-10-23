use std::time::Duration;

use axum::{
    Json,
    body::{Body, to_bytes},
    extract::Request,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use reqwest::redirect::Policy;
use url::Url;

use crate::config::WebConfig;

pub(crate) const REQUEST_LIMIT: usize = 25 * 1024 * 1024;
pub(crate) const RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
pub(crate) const IMAGE_LIMIT: usize = 5 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
const PUBLIC_HEADERS: &[&str] = &["cookie", "origin", "x-csrf-token", "content-type"];
const WEBHOOK_HEADERS: &[&str] = &[
    "content-type",
    "x-hub-signature",
    "x-hub-signature-256",
    "x-github-delivery",
    "x-github-event",
    "x-github-hook-id",
    "x-github-hook-installation-target-id",
    "x-github-hook-installation-target-type",
];
const RESPONSE_HEADERS: &[&str] = &[
    "set-cookie",
    "location",
    "content-type",
    "cache-control",
    "x-content-type-options",
];

#[derive(Clone)]
pub(crate) struct AppState {
    pub config: WebConfig,
    client: reqwest::Client,
}

impl AppState {
    pub fn new(config: WebConfig) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(TIMEOUT)
            .no_proxy()
            .build()?;
        Ok(Self { config, client })
    }
}

#[derive(Clone, Copy)]
pub(crate) enum QueryPolicy {
    None,
    Gallery,
    Callback,
}

pub(crate) struct ProxySpec {
    pub path: String,
    pub query: QueryPolicy,
    pub response_limit: usize,
    github: bool,
    redirects: bool,
    headers: &'static [&'static str],
}

impl ProxySpec {
    pub fn core(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            query: QueryPolicy::None,
            response_limit: RESPONSE_LIMIT,
            github: false,
            redirects: false,
            headers: PUBLIC_HEADERS,
        }
    }

    pub fn auth(path: &str) -> Self {
        Self {
            github: true,
            redirects: true,
            ..Self::core(path)
        }
    }

    pub fn avatar() -> Self {
        Self {
            github: true,
            headers: &["cookie"],
            response_limit: 256 * 1024,
            ..Self::core("/auth/avatar")
        }
    }

    pub fn webhook() -> Self {
        Self {
            github: true,
            headers: WEBHOOK_HEADERS,
            ..Self::core("/webhooks/github")
        }
    }
}

pub(crate) async fn forward(state: AppState, request: Request, spec: ProxySpec) -> Response {
    match forward_checked(&state, request, spec).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn forward_checked(
    state: &AppState,
    request: Request,
    spec: ProxySpec,
) -> Result<Response, ProxyError> {
    let (parts, body) = request.into_parts();
    let public_host = parts
        .headers
        .get("host")
        .and_then(|value| value.to_str().ok());
    let target = target_url(state, &spec, parts.uri.query());
    let headers = select_headers(&parts.headers, spec.headers);
    let body = tokio::time::timeout(TIMEOUT, to_bytes(body, REQUEST_LIMIT))
        .await
        .map_err(|_| ProxyError::request_timeout())?
        .map_err(|_| ProxyError::too_large())?;
    let upstream = state
        .client
        .request(parts.method, target)
        .headers(headers)
        .body(body)
        .send()
        .await
        .map_err(ProxyError::upstream)?;
    check_redirect(&upstream, spec.redirects, public_host)?;
    let status = upstream.status();
    let headers = select_headers(upstream.headers(), RESPONSE_HEADERS);
    let body = bounded_response(upstream, spec.response_limit).await?;
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
        .headers_mut()
        .entry("cache-control")
        .or_insert_with(|| "no-store".parse().expect("static header"));
    Ok(response)
}

fn target_url(state: &AppState, spec: &ProxySpec, query: Option<&str>) -> Url {
    let base = if spec.github {
        &state.config.github_url
    } else {
        &state.config.core_url
    };
    let mut target = base.clone();
    target.set_path(&format!(
        "{}{}",
        base.path().trim_end_matches('/'),
        spec.path
    ));
    match spec.query {
        QueryPolicy::None => {}
        QueryPolicy::Callback => target.set_query(query),
        QueryPolicy::Gallery => gallery_query(&mut target, query.unwrap_or_default()),
    }
    target
}

fn gallery_query(target: &mut Url, query: &str) {
    let pairs = url::form_urlencoded::parse(query.as_bytes());
    let allowed =
        pairs.filter(|(key, _)| matches!(key.as_ref(), "query" | "cursor" | "limit" | "section"));
    target.query_pairs_mut().extend_pairs(allowed);
}

fn select_headers(source: &HeaderMap, allowed: &[&'static str]) -> HeaderMap {
    let mut selected = HeaderMap::new();
    for &name in allowed {
        if connection_names(source, name) {
            continue;
        }
        for value in source.get_all(name) {
            selected.append(name, value.clone());
        }
    }
    selected
}

fn connection_names(headers: &HeaderMap, name: &str) -> bool {
    headers.get_all("connection").iter().any(|value| {
        value
            .to_str()
            .unwrap_or_default()
            .split(',')
            .any(|part| part.trim().eq_ignore_ascii_case(name))
    })
}

async fn bounded_response(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, ProxyError> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(ProxyError::oversized_response());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(ProxyError::upstream)? {
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err(ProxyError::oversized_response());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn check_redirect(
    response: &reqwest::Response,
    allow_auth: bool,
    public_host: Option<&str>,
) -> Result<(), ProxyError> {
    if !response.status().is_redirection() {
        return Ok(());
    }
    for value in response.headers().get_all("location") {
        let location = value.to_str().map_err(|_| ProxyError::redirect())?;
        if !allow_auth || !safe_auth_location(location, public_host) {
            return Err(ProxyError::redirect());
        }
    }
    Ok(())
}

fn safe_auth_location(value: &str, public_host: Option<&str>) -> bool {
    if value.contains('\\') || value.chars().any(char::is_control) {
        return false;
    }
    if value.starts_with('/') {
        return !value.starts_with("//");
    }
    Url::parse(value).is_ok_and(|url| allowed_auth_url(&url, public_host))
}

fn github_location(url: &Url) -> bool {
    let host_allowed = url.host_str() == Some("github.com");
    let transport_allowed = url.scheme() == "https" && url.port_or_known_default() == Some(443);
    let no_credentials = url.username().is_empty() && url.password().is_none();
    host_allowed && transport_allowed && no_credentials
}

fn allowed_auth_url(url: &Url, public_host: Option<&str>) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    github_location(url) || public_host.is_some_and(|host| same_public_origin(url, host))
}

fn same_public_origin(url: &Url, host: &str) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let expected = Url::parse(&format!("{}://{host}/", url.scheme()));
    expected.is_ok_and(|expected| expected.origin() == url.origin())
}

pub(crate) struct ProxyError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl ProxyError {
    fn upstream(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::new(
                StatusCode::GATEWAY_TIMEOUT,
                "upstream_timeout",
                crate::locales::english("error.upstream_timeout"),
            )
        } else {
            Self::new(
                StatusCode::BAD_GATEWAY,
                "upstream_unavailable",
                crate::locales::english("error.upstream_unavailable"),
            )
        }
    }

    fn too_large() -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_too_large",
            crate::locales::english("error.request_too_large"),
        )
    }

    fn request_timeout() -> Self {
        Self::new(
            StatusCode::REQUEST_TIMEOUT,
            "request_timeout",
            crate::locales::english("error.request_timeout"),
        )
    }

    fn oversized_response() -> Self {
        Self::new(
            StatusCode::BAD_GATEWAY,
            "response_too_large",
            crate::locales::english("error.response_too_large"),
        )
    }

    fn redirect() -> Self {
        Self::new(
            StatusCode::BAD_GATEWAY,
            "unsafe_redirect",
            crate::locales::english("error.unsafe_redirect"),
        )
    }

    fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
        }
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": { "code": self.code, "message": self.message } });
        let mut response = (self.status, Json(body)).into_response();
        response
            .headers_mut()
            .insert("cache-control", "no-store".parse().expect("static header"));
        response
    }
}
