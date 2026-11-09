//! HTTP API, WebSocket console and the embedded web UI.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Multipart, Path, RawQuery, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::io::AsyncWriteExt;

use crate::auth::{self, Auth};
use crate::config::PanelConfig;
use crate::supervisor::{Event, Supervisor};
use crate::updater::Updater;
use crate::{configs, fsutil, mods};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<PanelConfig>,
    pub cfg_path: PathBuf,
    pub auth: Arc<Auth>,
    pub sup: Arc<Supervisor>,
    pub updater: Arc<Updater>,
}

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/status", get(status))
        .route("/api/server/start", post(server_start))
        .route("/api/server/stop", post(server_stop))
        .route("/api/server/restart", post(server_restart))
        .route("/api/ws", get(ws_upgrade))
        .route("/api/mods", get(mods_list))
        .route("/api/mods/upload", post(mods_upload).layer(DefaultBodyLimit::max(1024 * 1024 * 1024)))
        .route("/api/mods/{kind}/{name}/enable", post(mods_enable))
        .route("/api/mods/{kind}/{name}/disable", post(mods_disable))
        .route("/api/mods/{kind}/{name}", delete(mods_delete))
        .route("/api/mods/{name}/client-files", get(mods_client_files))
        .route("/api/configs", get(configs_list))
        .route("/api/configs/{name}", get(configs_get).put(configs_put))
        .route("/api/bridge/{*path}", get(bridge_proxy).post(bridge_proxy).patch(bridge_proxy).delete(bridge_proxy))
        .route("/api/bridge-mod", get(bridge_mod_info))
        .route("/api/bridge-mod/install", post(bridge_mod_install))
        .route("/api/update", get(update_info))
        .route("/api/update/check", post(update_check))
        .route("/api/update/start", post(update_start))
        .route("/api/update/rollback", post(update_rollback))
        .route("/api/update/backups/{name}", delete(update_delete_backup))
        .route("/api/logs", get(logs_list))
        .route("/api/logs/{dir}/{file}", get(logs_get))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .route("/api/auth/state", get(auth_state))
        .route("/api/auth/setup", post(auth_setup))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .merge(protected)
        .fallback(static_files)
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

// ---------- errors ----------

pub struct ApiError(StatusCode, String);

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(StatusCode::BAD_REQUEST, format!("{:#}", e.into()))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ---------- middleware ----------

fn session_token(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    auth::token_from_cookie(cookie).map(str::to_string)
}

/// Rejects requests whose Origin does not match Host: blocks cross-site WebSocket hijacking and
/// form posts from other sites (on top of the SameSite=Strict cookie).
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return true };
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else { return false };
    origin.split("://").nth(1).is_some_and(|o| o == host)
}

async fn require_auth(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let authed = session_token(req.headers()).is_some_and(|t| st.auth.touch(&t));
    if !authed {
        return ApiError(StatusCode::UNAUTHORIZED, "sign in required".into()).into_response();
    }
    let unsafe_method = req.method() != Method::GET && req.method() != Method::HEAD;
    let is_ws = req.headers().get(header::UPGRADE).is_some();
    if (unsafe_method || is_ws) && !same_origin(req.headers()) {
        return ApiError(StatusCode::FORBIDDEN, "cross-origin request refused".into()).into_response();
    }
    next.run(req).await
}

async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    res
}

fn session_cookie(token: &str, max_age: u64) -> HeaderValue {
    HeaderValue::from_str(&format!("{}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}", auth::COOKIE)).unwrap()
}

// ---------- auth ----------

#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

async fn auth_state(State(st): State<AppState>, headers: HeaderMap) -> Json<serde_json::Value> {
    let authed = session_token(&headers).is_some_and(|t| st.auth.touch(&t));
    Json(json!({
        "setup_required": st.auth.setup_required(),
        "authenticated": authed,
        "username": if authed { st.auth.username() } else { None },
    }))
}

async fn auth_setup(State(st): State<AppState>, headers: HeaderMap, Json(c): Json<Credentials>) -> ApiResult<Response> {
    if !same_origin(&headers) {
        return Err(ApiError(StatusCode::FORBIDDEN, "cross-origin request refused".into()));
    }
    let account = st.auth.setup(&c.username, &c.password)?;
    PanelConfig::persist_admin(&st.cfg_path, &account)?;
    tracing::info!(username = %account.username, "admin account created");
    let token = st.auth.create_session();
    Ok(([(header::SET_COOKIE, session_cookie(&token, auth::IDLE.as_secs()))], Json(json!({ "ok": true }))).into_response())
}

