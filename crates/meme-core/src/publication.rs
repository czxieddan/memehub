use crate::{
    App,
    model::{Index, Manifest},
};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

pub async fn synchronize(app: &App) -> anyhow::Result<Value> {
    let _lock = app.publication.lock().await;
    let Some(bytes) = app
        .github
        .content(
            &app.config.repository,
            "manifest/index.toml",
            &app.config.branch,
        )
        .await?
    else {
        return Ok(json!({"status":"awaiting_index"}));
    };
    ensure!(bytes.len() <= 512 * 1024, "index too large");
    let index: Index = toml::from_str(std::str::from_utf8(&bytes)?)?;
    index.validate()?;
    let previous = app.db.get("corestate", "publication").await?;
    if previous.as_ref().is_some_and(|r| {
        r.value["source_commit"] == index.source_commit
            && r.value["repository"] == app.config.repository
    }) {
        return Ok(json!({"status":"unchanged","source_commit":index.source_commit}));
    }
    let memes = read_manifests(app, &index).await?;
    let old = app.db.all("corememes").await?;
    let mut operations: Vec<Value>=old.iter().map(|r| json!({"op":"delete","namespace":"corememes","key":r.key,"expected_version":r.version})).collect();
    for meme in &memes {
        operations.push(
            json!({"op":"put","namespace":"corememes","key":meme.manifest_hash,"value":meme}),
        );
    }
    operations.push(json!({"op":"put","namespace":"corestate","key":"publication","expected_version":previous.as_ref().map_or(0,|r| r.version),"value":{"source_commit":index.source_commit,"repository":app.config.repository}}));
    app.db.transaction(operations).await?;
    app.cache.lock().await.clear();
    Ok(json!({"status":"updated","source_commit":index.source_commit,"count":memes.len()}))
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_is_bound_to_hash_owner_and_path() {
        let hash = "a".repeat(64);
        let text = format!(
            "schema_version=1\nhash='{hash}'\nuploader_id=123\nname='BigPig'\ndisplay_name='大猪猪'\npath='/data/123/BigPig/meme.png'\n"
        );
        let manifest = parse_manifest(text.as_bytes(), &hash).unwrap();
        let value = serde_json::to_value(manifest.meme()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert_eq!(value["display_name"], "大猪猪");
        assert!(
            parse_manifest(text.replace("/data/123/", "/data/124/").as_bytes(), &hash).is_err()
        );
        assert!(parse_manifest(text.as_bytes(), &"b".repeat(64)).is_err());
        let index = Index {
            schema_version: 1,
            source_commit: "c".repeat(40),
            hashes: vec![hash.clone(), hash],
        };
        assert!(index.validate().is_err());
    }
}
