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
# Stage 2a: chef base — toolchain + cargo-chef, cached independent of source
# ─────────────────────────────────────────────
FROM rust:1.94-alpine AS chef

WORKDIR /app

ARG TARGETARCH
ARG BUILDARCH

# musl-dev + gcc required by libsqlite3-sys bundled (compiles SQLite from C source)
# cross-compilation uses a prebuilt musl.cc toolchain only when building x86_64 -> arm64
RUN apk add --no-cache musl-dev gcc curl && \
    if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDARCH" = "amd64" ]; then \
      curl -fsSL https://github.com/kkodecs/livrarr/releases/download/toolchain/aarch64-linux-musl-cross.tgz | tar -xz -C /usr/local && \
      rustup target add aarch64-unknown-linux-musl; \
    fi && \
    cargo install cargo-chef --version 0.1.77 --locked

# ─────────────────────────────────────────────
# Stage 2b: planner — compute the dependency-only recipe from Cargo.lock
# ─────────────────────────────────────────────
FROM chef AS planner

COPY Cargo.toml Cargo.lock ./
COPY .cargo/ ./.cargo/
COPY crates/ ./crates/
RUN cargo chef prepare --recipe-path recipe.json

# ─────────────────────────────────────────────
# Stage 2c: backend — cook deps from the recipe (cache hit unless Cargo.lock
# changed), then build our own crates against the pre-built deps.
# ─────────────────────────────────────────────
FROM chef AS backend

ARG TARGETARCH
ARG BUILDARCH

COPY --from=planner /app/recipe.json recipe.json
RUN if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDARCH" = "amd64" ]; then \
      CC_aarch64_unknown_linux_musl=/usr/local/aarch64-linux-musl-cross/bin/aarch64-linux-musl-gcc \
      cargo chef cook --release -p livrarr-server --target aarch64-unknown-linux-musl --recipe-path recipe.json; \
    else \
      CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=gcc \
      cargo chef cook --release -p livrarr-server --recipe-path recipe.json; \
    fi

COPY Cargo.toml Cargo.lock ./
COPY .cargo/ ./.cargo/
COPY crates/ ./crates/

RUN if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDARCH" = "amd64" ]; then \
      CC_aarch64_unknown_linux_musl=/usr/local/aarch64-linux-musl-cross/bin/aarch64-linux-musl-gcc \
      cargo build --release -p livrarr-server --target aarch64-unknown-linux-musl; \
    else \
      CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=gcc \
      cargo build --release -p livrarr-server; \
    fi

# ─────────────────────────────────────────────
# Stage 3: Runtime image (~35-40MB)
# ─────────────────────────────────────────────
FROM alpine:3.21

RUN apk add --no-cache ca-certificates tzdata tini su-exec

# Non-root user
RUN addgroup -g 1000 livrarr && \
    adduser -u 1000 -G livrarr -D -H -s /sbin/nologin livrarr

WORKDIR /app

ARG TARGETARCH
ARG BUILDARCH
RUN --mount=type=bind,from=backend,source=/app,target=/build \
    if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDARCH" = "amd64" ]; then \
      cp /build/target/aarch64-unknown-linux-musl/release/livrarr ./livrarr; \
    else \
      cp /build/target/release/livrarr ./livrarr; \
    fi
COPY --from=frontend /app/dist ./ui
COPY docker/entrypoint.sh /entrypoint.sh

# Start as root so the entrypoint can apply PUID/PGID, fix /config ownership, and drop
# privileges (LinuxServer.io pattern). The entrypoint always execs the app as a non-root
# user (default 1000:1000); it never leaves the app running as root.
RUN chown -R livrarr:livrarr /app && chmod +x /entrypoint.sh

VOLUME ["/config"]

EXPOSE 8789

ENTRYPOINT ["/entrypoint.sh"]
