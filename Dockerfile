# ─────────────────────────────────────────────
# Stage 1: Build frontend
# ─────────────────────────────────────────────
FROM node:20-alpine AS frontend

WORKDIR /app

# Install pnpm
RUN corepack enable && corepack prepare pnpm@10.33.0 --activate

# Install dependencies (cache layer)
COPY frontend/package.json frontend/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile

# Build
COPY frontend/ ./
RUN pnpm build

# ─────────────────────────────────────────────
# Stage 2: Build backend
# ─────────────────────────────────────────────
FROM rust:1.88-slim-bookworm AS backend

WORKDIR /app

# Build dependencies (no libssl-dev needed — reqwest uses rustls, not native OpenSSL)
RUN apt-get update && \
    apt-get install -y --no-install-recommends pkg-config && \
    rm -rf /var/lib/apt/lists/*

# Copy workspace manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# Build release binary (server only — skips unused test deps)
RUN cargo build --release -p livrarr-server

# ─────────────────────────────────────────────
# Stage 3: Runtime image
# ─────────────────────────────────────────────
FROM debian:bookworm-slim

# CA certs for HTTPS outbound + wget for healthcheck
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates wget && \
    rm -rf /var/lib/apt/lists/*

# Non-root user with fixed UID/GID 1000
RUN groupadd -g 1000 livrarr && \
    useradd -u 1000 -g livrarr -d /app -s /usr/sbin/nologin livrarr

WORKDIR /app

# Copy binary and frontend assets
COPY --from=backend /app/target/release/livrarr ./livrarr
COPY --from=frontend /app/dist ./ui

RUN chown -R livrarr:livrarr /app

USER livrarr

# Config and data live in a volume — not baked in
VOLUME ["/config"]

EXPOSE 8789

ENTRYPOINT ["/app/livrarr", "--data", "/config", "--ui-dir", "/app/ui"]
