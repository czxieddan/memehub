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
    println!("已保存 {key}；缓存热更新请使用 admin cache set，其他设置需重启所属服务");
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_field_cannot_change_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "[web]\nlisten='127.0.0.1:1'\n").unwrap();
        assert!(set(&path, "web.unknown", "value").is_err());
        set(&path, "web.listen", "127.0.0.1:2").unwrap();
        assert!(
            std::fs::read_to_string(path)
                .unwrap()
                .contains("127.0.0.1:2")
        );
    }
}