async fn auth_login(
    State(st): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(c): Json<Credentials>,
) -> ApiResult<Response> {
    if !same_origin(&headers) {
        return Err(ApiError(StatusCode::FORBIDDEN, "cross-origin request refused".into()));
    }
    let client = addr.ip().to_string();
    let creds_ok = { let a = st.auth.clone(); tokio::task::spawn_blocking(move || a.verify(&c.username, &c.password)).await.unwrap_or(false) };
    if !creds_ok {
        let wait = st.auth.note_failure(&client);
        tracing::warn!(%client, "failed login");
        tokio::time::sleep(wait).await;
        return Err(ApiError(StatusCode::UNAUTHORIZED, "wrong username or password".into()));
    }
    st.auth.clear_failures(&client);
    let token = st.auth.create_session();
    Ok(([(header::SET_COOKIE, session_cookie(&token, auth::IDLE.as_secs()))], Json(json!({ "ok": true }))).into_response())
}

async fn auth_logout(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(t) = session_token(&headers) {
        st.auth.end_session(&t);
    }
    ([(header::SET_COOKIE, session_cookie("", 0))], Json(json!({ "ok": true }))).into_response()
}

// ---------- server control ----------

async fn status(State(st): State<AppState>) -> Json<serde_json::Value> {
    let exe = st.cfg.entry_file();
    Json(json!({
        "status": st.sup.status(),
        "panel_version": env!("CARGO_PKG_VERSION"),
        "server_dir": st.cfg.server_dir,
        "executable": st.cfg.executable,
        "executable_found": exe.is_file(),
        "username": st.auth.username(),
    }))
}

fn ensure_not_updating(st: &AppState) -> ApiResult<()> {
    if st.updater.busy() {
        return Err(ApiError(StatusCode::CONFLICT, "an update is in progress".into()));
    }
    Ok(())
}

async fn server_start(State(st): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    ensure_not_updating(&st)?;
    st.sup.start()?;
    Ok(Json(json!({ "ok": true })))
}

async fn server_stop(State(st): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    st.sup.stop()?;
    Ok(Json(json!({ "ok": true })))
}

async fn server_restart(State(st): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    ensure_not_updating(&st)?;
    let sup = st.sup.clone();
    // Restart can take a while (graceful stop); don't hold the request open for it.
    tokio::spawn(async move {
        if let Err(e) = sup.restart().await {
            tracing::error!("restart failed: {e:#}");
        }
    });
    Ok(Json(json!({ "ok": true })))
}

// ---------- websocket ----------

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMsg {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

async fn ws_upgrade(State(st): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| ws_session(st, socket))
}

async fn ws_session(st: AppState, socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    let (scrollback, mut events) = st.sup.subscribe();
    let hello = json!({ "type": "status", "status": st.sup.status() }).to_string();
    if tx.send(Message::Text(hello.into())).await.is_err() {
        return;
    }
    if !scrollback.is_empty() && tx.send(Message::Binary(scrollback.into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(Event::Output(bytes)) => {
                    if tx.send(Message::Binary(bytes)).await.is_err() { break; }
                }
                Ok(Event::Status(s)) => {
                    let msg = json!({ "type": "status", "status": s }).to_string();
                    if tx.send(Message::Text(msg.into())).await.is_err() { break; }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let msg = json!({ "type": "lagged", "skipped": n }).to_string();
                    if tx.send(Message::Text(msg.into())).await.is_err() { break; }
                }
                Err(_) => break,
            },
            msg = rx.next() => match msg {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMsg>(&text) {
                    Ok(ClientMsg::Input { data }) => { let _ = st.sup.write_input(data.as_bytes()); }
                    Ok(ClientMsg::Resize { cols, rows }) => st.sup.resize(cols, rows),
                    Err(_) => {}
                },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
        }
    }
}

// ---------- mods ----------

async fn mods_list(State(st): State<AppState>) -> Json<serde_json::Value> {
    let cfg = st.cfg.clone();
    let list = tokio::task::spawn_blocking(move || mods::list(&cfg)).await.unwrap_or_default();
    Json(json!({ "mods": list, "restart_required": st.sup.status().restart_required }))
}

