use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tower::ServiceExt;

use crate::{
    config,
    proxy::{AppState, IMAGE_LIMIT, REQUEST_LIMIT, RESPONSE_LIMIT},
    routes,
};

type Observation = (String, HeaderMap, Bytes);

#[derive(Clone)]
struct FakeState {
    observations: mpsc::UnboundedSender<Observation>,
    calls: Arc<AtomicUsize>,
    mode: &'static str,
}

struct FakeServer {
    url: String,
    observations: mpsc::UnboundedReceiver<Observation>,
    calls: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn fake_server(mode: &'static str) -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (sender, observations) = mpsc::unbounded_channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let state = FakeState {
        observations: sender,
        calls: calls.clone(),
        mode,
    };
    let app = Router::new().fallback(any(fake_upstream)).with_state(state);
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    FakeServer {
        url,
        observations,
        calls,
        task,
    }
}

async fn fake_upstream(State(state): State<FakeState>, request: Request) -> Response {
    state.calls.fetch_add(1, Ordering::SeqCst);
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, REQUEST_LIMIT).await.unwrap();
    state
        .observations
        .send((parts.uri.to_string(), parts.headers, bytes))
        .unwrap();
    match state.mode {
        "redirect" => (
            StatusCode::FOUND,
            [(header::LOCATION, "/followed")],
            "redirect",
        )
            .into_response(),
        "github" => (
            StatusCode::FOUND,
            [(
                header::LOCATION,
                "https://github.com/login/oauth/authorize?client_id=test",
            )],
            "",
        )
            .into_response(),
        "evil" => (
            StatusCode::FOUND,
            [(header::LOCATION, "https://example.invalid/steal")],
            "",
        )
            .into_response(),
        "oversized" => Body::from(vec![0; RESPONSE_LIMIT + 1]).into_response(),
        "image_big" => Body::from(vec![0; IMAGE_LIMIT + 1]).into_response(),
        "error" => (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "application/json")],
            "{\"error\":{\"message\":\"invalid image\"}}",
        )
            .into_response(),
        _ => successful_response(),
    }
}

