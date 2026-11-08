//! Server mod management: list, enable/disable, delete, install from a .zip.
//!
//! The server loads every folder in `user/mods/` (and prepatchers from `user/patchers/`).
//! Disabling a mod moves its folder to `user/mods.disabled/`, which the loader never scans.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::config::PanelConfig;
use crate::fsutil;

const MAX_UNPACKED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModKind {
    Mod,
    Patcher,
}

impl ModKind {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "mod" => Ok(Self::Mod),
            "patcher" => Ok(Self::Patcher),
            _ => bail!("unknown mod kind: {s}"),
        }
    }
    fn enabled_dir(self) -> &'static str {
        match self {
            Self::Mod => "mods",
            Self::Patcher => "patchers",
        }
    }
    fn disabled_dir(self) -> &'static str {
        match self {
            Self::Mod => "mods.disabled",
            Self::Patcher => "patchers.disabled",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ModInfo {
    pub name: String,
    pub kind: ModKind,
    pub enabled: bool,
    /// Relative to the server folder, e.g. `user/mods/BetterLoot`.
    pub path: String,
    pub size_bytes: u64,
    pub dlls: Vec<String>,
    pub has_client_files: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct InstallReport {
    pub installed: Vec<String>,
    pub replaced: Vec<String>,
    /// Name of the stored client-files archive, if the upload contained BepInEx files.
    pub client_files: Option<String>,
}

pub fn list(cfg: &PanelConfig) -> Vec<ModInfo> {
    let user = cfg.user_path();
    let mut out = Vec::new();
    for kind in [ModKind::Mod, ModKind::Patcher] {
        for (sub, enabled) in [(kind.enabled_dir(), true), (kind.disabled_dir(), false)] {
            let Ok(entries) = std::fs::read_dir(user.join(sub)) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let mut dlls: Vec<String> = std::fs::read_dir(&path)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.to_ascii_lowercase().ends_with(".dll"))
                    .collect();
                dlls.sort();
                out.push(ModInfo {
                    has_client_files: client_files_path(cfg, &name).is_file(),
                    path: format!("user/{sub}/{name}"),
                    size_bytes: fsutil::dir_size(&path),
                    name,
                    kind,
                    enabled,
                    dlls,
                });
            }
        }
    }
    out.sort_by_key(|m| m.name.to_lowercase());
    out
}

pub fn set_enabled(cfg: &PanelConfig, kind: &str, name: &str, enabled: bool) -> Result<()> {
    let kind = ModKind::parse(kind)?;
    check_name(name)?;
    let user = cfg.user_path();
    let (from, to) = if enabled {
        (user.join(kind.disabled_dir()).join(name), user.join(kind.enabled_dir()).join(name))
    } else {
        (user.join(kind.enabled_dir()).join(name), user.join(kind.disabled_dir()).join(name))
    };
    if !from.is_dir() {
        bail!("{} not found", from.display());
    }
    if to.exists() {
        bail!("{} already exists", to.display());
    }
    std::fs::create_dir_all(to.parent().unwrap())?;
    fsutil::rename_or_copy(&from, &to)
}

/// Moves the mod folder to the panel's trash rather than deleting it outright.
pub fn delete(cfg: &PanelConfig, kind: &str, name: &str) -> Result<PathBuf> {
    let kind = ModKind::parse(kind)?;
    check_name(name)?;
    let user = cfg.user_path();
    for sub in [kind.enabled_dir(), kind.disabled_dir()] {
        let p = user.join(sub).join(name);
        if p.is_dir() {
            return fsutil::move_to_trash(&p, &cfg.data_dir.join("trash"));
        }
    }
    bail!("mod {name} not found")
}

pub fn client_files_path(cfg: &PanelConfig, name: &str) -> PathBuf {
    cfg.data_dir.join("client-files").join(format!("{name}.zip"))
}

fn check_name(name: &str) -> Result<()> {
    if !fsutil::is_safe_name(name) {
        bail!("invalid mod name");
    }
    Ok(())
}

/// Where one archive entry goes.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    /// `user/mods/<rest>` or `user/patchers/<rest>`.
    Server { kind: ModKind, rest: PathBuf },
    /// Client-side file, kept as `BepInEx/<rest>`.
    Client { rest: PathBuf },
    Skip,
}

fn classify(name: &str) -> Target {
    let norm = name.replace('\\', "/");
    let lower = norm.to_ascii_lowercase();
    for (marker, kind) in [("user/mods/", ModKind::Mod), ("user/patchers/", ModKind::Patcher)] {
        if let Some(i) = find_segment(&lower, marker) {
            let rest = &norm[i + marker.len()..];
            if !rest.is_empty() {
                return Target::Server { kind, rest: PathBuf::from(rest) };
            }
            return Target::Skip;
        }
    }
    if let Some(i) = find_segment(&lower, "bepinex/") {
        let rest = &norm[i..];
        return Target::Client { rest: PathBuf::from(rest) };
    }
    Target::Skip
}

