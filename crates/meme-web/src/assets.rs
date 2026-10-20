use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};

use crate::proxy::AppState;

const INDEX: &str = include_str!("../assets/index.html");
const CSS: &str = include_str!("../assets/app.css");
const JS: &str = include_str!("../assets/app.js");
const SOURCE: &str = include_str!("../assets/source.html");
const ICON: &str = include_str!("../assets/icon.svg");
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; object-src 'none'";

pub(crate) async fn index() -> Response {
    asset(INDEX.to_owned(), "text/html; charset=utf-8")
}

pub(crate) async fn css() -> Response {
    asset(CSS.to_owned(), "text/css; charset=utf-8")
}

pub(crate) async fn js() -> Response {
    asset(JS.to_owned(), "text/javascript; charset=utf-8")
}

pub(crate) async fn icon() -> Response {
    asset(ICON.to_owned(), "image/svg+xml; charset=utf-8")
}

pub(crate) async fn source(State(state): State<AppState>) -> Response {
    let content = match state.config.source_url {
        Some(url) => {
            let escaped = escape_html(url.as_str());
            format!("<p>此部署配置的源码地址：</p><p class=\"source-address\"><a href=\"{escaped}\" target=\"_blank\" rel=\"noopener noreferrer\">{escaped} ↗</a></p><p>请在该地址查看对应版本的源码、变更历史与许可证文件。具体条款以仓库中的许可证为准。</p>")
        }
        None => "<div class=\"notice warning\"><strong>尚未配置源码地址</strong><p>本部署未设置源码链接。请联系部署维护者，获取对应版本的源码、变更历史和许可证文件。</p></div>".to_owned(),
    };
    asset(
        SOURCE.replace("{{source_content}}", &content),
        "text/html; charset=utf-8",
    )
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

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
