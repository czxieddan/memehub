use std::{collections::BTreeMap, sync::LazyLock};

pub(crate) const ENGLISH_JSON: &str = include_str!("../assets/locales/en.json");

static ENGLISH: LazyLock<BTreeMap<String, String>> =
    LazyLock::new(|| serde_json::from_str(ENGLISH_JSON).expect("valid embedded English locale"));

pub(crate) fn english(key: &str) -> &'static str {
    ENGLISH.get(key).map(String::as_str).unwrap_or_default()
}

pub(crate) fn render_english(template: &str) -> String {
    let mut result = template.to_owned();
    for (key, value) in ENGLISH.iter() {
        result = result.replace(&format!("{{{{{key}}}}}"), &escape_html(value));
    }
    result
}

pub(crate) fn resource(name: &str) -> Option<&'static str> {
    match name {
        "en.json" => Some(ENGLISH_JSON),
        "zh-CN.json" => Some(include_str!("../assets/locales/zh-CN.json")),
        "ja.json" => Some(include_str!("../assets/locales/ja.json")),
        "ko.json" => Some(include_str!("../assets/locales/ko.json")),
        "ru.json" => Some(include_str!("../assets/locales/ru.json")),
        _ => None,
    }
}

pub(crate) fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
