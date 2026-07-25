# Architecture Overview

Livrarr is a 17-crate Rust workspace (14 application crates + `livrarr-jobs`, `livrarr-cli`, `livrarr-behavioral`) with a React/TypeScript frontend. All dependency arrows point toward `livrarr-domain`. `livrarr-server` is the composition root — no production crate depends on it (the behavioral test crate dev-depends on it, `crates/livrarr-behavioral/Cargo.toml:42`). It does **not** depend on everything: 11 of the other 16 members, per its own `Cargo.toml` — see the graph below. Authoritative member list: `Cargo.toml` `[workspace].members`. This page documents the **live** graph; the **intended** topology — which code must conform to — is the canonical model at `docs/canonical-model.yaml` (flow-side companion: [roads](roads.md)).

## Crate Dependency Graph

```
livrarr-domain (foundation — entities, traits, enums, errors)
│
├── livrarr-http          → domain (composable HTTP client middleware via tower)
├── livrarr-db            → domain (SQLite via sqlx, all SQL queries, migrations)
├── livrarr-tagwrite      → domain (EPUB/M4B/MP3 tag writing; audio writers disabled — OOM)
├── livrarr-matching      → domain (M1-M4 matching engine, extract/reconcile)
├── livrarr-jobs          → domain (JobService/DownloadPoller/AuthorMonitor traits.
│                           ⚠ NOTHING DEPENDS ON IT: no crate lists livrarr-jobs and no
│                           source file imports livrarr_jobs — the seam is declared but
│                           unwired. crates/livrarr-jobs/src/lib.rs:5, :38, :60)
│
├── livrarr-external-data → domain, http (provider substrate: transport, normalize, GCRA
│                           rate-limit, circuit breaker, payload cache; home of
│                           NormalizedWorkDetail / ProviderOutcome)
├── livrarr-identity      → domain, external-data, http ("what work is this?" — resolvers;
│                           never names enrichment)
├── livrarr-enrichment    → domain, external-data, db, http ("fill the record" — merge
│                           engine, provider retry queue; never names identity)
├── livrarr-materialize   → domain, http, tagwrite (the save home: covers + tag projection)
├── livrarr-library       → domain, db (import pipeline, file layout, CWA copy. The
│                           tagwrite edge that was conformance backlog is GONE —
│                           crates/livrarr-library/Cargo.toml:7-16)
├── livrarr-download      → domain, http, db (indexers + qBit/SAB/Transmission clients)
├── livrarr-metadata      → domain, http, db, matching, external-data, identity, enrichment,
│                           materialize (shrinking orchestrator: work_service spine)
│
├── livrarr-handlers      → domain, http, matching (ALL route handlers, each generic over
│                           the narrow `Has*` traits it uses — see Compile Wall below)
│                           COMPILE WALL: must NOT depend on db, metadata, tagwrite, download
│                           (nor the metadata extractions)
│
├── livrarr-behavioral    → domain, db, tagwrite as real deps; everything else is a
│                           DEV-dependency: metadata, matching, download, library,
│                           handlers, http, external-data, materialize, enrichment,
│                           and livrarr-server (Cargo.toml:15-18, :21-53)
├── livrarr-cli           → (stub binary — no internal deps currently)
│
├── livrarr-server        → 11 crates, not all: domain, db, http, matching, external-data,
│                           materialize, library, download, metadata, tagwrite, handlers
│                           (Cargo.toml). NOT identity, enrichment, jobs or cli — identity
│                           and enrichment are reached transitively through metadata.
│                           Zero route handlers — all routing delegates to livrarr-handlers
└── frontend              → (React SPA, communicates via HTTP API only)
```

## Key Architectural Invariants

