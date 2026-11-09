//! Panel configuration (`panel.toml`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelConfig {
    /// Address the panel listens on.
    pub bind: String,
    /// Folder that holds the server executable, `sptLogger.json`, `SPT_Data/` and `user/`.
    pub server_dir: PathBuf,
    /// Executable name inside `server_dir`.
    pub executable: String,
    /// Extra arguments passed to the server.
    pub args: Vec<String>,
    /// Run the server through this .NET host (e.g. "dotnet") as `dotnet <executable>.dll` instead of
    /// the native launcher. Needed where the release's launcher binary cannot run, such as NixOS
    /// without nix-ld. Empty = run the launcher directly.
    pub dotnet: String,
    /// Config folder, relative to `server_dir`.
    pub configs_dir: PathBuf,
    /// Panel state (backups, client files, trash), relative to the panel config file unless absolute.
    pub data_dir: PathBuf,
    /// Start the server when the panel starts.
    pub autostart: bool,
    /// Restart the server automatically if it exits on its own.
    pub auto_restart: bool,
    /// Seconds to wait after Ctrl+C before escalating to SIGTERM / SIGKILL.
    pub stop_timeout_secs: u64,
    /// Localhost port for the TushonkaPanelBridge server mod (Database / Profiles).
    pub bridge_port: u16,
    /// Check GitHub for new server releases every few hours.
    pub update_check: bool,
    /// GitHub repository with the server releases.
    pub update_repo: String,
    /// Admin account; `None` until first-run setup.
    pub admin: Option<AdminAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminAccount {
    pub username: String,
    /// Argon2id PHC string.
    pub password_hash: String,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8080".into(),
            server_dir: PathBuf::from("./server"),
            executable: "SPT.Server.Linux".into(),
            args: Vec::new(),
            dotnet: String::new(),
            configs_dir: PathBuf::from("SPT_Data/configs"),
            data_dir: PathBuf::from("./panel-data"),
            autostart: false,
            auto_restart: false,
            stop_timeout_secs: 20,
            bridge_port: 6970,
            update_check: true,
            update_repo: "SP-Tushonka/build".into(),
            admin: None,
        }
    }
}

impl PanelConfig {
    /// Loads the config, creating a default file if none exists. Relative paths are resolved
    /// against the config file's folder so the panel behaves the same from any working directory.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        let mut cfg = if path.exists() {
            let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            toml::from_str::<PanelConfig>(&text).with_context(|| format!("parsing {}", path.display()))?
        } else {
            let cfg = PanelConfig::default();
            cfg.save(path)?;
            tracing::info!("created default config at {}", path.display());
            cfg
        };
        // Environment overrides, handy for containers.
        if let Some(v) = std::env::var_os("TUSHONKA_SERVER_DIR") {
            cfg.server_dir = PathBuf::from(v);
        }
        if let Some(v) = std::env::var_os("TUSHONKA_DATA_DIR") {
            cfg.data_dir = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("TUSHONKA_BIND") {
            cfg.bind = v;
        }
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if cfg.server_dir.is_relative() {
            cfg.server_dir = base.join(&cfg.server_dir);
        }
        if cfg.data_dir.is_relative() {
            cfg.data_dir = base.join(&cfg.data_dir);
        }
        // The server is spawned with its own folder as cwd, so relative paths would resolve twice.
        cfg.server_dir = std::path::absolute(&cfg.server_dir)?;
        cfg.data_dir = std::path::absolute(&cfg.data_dir)?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self)?;
        crate::fsutil::write_atomic(path, text.as_bytes())
    }

    /// Writes the admin account into the config file without touching (or resolving) other keys.
    pub fn persist_admin(path: &Path, admin: &AdminAccount) -> Result<()> {
        let mut raw = if path.exists() {
            toml::from_str::<PanelConfig>(&std::fs::read_to_string(path)?)?
        } else {
            PanelConfig::default()
        };
        raw.admin = Some(admin.clone());
        raw.save(path)
    }

    /// The file that must exist to start the server: the launcher, or `<executable>.dll` in dotnet mode.
    pub fn entry_file(&self) -> PathBuf {
        if self.dotnet.trim().is_empty() {
            self.server_dir.join(&self.executable)
        } else {
            self.server_dir.join(format!("{}.dll", self.executable))
        }
    }

    pub fn configs_path(&self) -> PathBuf {
        self.server_dir.join(&self.configs_dir)
    }

    pub fn user_path(&self) -> PathBuf {
        self.server_dir.join("user")
    }
}
