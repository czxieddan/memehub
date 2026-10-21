use anyhow::{Context, ensure};
use std::path::Path;

pub fn show(path: &Path) -> anyhow::Result<()> {
    let document: toml::Value = toml::from_str(&std::fs::read_to_string(path)?)?;
    println!("{}", toml::to_string_pretty(&document)?);
    Ok(())
}
pub fn set(path: &Path, key: &str, raw: &str) -> anyhow::Result<()> {
    let mut parts = key.split('.');
    let section = parts.next().context("section required")?;
    let field = parts.next().context("field required")?;
    ensure!(parts.next().is_none(), "key requires section.field");
    let mut document: toml::Value = toml::from_str(&std::fs::read_to_string(path)?)?;
    let slot = document
        .get_mut(section)
        .and_then(|v| v.get_mut(field))
        .context("unknown configuration field")?;
    *slot = convert(slot, raw)?;
    crate::secrets::replace(path, toml::to_string_pretty(&document)?.as_bytes())?;
    println!(
        "Saved {key}; use admin cache set to apply cache changes immediately. Other settings require a service restart."
    );
    Ok(())
}
fn convert(current: &toml::Value, raw: &str) -> anyhow::Result<toml::Value> {
    Ok(match current {
        toml::Value::String(_) => toml::Value::String(raw.into()),
        toml::Value::Integer(_) => toml::Value::Integer(raw.parse()?),
        toml::Value::Boolean(_) => toml::Value::Boolean(raw.parse()?),
        toml::Value::Array(_) => toml::from_str::<toml::Value>(&format!("value={raw}"))?
            .get("value")
            .context("array value")?
            .clone(),
        _ => anyhow::bail!("unsupported configuration value"),
    })
}
