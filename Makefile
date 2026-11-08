# `nix-shell` provides cargo + node on NixOS. Elsewhere install Rust (rustup) and Node 22.
.PHONY: all web build release dev test clean bridge

all: release

web/node_modules: web/package.json
	cd web && npm ci --no-audit --no-fund

web: web/node_modules
	cd web && npm run build

build: web
	cargo build

release: web
	cargo build --release
	@echo "binary: target/release/tushonka-panel"

# Backend on :8080 (per panel.toml) + Vite dev server with hot reload on :5173.
dev: web/node_modules
	cargo run & cd web && npm run dev

test:
	cargo test

# Companion server mod. Point SPT_DIR at a server folder (with SPTarkov.Server.Core.dll),
# or SPT_SRC at a server-csharp checkout. The DLL lands in bridge/dist and is bundled into the
# next panel build, so the panel can install it with one click.
SPT_DIR ?= ./server
bridge:
	cd bridge/TushonkaPanelBridge && dotnet build -c Release $(if $(SPT_SRC),-p:SptSourceDir=$(abspath $(SPT_SRC)),-p:SptDir=$(abspath $(SPT_DIR)))
	mkdir -p bridge/dist
	cp bridge/TushonkaPanelBridge/bin/Release/net10.0/TushonkaPanelBridge.dll bridge/dist/

clean:
	cargo clean
	rm -rf web/dist web/node_modules
