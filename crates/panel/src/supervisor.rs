//! Runs the C# server as a child process inside a pseudo-terminal.
//!
//! A PTY (rather than plain pipes) matters for this server:
//! - Spectre.Console drops colors when output is redirected, so the console would turn grey.
//! - On fatal errors the server waits on "Press any key to exit", which needs a real stdin.
//! - Ctrl+C written into the PTY delivers SIGINT the same way a terminal would, so the server
//!   shuts down through its normal path and saves profiles.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tokio::runtime::Handle;
use tokio::sync::broadcast;

use crate::config::PanelConfig;

const SCROLLBACK_BYTES: usize = 1024 * 1024;
const READY_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub state: RunState,
    pub pid: Option<u32>,
    /// Unix milliseconds when the current process was started.
    pub started_at: Option<i64>,
    pub exit_code: Option<u32>,
    /// Percent of one core.
    pub cpu: Option<f32>,
    pub mem_bytes: Option<u64>,
    /// Mods or configs changed since the server started.
    pub restart_required: bool,
    /// Where the game server is expected to listen (from http.json).
    pub endpoint: String,
    /// TushonkaPanelBridge mod: `online` once it answers, `offline` while the server runs without it.
    pub bridge: Option<BridgeState>,
    /// The server printed "Press any key to exit": it hit a fatal error and is waiting.
    pub awaiting_key: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BridgeState {
    Online,
    Offline,
}

#[derive(Clone, Debug)]
pub enum Event {
    Output(Bytes),
    Status(Status),
}

struct Proc {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
}

struct Inner {
    status: Status,
    proc: Option<Proc>,
    stop_requested: bool,
    generation: u64,
    size: PtySize,
}

pub struct Supervisor {
    cfg: Arc<PanelConfig>,
    inner: Mutex<Inner>,
    scrollback: Mutex<VecDeque<u8>>,
    events: broadcast::Sender<Event>,
    rt: Handle,
    /// Random per-panel-run secret the bridge mod requires on every request.
    bridge_token: String,
    http: reqwest::Client,
}

impl Supervisor {
    pub fn new(cfg: Arc<PanelConfig>) -> Arc<Self> {
        let (events, _) = broadcast::channel(1024);
        let endpoint = game_endpoint(&cfg).0;
        let sup = Arc::new(Self {
            cfg,
            inner: Mutex::new(Inner {
                status: Status {
                    state: RunState::Stopped,
                    pid: None,
                    started_at: None,
                    exit_code: None,
                    cpu: None,
                    mem_bytes: None,
                    restart_required: false,
                    endpoint,
                    bridge: None,
                    awaiting_key: false,
                },
                proc: None,
                stop_requested: false,
                generation: 0,
                size: PtySize { rows: 40, cols: 140, pixel_width: 0, pixel_height: 0 },
            }),
            scrollback: Mutex::new(VecDeque::with_capacity(SCROLLBACK_BYTES)),
            events,
            rt: Handle::current(),
            bridge_token: random_token(),
            http: reqwest::Client::builder().timeout(Duration::from_secs(30)).build().expect("http client"),
        });
        sup.clone().spawn_stats_loop();
        sup
    }

    pub fn status(&self) -> Status {
        self.inner.lock().unwrap().status.clone()
    }

    /// Current scrollback plus a receiver for everything after it, with no gap or overlap.
    pub fn subscribe(&self) -> (Vec<u8>, broadcast::Receiver<Event>) {
        let sb = self.scrollback.lock().unwrap();
        let rx = self.events.subscribe();
        (sb.iter().copied().collect(), rx)
    }

    pub fn mark_restart_required(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.proc.is_some() && !inner.status.restart_required {
            inner.status.restart_required = true;
            self.emit_status(&inner);
        }
    }

    pub fn start(self: &Arc<Self>) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.proc.is_some() {
            bail!("the server is already running");
        }
        let exe = self.cfg.server_dir.join(&self.cfg.executable);
        if !exe.is_file() {
            bail!("server executable not found: {}", exe.display());
        }

        let pair = native_pty_system().openpty(inner.size).context("opening a pseudo-terminal")?;
        let mut cmd = CommandBuilder::new(&exe);
        cmd.args(&self.cfg.args);
        cmd.cwd(&self.cfg.server_dir);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TUSHONKA_BRIDGE_TOKEN", &self.bridge_token);
        cmd.env("TUSHONKA_BRIDGE_PORT", self.cfg.bridge_port.to_string());
        let mut child = pair.slave.spawn_command(cmd).with_context(|| format!("starting {}", exe.display()))?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let killer = child.clone_killer();
        let pid = child.process_id();

