//! Server updates from GitHub releases (SP-Tushonka/build by default).
//!
//! An update: download the release `.7z` -> verify its SHA-256 -> unpack -> stop the server ->
//! move the current program files into a backup and the new ones into place (`user/` is never
//! touched) -> carry over edited configs -> start the server again. Any backup can be rolled back.
//!
//! An empty server folder gets a fresh install of the latest release instead (no backup, nothing
//! to carry over). It refuses to install into a folder that already holds other files.
//!
//! Edited configs are detected with the release's own manifest, `SPT_Data/checks.dat`
//! (XXH3-64 per file): a config whose hash differs from the manifest was changed by the user.
//! Those keep the user's values and only gain keys that the new version added.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tokio::io::AsyncWriteExt;

use crate::config::PanelConfig;
use crate::fsutil;
use crate::supervisor::{RunState, Supervisor};

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);

#[derive(Clone, Debug, Serialize)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub published_at: String,
    pub notes: String,
    pub html_url: String,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigMerge {
    pub name: String,
    /// Keys from the new default that were added to the edited file.
    pub added_keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateResult {
    pub from: Option<String>,
    pub to: String,
    /// Backup of the replaced files; `None` for a fresh install.
    pub backup: Option<String>,
    /// Edited configs that were kept, with keys added from the new defaults.
    pub configs_kept: Vec<ConfigMerge>,
    /// Configs the user never edited: replaced by the new defaults.
    pub configs_updated: usize,
    pub warnings: Vec<String>,
    /// "running", "failed" (crashed or waiting at "Press any key"), or "not started".
    pub server: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Job {
    pub running: bool,
    pub kind: String,
    pub stage: String,
    /// 0..1 while downloading.
    pub progress: Option<f32>,
    pub log: Vec<String>,
    pub error: Option<String>,
    pub result: Option<UpdateResult>,
}

/// Written next to each server backup so it can be listed and rolled back.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub name: String,
    pub from: Option<String>,
    pub to: String,
    pub created: String,
    /// Top-level entries of the server folder that were moved into this backup.
    pub entries: Vec<String>,
    pub rolled_back: bool,
    #[serde(default)]
    pub size_bytes: u64,
}

#[derive(Default)]
struct State {
    latest: Option<Release>,
    checked_at: Option<i64>,
    check_error: Option<String>,
    job: Job,
}

pub struct Updater {
    cfg: Arc<PanelConfig>,
    sup: Arc<Supervisor>,
    http: reqwest::Client,
    state: Mutex<State>,
    busy: AtomicBool,
}

impl Updater {
    pub fn new(cfg: Arc<PanelConfig>, sup: Arc<Supervisor>) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("tushonka-panel/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .expect("http client");
        let me = Arc::new(Self { cfg, sup, http, state: Mutex::default(), busy: AtomicBool::new(false) });
        if me.cfg.update_check {
            let bg = me.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(10)).await;
                loop {
                    if let Err(e) = bg.check().await {
                        tracing::warn!("update check failed: {e:#}");
                    }
                    tokio::time::sleep(CHECK_INTERVAL).await;
                }
            });
        }
        me
    }

    /// True while an update or rollback is in progress; the server must not be started meanwhile.
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    fn updates_dir(&self) -> PathBuf {
        self.cfg.data_dir.join("updates")
    }

    fn backups_dir(&self) -> PathBuf {
        self.cfg.data_dir.join("backups")
    }

    /// Installed version from `<executable>.deps.json`, e.g. `4.1.6-RELEASE+731d7a2.20260918`.
    pub fn installed_version(&self) -> Option<String> {
        version_in(&self.cfg.server_dir, &self.cfg.executable)
    }

    /// Is there a server in `server_dir` at all (whether or not its version is readable)?
    pub fn server_present(&self) -> bool {
        let dir = &self.cfg.server_dir;
        dir.join(&self.cfg.executable).is_file() || dir.join(format!("{}.dll", self.cfg.executable)).is_file()
    }

    /// Why a fresh install into `server_dir` is not allowed, if it is not.
    pub fn install_blocker(&self) -> Option<String> {
        install_blocker(&self.cfg.server_dir, &self.cfg.executable)
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let installed = self.installed_version();
        let present = self.server_present();
        let install_blocker = if present { None } else { self.install_blocker() };
        let st = self.state.lock().unwrap();
        let available = match (&st.latest, &installed) {
            (Some(l), Some(i)) => newer(&l.tag, i),
            (Some(_), None) => !present,
            _ => false,
        };
        serde_json::json!({
            "installed": installed,
            "present": present,
            "server_dir": self.cfg.server_dir,
            "install_blocker": install_blocker,
            "latest": st.latest,
            "available": available,
            "checked_at": st.checked_at,
            "check_error": st.check_error,
            "repo": self.cfg.update_repo,
            "job": st.job,
            "backups": self.backups(),
        })
    }

    pub async fn check(&self) -> Result<()> {
        let url = format!("https://api.github.com/repos/{}/releases/latest", self.cfg.update_repo);
        let res = self.http.get(&url).header("accept", "application/vnd.github+json").timeout(Duration::from_secs(30)).send().await;
        let outcome = async {
            let res = res?;
            if !res.status().is_success() {
                bail!("GitHub answered {}", res.status());
            }
            let v: serde_json::Value = serde_json::from_slice(&res.bytes().await?)?;
            parse_release(&v)
        }
        .await;
        let mut st = self.state.lock().unwrap();
        st.checked_at = Some(chrono::Utc::now().timestamp_millis());
        match outcome {
            Ok(r) => {
                st.latest = Some(r);
                st.check_error = None;
                Ok(())
            }
            Err(e) => {
                st.check_error = Some(format!("{e:#}"));
                Err(e)
            }
        }
    }

    // ---------- job plumbing ----------

    fn begin(&self, kind: &str) -> Result<()> {
        if self.busy.swap(true, Ordering::SeqCst) {
            bail!("an update or rollback is already running");
        }
        let mut st = self.state.lock().unwrap();
        st.job = Job { running: true, kind: kind.into(), ..Default::default() };
        Ok(())
    }

    fn stage(&self, stage: &str) {
        tracing::info!("update: {stage}");
        let mut st = self.state.lock().unwrap();
        st.job.stage = stage.into();
        st.job.progress = None;
        let ts = chrono::Local::now().format("%H:%M:%S");
        st.job.log.push(format!("[{ts}] {stage}"));
    }

    fn note(&self, msg: impl Into<String>) {
        let msg = msg.into();
        tracing::info!("update: {msg}");
        let ts = chrono::Local::now().format("%H:%M:%S");
        self.state.lock().unwrap().job.log.push(format!("[{ts}]   {msg}"));
    }

    fn finish(&self, result: Result<UpdateResult>) {
        let mut st = self.state.lock().unwrap();
        st.job.running = false;
        st.job.progress = None;
        match result {
            Ok(r) => {
                st.job.stage = "Done".into();
                st.job.result = Some(r);
            }
            Err(e) => {
                tracing::error!("update failed: {e:#}");
                st.job.stage = "Failed".into();
                st.job.error = Some(format!("{e:#}"));
            }
        }
        drop(st);
        self.busy.store(false, Ordering::SeqCst);
    }

    // ---------- update ----------

    pub fn start_update(self: &Arc<Self>, tag: &str) -> Result<()> {
        let release = self.state.lock().unwrap().latest.clone().context("check for updates first")?;
        if release.tag != tag {
            bail!("only the latest release ({}) can be installed", release.tag);
        }
        let fresh = !self.server_present();
        if fresh {
            if let Some(why) = self.install_blocker() {
                bail!(why);
            }
        }
        self.begin(if fresh { "install" } else { "update" })?;
        let me = self.clone();
        tokio::spawn(async move {
            let result = if fresh { me.run_install(release).await } else { me.run_update(release).await };
            me.finish(result);
        });
        Ok(())
    }

    /// Downloads (or reuses), verifies and unpacks the release. Returns the staging folder and the
    /// folder inside it that holds the server.
    async fn fetch_release(&self, release: &Release) -> Result<(PathBuf, PathBuf)> {
        let expected = release.sha256.clone().context("the release has no published SHA-256, refusing to install an unverified archive")?;
        let updates = self.updates_dir();
        tokio::fs::create_dir_all(&updates).await?;

        // 1. Download (reusing a previously downloaded, verified archive).
        let archive = updates.join(&release.asset_name);
        if archive.is_file() && sha256_file(&archive).await? == expected {
            self.stage("Using the already downloaded archive");
        } else {
            self.stage(&format!("Downloading {} ({})", release.asset_name, human(release.asset_size)));
            self.download(&release.asset_url, &archive, release.asset_size).await?;
            self.stage("Verifying SHA-256");
            let got = sha256_file(&archive).await?;
            if got != expected {
                bail!("checksum mismatch: expected {expected}, got {got}");
            }
        }
        self.note("checksum OK");

        // 2. Unpack into a staging folder we own.
        self.stage("Unpacking");
        let staging = updates.join(format!("{}-staging", release.tag));
        remove_own_dir(&staging, &updates)?;
        {
            let (a, s) = (archive.clone(), staging.clone());
            tokio::task::spawn_blocking(move || sevenz_rust2::decompress_file(&a, &s)).await?.context("unpacking the archive")?;
        }
        let runtime = find_runtime(&staging, &self.cfg.executable)
            .with_context(|| format!("the archive contains no {}", self.cfg.executable))?;
        Ok((staging, runtime))
    }

    async fn run_install(&self, release: Release) -> Result<UpdateResult> {
        let updates = self.updates_dir();
        let (staging, runtime) = self.fetch_release(&release).await?;
        let to = version_in(&runtime, &self.cfg.executable).unwrap_or_else(|| release.tag.clone());
        self.note(format!("version: {to}"));
        // Checked again: the folder could have changed during the download.
        if let Some(why) = self.install_blocker() {
            bail!(why);
        }
        self.stage(&format!("Installing into {}", self.cfg.server_dir.display()));
        self.install_fresh(&runtime)?;
        make_executable(&self.cfg.server_dir.join(&self.cfg.executable));
        let _ = remove_own_dir(&staging, &updates);
        self.note("installed. Start the server to finish its first-run setup.");
        Ok(UpdateResult { from: None, to, backup: None, configs_kept: vec![], configs_updated: 0, warnings: vec![], server: "not started".into() })
    }

    async fn run_update(&self, release: Release) -> Result<UpdateResult> {
        let from = self.installed_version();
        let updates = self.updates_dir();
        let (staging, runtime) = self.fetch_release(&release).await?;
        let to = version_in(&runtime, &self.cfg.executable).unwrap_or_else(|| release.tag.clone());
        self.note(format!("new version: {to}"));

        // 3. Stop the server.
        let was_running = self.stop_server().await?;

        // 4. Swap program files; user/ stays.
        self.stage("Installing (current files go to a backup)");
        let name = format!("server-{}-{}", short_version(from.as_deref().unwrap_or("unknown")), fsutil::stamp());
        let backup = self.backups_dir().join(&name);
        let entries = self.swap_in(&runtime, &backup)?;
        let manifest = BackupManifest {
            size_bytes: fsutil::dir_size(&backup),
            name: name.clone(),
            from: from.clone(),
            to: to.clone(),
            created: chrono::Local::now().to_rfc3339(),
            entries,
            rolled_back: false,
        };
        fsutil::write_atomic(&backup.join("manifest.json"), serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        make_executable(&self.cfg.server_dir.join(&self.cfg.executable));

        // 5. Configs.
        self.stage("Carrying over edited configs");
        let (configs_kept, configs_updated) = self.merge_configs(&backup)?;
        for c in &configs_kept {
            self.note(format!("kept your {}.json{}", c.name, if c.added_keys.is_empty() { String::new() } else { format!(" (+{} new keys)", c.added_keys.len()) }));
        }

        let mut warnings = Vec::new();
        if minor_changed(from.as_deref(), &to) {
            warnings.push(format!(
                "{} → {} is a minor-version change: server mods made for the old version (including TushonkaPanelBridge) may need updating.",
                short_version(from.as_deref().unwrap_or("?")),
                short_version(&to)
            ));
        }
        let _ = remove_own_dir(&staging, &updates);

        // 6. Start again if it was running.
        let server = if was_running {
            self.stage("Starting the server");
            self.start_and_watch().await
        } else {
            "not started".to_string()
        };
        if server == "failed" {
            warnings.push("The server did not start cleanly on the new version. Check the console; you can roll back from Updates.".into());
        }
        Ok(UpdateResult { from, to, backup: Some(name), configs_kept, configs_updated, warnings, server })
    }

    /// Moves the release into an empty server folder. The release's `user/` is only used when the
    /// folder has none yet. On error, everything moved so far goes back to staging.
    fn install_fresh(&self, runtime: &Path) -> Result<()> {
        let server = &self.cfg.server_dir;
        std::fs::create_dir_all(server)?;
        let mut names: Vec<String> = std::fs::read_dir(runtime)?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        let mut moved: Vec<String> = Vec::new();
        let result = (|| -> Result<()> {
            for n in &names {
                let dest = server.join(n);
                if dest.exists() || dest.is_symlink() {
                    if n == "user" {
                        continue;
                    }
                    bail!("{} appeared during the install", dest.display());
                }
                fsutil::rename_or_copy(&runtime.join(n), &dest)?;
                moved.push(n.clone());
            }
            Ok(())
        })();
        if let Err(e) = result {
            for n in moved.iter().rev() {
                let _ = fsutil::rename_or_copy(&server.join(n), &runtime.join(n));
            }
            return Err(e.context("installing failed; the folder was left as it was"));
        }
        Ok(())
    }

    async fn download(&self, url: &str, dest: &Path, size: u64) -> Result<()> {
        let part = dest.with_extension("part");
        let mut res = self.http.get(url).send().await?.error_for_status()?;
        let total = res.content_length().unwrap_or(size).max(1);
        let mut file = tokio::fs::File::create(&part).await?;
        let mut done: u64 = 0;
        while let Some(chunk) = res.chunk().await? {
            file.write_all(&chunk).await?;
            done += chunk.len() as u64;
            self.state.lock().unwrap().job.progress = Some((done as f64 / total as f64).min(1.0) as f32);
        }
        file.flush().await?;
        drop(file);
        tokio::fs::rename(&part, dest).await?;
        Ok(())
    }

    /// Stops the server if it runs. Returns whether it was running.
    async fn stop_server(&self) -> Result<bool> {
        let st = self.sup.status();
        if st.pid.is_none() {
            return Ok(false);
        }
        self.stage("Stopping the server");
        let _ = self.sup.stop();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(self.cfg.stop_timeout_secs + 20);
        while self.sup.status().pid.is_some() {
            if tokio::time::Instant::now() > deadline {
                bail!("the server did not stop");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Ok(matches!(st.state, RunState::Running | RunState::Starting))
    }

    async fn start_and_watch(&self) -> String {
        if let Err(e) = self.sup.start() {
            self.note(format!("start failed: {e:#}"));
            return "failed".into();
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let st = self.sup.status();
            if st.awaiting_key || matches!(st.state, RunState::Crashed | RunState::Stopped) {
                return "failed".into();
            }
            if st.state == RunState::Running {
                return "running".into();
            }
            if tokio::time::Instant::now() > deadline {
                return "failed".into();
            }
        }
    }

    /// Moves each top-level entry of the new release into the server folder, after moving the
    /// existing entry of the same name into `backup`. On error, everything moved so far is put back.
    fn swap_in(&self, runtime: &Path, backup: &Path) -> Result<Vec<String>> {
        std::fs::create_dir_all(backup)?;
        let server = &self.cfg.server_dir;
        std::fs::create_dir_all(server)?;
        let mut names: Vec<String> = std::fs::read_dir(runtime)?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "user")
            .collect();
        names.sort();

        let mut moved_old: Vec<String> = Vec::new();
        let mut moved_new: Vec<String> = Vec::new();
        let result = (|| -> Result<()> {
            for n in &names {
                let cur = server.join(n);
                if cur.exists() || cur.is_symlink() {
                    fsutil::rename_or_copy(&cur, &backup.join(n))?;
                    moved_old.push(n.clone());
                }
                fsutil::rename_or_copy(&runtime.join(n), &cur)?;
                moved_new.push(n.clone());
            }
            Ok(())
        })();
        if let Err(e) = result {
            // Undo: new entries back to staging, old entries back into place.
            for n in moved_new.iter().rev() {
                let _ = fsutil::rename_or_copy(&server.join(n), &runtime.join(n));
            }
            for n in moved_old.iter().rev() {
                let _ = fsutil::rename_or_copy(&backup.join(n), &server.join(n));
            }
            return Err(e.context("installing failed; the previous files were put back"));
        }
        Ok(moved_old)
    }

    /// For each config of the previous install: unedited (hash matches the old checks.dat) ->
    /// take the new default; edited -> keep the user's file plus keys the new version added.
    fn merge_configs(&self, backup: &Path) -> Result<(Vec<ConfigMerge>, usize)> {
        let old_dir = backup.join(&self.cfg.configs_dir);
        let new_dir = self.cfg.configs_path();
        let stock = read_checks(&backup.join("SPT_Data/checks.dat"));
        if stock.is_none() {
            self.note("previous install has no checks.dat: treating every config that differs from the new default as edited");
        }
        let defaults_dir = backup.join("new-defaults");
        let mut kept = Vec::new();
        let mut updated = 0;
        let Ok(entries) = std::fs::read_dir(&old_dir) else { return Ok((kept, 0)) };
        for e in entries.flatten() {
            let fname = e.file_name().to_string_lossy().into_owned();
            let Some(stem) = fname.strip_suffix(".json") else { continue };
            let old_bytes = std::fs::read(e.path())?;
            let new_path = new_dir.join(&fname);
            let edited = match &stock {
                Some(map) => map.get(&format!("configs/{fname}")).is_none_or(|h| *h != xxh3_hex(&old_bytes)),
                None => std::fs::read(&new_path).map(|n| n != old_bytes).unwrap_or(true),
            };
            if !edited {
                updated += 1;
                continue;
            }
            if !new_path.is_file() {
                // The new version dropped this config; keep the user's copy in the backup only.
                self.note(format!("{fname} no longer exists in the new version (your copy is in the backup)"));
                continue;
            }
            let new_bytes = std::fs::read(&new_path)?;
            let (Ok(mut user), Ok(default)) = (serde_json::from_slice::<serde_json::Value>(&old_bytes), serde_json::from_slice::<serde_json::Value>(&new_bytes)) else {
                self.note(format!("{fname}: could not parse, kept the new default (your copy is in the backup)"));
                continue;
            };
            let mut added = Vec::new();
            add_missing(&mut user, &default, "", &mut added);
            std::fs::create_dir_all(&defaults_dir)?;
            std::fs::write(defaults_dir.join(&fname), &new_bytes)?;
            let mut text = serde_json::to_string_pretty(&user)?;
            text.push('\n');
            fsutil::write_atomic(&new_path, text.as_bytes())?;
            kept.push(ConfigMerge { name: stem.to_string(), added_keys: added });
        }
        kept.sort_by(|a, b| a.name.cmp(&b.name));
        Ok((kept, updated))
    }

    // ---------- backups / rollback ----------

    pub fn backups(&self) -> Vec<BackupManifest> {
        let mut out: Vec<BackupManifest> = std::fs::read_dir(self.backups_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("server-"))
            .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path().join("manifest.json")).ok()?).ok())
            .collect();
        out.sort_by(|a, b| b.created.cmp(&a.created));
        out
    }

    fn backup_path(&self, name: &str) -> Result<PathBuf> {
        if !name.starts_with("server-") || !fsutil::is_safe_name(name) {
            bail!("invalid backup name");
        }
        let p = self.backups_dir().join(name);
        if !p.join("manifest.json").is_file() {
            bail!("backup not found");
        }
        Ok(p)
    }

    pub fn start_rollback(self: &Arc<Self>, name: &str) -> Result<()> {
        let path = self.backup_path(name)?;
        let manifest: BackupManifest = serde_json::from_slice(&std::fs::read(path.join("manifest.json"))?)?;
        if manifest.rolled_back {
            bail!("this backup was already restored");
        }
        self.begin("rollback")?;
        let me = self.clone();
        tokio::spawn(async move {
            let result = me.run_rollback(path, manifest).await;
            me.finish(result);
        });
        Ok(())
    }

    async fn run_rollback(&self, path: PathBuf, mut manifest: BackupManifest) -> Result<UpdateResult> {
        let current = self.installed_version();
        // A crash (typically right after a failed update) counts as "should be running".
        let crashed = self.sup.status().state == RunState::Crashed;
        let was_running = self.stop_server().await? || crashed;
        self.stage(&format!("Restoring {}", manifest.from.as_deref().unwrap_or("the previous version")));
        // The version being replaced goes into its own backup, so a rollback can be undone too.
        let name = format!("server-{}-{}", short_version(current.as_deref().unwrap_or("unknown")), fsutil::stamp());
        let undo = self.backups_dir().join(&name);
        std::fs::create_dir_all(&undo)?;
        let server = &self.cfg.server_dir;
        let mut moved = Vec::new();
        for n in &manifest.entries {
            let cur = server.join(n);
            if cur.exists() {
                fsutil::rename_or_copy(&cur, &undo.join(n))?;
                moved.push(n.clone());
            }
            let saved = path.join(n);
            if saved.exists() {
                fsutil::rename_or_copy(&saved, &cur)?;
            }
        }
        make_executable(&server.join(&self.cfg.executable));
        manifest.rolled_back = true;
        fsutil::write_atomic(&path.join("manifest.json"), serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        let undo_manifest = BackupManifest {
            size_bytes: fsutil::dir_size(&undo),
            name: name.clone(),
            from: current.clone(),
            to: manifest.from.clone().unwrap_or_default(),
            created: chrono::Local::now().to_rfc3339(),
            entries: moved,
            rolled_back: false,
        };
        fsutil::write_atomic(&undo.join("manifest.json"), serde_json::to_string_pretty(&undo_manifest)?.as_bytes())?;

        let server_state = if was_running {
            self.stage("Starting the server");
            self.start_and_watch().await
        } else {
            "not started".into()
        };
        Ok(UpdateResult {
            from: current,
            to: manifest.from.unwrap_or_default(),
            backup: Some(name),
            configs_kept: vec![],
            configs_updated: 0,
            warnings: vec![],
            server: server_state,
        })
    }

    /// Deletes a server backup folder (explicit user action from the UI).
    pub fn delete_backup(&self, name: &str) -> Result<()> {
        if self.busy() {
            bail!("an update or rollback is running");
        }
        let path = self.backup_path(name)?;
        let manifest: BackupManifest = serde_json::from_slice(&std::fs::read(path.join("manifest.json"))?)?;
        tracing::info!("deleting server backup {} ({} entries)", path.display(), manifest.entries.len());
        std::fs::remove_dir_all(&path)?;
        Ok(())
    }
}

