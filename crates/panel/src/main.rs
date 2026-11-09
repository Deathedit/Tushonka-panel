mod auth;
mod config;
mod configs;
mod fsutil;
mod mods;
mod supervisor;
mod updater;
mod web;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

fn usage() -> ! {
    eprintln!(
        "tushonka-panel {}\n\nUSAGE:\n  tushonka-panel [--config <panel.toml>]\n\nThe config path can also be set with TUSHONKA_PANEL_CONFIG.\nA default config is created on first run.",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let mut cfg_path = std::env::var_os("TUSHONKA_PANEL_CONFIG").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("panel.toml"));
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-c" | "--config" => cfg_path = args.next().map(PathBuf::from).unwrap_or_else(|| usage()),
            "-h" | "--help" => usage(),
            "-V" | "--version" => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => usage(),
        }
    }

    let cfg = Arc::new(config::PanelConfig::load_or_create(&cfg_path)?);
    std::fs::create_dir_all(&cfg.data_dir).with_context(|| format!("creating {}", cfg.data_dir.display()))?;
    tracing::info!(server_dir = %cfg.server_dir.display(), "config loaded from {}", cfg_path.display());
    if !cfg.entry_file().is_file() {
        tracing::warn!("server executable not found at {} (set server_dir / executable in {}, or install the server from the Updates page)", cfg.entry_file().display(), cfg_path.display());
    }

    let auth = Arc::new(auth::Auth::new(cfg.admin.clone()));
    if auth.setup_required() {
        tracing::info!("no admin account yet: open the panel in a browser to create one");
    }
    let sup = supervisor::Supervisor::new(cfg.clone());
    if cfg.autostart {
        if let Err(e) = sup.start() {
            tracing::error!("autostart failed: {e:#}");
        }
    }

    let updater = updater::Updater::new(cfg.clone(), sup.clone());
    let state = web::AppState { cfg: cfg.clone(), cfg_path, auth, sup: sup.clone(), updater };
    let app = web::router(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await.with_context(|| format!("binding {}", cfg.bind))?;
    tracing::info!("panel listening on http://{}", cfg.bind);

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown(sup, cfg.stop_timeout_secs))
        .await?;
    Ok(())
}

/// On SIGINT/SIGTERM (e.g. `docker stop`), stop the game server cleanly before exiting
/// so profiles are saved.
async fn shutdown(sup: Arc<supervisor::Supervisor>, timeout: u64) {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    tracing::info!("shutting down");
    if sup.stop().is_ok() {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout + 8);
        while sup.status().pid.is_some() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}
