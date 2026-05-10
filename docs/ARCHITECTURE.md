# Architecture

Livrarr is a self-hosted book management application for ebooks and audiobooks, built as a 13-crate Rust workspace with a React/TypeScript frontend.

## System Overview

```
┌─────────────────────────────────────────────────┐
│                  livrarr-server                  │
│          (composition root + axum HTTP)          │
├──────────┬──────────┬──────────┬────────────────┤
│ handlers │   jobs   │  state   │  auth/middleware│
└────┬─────┴────┬─────┴────┬─────┴────────┬───────┘
     │          │          │              │
┌────▼────┐ ┌──▼──────┐ ┌─▼────────┐ ┌──▼────────┐
│metadata │ │download │ │ library  │ │ tagwrite  │
│(enrich, │ │(indexer,│ │(import,  │ │(epub/m4b/ │
│ search) │ │ qbit)   │ │ layout)  │ │  mp3 tags)│
└────┬────┘ └──┬──────┘ └─┬────────┘ └───────────┘
     │         │           │
┌────▼─────────▼───────────▼──────────────────────┐
│                  livrarr-http                     │
│           (SSRF, rate limiting, retry)            │
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
| `livrarr-db` | All SQL queries and migrations. Trait-based data access. SQLite WAL mode. |
| `livrarr-http` | SSRF-safe HTTP client, rate limiting, retry, user-agent injection via tower middleware. |
| `livrarr-metadata` | Enrichment pipeline. Provider clients (Hardcover, OpenLibrary, Audnexus, Goodreads). LLM validator. Cover cache. |
| `livrarr-download` | Download client integration (qBittorrent, SABnzbd). Indexer search (Torznab). |
| `livrarr-matching` | M1-M4 matching engine: embedded metadata, path parsing, string matching, scoring. |
| `livrarr-library` | Import workflow, file layout enforcement, CWA downstream copy. |
| `livrarr-tagwrite` | EPUB/M4B/MP3 metadata tag writing. Format-specific heavy dependencies isolated here. |
| `livrarr-handlers` | All Axum route handlers + DTOs. Generic over `AppContext`. Compile wall: cannot depend on db, metadata, tagwrite, or download. |
| `livrarr-server` | Composition root. AppState, service wiring, background jobs, auth, startup sequence. |
| `livrarr-jobs` | Thin trait crate so handlers can trigger jobs without depending on server. |
| `livrarr-behavioral` | Cross-crate behavioral tests and test stubs. |
| `livrarr-cli` | Command-line client (stub). |
| `frontend` | React 19 SPA. Separate toolchain (Node/TypeScript). Served as static files. |

## Key Invariants

- All dependency arrows point toward `livrarr-domain`. No cycles.
- No SQL outside `livrarr-db`. No business logic in handlers.
- All blocking file I/O in `tokio::spawn_blocking`.
- Multi-user from day one: every user-scoped table has `user_id`, every query filters by it.
- Metadata enrichment is deterministic first, LLM fallback only.
- Files are the artifact: metadata written into EPUB/M4B/MP3 at import time.
- Compile wall enforced by crate boundaries: `livrarr-handlers` cannot import `livrarr-db`, `livrarr-metadata`, `livrarr-tagwrite`, or `livrarr-download`.

## Data Layer

SQLite with WAL mode. Single write connection, multiple readers. Per-connection pragmas: `foreign_keys = ON`, `busy_timeout = 5000`. Migrations via sqlx (embedded, run at startup with backup).

## Deployment

Single-container Docker on Linux. Multi-stage build (rust:bookworm builder, debian:bookworm-slim runtime). PUID/PGID user creation in entrypoint. Target hardware floor: Raspberry Pi 4.

## Detailed Documentation

- Domain entities: `wiki/domain/`
- Subsystem deep-dives: `wiki/architecture/`
- Patterns: `wiki/patterns/`
- Key decisions: `wiki/decisions/`
- Crate reference: `wiki/crates/`