fn successful_response() -> Response {
    let mut response = (
        StatusCode::ACCEPTED,
        [(header::CONTENT_TYPE, "application/json")],
        "{\"id\":\"submission\"}",
    )
        .into_response();
    let headers = response.headers_mut();
    headers.append(
        header::SET_COOKIE,
        "session=one; HttpOnly; Path=/".parse().unwrap(),
    );
    headers.append(header::SET_COOKIE, "other=two; Path=/".parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    headers.insert("x-internal-secret", "do-not-copy".parse().unwrap());
    response
}

fn app(core: &str, github: &str) -> Router {
    let config = config::parse(&format!(
        "[web]\nlisten='127.0.0.1:0'\ncore_url='{core}'\ngithub_url='{github}'"
    ))
    .unwrap();
    routes::router(AppState::new(config).unwrap())
}

fn request(method: &str, path: &str, body: impl Into<Body>) -> Request {
    Request::builder()
        .method(method)
        .uri(path)
        .body(body.into())
        .unwrap()
}

async fn response_body(response: Response) -> String {
    String::from_utf8(
        to_bytes(response.into_body(), RESPONSE_LIMIT)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

#[tokio::test]
async fn protected_and_malformed_paths_never_reach_upstream() {
    let fake = fake_server("success").await;
    let app = app(&fake.url, &fake.url);
    let paths = [
        "/admin",
        "/v1/gallery",
        "/api/admin",
        "/api/db",
        "/api/me/secrets",
        "/api/images/short",
        "/api/images/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ];
    for path in paths {
        let response = app.clone().oneshot(request("GET", path, "")).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unsupported_methods_do_not_reach_upstream() {
    let fake = fake_server("success").await;
    let app = app(&fake.url, &fake.url);
    for (method, path) in [
        ("POST", "/api/gallery"),
        ("DELETE", "/api/submissions"),
        ("HEAD", "/api/gallery"),
        ("HEAD", "/auth/github"),
        ("GET", "/webhooks/github"),
        ("GET", "/auth/logout"),
    ] {
        let response = app
            .clone()
            .oneshot(request(method, path, ""))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn gallery_filters_query_and_drops_client_credentials() {
    let mut fake = fake_server("success").await;
    let mut request = request(
        "GET",
        "/api/gallery?query=%E7%AC%91&cursor=a%2Fb&limit=12&admin=1",
        "",
    );
    request
        .headers_mut()
        .insert("cookie", "session=ok".parse().unwrap());
    request
        .headers_mut()
        .insert("origin", "https://meme.test".parse().unwrap());
    request
        .headers_mut()
        .insert("authorization", "Bearer internal".parse().unwrap());
    request
        .headers_mut()
        .insert("x-internal-token", "secret".parse().unwrap());
    app(&fake.url, &fake.url).oneshot(request).await.unwrap();
    let (uri, headers, _) = fake.observations.recv().await.unwrap();
    assert_eq!(uri, "/v1/gallery?query=%E7%AC%91&cursor=a%2Fb&limit=12");
    assert_eq!(headers["cookie"], "session=ok");
    assert_eq!(headers["origin"], "https://meme.test");
    assert!(!headers.contains_key("authorization"));
    assert!(!headers.contains_key("x-internal-token"));
}

#[tokio::test]
async fn multipart_body_csrf_cookie_and_response_headers_survive() {
    let mut fake = fake_server("success").await;
    let body = "--boundary\r\nContent-Disposition: form-data; name=\"consent\"\r\n\r\ntrue\r\n--boundary--\r\n";
    let mut request = request("POST", "/api/submissions", body);
    request.headers_mut().insert(
        "content-type",
        "multipart/form-data; boundary=boundary".parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("cookie", "session=ok".parse().unwrap());
    request
        .headers_mut()
        .insert("x-csrf-token", "csrf-value".parse().unwrap());
    let response = app(&fake.url, &fake.url).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(response.headers().get_all("set-cookie").iter().count(), 2);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(!response.headers().contains_key("x-internal-secret"));
    assert_eq!(response_body(response).await, "{\"id\":\"submission\"}");
    let (uri, headers, forwarded) = fake.observations.recv().await.unwrap();
    assert_eq!(uri, "/v1/submissions");
    assert_eq!(headers["x-csrf-token"], "csrf-value");
    assert_eq!(headers["cookie"], "session=ok");
    assert_eq!(
        headers["content-type"],
        "multipart/form-data; boundary=boundary"
    );
    assert_eq!(forwarded, body);
}

#[tokio::test]
async fn webhook_preserves_raw_bytes_and_signatures_without_session_headers() {
    let mut core = fake_server("success").await;
    let mut github = fake_server("success").await;
    let bytes = vec![0, 255, 13, 10, b'{', b'}'];
    let mut request = request("POST", "/webhooks/github", bytes.clone());
    request
        .headers_mut()
        .insert("x-hub-signature-256", "sha256=signature".parse().unwrap());
    request
        .headers_mut()
        .insert("x-github-event", "pull_request".parse().unwrap());
    request
        .headers_mut()
        .insert("x-github-delivery", "delivery-id".parse().unwrap());
    request
        .headers_mut()
        .insert("cookie", "session=private".parse().unwrap());
    request
        .headers_mut()
        .insert("authorization", "Bearer private".parse().unwrap());
    app(&core.url, &github.url).oneshot(request).await.unwrap();
    let (uri, headers, forwarded) = github.observations.recv().await.unwrap();
    assert_eq!(uri, "/webhooks/github");
    assert_eq!(forwarded, bytes);
    assert_eq!(headers["x-hub-signature-256"], "sha256=signature");
    assert_eq!(headers["x-github-event"], "pull_request");
    assert_eq!(headers["x-github-delivery"], "delivery-id");
    assert!(!headers.contains_key("cookie"));
    assert!(!headers.contains_key("authorization"));
    assert!(core.observations.try_recv().is_err());
}

#[tokio::test]
async fn auth_redirects_are_returned_without_following_and_callback_query_survives() {
    let mut fake = fake_server("redirect").await;
    let response = app(&fake.url, &fake.url)
        .oneshot(request(
            "GET",
            "/auth/callback?code=abc%2Fdef&state=one",
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(response.headers()["location"], "/followed");
    let (uri, _, _) = fake.observations.recv().await.unwrap();
    assert_eq!(uri, "/auth/callback?code=abc%2Fdef&state=one");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn github_oauth_location_is_allowed_and_unexpected_location_is_rejected() {
    let github = fake_server("github").await;
    let response = app(&github.url, &github.url)
        .oneshot(request("GET", "/auth/github", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    assert!(
        response.headers()["location"]
            .to_str()
            .unwrap()
            .starts_with("https://github.com/")
    );
    let evil = fake_server("evil").await;
    let response = app(&evil.url, &evil.url)
        .oneshot(request("GET", "/auth/github", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(response_body(response).await.contains("unsafe_redirect"));
}

#[tokio::test]
async fn core_redirects_are_rejected_without_following() {
    let fake = fake_server("redirect").await;
    let response = app(&fake.url, &fake.url)
        .oneshot(request("GET", "/api/me", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn oversized_requests_are_rejected_before_contacting_upstream() {
    let fake = fake_server("success").await;
    let response = app(&fake.url, &fake.url)
        .oneshot(request(
            "POST",
            "/api/submissions",
            vec![0; REQUEST_LIMIT + 1],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert!(response_body(response).await.contains("request_too_large"));
}

#[tokio::test]
async fn response_and_image_limits_are_enforced() {
    let fake = fake_server("oversized").await;
    let response = app(&fake.url, &fake.url)
        .oneshot(request("GET", "/api/gallery", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(response_body(response).await.contains("response_too_large"));
    let fake = fake_server("image_big").await;
    let path = format!("/api/images/{}", "a".repeat(64));
    let response = app(&fake.url, &fake.url)
        .oneshot(request("GET", &path, ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn upstream_errors_remain_readable_and_connection_failures_are_json() {
    let fake = fake_server("error").await;
    let response = app(&fake.url, &fake.url)
        .oneshot(request("GET", "/api/me", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response_body(response).await,
        "{\"error\":{\"message\":\"invalid image\"}}"
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unused = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let response = app(&unused, &unused)
        .oneshot(request("GET", "/api/gallery", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(
        response_body(response)
            .await
            .contains("upstream_unavailable")
    );
}

#[tokio::test]
async fn source_handles_missing_url_escaping_and_embedded_security_headers() {
    let base = "[web]\nlisten='127.0.0.1:0'\ncore_url='http://127.0.0.1:1'\ngithub_url='http://127.0.0.1:1'\n";
    let config = config::parse(base).unwrap();
    let response = routes::router(AppState::new(config).unwrap())
        .oneshot(request("GET", "/source", ""))
        .await
        .unwrap();
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    assert!(response_body(response).await.contains("尚未配置源码地址"));
    let config = config::parse(&format!(
        "{base}source_url='https://example.test/src?a=1&b=2'"
    ))
    .unwrap();
    let response = routes::router(AppState::new(config).unwrap())
        .oneshot(request("GET", "/source", ""))
        .await
        .unwrap();
    assert!(response_body(response).await.contains("a=1&amp;b=2"));
    assert!(config::parse(&format!("{base}source_url='javascript:alert(1)'")).is_err());
}

#[tokio::test]
async fn health_and_assets_work_without_an_upstream() {
    let app = app("http://127.0.0.1:1", "http://127.0.0.1:1");
    for path in ["/", "/assets/app.css", "/assets/app.js", "/health/live"] {
        let response = app.clone().oneshot(request("GET", path, "")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
}

#[tokio::test]
async fn absolute_public_callback_location_is_preserved() {
    let fake = fake_server("evil").await;
    let mut request = request("GET", "/auth/callback", "");
    request
        .headers_mut()
        .insert("host", "example.invalid".parse().unwrap());
    let response = app(&fake.url, &fake.url).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response.headers()["location"],
        "https://example.invalid/steal"
    );
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn connection_named_headers_are_never_forwarded() {
    let mut fake = fake_server("success").await;
    let mut request = request("GET", "/api/me", "");
    request
        .headers_mut()
        .insert("connection", "cookie, x-csrf-token".parse().unwrap());
    request
        .headers_mut()
        .insert("cookie", "session=hidden".parse().unwrap());
    request
        .headers_mut()
        .insert("x-csrf-token", "hidden".parse().unwrap());
    app(&fake.url, &fake.url).oneshot(request).await.unwrap();
    let (_, headers, _) = fake.observations.recv().await.unwrap();
    assert!(!headers.contains_key("cookie"));
    assert!(!headers.contains_key("x-csrf-token"));
    assert!(!headers.contains_key("connection"));
}

#[tokio::test]
async fn image_and_private_reads_use_core_and_logout_uses_github() {
    let mut core = fake_server("success").await;
    let mut github = fake_server("success").await;
    let app = app(&core.url, &github.url);
    let hash = "a".repeat(64);
    for route in [
        "me".to_owned(),
        "submissions".to_owned(),
        format!("images/{hash}"),
    ] {
        app.clone()
            .oneshot(request("GET", &format!("/api/{route}?admin=1"), ""))
            .await
            .unwrap();
        let (uri, _, _) = core.observations.recv().await.unwrap();
        assert_eq!(uri, format!("/v1/{route}"));
    }
    let mut request = request("POST", "/auth/logout", "logout-body");
    request
        .headers_mut()
        .insert("cookie", "session=logout".parse().unwrap());
    request
        .headers_mut()
        .insert("x-csrf-token", "csrf-logout".parse().unwrap());
    app.oneshot(request).await.unwrap();
    let (uri, headers, bytes) = github.observations.recv().await.unwrap();
    assert_eq!(uri, "/auth/logout");
    assert_eq!(headers["cookie"], "session=logout");
    assert_eq!(headers["x-csrf-token"], "csrf-logout");
    assert_eq!(bytes, "logout-body");
}

#[tokio::test]
async fn chunked_response_cannot_evade_the_size_limit() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n").await.unwrap();
        let chunk = vec![b'a'; 65536];
        for _ in 0..=RESPONSE_LIMIT / chunk.len() {
            if write_chunk(&mut socket, &chunk).await.is_err() {
                break;
            }
        }
        let _ = socket.write_all(b"0\r\n\r\n").await;
    });
    let response = app(&address, &address)
        .oneshot(request("GET", "/api/gallery", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(response_body(response).await.contains("response_too_large"));
    task.abort();
}

async fn write_chunk(socket: &mut tokio::net::TcpStream, chunk: &[u8]) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    socket.write_all(b"10000\r\n").await?;
    socket.write_all(chunk).await?;
    socket.write_all(b"\r\n").await
}

#[test]
fn configuration_rejects_unsafe_urls_without_reading_other_services() {
    let base = "[web]\nlisten='127.0.0.1:3000'\ngithub_url='http://127.0.0.1:1'\n";
    for url in [
        "file:///secret",
        "http://user:secret@localhost",
        "http://localhost?internal=1",
    ] {
        assert!(config::parse(&format!("{base}core_url='{url}'")).is_err());
    }
    let document =
        format!("{base}core_url='http://127.0.0.1:2'\n[unrelated]\nsecret_file='does-not-exist'");
    assert!(config::parse(&document).is_ok());
}