// ---------- helpers ----------

fn parse_release(v: &serde_json::Value) -> Result<Release> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let assets = v.get("assets").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    let asset = assets
        .iter()
        .find(|a| a.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.to_ascii_lowercase().ends_with(".7z")))
        .context("the latest release has no .7z archive")?;
    let a = |k: &str| asset.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let sha256 = asset.get("digest").and_then(|d| d.as_str()).and_then(|d| d.strip_prefix("sha256:")).map(str::to_ascii_lowercase);
    Ok(Release {
        tag: s("tag_name"),
        name: s("name"),
        published_at: s("published_at"),
        notes: s("body"),
        html_url: s("html_url"),
        asset_name: a("name"),
        asset_url: a("browser_download_url"),
        asset_size: asset.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
        sha256,
    })
}

fn version_in(dir: &Path, executable: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(format!("{executable}.deps.json"))).ok()?;
    let marker = format!("\"{executable}/");
    let start = text.find(&marker)? + marker.len();
    let end = text[start..].find('"')?;
    Some(text[start..start + end].to_string())
}

/// `4.1.6-RELEASE+731d7a2.20260918` -> `4.1.6`.
pub fn short_version(v: &str) -> String {
    v.split(['-', '+']).next().unwrap_or(v).trim_start_matches('v').to_string()
}

