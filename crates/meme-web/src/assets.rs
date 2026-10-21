use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::{locales, proxy::AppState};

const INDEX: &str = include_str!("../assets/index.html");
const CSS: &str = include_str!("../assets/app.css");
const JS: &str = include_str!("../assets/app.js");
const I18N: &str = include_str!("../assets/i18n.js");
const SOURCE: &str = include_str!("../assets/source.html");
const ICON: &str = include_str!("../assets/icon.svg");
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; object-src 'none'";

pub(crate) async fn index() -> Response {
    asset(locales::render_english(INDEX), "text/html; charset=utf-8")
}

pub(crate) async fn css() -> Response {
    asset(CSS.to_owned(), "text/css; charset=utf-8")
}

pub(crate) async fn js() -> Response {
    asset(JS.to_owned(), "text/javascript; charset=utf-8")
}

pub(crate) async fn i18n() -> Response {
    asset(
        format!("const ENGLISH = {};\n{I18N}", locales::ENGLISH_JSON),
        "text/javascript; charset=utf-8",
    )
}

pub(crate) async fn locale(Path(name): Path<String>) -> Response {
    match locales::resource(&name) {
        Some(json) => asset(json.to_owned(), "application/json; charset=utf-8"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(crate) async fn icon() -> Response {
    asset(ICON.to_owned(), "image/svg+xml; charset=utf-8")
}

pub(crate) async fn source(State(state): State<AppState>) -> Response {
    let (url, configured, unconfigured) = match state.config.source_url {
        Some(url) => (locales::escape_html(url.as_str()), "", "hidden"),
        None => (String::new(), "hidden", ""),
    };
    let content = locales::render_english(SOURCE)
        .replace("{{source_configured_hidden}}", configured)
        .replace("{{source_unconfigured_hidden}}", unconfigured)
        .replace("{{source_url}}", &url);
    asset(content, "text/html; charset=utf-8")
}

fn asset(content: String, content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CONTENT_SECURITY_POLICY, CSP),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::X_FRAME_OPTIONS, "DENY"),
        ],
        content,
    )
        .into_response()
}