- **No SQL outside livrarr-db.** All queries live in Db traits.
- **No business logic in handlers.** Handlers: validate → call trait → map result.
- **All blocking I/O in spawn_blocking.** Never block the async executor.
- **Trait-based boundaries everywhere.** Production uses `SqliteDb`; tests use `:memory:`.
- **`trait_variant::make(Send)`** for async traits (not `async-trait`). Produces non-dyn-compatible traits — use generics/monomorphization exclusively.
- **Metadata matching is deterministic, full stop — there is no LLM fallback for it.** Every provider picks with the shared `identity_matching::pick_best_candidate` and abstains below the bar: Goodreads ("no LLM is involved in the pick", `crates/livrarr-external-data/src/provider_client.rs:1967-1976`), Google Books (`google_books.rs:455-460`), Hardcover (`hardcover.rs:255-263`). Identity settles through the deterministic FLM gate (`crates/livrarr-identity/src/async_resolver.rs:318-353`), and the one LLM identity-verify function in the tree has no caller (`:46-97`). The LLM's remaining jobs are repair and cleanup — a failed foreign HTML parse, bibliography/series-list tidying — never selection.
- **Compile wall enforced by crate boundaries.** `livrarr-handlers` cannot import `livrarr-db`, `livrarr-metadata`, `livrarr-tagwrite`, or `livrarr-download`. Enforced at compile time, not convention.

## Compile Wall (Phase 5)

`livrarr-handlers` owns all HTTP route handlers. The compiler enforces that it cannot depend on `livrarr-db`, `livrarr-metadata`, `livrarr-tagwrite`, or `livrarr-download`. Verified via `cargo tree -p livrarr-handlers`.

Handlers are generic over the **narrow capability traits they actually use** — e.g. `pub async fn list<S: HasQueueService>` (`crates/livrarr-handlers/src/queue.rs:18`) — not over `AppContext`.

`AppContext` declares no associated types and no methods of its own. It is an empty supertrait union of the 49 `Has*` capability traits, with a blanket impl for any type satisfying them (`crates/livrarr-handlers/src/context.rs:294-345`, impl at `:347`). It exists for route composition, where the router needs every capability at once. `AppState` satisfies it by implementing each `Has*` trait with concrete types.

**Pattern for service fields in AppState** (one `Has*` trait per service, `context.rs:26-29`)**:**
- Field: `Arc<LiveFooService>` (Clone via Arc, service impl doesn't need Clone)
- AppContext type: `type FooSvc = LiveFooService` (inner type, not Arc)
- Accessor: `fn foo_service(&self) -> &Self::FooSvc { &self.foo_service }` (deref coercion: `&Arc<T>` → `&T`)

All services use explicit constructor injection — no `OnceLock` or late-init patterns remain.

## Composition Root (livrarr-server)

`AppState` wires all trait implementations using concrete types via type aliases in `state.rs`. All service fields are `Arc<ServiceImpl>`. The server owns utility modules (import pipeline, email SMTP, matching engine, readarr client) that handlers access through service traits.

The server runs a **15-step** numbered startup sequence (`crates/livrarr-server/src/main.rs`). Steps 1–10 gate the database, and their **order matters**: data dir (`:548`) → config + tracing (`:559`) → permission check (`:592`) → PID lock (`:598`) → SQLite pool (`:604`) → **pre-migration backup (`:614`) → migrations (`:628`) → version gate (`:636`)** → three identity backfills, 9b/9c/9d (`:643`, `:652`, `:662`) → backup cleanup, keep 3 (`:672`). Steps 11–15 follow: startup recovery (`:528`), background jobs (`:533`), router (`:536`), listener bind (`:970`), serve with graceful shutdown (`:1032`).

The backup is taken **before** migrations run and the version gate fires **after** them — not the other way round. There is no `PRAGMA quick_check` and no foreign-key check in the sequence; `foreign_keys=ON` is a pool pragma (`crates/livrarr-db/src/pool.rs:21`), not a startup verification step.

## Frontend

React 19 SPA served as static files from `{data-dir}/ui/`. Communicates exclusively via REST API (`/api/v1/*`). Separate toolchain (Node/TypeScript), not part of the Rust workspace.
