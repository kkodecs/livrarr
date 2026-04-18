# Architecture

Livrarr is a self-hosted book management application for ebooks and audiobooks, built as a 10-crate Rust workspace with a React/TypeScript frontend.

## System Overview

```
┌─────────────────────────────────────────────────┐
│                  livrarr-server                  │
│          (composition root + axum HTTP)          │
├──────────┬──────────┬──────────┬────────────────┤
│ handlers │   jobs   │  state   │    auth/middleware│
└────┬─────┴────┬─────┴────┬─────┴────────┬───────┘
     │          │          │              │
┌────▼────┐ ┌──▼──────┐ ┌─▼────────┐ ┌──▼────────┐
│metadata │ │download │ │ organize │ │ tagwrite  │
│(enrich, │ │(indexer,│ │(import,  │ │(epub/m4b/ │
│ search) │ │ qbit)   │ │ layout)  │ │  mp3 tags)│
└────┬────┘ └──┬──────┘ └─┬────────┘ └───────────┘
     │         │           │
┌────▼─────────▼───────────▼──────────────────────┐
│                  livrarr-http                     │
│        (tower middleware: retry, rate limit)      │
└─────────────────────┬───────────────────────────┘
                      │
┌─────────────────────▼───────────────────────────┐
│                 livrarr-domain                    │
│     (entities, traits, enums, error types)        │
└─────────────────────────────────────────────────┘
                      │
┌─────────────────────▼───────────────────────────┐
│                  livrarr-db                       │
│          (SQLite via sqlx, migrations)            │
└─────────────────────────────────────────────────┘
```

## Crate Responsibilities

| Crate | Purpose |
|-------|---------|
| `livrarr-domain` | Foundation. Entities, ID types, enums, error types, service traits. Zero external deps beyond serde/chrono. |
| `livrarr-db` | All SQL queries and migrations. Trait-based data access (12 Db traits). SQLite WAL mode. |
| `livrarr-http` | Composable HTTP client middleware via tower. Timeout, retry, rate limiting, user-agent. |
| `livrarr-metadata` | Enrichment pipeline. Provider clients (Hardcover, OpenLibrary, Audnexus, GoodReads). LLM validator. Cover cache. |
| `livrarr-download` | Download client integration. Indexer search (Torznab). qBittorrent/SABnzbd clients. |
| `livrarr-organize` | Import pipeline. File layout enforcement. Manual scan. CWA downstream copy. |
| `livrarr-tagwrite` | EPUB/M4B/MP3 metadata tag writing. Format-specific heavy dependencies isolated here. |
| `livrarr-server` | Composition root. Axum HTTP server, route handlers, background jobs, auth, startup sequence. |
| `livrarr-behavioral` | Cross-crate behavioral tests and test stubs. |
| `frontend` | React 19 SPA. Separate toolchain (Node/TypeScript). Served as static files. |

## Key Invariants

- All dependency arrows point toward `livrarr-domain`. No cycles.
- No SQL outside `livrarr-db`. No business logic in handlers.
- All blocking file I/O in `tokio::spawn_blocking`.
- Multi-user from day one: every user-scoped table has `user_id`, every query filters by it.
- Metadata enrichment is deterministic first, LLM fallback only.
- Files are the artifact: metadata written into EPUB/M4B/MP3 at import time.

## Data Layer

SQLite with WAL mode. Single write connection, multiple readers. Per-connection pragmas: `foreign_keys = ON`, `busy_timeout = 5000`. Migrations via sqlx (embedded, run at startup with backup).

## Deployment

Single-container Docker on Linux. Multi-stage build (rust:bookworm builder, debian:bookworm-slim runtime). PUID/PGID user creation in entrypoint. Target hardware floor: Raspberry Pi 4.

## Detailed Documentation

- Domain entities: `wiki/domain/`
- Subsystem deep-dives: `wiki/architecture/`
- Patterns: `wiki/patterns/`
- Key decisions: `wiki/decisions/`
