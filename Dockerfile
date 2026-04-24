# ─────────────────────────────────────────────
# Stage 1: Build frontend
# ─────────────────────────────────────────────
FROM node:20-alpine AS frontend

WORKDIR /app

RUN corepack enable && corepack prepare pnpm@10.33.0 --activate

COPY frontend/package.json frontend/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile

COPY frontend/ ./
RUN pnpm build

# ─────────────────────────────────────────────
# Stage 2: Build backend (musl static binary)
# ─────────────────────────────────────────────
FROM rust:1.94-alpine AS backend

WORKDIR /app

# musl-dev + gcc required by sqlx bundled (compiles SQLite from C source)
RUN apk add --no-cache musl-dev gcc

COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# Alpine's default target is x86_64-unknown-linux-musl — no --target flag needed
RUN cargo build --release -p livrarr-server

# ─────────────────────────────────────────────
# Stage 3: Runtime image (~35-40MB)
# ─────────────────────────────────────────────
FROM alpine:3.21

RUN apk add --no-cache ca-certificates tzdata tini curl

# Non-root user
RUN addgroup -g 1000 livrarr && \
    adduser -u 1000 -G livrarr -D -H -s /sbin/nologin livrarr

WORKDIR /app

COPY --from=backend /app/target/release/livrarr ./livrarr
COPY --from=frontend /app/dist ./ui

RUN chown -R livrarr:livrarr /app

USER livrarr

VOLUME ["/config"]

EXPOSE 8789

ENTRYPOINT ["/sbin/tini", "--", "/app/livrarr", "--data", "/config", "--ui-dir", "/app/ui"]