fn numbers(v: &str) -> Vec<u64> {
    short_version(v).split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Is release tag `latest` newer than the installed version?
pub fn newer(latest: &str, installed: &str) -> bool {
    numbers(latest) > numbers(installed)
}

fn minor_changed(from: Option<&str>, to: &str) -> bool {
    let Some(from) = from else { return false };
    let (a, b) = (numbers(from), numbers(to));
    a.get(..2) != b.get(..2)
}

/// Why a fresh install into `dir` is not allowed, if it is not. Only an empty or missing folder
/// qualifies (a `user/` folder may already be there), so nothing is ever overwritten.
fn install_blocker(dir: &Path, executable: &str) -> Option<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return None };
    let mut others: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "user" && n != "lost+found")
        .collect();
    if others.is_empty() {
        return None;
    }
    if let Some(found) = find_runtime(dir, executable) {
        return Some(format!(
            "The server is in {}. Set server_dir to that folder (with Docker, mount that folder at the server path).",
            found.display()
        ));
    }
    others.sort();
    let more = if others.len() > 5 { format!(" and {} more", others.len() - 5) } else { String::new() };
    others.truncate(5);
    Some(format!(
        "{} has no {} but is not empty ({}{more}). A fresh install only goes into an empty folder, so nothing of yours is overwritten.",
        dir.display(),
        executable,
        others.join(", ")
    ))
}

