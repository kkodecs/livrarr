# Architecture Overview

Livrarr is a 10-crate Rust workspace with a React/TypeScript frontend. All dependency arrows point toward `livrarr-domain`. `livrarr-server` is the composition root — it depends on everything, nothing depends on it.

## Crate Dependency Graph

```
livrarr-domain (foundation — entities, traits, enums, errors)
│
├── livrarr-http        → domain (composable HTTP client middleware via tower)
├── livrarr-db          → domain (SQLite via sqlx, all SQL queries, migrations)
├── livrarr-organize    → domain (import pipeline, file layout, CWA copy)
├── livrarr-tagwrite    → domain (EPUB/M4B/MP3 metadata tag writing)
│
├── livrarr-metadata    → domain, http (enrichment pipeline, provider clients, LLM)
├── livrarr-download    → domain, http (Prowlarr, qBit, indexer clients)
│
├── livrarr-behavioral  → domain, db, metadata, download, organize, tagwrite (cross-crate tests)
│
├── livrarr-server      → ALL (composition root: axum, handlers, jobs, state)
└── frontend            → (React SPA, communicates via HTTP API only)
```

## Key Architectural Invariants

- **No SQL outside livrarr-db.** All queries live in Db traits.
- **No business logic in handlers.** Handlers: validate -> call trait -> map result.
- **All blocking I/O in spawn_blocking.** Never block the async executor.
- **Trait-based boundaries everywhere.** Production uses `SqliteDb`; tests use `:memory:`.
- **`trait_variant::make(Send)`** for async traits (not `async-trait`). Produces non-dyn-compatible traits — use generics/monomorphization exclusively.
- **Metadata matching is deterministic first.** LLM is a fallback for ambiguity, never the primary path.

## Composition Root (livrarr-server)

`AppState` wires all trait implementations using concrete types via type aliases (not `Arc<dyn Trait>`, not generics). Decision made via pk-confer (unanimous).

The server runs a 10-step startup sequence including permission check, PID lock, SQLite pool, PRAGMA quick_check, version gate, backup, migrations, FK checks, and cleanup.

## Frontend

React 19 SPA served as static files from `{data-dir}/ui/`. Communicates exclusively via REST API (`/api/v1/*`). Separate toolchain (Node/TypeScript), not part of the Rust workspace.
