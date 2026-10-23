use crate::{
    App,
    catalog::{self, Catalog, MAX_PUBLISHED},
    config::now,
    error::Error,
    gallery_page::{self, Context, GalleryQuery, Section},
    model::{Meme, valid_hash},
    routes,
};
use anyhow::{Context as _, ensure};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{cmp::Reverse, collections::HashSet};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GalleryConfig {
    featured: Vec<String>,
}
impl GalleryConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.featured.len() > MAX_PUBLISHED {
            return Err(Error::bad("Featured gallery supports at most 500 hashes"));
        }
        if !self.featured.iter().all(|hash| valid_hash(hash)) {
            return Err(Error::bad("Invalid featured hash"));
        }
        let unique: HashSet<_> = self.featured.iter().collect();
        if unique.len() != self.featured.len() {
            return Err(Error::bad("Duplicate featured hash"));
        }
        Ok(())
    }
}
pub async fn list(
    State(app): State<App>,
    Query(query): Query<GalleryQuery>,
) -> Result<Json<Value>, Error> {
    let term = gallery_page::term(&query)?;
    let _lock = app.publication.lock().await;
    let mut items = app
        .db
        .all("corememes")
        .await?
        .into_iter()
        .map(|r| serde_json::from_value::<Meme>(r.value))
        .collect::<Result<Vec<_>, _>>()?;
    items.retain(|item| search(item, &term));
    if query.section == Section::All {
        return Ok(Json(gallery_page::all(items, &query)?));
    }
    let catalog = catalog::load(&app).await?;
    let (config, config_version) = configuration(&app).await?;
    let publication = app.db.get("corestate", "publication").await?;
    let context = Context {
        key: &app.admin_token,
        section: query.section,
        term,
        stamp: [
            publication.map_or(0, |r| r.version),
            catalog.version,
            config_version,
        ],
    };
    order(&mut items, query.section, &catalog.entries, &config)?;
    Ok(Json(gallery_page::ordered(items, &query, context)?))
}
fn search(item: &Meme, term: &str) -> bool {
    item.name.to_lowercase().contains(term) || item.display_name.to_lowercase().contains(term)
}
fn order(
    items: &mut Vec<Meme>,
    section: Section,
    catalog: &Catalog,
    config: &GalleryConfig,
) -> anyhow::Result<()> {
    match section {
        Section::Latest => newest(items, catalog)?,
        Section::Popular => popular(items, catalog)?,
        Section::Featured => {
            items.retain(|item| config.featured.contains(&item.manifest_hash));
            items.sort_by_key(|item| {
                config
                    .featured
                    .iter()
                    .position(|hash| hash == &item.manifest_hash)
            });
        }
        Section::All => items.sort_by(|a, b| a.manifest_hash.cmp(&b.manifest_hash)),
    }
    Ok(())
}
fn require_metadata(items: &[Meme], catalog: &Catalog) -> anyhow::Result<()> {
    ensure!(
        items
            .iter()
            .all(|item| catalog.contains_key(&item.manifest_hash)),
        "publication catalog requires synchronization"
    );
    Ok(())
}
fn newest(items: &mut [Meme], catalog: &Catalog) -> anyhow::Result<()> {
    require_metadata(items, catalog)?;
    items.sort_by_key(|item| {
        (
            Reverse(catalog[&item.manifest_hash].0),
            item.manifest_hash.clone(),
        )
    });
    Ok(())
}
fn popular(items: &mut [Meme], catalog: &Catalog) -> anyhow::Result<()> {
    require_metadata(items, catalog)?;
    items.sort_by_key(|item| {
        let metrics = &catalog[&item.manifest_hash];
        (
            Reverse(metrics.1),
            Reverse(metrics.0),
            item.manifest_hash.clone(),
        )
    });
    Ok(())
}
pub async fn detail(
    State(app): State<App>,
    Path(hash): Path<String>,
) -> Result<Json<Value>, Error> {
    let _lock = app.publication.lock().await;
    Ok(Json(json!({"item":published(&app, &hash).await?})))
}
async fn published(app: &App, hash: &str) -> Result<Meme, Error> {
    if !valid_hash(hash) {
        return Err(Error::missing());
    }
    app.db
        .value("corememes", hash)
        .await?
        .ok_or_else(Error::missing)
}
pub async fn view(
    State(app): State<App>,
    Path(hash): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, Error> {
    crate::submissions::verify_origin(&headers, &app.config.public_base)?;
    let _lock = app.publication.lock().await;
    published(&app, &hash).await?;
    let mut catalog = catalog::load(&app).await?;
    let metrics = catalog
        .entries
        .get_mut(&hash)
        .context("publication catalog requires synchronization")?;
    let recorded = metrics.record_view(now());
    if recorded {
        app.db
            .transaction(vec![catalog.operation(&catalog.entries)])
            .await?;
    }
    Ok(Json(json!({"recorded":recorded})))
}
async fn configuration(app: &App) -> Result<(GalleryConfig, u64), Error> {
    let record = app.db.get("coreconfig", "gallery").await?;
    let version = record.as_ref().map_or(0, |r| r.version);
    let config: GalleryConfig = record
        .map(|r| serde_json::from_value(r.value))
        .transpose()?
        .unwrap_or_default();
    config.validate()?;
    Ok((config, version))
}
pub async fn show(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<GalleryConfig>, Error> {
    routes::admin(&app, &headers)?;
    Ok(Json(configuration(&app).await?.0))
}
pub async fn set(
    State(app): State<App>,
    headers: HeaderMap,
    Json(config): Json<GalleryConfig>,
) -> Result<Json<GalleryConfig>, Error> {
    routes::admin(&app, &headers)?;
    config.validate()?;
    let _publication = app.publication.lock().await;
    for hash in &config.featured {
        let _: Meme = app
            .db
            .value("corememes", hash)
            .await?
            .ok_or_else(|| Error::bad("Featured hashes must be published"))?;
    }
    let _settings = app.settings.lock().await;
    app.db.put("coreconfig", "gallery", &config, None).await?;
    Ok(Json(config))
}