async fn mods_upload(State(st): State<AppState>, mut form: Multipart) -> ApiResult<Json<serde_json::Value>> {
    let uploads = st.cfg.data_dir.join("uploads");
    tokio::fs::create_dir_all(&uploads).await?;
    let mut reports = Vec::new();
    while let Some(mut field) = form.next_field().await? {
        let original = field.file_name().unwrap_or("upload.zip").to_string();
        let tmp = uploads.join(format!("{}-{}.zip", fsutil::stamp(), reports.len()));
        let mut file = tokio::fs::File::create(&tmp).await?;
        while let Some(chunk) = field.chunk().await? {
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        drop(file);
        let cfg = st.cfg.clone();
        let (orig, path) = (original.clone(), tmp.clone());
        let result = tokio::task::spawn_blocking(move || mods::install_zip(&cfg, &path, &orig)).await?;
        let _ = tokio::fs::remove_file(&tmp).await;
        let report = result.map_err(|e| anyhow::anyhow!("{original}: {e:#}"))?;
        tracing::info!(file = %original, installed = ?report.installed, "mod installed");
        reports.push(report);
    }
    st.sup.mark_restart_required();
    Ok(Json(json!({ "reports": reports })))
}

async fn mods_enable(State(st): State<AppState>, Path((kind, name)): Path<(String, String)>) -> ApiResult<Json<serde_json::Value>> {
    mods::set_enabled(&st.cfg, &kind, &name, true)?;
    st.sup.mark_restart_required();
    Ok(Json(json!({ "ok": true })))
}

async fn mods_disable(State(st): State<AppState>, Path((kind, name)): Path<(String, String)>) -> ApiResult<Json<serde_json::Value>> {
    mods::set_enabled(&st.cfg, &kind, &name, false)?;
    st.sup.mark_restart_required();
    Ok(Json(json!({ "ok": true })))
}

async fn mods_delete(State(st): State<AppState>, Path((kind, name)): Path<(String, String)>) -> ApiResult<Json<serde_json::Value>> {
    let trashed = mods::delete(&st.cfg, &kind, &name)?;
    st.sup.mark_restart_required();
    Ok(Json(json!({ "ok": true, "moved_to": trashed })))
}

async fn mods_client_files(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Response> {
    if !fsutil::is_safe_name(&name) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid name".into()));
    }
    let path = mods::client_files_path(&st.cfg, &name);
    download(&path, &format!("{name}-client.zip")).await
}

// ---------- configs ----------

async fn configs_list(State(st): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let files = configs::list(&st.cfg)?;
    Ok(Json(json!({ "files": files, "dir": st.cfg.configs_path() })))
}

async fn configs_get(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(configs::read(&st.cfg, &name)?))
}

async fn configs_put(State(st): State<AppState>, Path(name): Path<String>, Json(value): Json<serde_json::Value>) -> ApiResult<Json<serde_json::Value>> {
    let backup = configs::write(&st.cfg, &name, &value)?;
    st.sup.mark_restart_required();
    tracing::info!(%name, "config saved");
    Ok(Json(json!({ "ok": true, "backup": backup })))
}

// ---------- bridge (Database / Profiles) ----------

/// Forwards `/api/bridge/<path>` to the TushonkaPanelBridge mod with the per-run token.
async fn bridge_proxy(
    State(st): State<AppState>,
    method: Method,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
    body: bytes::Bytes,
) -> Response {
    let target = match query {
        Some(q) => format!("{path}?{q}"),
        None => path.clone(),
    };
    let method = reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let body = (!body.is_empty()).then_some(body);
    match st.sup.bridge_request(method, &target, body).await {
        Ok((status, data)) => {
            let mut res = Response::builder()
                .status(StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY))
                .header(header::CONTENT_TYPE, "application/json");
            if path.ends_with("/raw") {
                let id = path.split('/').nth(1).unwrap_or("profile");
                res = res.header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{id}.json\""));
            }
            res.body(Body::from(data)).unwrap()
        }
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "the panel bridge is not reachable: start the server with the TushonkaPanelBridge mod installed", "offline": true })),
        )
            .into_response(),
    }
}

/// Bridge DLL bundled into the panel at build time (`make bridge`), if any.
#[derive(rust_embed::RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../bridge/dist"]
struct BridgeDist;

const BRIDGE_DLL: &str = "TushonkaPanelBridge.dll";

fn bridge_installed(cfg: &PanelConfig) -> (bool, bool) {
    let user = cfg.user_path();
    let enabled = user.join("mods/TushonkaPanelBridge").join(BRIDGE_DLL).is_file();
    let disabled = user.join("mods.disabled/TushonkaPanelBridge").join(BRIDGE_DLL).is_file();
    (enabled, disabled)
}

async fn bridge_mod_info(State(st): State<AppState>) -> Json<serde_json::Value> {
    let (enabled, disabled) = bridge_installed(&st.cfg);
    Json(json!({
        "installed": enabled,
        "disabled": disabled,
        "bundled": BridgeDist::get(BRIDGE_DLL).is_some(),
        "status": st.sup.status().bridge,
    }))
}