fn find_runtime(staging: &Path, executable: &str) -> Option<PathBuf> {
    if staging.join(executable).is_file() {
        return Some(staging.to_path_buf());
    }
    std::fs::read_dir(staging).ok()?.flatten().map(|e| e.path()).find(|p| p.join(executable).is_file())
}

/// Deletes a scratch folder, but only one inside `parent` (our own updates folder).
fn remove_own_dir(dir: &Path, parent: &Path) -> Result<()> {
    if !dir.starts_with(parent) || dir == parent {
        bail!("refusing to delete {}", dir.display());
    }
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perm = meta.permissions();
        perm.set_mode(perm.mode() | 0o755);
        let _ = std::fs::set_permissions(path, perm);
    }
}

async fn sha256_file(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<String> {
        let mut f = std::fs::File::open(&path)?;
        let mut h = sha2::Sha256::new();
        std::io::copy(&mut f, &mut h)?;
        Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
    })
    .await?
}

fn xxh3_hex(data: &[u8]) -> String {
    format!("{:016X}", xxhash_rust::xxh3::xxh3_64(data))
}

/// `checks.dat`: base64 of a JSON list of `{ "Path": "configs/x.json", "Hash": "<XXH3-64 hex>" }`.
fn read_checks(path: &Path) -> Option<std::collections::HashMap<String, String>> {
    use base64::Engine;
    let raw = std::fs::read_to_string(path).ok()?;
    let json = base64::engine::general_purpose::STANDARD.decode(raw.trim()).ok()?;
    let list: Vec<serde_json::Value> = serde_json::from_slice(&json).ok()?;
    Some(
        list.into_iter()
            .filter_map(|e| Some((e.get("Path")?.as_str()?.replace('\\', "/"), e.get("Hash")?.as_str()?.to_ascii_uppercase())))
            .collect(),
    )
}

