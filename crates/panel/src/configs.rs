//! Server config files in `SPT_Data/configs/*.json`.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::config::PanelConfig;
use crate::fsutil;

#[derive(Debug, Serialize)]
pub struct ConfigFile {
    /// Without the `.json` extension.
    pub name: String,
    pub size_bytes: u64,
}

pub fn list(cfg: &PanelConfig) -> Result<Vec<ConfigFile>> {
    let dir = cfg.configs_path();
    let entries = std::fs::read_dir(&dir).with_context(|| format!("config folder not found: {}", dir.display()))?;
    let mut out: Vec<ConfigFile> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".json")?.to_string();
            fsutil::is_safe_name(&stem).then(|| ConfigFile { name: stem, size_bytes: e.metadata().map(|m| m.len()).unwrap_or(0) })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn path_for(cfg: &PanelConfig, name: &str) -> Result<PathBuf> {
    if !fsutil::is_safe_name(name) || name.contains('.') {
        bail!("invalid config name");
    }
    Ok(cfg.configs_path().join(format!("{name}.json")))
}

/// Returns the parsed config. Key order is preserved (`serde_json/preserve_order`).
pub fn read(cfg: &PanelConfig, name: &str) -> Result<serde_json::Value> {
    let path = path_for(cfg, name)?;
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{} is not valid JSON", path.display()))
}

/// Backs up the current file to `<data_dir>/backups/<stamp>/configs/` and writes the new value.
/// Returns the backup path.
pub fn write(cfg: &PanelConfig, name: &str, value: &serde_json::Value) -> Result<Option<PathBuf>> {
    let path = path_for(cfg, name)?;
    if !path.is_file() {
        bail!("{name}.json does not exist");
    }
    let backup = cfg.data_dir.join("backups").join(fsutil::stamp()).join("configs").join(format!("{name}.json"));
    std::fs::create_dir_all(backup.parent().unwrap())?;
    std::fs::copy(&path, &backup)?;
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    fsutil::write_atomic(&path, text.as_bytes())?;
    Ok(Some(backup))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_backs_up_and_keeps_order() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = PanelConfig { server_dir: dir.path().join("srv"), data_dir: dir.path().join("data"), ..Default::default() };
        let configs = cfg.configs_path();
        std::fs::create_dir_all(&configs).unwrap();
        std::fs::write(configs.join("http.json"), r#"{"ip":"127.0.0.1","port":6969}"#).unwrap();

        assert_eq!(list(&cfg).unwrap()[0].name, "http");
        let mut v = read(&cfg, "http").unwrap();
        v["ip"] = "0.0.0.0".into();
        let backup = write(&cfg, "http", &v).unwrap().unwrap();
        assert!(std::fs::read_to_string(backup).unwrap().contains("127.0.0.1"));
        let text = std::fs::read_to_string(configs.join("http.json")).unwrap();
        assert!(text.find("\"ip\"").unwrap() < text.find("\"port\"").unwrap());
        assert!(text.contains("0.0.0.0"));

        assert!(read(&cfg, "../secret").is_err());
        assert!(write(&cfg, "missing", &v).is_err());
    }
}
