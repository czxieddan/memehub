use crate::{
    App, catalog,
    database::Record,
    model::{Index, Manifest, Meme},
};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

pub async fn synchronize(app: &App) -> anyhow::Result<Value> {
    let _lock = app.publication.lock().await;
    let Some(index) = read_index(app).await? else {
        return Ok(json!({"status":"awaiting_index"}));
    };
    let previous = app.db.get("corestate", "publication").await?;
    let old = app.db.all("corememes").await?;
    if unchanged(app, &index, previous.as_ref(), &old) {
        let (operation, backfilled) = catalog::prepare(app, &index.hashes).await?;
        if backfilled {
            let version = previous.as_ref().map_or(0, |r| r.version);
            app.db.transaction(vec![
                json!({"op":"check","namespace":"corestate","key":"publication","expected_version":version}),
                operation,
            ]).await?;
        }
        return Ok(
            json!({"status":"unchanged","source_commit":index.source_commit,"catalog_backfilled":backfilled}),
        );
    }
    let memes = read_manifests(app, &index).await?;
    commit(app, &index, &memes, &old, previous.as_ref()).await?;
    app.cache.lock().await.clear();
    Ok(json!({"status":"updated","source_commit":index.source_commit,"count":memes.len()}))
}
async fn read_index(app: &App) -> anyhow::Result<Option<Index>> {
    let bytes = app
        .github
        .content(
            &app.config.repository,
            "manifest/index.toml",
            &app.config.branch,
        )
        .await?;
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    ensure!(bytes.len() <= 512 * 1024, "index too large");
    let index: Index = toml::from_str(std::str::from_utf8(&bytes)?)?;
    index.validate()?;
    Ok(Some(index))
}
fn unchanged(app: &App, index: &Index, previous: Option<&Record>, old: &[Record]) -> bool {
    let same_source = previous.is_some_and(|r| {
        r.value["source_commit"] == index.source_commit
            && r.value["repository"] == app.config.repository
    });
    let same_hashes =
        old.len() == index.hashes.len() && old.iter().all(|r| index.hashes.contains(&r.key));
    same_source && same_hashes
}
async fn commit(
    app: &App,
    index: &Index,
    memes: &[Meme],
    old: &[Record],
    previous: Option<&Record>,
) -> anyhow::Result<()> {
    let (catalog, _) = catalog::prepare(app, &index.hashes).await?;
    let mut operations: Vec<Value> = old.iter().map(|r| {
        json!({"op":"delete","namespace":"corememes","key":r.key,"expected_version":r.version})
    }).collect();
    for meme in memes {
        operations.push(
            json!({"op":"put","namespace":"corememes","key":meme.manifest_hash,"value":meme}),
        );
    }
    operations.push(catalog);
    operations.push(
        json!({"op":"put","namespace":"corestate","key":"publication",
        "expected_version":previous.map_or(0, |r| r.version),
        "value":{"source_commit":index.source_commit,"repository":app.config.repository}}),
    );
    app.db.transaction(operations).await
}
async fn read_manifests(app: &App, index: &Index) -> anyhow::Result<Vec<crate::model::Meme>> {
    let mut memes = Vec::with_capacity(index.hashes.len());
    let mut names = std::collections::HashSet::new();
    for hash in &index.hashes {
        let path = format!("manifest/{hash}/manifest.toml");
        let bytes = app
            .github
            .content(&app.config.repository, &path, &index.source_commit)
            .await?
            .context("manifest listed in index is missing")?;
        let manifest = parse_manifest(&bytes, hash)?;
        ensure!(
            names.insert((manifest.uploader_id, manifest.name.clone())),
            "duplicate user/Name in index"
        );
        memes.push(manifest.meme());
    }
    Ok(memes)
}
pub fn parse_manifest(bytes: &[u8], hash: &str) -> anyhow::Result<Manifest> {
    ensure!(bytes.len() <= 65536, "manifest too large");
    let manifest: Manifest = toml::from_str(std::str::from_utf8(bytes)?)?;
    manifest.validate(hash)?;
    Ok(manifest)
}
pub async fn cached_content(
    app: &App,
    path: &str,
    reference: &str,
) -> anyhow::Result<std::sync::Arc<Vec<u8>>> {
    let key = format!("{reference}:{path}");
    if let Some(bytes) = app.cache.lock().await.get(&key) {
        return Ok(bytes);
    }
    let _lock = app.download.lock().await;
    if let Some(bytes) = app.cache.lock().await.get(&key) {
        return Ok(bytes);
    }
    let bytes = app
        .github
        .content(&app.config.repository, path, reference)
        .await?
        .context("published file missing")?;
    let result = std::sync::Arc::new(bytes.clone());
    app.cache.lock().await.insert(key, bytes);
    Ok(result)
}
