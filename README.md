# Tushonka Panel

A web panel for the [SP-Tushonka server](https://github.com/SP-Tushonka/server-csharp). It runs the unmodified C# server as a child process and adds:

- **Live console**: the server's real terminal output (colors included) streamed to the browser, with input, search and log-file downloads.
- **Start / stop / restart**: stop sends Ctrl+C so the server shuts down normally and saves profiles. If it hangs, the panel escalates to SIGTERM and then SIGKILL.
- **Mods**: drag-and-drop `.zip` install, enable/disable, delete (moved to a trash folder). Client-side files (`BepInEx/`) in an archive are stored for download, since they belong on the game PC.
- **Settings**: a form editor and raw JSON editor for `SPT_Data/configs/*.json`. The previous version is backed up on every save.
- **Updates**: checks [SP-Tushonka/build](https://github.com/SP-Tushonka/build/releases) for new releases and installs one with a click:
  1. downloads the `.7z` and verifies its published SHA-256
  2. stops the server
  3. moves the current program files to a backup (`user/` is never touched)
  4. installs the new version and keeps configs you edited
  5. starts the server again

  Edited configs are detected with the release's own `checks.dat` hashes. They keep your values and gain any keys the new version added. Every backup can be restored with one click, and a restore can itself be undone.
- **Login**: an admin account is created on first run. Passwords are hashed with Argon2id, the session cookie is HttpOnly and SameSite=Strict, and requests from other origins are refused.

One self-contained binary: the Svelte UI is embedded.

- **Database**: browse items, traders and quests as the running server sees them, including changes made by mods. A "Changed by mods" filter shows records that mods added or edited.
- **Profiles**: create profiles (with or without a character) and edit level, skills, trader loyalty and standing, quests and hideout levels. Download or delete profiles; a deleted profile is copied to `user/profiles-deleted/` first.

Database and Profiles need the companion server mod **TushonkaPanelBridge** (`bridge/`).

### Panel bridge (server mod)

A small C# mod that serves a JSON API on `127.0.0.1:6970` (`bridge_port` in panel.toml). The panel starts the server with a random per-run token, and the bridge rejects any request without it. When the server is started without the panel, the bridge stays idle and opens no port.

```sh
make bridge SPT_DIR=/path/to/server     # build against an installed server (needs the .NET 10 SDK)
make bridge SPT_SRC=/path/to/server-csharp   # or against a source checkout
make release                            # bundles bridge/dist/TushonkaPanelBridge.dll into the panel
```

With a bundled DLL, the Database and Profiles pages offer an **Install bridge** button. Without one, copy the DLL into `user/mods/TushonkaPanelBridge/` yourself.

How "changed by mods" works: the bridge records which ids exist right after the database import, before any mod runs, so anything added later is marked "added". It also fingerprints every record after the server's own startup, so later edits are marked "changed". Edits a mod makes during Preload are not detected. Trader stock is not compared, because it restocks at runtime.

## Quick start (Linux)

```sh
make release                                   # needs Rust + Node 22 (`nix-shell` on NixOS)
cp panel.example.toml panel.toml               # set server_dir to your server folder
./target/release/tushonka-panel                # open http://<host>:8080
```

`server_dir` is the folder that contains `SPT.Server.Linux`, `sptLogger.json`, `SPT_Data/` and `user/`. The server refuses to start from any other working directory, and the panel always launches it from there.

**NixOS:** the release's `SPT.Server.Linux` launcher expects a standard Linux loader. Without `nix-ld`, set `dotnet = "dotnet"` in panel.toml so the panel runs `dotnet SPT.Server.Linux.dll` instead. The panel prints this hint in the console when the launcher fails with exit code 127.

To reach the game server from other machines, set `"ip": "0.0.0.0"` in `http.json`. The Settings page warns when it is `127.0.0.1`.

## Docker

```sh
mkdir -p spt data        # put the server files in ./spt
docker compose up -d --build
```

The panel is served on port 8080 and the game server on 6969. `docker stop` gives the server time to save before the container exits.

## GitHub Actions

`.github/workflows/build.yml` runs on every push and pull request:

| Job | What it does |
|---|---|
| Bridge mod | Downloads the latest SP-Tushonka release, verifies its SHA-256, and builds `TushonkaPanelBridge` against the release's own server DLLs. Produces `TushonkaPanelBridge-spt-<version>.zip`, which installs through the panel's Mods page. |
| Panel | Builds the web UI, runs clippy and tests, and builds a **static musl binary** (runs on any x86_64 Linux, including NixOS) with the bridge bundled. |
| Docker image | Builds the image, with the bridge bundled. |

Build outputs are attached to each run as artifacts. Pushing a tag such as `v0.2.0` also creates a GitHub release with the binary, checksum and bridge zip, and pushes the image to `ghcr.io/<owner>/<repo>`. To build the bridge against a specific SP-Tushonka version, start the workflow manually and fill in `spt_tag`.

## Development

```sh
nix-shell        # NixOS: provides cargo, rustc, node
make test        # Rust unit tests (mod install / zip-slip, config backup, auth)
make dev         # panel on :8080 + Vite with hot reload on :5173 (proxies /api)
```

Layout:

| Path | What |
|---|---|
| `crates/panel/src/supervisor.rs` | PTY process control, readiness check (TCP connect to `http.json` ip:port), CPU/RAM stats |
| `crates/panel/src/mods.rs` | Mod listing and archive install |
| `crates/panel/src/configs.rs` | Config read/write with backups |
| `crates/panel/src/auth.rs` | Admin account and sessions |
| `crates/panel/src/web.rs` | HTTP API, `/api/ws` (status + console), embedded UI |
| `web/` | Svelte 5 + Vite UI (field-kit dark theme) |