        inner.generation += 1;
        let gen = inner.generation;
        inner.stop_requested = false;
        inner.proc = Some(Proc { master: pair.master, writer, killer, pid });
        inner.status.state = RunState::Starting;
        inner.status.pid = pid;
        inner.status.started_at = Some(chrono::Utc::now().timestamp_millis());
        inner.status.exit_code = None;
        inner.status.restart_required = false;
        inner.status.awaiting_key = false;
        inner.status.bridge = None;
        inner.status.endpoint = game_endpoint(&self.cfg).0;
        self.emit_status(&inner);
        drop(inner);

        self.panel_line(&format!("starting {}", exe.display()));
        tracing::info!(?pid, "server started");

        // Output pump.
        let me = self.clone();
        std::thread::Builder::new().name("pty-reader".into()).spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => me.push_output(&buf[..n]),
                }
            }
        })?;

        // Exit watcher.
        let me = self.clone();
        std::thread::Builder::new().name("pty-wait".into()).spawn(move || {
            let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
            me.on_exit(gen, code);
        })?;

        // Readiness: the server counts as running once its HTTP port accepts connections.
        let me = self.clone();
        self.rt.spawn(async move { me.wait_ready(gen).await });
        Ok(())
    }

    pub fn stop(self: &Arc<Self>) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let gen = inner.generation;
        let Some(proc) = inner.proc.as_mut() else { bail!("the server is not running") };
        let pid = proc.pid;
        // Ctrl+C through the terminal: the server's normal shutdown path.
        let _ = proc.writer.write_all(b"\x03");
        let _ = proc.writer.flush();
        inner.stop_requested = true;
        inner.status.state = RunState::Stopping;
        self.emit_status(&inner);
        drop(inner);
        self.panel_line("stopping (Ctrl+C sent)");

        let me = self.clone();
        let grace = Duration::from_secs(self.cfg.stop_timeout_secs.max(1));
        self.rt.spawn(async move {
            tokio::time::sleep(grace).await;
            if !me.is_alive(gen) {
                return;
            }
            me.panel_line("still running, sending SIGTERM");
            if let Some(pid) = pid {
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
            if !me.is_alive(gen) {
                return;
            }
            me.panel_line("still running, killing");
            if let Some(proc) = me.inner.lock().unwrap().proc.as_mut() {
                let _ = proc.killer.kill();
            }
        });
        Ok(())
    }

    pub async fn restart(self: &Arc<Self>) -> Result<()> {
        let gen = self.inner.lock().unwrap().generation;
        if self.is_alive(gen) {
            self.stop()?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(self.cfg.stop_timeout_secs + 15);
            while self.is_alive(gen) {
                if tokio::time::Instant::now() > deadline {
                    bail!("the server did not stop in time");
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        self.start()
    }

    pub fn write_input(&self, data: &[u8]) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let Some(proc) = inner.proc.as_mut() else { bail!("the server is not running") };
        proc.writer.write_all(data)?;
        proc.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if cols < 10 || rows < 3 {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        inner.size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
        let size = inner.size;
        if let Some(proc) = inner.proc.as_ref() {
            let _ = proc.master.resize(size);
        }
    }

    fn is_alive(&self, gen: u64) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.generation == gen && inner.proc.is_some()
    }

    fn on_exit(self: &Arc<Self>, gen: u64, code: u32) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != gen {
            return;
        }
        let intentional = inner.stop_requested;
        inner.proc = None;
        inner.stop_requested = false;
        inner.status.state = if intentional || code == 0 { RunState::Stopped } else { RunState::Crashed };
        inner.status.exit_code = Some(code);
        inner.status.pid = None;
        inner.status.cpu = None;
        inner.status.mem_bytes = None;
        inner.status.restart_required = false;
        inner.status.bridge = None;
        inner.status.awaiting_key = false;
        self.emit_status(&inner);
        drop(inner);
        self.panel_line(&format!("server exited with code {code}"));
        tracing::info!(code, intentional, "server exited");

        if !intentional && self.cfg.auto_restart {
            let me = self.clone();
            self.rt.spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let still_down = me.inner.lock().unwrap().proc.is_none();
                if still_down {
                    me.panel_line("auto-restart");
                    if let Err(e) = me.start() {
                        me.panel_line(&format!("auto-restart failed: {e:#}"));
                    }
                }
            });
        }
    }

    async fn wait_ready(self: Arc<Self>, gen: u64) {
        let (_, addr) = game_endpoint(&self.cfg);
        let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(500)).await;
            {
                let inner = self.inner.lock().unwrap();
                if inner.generation != gen || inner.status.state != RunState::Starting {
                    return;
                }
            }
            let Some(addr) = addr else { continue };
            let ok = tokio::task::spawn_blocking(move || TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok())
                .await
                .unwrap_or(false);
            if ok {
                let mut inner = self.inner.lock().unwrap();
                if inner.generation == gen && inner.status.state == RunState::Starting {
                    inner.status.state = RunState::Running;
                    self.emit_status(&inner);
                }
                return;
            }
        }
    }

    fn spawn_stats_loop(self: Arc<Self>) {
        let rt = self.rt.clone();
        rt.spawn(async move {
            let mut sys = match sysinfo::System::new() {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("process stats unavailable: {e}");
                    return;
                }
            };
            let mut tick: u64 = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                tick += 1;
                let running = self.inner.lock().unwrap().status.state == RunState::Running;
                if running && tick % 3 == 1 {
                    let state = match self.bridge_get("health").await {
                        Ok((200, _)) => BridgeState::Online,
                        _ => BridgeState::Offline,
                    };
                    let mut inner = self.inner.lock().unwrap();
                    if inner.status.state == RunState::Running && inner.status.bridge != Some(state) {
                        inner.status.bridge = Some(state);
                        self.emit_status(&inner);
                    }
                }
                let pid = self.inner.lock().unwrap().status.pid;
                let Some(pid) = pid else { continue };
                let spid = sysinfo::Pid::from_u32(pid);
                sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[spid]), true);
                let (cpu, mem) = match sys.process(spid) {
                    Some(p) => (Some(p.cpu_usage()), Some(p.memory())),
                    None => (None, None),
                };
                let mut inner = self.inner.lock().unwrap();
                if inner.status.pid == Some(pid) {
                    inner.status.cpu = cpu;
                    inner.status.mem_bytes = mem;
                    self.emit_status(&inner);
                }
            }
        });
    }

    /// Forwards a request to the bridge mod. Returns status and body.
    pub async fn bridge_request(&self, method: reqwest::Method, path_and_query: &str, body: Option<bytes::Bytes>) -> Result<(u16, bytes::Bytes)> {
        let url = format!("http://127.0.0.1:{}/{}", self.cfg.bridge_port, path_and_query.trim_start_matches('/'));
        let mut req = self.http.request(method, url).bearer_auth(&self.bridge_token);
        if let Some(b) = body {
            req = req.header("content-type", "application/json").body(b);
        }
        let res = req.send().await?;
        let status = res.status().as_u16();
        Ok((status, res.bytes().await?))
    }

    async fn bridge_get(&self, path: &str) -> Result<(u16, bytes::Bytes)> {
        self.bridge_request(reqwest::Method::GET, path, None).await
    }

    fn push_output(&self, data: &[u8]) {
        let mut sb = self.scrollback.lock().unwrap();
        sb.extend(data);
        let excess = sb.len().saturating_sub(SCROLLBACK_BYTES);
        if excess > 0 {
            sb.drain(..excess);
        }
        let _ = self.events.send(Event::Output(Bytes::copy_from_slice(data)));
        drop(sb);
        if contains(data, b"Press any key to exit") {
            let mut inner = self.inner.lock().unwrap();
            if inner.proc.is_some() && !inner.status.awaiting_key {
                inner.status.awaiting_key = true;
                self.emit_status(&inner);
            }
        }
    }

    /// A dim `[panel]` line in the console, so actions show up in context with server output.
    fn panel_line(&self, msg: &str) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        self.push_output(format!("\r\n\x1b[38;5;242m[panel {ts}] {msg}\x1b[0m\r\n").as_bytes());
    }

    fn emit_status(&self, inner: &Inner) {
        let _ = self.events.send(Event::Status(inner.status.clone()));
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn random_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Reads ip/port from `http.json`. Returns a display string and a connectable address.
pub fn game_endpoint(cfg: &PanelConfig) -> (String, Option<SocketAddr>) {
    let path = cfg.configs_path().join("http.json");
    let (ip, port) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|v| {
            let ip = v.get("ip").and_then(|x| x.as_str()).unwrap_or("127.0.0.1").to_string();
            let port = v.get("port").and_then(|x| x.as_u64()).unwrap_or(6969) as u16;
            (ip, port)
        })
        .unwrap_or_else(|| ("127.0.0.1".into(), 6969));
    let display = format!("{ip}:{port}");
    let connect_ip = match ip.as_str() {
        "" | "0.0.0.0" | "::" | "*" => "127.0.0.1".to_string(),
        other => other.to_string(),
    };
    let addr = (connect_ip.as_str(), port).to_socket_addrs().ok().and_then(|mut a| a.next());
    (display, addr)
}
