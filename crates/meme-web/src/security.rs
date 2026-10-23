use axum::{extract::Request, http::header, middleware::Next, response::Response};

pub(crate) const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; object-src 'none'";

pub(crate) async fn headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, CSP.parse().unwrap());
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    headers.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    headers.insert(header::X_FRAME_OPTIONS, "DENY".parse().unwrap());
    headers.insert(
        "permissions-policy",
        "camera=(), microphone=(), geolocation=()".parse().unwrap(),
    );
    response
}
