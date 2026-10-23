use crate::{
    error::Error,
    model::{Meme, valid_hash},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    #[default]
    All,
    Latest,
    Popular,
    Featured,
}
#[derive(Deserialize)]
pub struct GalleryQuery {
    pub query: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    #[serde(default)]
    pub section: Section,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    section: Section,
    term: String,
    stamp: [u64; 3],
    after: String,
}
pub struct Context<'a> {
    pub key: &'a str,
    pub section: Section,
    pub term: String,
    pub stamp: [u64; 3],
}
pub fn term(query: &GalleryQuery) -> Result<String, Error> {
    let term = query.query.as_deref().unwrap_or_default();
    if term.len() > 256 {
        return Err(Error::bad("Search query exceeds 256 bytes"));
    }
    Ok(term.to_lowercase())
}
pub fn all(mut items: Vec<Meme>, query: &GalleryQuery) -> Result<Value, Error> {
    let cursor = query.cursor.as_deref().unwrap_or_default();
    if !cursor.is_empty() && !valid_hash(cursor) {
        return Err(Error::bad("Invalid gallery cursor"));
    }
    items.retain(|item| item.manifest_hash.as_str() > cursor);
    items.sort_by(|a, b| a.manifest_hash.cmp(&b.manifest_hash));
    let limit = query.limit.unwrap_or(40).clamp(1, 100);
    let more = items.len() > limit;
    items.truncate(limit);
    let next = more.then(|| items.last().unwrap().manifest_hash.clone());
    Ok(json!({"items":items,"next_cursor":next}))
}
pub fn ordered(
    mut items: Vec<Meme>,
    query: &GalleryQuery,
    context: Context<'_>,
) -> Result<Value, Error> {
    if let Some(cursor) = query.cursor.as_deref().filter(|value| !value.is_empty()) {
        let after = decode(cursor, &context)?;
        let position = items
            .iter()
            .position(|item| item.manifest_hash == after)
            .ok_or_else(|| Error::bad("Gallery cursor does not identify a matching item"))?;
        items.drain(..=position);
    }
    let limit = query.limit.unwrap_or(40).clamp(1, 100);
    let more = items.len() > limit;
    items.truncate(limit);
    let next = if more {
        Some(encode(&context, &items.last().unwrap().manifest_hash)?)
    } else {
        None
    };
    Ok(json!({"items":items,"next_cursor":next}))
}
fn encode(context: &Context<'_>, after: &str) -> Result<String, Error> {
    let cursor = Cursor {
        version: 1,
        section: context.section,
        term: context.term.clone(),
        stamp: context.stamp,
        after: after.into(),
    };
    let mut bytes = serde_json::to_vec(&cursor)?;
    let mut auth = authenticator(context.key)?;
    auth.update(&bytes);
    bytes.extend_from_slice(&auth.finalize().into_bytes());
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn decode(value: &str, context: &Context<'_>) -> Result<String, Error> {
    if value.len() > 2048 {
        return Err(Error::bad("Invalid gallery cursor"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| Error::bad("Invalid gallery cursor"))?;
    let split = bytes
        .len()
        .checked_sub(32)
        .ok_or_else(|| Error::bad("Invalid gallery cursor"))?;
    let (message, signature) = bytes.split_at(split);
    let mut auth = authenticator(context.key)?;
    auth.update(message);
    auth.verify_slice(signature)
        .map_err(|_| Error::bad("Invalid gallery cursor"))?;
    let cursor: Cursor =
        serde_json::from_slice(message).map_err(|_| Error::bad("Invalid gallery cursor"))?;
    let binding = (
        cursor.version,
        cursor.section,
        cursor.term.as_str(),
        cursor.stamp,
    );
    if binding != (1, context.section, context.term.as_str(), context.stamp) {
        return Err(Error::bad(
            "Gallery cursor is stale or belongs to another section or query",
        ));
    }
    Ok(cursor.after)
}
fn authenticator(key: &str) -> Result<Hmac<Sha256>, Error> {
    Hmac::<Sha256>::new_from_slice(key.as_bytes())
        .map_err(|_| anyhow::anyhow!("Invalid gallery cursor key").into())
}
