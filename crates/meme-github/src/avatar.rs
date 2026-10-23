use std::time::Duration;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};

use crate::{App, error::Error, session};

mod image;

const LIMIT: usize = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(8);
const TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

pub async fn get(State(app): State<App>, headers: HeaderMap, uri: Uri) -> Response {
    let result = tokio::time::timeout(TIMEOUT, current(&app, &headers, &uri))
        .await
        .unwrap_or_else(|_| Err(timed_out()));
    private(result.unwrap_or_else(IntoResponse::into_response))
}

pub async fn head() -> Response {
    private(StatusCode::METHOD_NOT_ALLOWED.into_response())
}

fn private(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn current(app: &App, headers: &HeaderMap, uri: &Uri) -> Result<Response, Error> {
    let upstream = authenticated_request(app, headers, uri)
        .await?
        .send()
        .await
        .map_err(upstream_error)?;
    read_image(upstream).await
}

async fn authenticated_request(
    app: &App,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<reqwest::RequestBuilder, Error> {
    let identity = session::identity(app, headers).await?;
    if uri.query().is_some() {
        return Err(Error::bad("Avatar query parameters are not supported"));
    }
    request(identity.user.id)
}

fn source(user_id: u64) -> Result<String, Error> {
    if user_id == 0 {
        return Err(Error::unauthorized());
    }
    Ok(format!(
        "https://avatars.githubusercontent.com/u/{user_id}?s=96&v=4"
    ))
}

fn request(user_id: u64) -> Result<reqwest::RequestBuilder, Error> {
    let url = source(user_id)?;
    // Do not reuse the authenticated GitHub/DB client or accept an upstream URL.
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(TIMEOUT)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .user_agent("memehub/0.1")
        .build()
        .map_err(upstream_error)?;
    Ok(client
        .get(url)
        .header(
            header::ACCEPT,
            "image/png, image/jpeg, image/gif, image/webp",
        )
        .header(header::ACCEPT_ENCODING, "identity"))
}

async fn read_image(response: reqwest::Response) -> Result<Response, Error> {
    if response.status() != StatusCode::OK {
        return Err(unavailable());
    }
    reject_encoding(response.headers())?;
    let content_type = content_type(response.headers())?;
    let body = bounded_body(response).await?;
    if !image::valid(content_type, &body) {
        return Err(unavailable());
    }
    let mut response = Response::new(Body::from(body));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    Ok(response)
}

fn reject_encoding(headers: &HeaderMap) -> Result<(), Error> {
    if headers.contains_key(header::CONTENT_ENCODING) {
        return Err(unavailable());
    }
    Ok(())
}

fn content_type(headers: &HeaderMap) -> Result<&'static str, Error> {
    let mut values = headers.get_all(header::CONTENT_TYPE).iter();
    let value = values.next().ok_or_else(unavailable)?;
    if values.next().is_some() {
        return Err(unavailable());
    }
    let value = value.to_str().map_err(|_| unavailable())?;
    let mime = value.split(';').next().unwrap_or_default().trim();
    TYPES
        .iter()
        .copied()
        .find(|allowed| allowed.eq_ignore_ascii_case(mime))
        .ok_or_else(unavailable)
}

async fn bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, Error> {
    if response
        .content_length()
        .is_some_and(|size| size > LIMIT as u64)
    {
        return Err(unavailable());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(upstream_error)? {
        if chunk.len() > LIMIT.saturating_sub(body.len()) {
            return Err(unavailable());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn upstream_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        timed_out()
    } else {
        unavailable()
    }
}

fn unavailable() -> Error {
    Error(
        StatusCode::BAD_GATEWAY,
        "GitHub avatar is unavailable".into(),
    )
}

fn timed_out() -> Error {
    Error(
        StatusCode::GATEWAY_TIMEOUT,
        "GitHub avatar request timed out".into(),
    )
}