async fn bridge_mod_install(State(st): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let dll = BridgeDist::get(BRIDGE_DLL).ok_or_else(|| anyhow::anyhow!("this panel build has no bundled bridge; build it with `make bridge` or copy the DLL into user/mods/TushonkaPanelBridge"))?;
    let dir = st.cfg.user_path().join("mods/TushonkaPanelBridge");
    let disabled = st.cfg.user_path().join("mods.disabled/TushonkaPanelBridge");
    if disabled.exists() {
        fsutil::move_to_trash(&disabled, &st.cfg.data_dir.join("trash"))?;
    }
    fsutil::write_atomic(&dir.join(BRIDGE_DLL), &dll.data)?;
    st.sup.mark_restart_required();
    tracing::info!("installed bundled TushonkaPanelBridge into {}", dir.display());
    Ok(Json(json!({ "ok": true })))
}

// ---------- updates ----------

#[derive(Deserialize)]
struct UpdateStart {
    tag: String,
}

#[derive(Deserialize)]
struct Rollback {
    backup: String,
}

async fn update_info(State(st): State<AppState>) -> Json<serde_json::Value> {
    let up = st.updater.clone();
    Json(tokio::task::spawn_blocking(move || up.snapshot()).await.unwrap_or_default())
}

async fn update_check(State(st): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    st.updater.check().await?;
    Ok(update_info(State(st)).await)
}

async fn update_start(State(st): State<AppState>, Json(req): Json<UpdateStart>) -> ApiResult<Json<serde_json::Value>> {
    st.updater.start_update(&req.tag)?;
    Ok(Json(json!({ "ok": true })))
}

async fn update_rollback(State(st): State<AppState>, Json(req): Json<Rollback>) -> ApiResult<Json<serde_json::Value>> {
    st.updater.start_rollback(&req.backup)?;
    Ok(Json(json!({ "ok": true })))
}

async fn update_delete_backup(State(st): State<AppState>, Path(name): Path<String>) -> ApiResult<Json<serde_json::Value>> {
    let up = st.updater.clone();
    tokio::task::spawn_blocking(move || up.delete_backup(&name)).await??;
    Ok(Json(json!({ "ok": true })))
}

// ---------- logs ----------

const LOG_DIRS: [&str; 2] = ["spt", "requests"];

async fn logs_list(State(st): State<AppState>) -> Json<serde_json::Value> {
    let mut out = Vec::new();
    for dir in LOG_DIRS {
        let Ok(entries) = std::fs::read_dir(st.cfg.user_path().join("logs").join(dir)) else { continue };
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().ok().map(|m| chrono::DateTime::<chrono::Utc>::from(m).timestamp_millis());
            out.push(json!({ "dir": dir, "name": e.file_name().to_string_lossy(), "size_bytes": meta.len(), "modified": modified }));
        }
    }
    out.sort_by(|a, b| b["modified"].as_i64().cmp(&a["modified"].as_i64()));
    Json(json!({ "files": out }))
}

async fn logs_get(State(st): State<AppState>, Path((dir, file)): Path<(String, String)>) -> ApiResult<Response> {
    if !LOG_DIRS.contains(&dir.as_str()) || !fsutil::is_safe_name(&file) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid log path".into()));
    }
    download(&st.cfg.user_path().join("logs").join(&dir).join(&file), &file).await
}

async fn download(path: &std::path::Path, filename: &str) -> ApiResult<Response> {
    let data = tokio::fs::read(path).await.map_err(|_| ApiError(StatusCode::NOT_FOUND, "file not found".into()))?;
    let mime = mime_guess::from_path(filename).first_or_octet_stream();
    let disposition = format!("attachment; filename=\"{}\"", filename.replace('"', ""));
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CONTENT_DISPOSITION, disposition)
        .body(Body::from(data))
        .unwrap())
}

// ---------- static UI ----------

#[derive(rust_embed::RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist"]
struct Assets;

async fn static_files(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return ApiError(StatusCode::NOT_FOUND, "not found".into()).into_response();
    }
    let (file, name) = match Assets::get(path) {
        Some(f) if !path.is_empty() => (f, path),
        _ => match Assets::get("index.html") {
            Some(f) => (f, "index.html"),
            None => return (StatusCode::NOT_FOUND, "web UI not built: run `npm run build` in web/ and rebuild").into_response(),
        },
    };
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    let cache = if name.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(file.data.into_owned()))
        .unwrap()
}