/// Adds keys that exist in `default` but not in `user`, recursively through objects.
/// The user's values always win; arrays are left as the user has them.
fn add_missing(user: &mut serde_json::Value, default: &serde_json::Value, path: &str, added: &mut Vec<String>) {
    let (Some(u), Some(d)) = (user.as_object_mut(), default.as_object()) else { return };
    for (k, dv) in d {
        let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
        match u.get_mut(k) {
            Some(uv) => add_missing(uv, dv, &p, added),
            None => {
                u.insert(k.clone(), dv.clone());
                added.push(p);
            }
        }
    }
}

fn human(n: u64) -> String {
    format!("{:.0} MB", n as f64 / 1024.0 / 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions() {
        assert_eq!(short_version("4.1.6-RELEASE+731d7a2.20260918"), "4.1.6");
        assert!(newer("4.1.6", "4.1.5-BLEEDINGEDGEMODS+T10NVR.0"));
        assert!(!newer("4.1.6", "4.1.6-RELEASE+x"));
        assert!(newer("4.2.0", "4.1.9"));
        assert!(!newer("4.1.5", "4.1.6"));
        assert!(minor_changed(Some("4.1.6"), "4.2.0"));
        assert!(!minor_changed(Some("4.1.5"), "4.1.6"));
    }

    #[test]
    fn version_from_deps_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("SPT.Server.Linux.deps.json"), r#"{"targets":{"x":{"SPT.Server.Linux/4.1.6-RELEASE+731d7a2.20260918":{}}}}"#).unwrap();
        assert_eq!(version_in(dir.path(), "SPT.Server.Linux").as_deref(), Some("4.1.6-RELEASE+731d7a2.20260918"));
    }

    #[test]
    fn merge_keeps_user_values_and_adds_new_keys() {
        let mut user = json!({ "ip": "0.0.0.0", "port": 6969, "nested": { "a": 1 }, "list": [1] });
        let default = json!({ "ip": "127.0.0.1", "port": 6969, "nested": { "a": 2, "b": 3 }, "list": [1, 2], "newKey": true });
        let mut added = Vec::new();
        add_missing(&mut user, &default, "", &mut added);
        assert_eq!(user["ip"], "0.0.0.0");
        assert_eq!(user["nested"]["a"], 1);
        assert_eq!(user["nested"]["b"], 3);
        assert_eq!(user["list"], json!([1]));
        assert_eq!(user["newKey"], true);
        assert_eq!(added, vec!["nested.b", "newKey"]);
    }

    #[test]
    fn xxh3_matches_server_manifest() {
        // Value from SPT 4.1.6 checks.dat would need the real file; check the format at least.
        assert_eq!(xxh3_hex(b"").len(), 16);
        assert_eq!(xxh3_hex(b""), "2D06800538D394C2");
    }

    #[test]
    fn release_parsing() {
        let v = json!({
            "tag_name": "4.1.6", "name": "SPT 4.1.6", "published_at": "2026-09-18T15:45:30Z", "body": "notes", "html_url": "u",
            "assets": [{ "name": "SPT-4.1.6.7z", "browser_download_url": "d", "size": 10, "digest": "sha256:ABC" }]
        });
        let r = parse_release(&v).unwrap();
        assert_eq!(r.sha256.as_deref(), Some("abc"));
        assert_eq!(r.asset_name, "SPT-4.1.6.7z");
        assert!(parse_release(&json!({ "assets": [] })).is_err());
    }

    #[test]
    fn fresh_install_only_into_empty_folder() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("srv");
        assert!(install_blocker(&dir, "SPT.Server.Linux").is_none(), "missing folder is fine");
        std::fs::create_dir_all(dir.join("user/profiles")).unwrap();
        assert!(install_blocker(&dir, "SPT.Server.Linux").is_none(), "only user/ is fine");
        std::fs::create_dir_all(dir.join("SPT_Runtime")).unwrap();
        std::fs::write(dir.join("SPT_Runtime/SPT.Server.Linux"), b"").unwrap();
        assert!(install_blocker(&dir, "SPT.Server.Linux").unwrap().contains("SPT_Runtime"), "points at the nested server");
        std::fs::remove_file(dir.join("SPT_Runtime/SPT.Server.Linux")).unwrap();
        assert!(install_blocker(&dir, "SPT.Server.Linux").unwrap().contains("not empty"));
    }

    #[test]
    fn remove_own_dir_is_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("updates");
        std::fs::create_dir_all(parent.join("x-staging")).unwrap();
        assert!(remove_own_dir(&dir.path().join("elsewhere"), &parent).is_err());
        assert!(remove_own_dir(&parent, &parent).is_err());
        remove_own_dir(&parent.join("x-staging"), &parent).unwrap();
        assert!(!parent.join("x-staging").exists());
    }
}