/// Finds `marker` at the start of the string or right after a `/`.
fn find_segment(haystack: &str, marker: &str) -> Option<usize> {
    let mut start = 0;
    while let Some(off) = haystack[start..].find(marker) {
        let i = start + off;
        if i == 0 || haystack.as_bytes()[i - 1] == b'/' {
            return Some(i);
        }
        start = i + 1;
    }
    None
}

fn is_symlink<R: Read>(file: &zip::read::ZipFile<'_, R>) -> bool {
    file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
}

/// Installs a mod archive. Server files are extracted to a staging folder first and then moved
/// into `user/mods` / `user/patchers`, replacing (and trashing) existing folders of the same name.
pub fn install_zip(cfg: &PanelConfig, zip_path: &Path, upload_name: &str) -> Result<InstallReport> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).context("not a valid .zip archive")?;

    let names: Vec<String> = (0..archive.len()).filter_map(|i| archive.by_index_raw(i).ok().map(|f| f.name().to_string())).collect();
    let mut plan: Vec<(usize, Target)> = names.iter().enumerate().map(|(i, n)| (i, classify(n))).collect();

    // Archives without a user/mods/ path: accept a single top-level folder that contains a DLL,
    // or loose DLLs at the root (named after the archive).
    if !plan.iter().any(|(_, t)| matches!(t, Target::Server { .. })) {
        let stem = Path::new(upload_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Mod".into());
        let candidates: Vec<&String> = names
            .iter()
            .filter(|n| !matches!(classify(n), Target::Client { .. }) && !n.ends_with('/'))
            .collect();
        let has_dll = candidates.iter().any(|n| n.to_ascii_lowercase().ends_with(".dll"));
        if !has_dll {
            bail!("couldn't find a user/mods/<Name> folder or any .dll in the archive");
        }
        let tops: std::collections::BTreeSet<&str> = candidates.iter().map(|n| n.split('/').next().unwrap_or("")).collect();
        let all_nested = candidates.iter().all(|n| n.contains('/'));
        for (i, t) in plan.iter_mut() {
            if !matches!(t, Target::Skip) || names[*i].ends_with('/') {
                continue;
            }
            let n = names[*i].replace('\\', "/");
            let rest = if tops.len() == 1 && all_nested { n } else { format!("{stem}/{n}") };
            *t = Target::Server { kind: ModKind::Mod, rest: PathBuf::from(rest) };
        }
    }

    let staging = cfg.data_dir.join("staging").join(fsutil::stamp());
    std::fs::create_dir_all(&staging)?;
    let result = (|| -> Result<InstallReport> {
        let mut total: u64 = 0;
        let mut client: Option<zip::ZipWriter<std::fs::File>> = None;
        let client_tmp = staging.join("client.zip");
        let mut roots: Vec<(ModKind, String)> = Vec::new();

        for (i, target) in &plan {
            let mut entry = archive.by_index(*i)?;
            if entry.is_dir() || is_symlink(&entry) {
                continue;
            }
            total += entry.size();
            if total > MAX_UNPACKED_BYTES {
                bail!("archive unpacks to more than 2 GiB");
            }
            match target {
                Target::Server { kind, rest } => {
                    let root = rest.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
                    if !fsutil::is_safe_name(&root) {
                        bail!("unsafe folder name in archive: {root}");
                    }
                    if !roots.iter().any(|(k, r)| *k == *kind && *r == root) {
                        roots.push((*kind, root));
                    }
                    let dest = fsutil::safe_join(&staging.join(kind.enabled_dir()), rest)?;
                    std::fs::create_dir_all(dest.parent().unwrap())?;
                    let mut out = std::fs::File::create(&dest)?;
                    std::io::copy(&mut entry, &mut out)?;
                }
                Target::Client { rest } => {
                    fsutil::safe_join(Path::new("/"), rest)?;
                    let w = match client.as_mut() {
                        Some(w) => w,
                        None => client.insert(zip::ZipWriter::new(std::fs::File::create(&client_tmp)?)),
                    };
                    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
                    w.start_file(rest.to_string_lossy().replace('\\', "/"), opts)?;
                    let mut buf = Vec::new();
                    entry.read_to_end(&mut buf)?;
                    w.write_all(&buf)?;
                }
                Target::Skip => {}
            }
        }

        if roots.is_empty() {
            bail!("the archive contains no server files");
        }

        let mut report = InstallReport::default();
        let user = cfg.user_path();
        let trash = cfg.data_dir.join("trash");
        for (kind, root) in &roots {
            let dest = user.join(kind.enabled_dir()).join(root);
            for existing in [dest.clone(), user.join(kind.disabled_dir()).join(root)] {
                if existing.exists() {
                    fsutil::move_to_trash(&existing, &trash)?;
                    report.replaced.push(root.clone());
                }
            }
            std::fs::create_dir_all(dest.parent().unwrap())?;
            fsutil::rename_or_copy(&staging.join(kind.enabled_dir()).join(root), &dest)?;
            report.installed.push(root.clone());
        }

        if let Some(w) = client {
            w.finish()?;
            let name = roots[0].1.clone();
            let dest = client_files_path(cfg, &name);
            std::fs::create_dir_all(dest.parent().unwrap())?;
            fsutil::rename_or_copy(&client_tmp, &dest)?;
            report.client_files = Some(name);
        }
        report.replaced.dedup();
        Ok(report)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_in(dir: &Path) -> PanelConfig {
        PanelConfig { server_dir: dir.join("server"), data_dir: dir.join("data"), ..Default::default() }
    }

    fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn classifies_paths() {
        assert_eq!(classify("SPT/user/mods/Foo/Foo.dll"), Target::Server { kind: ModKind::Mod, rest: "Foo/Foo.dll".into() });
        assert_eq!(classify("user\\patchers\\P\\p.dll"), Target::Server { kind: ModKind::Patcher, rest: "P/p.dll".into() });
        assert_eq!(classify("BepInEx/plugins/Foo.dll"), Target::Client { rest: "BepInEx/plugins/Foo.dll".into() });
        assert_eq!(classify("README.md"), Target::Skip);
        assert_eq!(classify("notuser/mods/x"), Target::Skip);
    }

    #[test]
    fn installs_spt_layout_with_client_files() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg_in(dir.path());
        let z = dir.path().join("BetterLoot.zip");
        make_zip(&z, &[
            ("SPT/user/mods/BetterLoot/BetterLoot.dll", b"dll"),
            ("SPT/user/mods/BetterLoot/config/config.json", b"{}"),
            ("BepInEx/plugins/BetterLoot.Client.dll", b"client"),
            ("README.txt", b"hi"),
        ]);
        let report = install_zip(&cfg, &z, "BetterLoot.zip").unwrap();
        assert_eq!(report.installed, vec!["BetterLoot"]);
        assert_eq!(report.client_files.as_deref(), Some("BetterLoot"));
        assert!(cfg.user_path().join("mods/BetterLoot/config/config.json").is_file());
        assert!(client_files_path(&cfg, "BetterLoot").is_file());

        let mods = list(&cfg);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].dlls, vec!["BetterLoot.dll"]);
        assert!(mods[0].has_client_files);

        // Reinstall replaces and trashes the old copy.
        let report = install_zip(&cfg, &z, "BetterLoot.zip").unwrap();
        assert_eq!(report.replaced, vec!["BetterLoot"]);
    }

    #[test]
    fn installs_bare_folder_and_loose_dll() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg_in(dir.path());
        let z = dir.path().join("a.zip");
        make_zip(&z, &[("Quiet/Quiet.dll", b"x")]);
        assert_eq!(install_zip(&cfg, &z, "a.zip").unwrap().installed, vec!["Quiet"]);
        let z2 = dir.path().join("Loose.zip");
        make_zip(&z2, &[("Loose.dll", b"x")]);
        assert_eq!(install_zip(&cfg, &z2, "Loose.zip").unwrap().installed, vec!["Loose"]);
    }

    #[test]
    fn rejects_traversal_and_junk() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg_in(dir.path());
        let z = dir.path().join("evil.zip");
        make_zip(&z, &[("user/mods/Evil/../../../../escape.dll", b"x")]);
        assert!(install_zip(&cfg, &z, "evil.zip").is_err());
        assert!(!dir.path().join("escape.dll").exists());

        let z2 = dir.path().join("junk.zip");
        make_zip(&z2, &[("readme.txt", b"x")]);
        assert!(install_zip(&cfg, &z2, "junk.zip").is_err());
    }

    #[test]
    fn enable_disable_delete() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg_in(dir.path());
        std::fs::create_dir_all(cfg.user_path().join("mods/Foo")).unwrap();
        set_enabled(&cfg, "mod", "Foo", false).unwrap();
        assert!(cfg.user_path().join("mods.disabled/Foo").is_dir());
        assert!(!list(&cfg)[0].enabled);
        set_enabled(&cfg, "mod", "Foo", true).unwrap();
        assert!(list(&cfg)[0].enabled);
        assert!(set_enabled(&cfg, "mod", "../x", true).is_err());
        delete(&cfg, "mod", "Foo").unwrap();
        assert!(list(&cfg).is_empty());
    }
}
