use super::*;
use hmac::{Hmac, Mac};
use sha2::Sha256;

#[test]
fn webhook_signatures_bind_raw_body() {
    let body = br#"{"action":"opened"}"#;
    let mut mac = Hmac::<Sha256>::new_from_slice(b"testing-webhook-secret").unwrap();
    mac.update(body);
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    assert!(webhooks::verify(b"testing-webhook-secret", &signature, body).is_ok());
    assert!(webhooks::verify(b"testing-webhook-secret", &signature, b"{}").is_err());
    assert!(webhooks::verify(b"testing-webhook-secret", "sha256=broken", body).is_err());
}
#[test]
fn oauth_and_paths_reject_forgery() {
    assert!(oauth::validate_state(&"a".repeat(64), &"a".repeat(64)).is_ok());
    assert!(oauth::validate_state(&"a".repeat(64), &"b".repeat(64)).is_err());
    assert!(oauth::validate_state("", "").is_err());
    for path in [
        "../key",
        "data/a/../b",
        "/data/a",
        "data\\x",
        "data/.git/config",
    ] {
        assert!(remote::validate_path(path).is_err());
    }
    assert!(remote::validate_path("data/123/BigPig/meme.png").is_ok());
}
#[test]
fn credentials_and_csrf_require_exact_secret() {
    let headers = axum::http::HeaderMap::new();
    assert!(session::credential(&headers, "not-empty").is_err());
    assert!(session::csrf(&headers, "not-empty").is_err());
    assert!(expired(&serde_json::json!({"expires_at":1})));
    assert!(!expired(&serde_json::json!({"expires_at":u64::MAX})));
}

async fn response(raw: &'static [u8]) -> reqwest::Response {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        stream.read(&mut buffer).await.unwrap();
        let _ = stream.write_all(raw).await;
    });
    reqwest::get(url).await.unwrap()
}
#[tokio::test]
async fn bounded_upstream_reads_reject_declared_streamed_and_truncated_bodies() {
    let known = b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nabcdefgh";
    assert_eq!(
        remote::bounded(response(known).await, 8).await.unwrap(),
        b"abcdefgh"
    );
    assert!(remote::bounded(response(known).await, 4).await.is_err());
    let streamed = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nabcd\r\n4\r\nefgh\r\n0\r\n\r\n";
    assert!(remote::bounded(response(streamed).await, 4).await.is_err());
    let truncated = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc";
    assert!(
        remote::bounded(response(truncated).await, 10)
            .await
            .is_err()
    );
}
