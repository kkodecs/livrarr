# Architecture Overview

Livrarr is a 17-crate Rust workspace (14 application crates + `livrarr-jobs`, `livrarr-cli`, `livrarr-behavioral`) with a React/TypeScript frontend. All dependency arrows point toward `livrarr-domain`. `livrarr-server` is the composition root — it depends on everything, nothing depends on it. Authoritative member list: `Cargo.toml` `[workspace].members`. This page documents the **live** graph; the **intended** topology — which code must conform to — is the canonical model at `docs/canonical-model.yaml` (flow-side companion: [roads](roads.md)).

## Crate Dependency Graph

```
livrarr-domain (foundation — entities, traits, enums, errors)
│
├── livrarr-http          → domain (composable HTTP client middleware via tower)
├── livrarr-db            → domain (SQLite via sqlx, all SQL queries, migrations)
├── livrarr-tagwrite      → domain (EPUB/M4B/MP3 tag writing; audio writers disabled — OOM)
├── livrarr-matching      → domain (M1-M4 matching engine, extract/reconcile)
├── livrarr-jobs          → domain (JobService trait ONLY — compile-wall-safe job
│                           triggering; job impls live in livrarr-server)
│
├── livrarr-external-data → domain, http (provider substrate: transport, normalize, GCRA
│                           rate-limit, circuit breaker, payload cache; home of
│                           NormalizedWorkDetail / ProviderOutcome)
├── livrarr-identity      → domain, external-data, http ("what work is this?" — resolvers;
│                           never names enrichment)
├── livrarr-enrichment    → domain, external-data, db, http ("fill the record" — merge
│                           engine, provider retry queue; never names identity)
├── livrarr-materialize   → domain, http, tagwrite (the save home: covers + tag projection)
├── livrarr-library       → domain, db, tagwrite (import pipeline, file layout, CWA copy;
│                           tagwrite edge is conformance backlog — intent: via materialize)
├── livrarr-download      → domain, http, db (indexers + qBit/SAB/Transmission clients)
├── livrarr-metadata      → domain, http, db, matching, external-data, identity, enrichment,
│                           materialize (shrinking orchestrator: work_service spine)
│
├── livrarr-handlers      → domain, http, matching (ALL route handlers, generic over AppContext)
│                           COMPILE WALL: must NOT depend on db, metadata, tagwrite, download
│                           (nor the metadata extractions)
│
├── livrarr-behavioral    → db, domain, download, external-data, handlers, http, library,
│                           matching, materialize, metadata (cross-crate tests)
├── livrarr-cli           → (stub binary — no internal deps currently)
│
├── livrarr-server        → ALL (composition root: axum, AppState, jobs, service impls)
│                           Zero route handlers — all routing delegates to livrarr-handlers
└── frontend              → (React SPA, communicates via HTTP API only)
```

## Key Architectural Invariants

- **No SQL outside livrarr-db.** All queries live in Db traits.
- **No business logic in handlers.** Handlers: validate → call trait → map result.
- **All blocking I/O in spawn_blocking.** Never block the async executor.
- **Trait-based boundaries everywhere.** Production uses `SqliteDb`; tests use `:memory:`.
- **`trait_variant::make(Send)`** for async traits (not `async-trait`). Produces non-dyn-compatible traits — use generics/monomorphization exclusively.
- **Metadata matching is deterministic first.** LLM is a fallback for ambiguity, never the primary path.
- **Compile wall enforced by crate boundaries.** `livrarr-handlers` cannot import `livrarr-db`, `livrarr-metadata`, `livrarr-tagwrite`, or `livrarr-download`. Enforced at compile time, not convention.

## Compile Wall (Phase 5)

`livrarr-handlers` owns all HTTP route handlers. The compiler enforces that it cannot depend on `livrarr-db`, `livrarr-metadata`, `livrarr-tagwrite`, or `livrarr-download`. Verified via `cargo tree -p livrarr-handlers`.

Handlers are generic over `S: AppContext`. The `AppContext` trait (29 associated types, ~40 accessors) defines the service surface available to handlers. `AppState` implements `AppContext` with concrete types.

**Pattern for service fields in AppState:**
- Field: `Arc<LiveFooService>` (Clone via Arc, service impl doesn't need Clone)
- AppContext type: `type FooSvc = LiveFooService` (inner type, not Arc)
- Accessor: `fn foo_service(&self) -> &Self::FooSvc { &self.foo_service }` (deref coercion: `&Arc<T>` → `&T`)

All services use explicit constructor injection — no `OnceLock` or late-init patterns remain.

## Composition Root (livrarr-server)

`AppState` wires all trait implementations using concrete types via type aliases in `state.rs`. All service fields are `Arc<ServiceImpl>`. The server owns utility modules (import pipeline, email SMTP, matching engine, readarr client) that handlers access through service traits.

The server runs a 10-step startup sequence including permission check, PID lock, SQLite pool, PRAGMA quick_check, version gate, backup, migrations, FK checks, and cleanup.

## Frontend

React 19 SPA served as static files from `{data-dir}/ui/`. Communicates exclusively via REST API (`/api/v1/*`). Separate toolchain (Node/TypeScript), not part of the Rust workspace.
