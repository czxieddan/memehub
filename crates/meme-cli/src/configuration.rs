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
    let slot = configuration_slot(&mut document, section, field)?;
    *slot = convert(slot, raw)?;
    crate::secrets::replace(path, toml::to_string_pretty(&document)?.as_bytes())?;
    println!(
        "Saved {key}; use admin cache set to apply cache changes immediately. Other settings require a service restart."
    );
    Ok(())
}
fn configuration_slot<'a>(
    document: &'a mut toml::Value,
    section: &str,
    field: &str,
) -> anyhow::Result<&'a mut toml::Value> {
    let table = document
        .get_mut(section)
        .and_then(toml::Value::as_table_mut)
        .context("unknown configuration section")?;
    if !table.contains_key(field) {
        let default = match (section, field) {
            ("core", "upload_policy_version") => toml::Value::String(String::new()),
            ("core", "upload_policy_claims") => toml::Value::Array(Vec::new()),
            _ => anyhow::bail!("unknown configuration field"),
        };
        table.insert(field.into(), default);
    }
    table.get_mut(field).context("unknown configuration field")
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
