//! Small filesystem helpers shared by the config, mods and settings code.

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Writes a file by writing a sibling temp file and renaming it over the target,
/// so a crash never leaves a half-written config behind.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path.file_name().context("path has no file name")?.to_string_lossy();
    let tmp = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Timestamp usable in folder names, e.g. `2026-11-09T14-03-22`.
pub fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H-%M-%S").to_string()
}

/// A single, safe path segment: letters, digits, `.`, `_`, `-`, space; no leading dot.
pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
}

/// Joins a relative path under `base`, refusing absolute paths and `..` components.
pub fn safe_join(base: &Path, rel: &Path) -> Result<PathBuf> {
    let mut out = base.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            _ => bail!("unsafe path: {}", rel.display()),
        }
    }
    Ok(out)
}

/// Total size of all files under `path`.
pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => total += entry.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

/// Moves a file or folder into `trash_root/<stamp>/`, returning the new location.
pub fn move_to_trash(path: &Path, trash_root: &Path) -> Result<PathBuf> {
    let dir = trash_root.join(stamp());
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(path.file_name().context("no file name")?);
    rename_or_copy(path, &target)?;
    Ok(target)
}

/// `rename`, falling back to copy + delete when crossing filesystems (common with Docker volumes).
pub fn rename_or_copy(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_recursive(from, to)?;
    if from.is_dir() {
        std::fs::remove_dir_all(from)?;
    } else {
        std::fs::remove_file(from)?;
    }
    Ok(())
}

fn copy_recursive(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        std::fs::copy(from, to)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_names() {
        assert!(is_safe_name("BetterLoot"));
        assert!(is_safe_name("core.json"));
        assert!(!is_safe_name("../etc"));
        assert!(!is_safe_name(".hidden"));
        assert!(!is_safe_name("a/b"));
        assert!(!is_safe_name(""));
    }

    #[test]
    fn safe_join_rejects_escape() {
        let base = Path::new("/srv");
        assert!(safe_join(base, Path::new("a/b")).is_ok());
        assert!(safe_join(base, Path::new("../x")).is_err());
        assert!(safe_join(base, Path::new("/abs")).is_err());
    }

    #[test]
    fn atomic_write_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/file.json");
        write_atomic(&p, b"{}").unwrap();
        write_atomic(&p, b"{\"a\":1}").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\"a\":1}");
        assert_eq!(std::fs::read_dir(p.parent().unwrap()).unwrap().count(), 1);
    }
}
