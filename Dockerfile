# syntax=docker/dockerfile:1

# --- Web UI -------------------------------------------------------------------
FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# --- Server -------------------------------------------------------------------
FROM rust:1-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
# Cache dependencies separately from our sources.
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release && rm -rf src
COPY migrations ./migrations
COPY src ./src
COPY --from=web /web/dist ./web/dist
RUN touch src/main.rs && cargo build --release

# --- Runtime ------------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ffmpeg ca-certificates tzdata tini \
 && rm -rf /var/lib/apt/lists/*
COPY --from=server /src/target/release/rewindtv /usr/local/bin/rewindtv
# Run unprivileged as uid/gid 568, TrueNAS's standard "apps" user.
RUN groupadd --system --gid 568 rewindtv \
 && useradd --system --uid 568 --gid 568 --home-dir /data --shell /usr/sbin/nologin rewindtv \
 && mkdir -p /data /markers && chown 568:568 /data /markers
USER 568:568
ENV REWINDTV_DATA=/data \
    REWINDTV_PORT=8420 \
    TZ=UTC
VOLUME /data
EXPOSE 8420
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s CMD ["rewindtv", "health"]
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["rewindtv"]
