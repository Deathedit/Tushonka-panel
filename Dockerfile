# syntax=docker/dockerfile:1
# Tushonka Panel + runtime for the SP-Tushonka server.
# The server files are not baked in: mount your server folder at /srv/spt.

FROM node:22-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY --from=web /src/web/dist web/dist
RUN cargo build --release --locked

# ASP.NET Core runtime: works whether the server build is framework-dependent or self-contained.
FROM mcr.microsoft.com/dotnet/aspnet:10.0
COPY --from=build /src/target/release/tushonka-panel /usr/local/bin/tushonka-panel
ENV TUSHONKA_PANEL_CONFIG=/data/panel.toml \
    TUSHONKA_SERVER_DIR=/srv/spt \
    TUSHONKA_DATA_DIR=/data/panel-data \
    TUSHONKA_BIND=0.0.0.0:8080
WORKDIR /data
VOLUME ["/data", "/srv/spt"]
EXPOSE 8080 6969
ENTRYPOINT ["/usr/local/bin/tushonka-panel"]
