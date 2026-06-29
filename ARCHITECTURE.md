# Livrarr — Architecture

This document has two parts:

- **Part 1 — Product Principles:** what Livrarr is and what it stands for. The *why* that makes the structural decisions make sense.
- **Part 2 — Structure:** the crates, dependencies, patterns, and conventions. The *how*.

For universal engineering principles that apply regardless of product, see [`PRINCIPLES.md`](PRINCIPLES.md). If this document and `PRINCIPLES.md` conflict, `PRINCIPLES.md` wins.

If the code and this document diverge, stop and resolve the mismatch. If the architecture changes intentionally, update this document in the same commit.

---

# Part 1 — Product Principles

## What Livrarr Is

Livrarr is a self-hosted book library manager for ebooks and audiobooks. It downloads books automatically from torrent and Usenet, organizes them, and lets users read and listen from a single app. It tracks authors and series, handles foreign languages, imports from external sources, and keeps metadata accurate and up to date.

The hard problem is metadata. Book metadata is fragmented across hostile sources that actively resist automated access. Livrarr's job is to get it right anyway — reliably, automatically, without getting banned, and without leaking user data to do it.

## Core Product Commitments

### User Intent Is Final — and Nothing Happens Without It

When a user makes a decision — affirm a book, resolve a conflict, select from search — the system must act on it immediately and completely. No silently ignored actions. No badge that never updates. No state the user cannot escape without deleting and re-adding.

User intent sits at the top of every decision hierarchy. If the system ignores a user action, that is a bug.

The corollary: Livrarr does not take actions on a user's files or data without explicit consent. No silent writes, no behind-the-scenes modifications. If Livrarr wants to do something to a user's file, it asks first. Tag writing is an offer, not an automatic step.

### Uncertainty Is Visible, Not Silent

When Livrarr cannot identify a book with confidence, it must say so. Works that cannot be identified go into a review queue for the user to resolve. There are no stuck states, no silent limbo, no permanent terminal states the user cannot clear.

An honest "I don't know" is better than a confident wrong answer.

### Identity Has One Confidence Hierarchy

Book identity is resolved in this order, highest confidence first:

1. **User selection** — the user picked it; this is final and cannot be overridden
2. **Known provider ID** — from import metadata or explicit pass-through; trust it fully
3. **Title + author fuzzy match** — the universal fallback; every book has these fields
4. **Unidentified** — goes to the review queue; never silent, never stuck

ISBN is a hint, not an authority. It identifies an edition, not a work, and is brittle in practice.

This hierarchy must be implemented once, in one place, not re-derived per entry point.

### Providers Are Interchangeable

A metadata provider (Goodreads, Hardcover, OpenLibrary, Google Books, Audnexus, Audible) is an implementation of a trait. The rest of the system does not care which provider runs.

Adding a new provider means implementing the trait contract — nothing else changes. Provider-specific behavior (auth, parsing, quirks) lives inside the provider and does not leak into the identity engine, the enrichment orchestrator, or the merge layer.

### The Canonical Transport Is the Only Transport

All outbound HTTP goes through `livrarr-http`. No exceptions.

`livrarr-http` is the sole owner of:
- rate limiting (per-provider, process-global — one shared instance)
- 429 handling and backoff
- SSRF protection
- retry policy
- user-agent injection

Production code must not issue raw `reqwest` calls outside this crate. Production code must not re-implement rate limiting locally. A new provider integration is incomplete until it uses this path.

### Files Are the Artifact

The book file on disk is what Livrarr manages. The file is the thing the user cares about — not Livrarr's database representation of it.

This means: Livrarr organizes, serves, and tracks files. It does not modify them without the user's explicit consent. Tag writing (embedding metadata into EPUB/M4B/MP3) is an offer the user can accept or decline — it is never done automatically or silently.

A library managed by Livrarr must remain useful if Livrarr is uninstalled. The files are the user's; Livrarr is the manager, not the owner.

### Privacy by Default

Only publicly available information leaves the machine.

External calls — to metadata providers (Goodreads, Hardcover, OpenLibrary, etc.) and to LLMs — send only what is already on the public internet: title, author, provider keys. This is not a privacy violation because any person could look this up themselves.

What never leaves the machine: file paths, filenames, checksums, reading history, reading position, user preferences, credentials, or any information that could identify the user or their specific copy of a file.

No telemetry. No tracking. Livrarr does not phone home.

---

# Part 2 — Structure

## System Overview

```
                        [user / browser]
                               │
                        livrarr-server
                    (composition root, axum)
                               │
              ┌────────────────┼────────────────┐
              │                │                │
       livrarr-handlers   background jobs   auth/middleware
       (COMPILE WALL ──────────────────────────────────────────┐)
              │                                                 │
              │ (trait calls only, no direct deps below wall)   │
              └────────────────────────────────────────────────┘
                               │ trait calls
              ┌────────────────┼────────────────────────────────┐
              │                │                │               │
       livrarr-metadata  livrarr-download  livrarr-library  livrarr-tagwrite
       (orchestration)   (qBit, SABnzbd,   (import,         (EPUB/M4B/MP3
                          Torznab)          file layout)      tag writing)
              │
    ┌─────────┼──────────┐
    │         │          │
livrarr-  livrarr-   livrarr-
identity  enrichment materialize
              │
    livrarr-external-data
    (GR, HC, OL, GB, Audnexus, Audible)
              │
        livrarr-http
    (transport, rate limit, SSRF)
              │
        livrarr-domain ◄── everything depends here
    (types, traits, enums)
              │
         livrarr-db
         (SQLite/sqlx)

Supporting: livrarr-matching (release parsing/scoring)
            livrarr-jobs (job trigger traits, compile-wall safe)
            livrarr-behavioral (test harness + stubs)
            livrarr-cli (stub)
```

## Dependency Rules

**All dependency arrows point toward `livrarr-domain`. No cycles.**

| Crate | May depend on |
|---|---|
| `livrarr-domain` | Nothing (serde, chrono, thiserror, tokio only) |
| `livrarr-db` | domain |
| `livrarr-http` | domain |
| `livrarr-matching` | domain |
| `livrarr-external-data` | domain, http |
| `livrarr-identity` | domain, http, db, external-data |
| `livrarr-enrichment` | domain, http, db, external-data |
| `livrarr-materialize` | domain, http, db |
| `livrarr-metadata` | domain, db, identity, enrichment, materialize, external-data |
| `livrarr-download` | domain, http, db |
| `livrarr-library` | domain, db, matching |
| `livrarr-tagwrite` | domain |
| `livrarr-jobs` | domain only |
| `livrarr-handlers` | **domain + jobs only — COMPILE WALL** |
| `livrarr-server` | everything (composition root) |

Verify the compile wall: `cargo tree -p livrarr-handlers`

Nothing depends on `livrarr-server`.

---

## Crate Responsibilities

### `livrarr-domain`
Pure type library. Entities, ID newtypes, enums, error types, service traits. Zero external deps beyond serde/chrono/thiserror. The canonical title normalizer (`text_norm`) lives here — it is the only normalizer.

**Non-responsibilities:** business logic, persistence, HTTP.

### `livrarr-db`
All SQL queries and migrations. `SqliteDb` implements the `*Db` traits defined in domain. No SQL anywhere else.

### `livrarr-http`
The canonical HTTP transport. Owns rate limiting, SSRF protection, retry, 429 handling, user-agent injection. All outbound HTTP goes through here. See Principle: The Canonical Transport Is the Only Transport.

### `livrarr-external-data`
Provider clients: Goodreads, Hardcover, OpenLibrary, Google Books, Audnexus, Audible. Each implements the provider trait. Transport via `livrarr-http`. Provider-specific auth, parsing, and quirks are isolated here.

### `livrarr-identity`
Identity resolution. `settle_identity` → provider fan-out → `run_quorum` → write anchors. Implements the confidence hierarchy. One quorum algorithm. Calls `text_norm` from domain — never its own normalizer.

### `livrarr-enrichment`
Enrichment pipeline. Fetches full payloads from providers, merges into one record. `DefaultMergeEngine` is the sole merge authority: deterministic, priority-ordered, null-guarded, single language-incompatibility chokepoint.

### `livrarr-materialize`
Covers, dimensions, atomic file writes. Downloads and decodes cover images, persists dimensions, performs atomic disk writes.

### `livrarr-metadata`
Orchestration. `WorkService` drives the full pipeline (identity → enrichment → materialize → persist). Background convergence, author monitor, refresh. **Must not grow into a god object** — split by domain concern when methods exceed a coherent unit.

### `livrarr-matching`
Release title parsing, candidate scoring, M1–M4 matching pipeline, embedded metadata extraction. Used by import flows.

### `livrarr-download`
Download client integrations (qBittorrent, SABnzbd) and Torznab indexer search.

### `livrarr-library`
Import workflow, file layout enforcement, CWA downstream copy. Owns where files live on disk.

### `livrarr-tagwrite`
EPUB, M4B, and MP3 metadata tag writing. Format-specific heavy dependencies isolated here.

### `livrarr-handlers`
All Axum route handlers and DTOs. Generic over `AppContext`. **Compile wall.** Handlers validate input, call a trait method, map the result. No business logic, no SQL, no file I/O.

```rust
async fn handler(State(s): State<S>, ...) -> Result<Json<Dto>, AppError> {
    let input = validate(raw_input)?;
    let result = s.some_service().do_thing(input).await?;
    Ok(Json(Dto::from(result)))
}
```

### `livrarr-server`
Composition root. Constructs `AppState`, wires all concrete implementations, starts background jobs, configures auth, runs the router. Nothing depends on this crate.

### `livrarr-jobs`
Thin trait crate. Job trigger traits so handlers can fire background jobs without depending on `livrarr-server`.

---

## Hard Invariants

These are non-negotiable. Violating them is a bug, not a judgment call.

- No SQL outside `livrarr-db`
- No outbound HTTP outside `livrarr-http`
- No business logic in handlers
- All blocking file I/O in `tokio::spawn_blocking`
- Every user-scoped table has `user_id`; every query filters by it
- The compile wall is real — verify with `cargo tree -p livrarr-handlers`
- Applied migrations are immutable — never edit a shipped migration file
- The rate limiter is process-global — never create a local one
- File paths, checksums, reading history, and preferences are never transmitted externally
- Tag writing (EPUB/M4B/MP3) is user-initiated only — never automatic or silent
- No telemetry, no analytics, no external reporting of any kind

## Current Conventions

These are established patterns. Follow them; deviate with a reason.

- Identity enrichment is deterministic first; LLM is a fallback only
- Async traits use `#[trait_variant::make(Send)]`, not `#[async_trait]` on new code
- Test DB is real SQLite `:memory:` via `create_test_db()` — no in-memory fakes
- HTTP stubs, LLM stubs, filesystem stubs are acceptable; DB stubs are not
- `chrono` for datetime, never the `time` crate

---

## Canonical Feature Flow

1. Request arrives at a handler (validate input, extract actor).
2. Handler calls a service trait method.
3. Service performs business logic, calls repository traits for persistence.
4. Background work spawned from handler if needed (`State<S>` is cloneable).
5. Service returns domain result; handler maps to DTO and HTTP response.

A feature that needs a side channel bypassing this flow is a design smell.

---

## Patterns

### Adding a New Metadata Provider

1. Add a client struct in `livrarr-external-data`. All HTTP through `livrarr-http`.
2. Implement the `ProviderClient` trait (defined in `livrarr-domain`).
3. Add the provider to the enrichment dispatch table in `livrarr-enrichment`.
4. Add a `RateBucket` variant in `livrarr-http`. The limiter is process-global and shared — do not create a local one.
5. Add behavioral tests in `livrarr-behavioral`.

### Adding a New Route Handler

1. Add a use-case method to the appropriate service trait in `livrarr-domain`.
2. Implement it in the relevant service crate.
3. Add a `Has*` capability trait in `livrarr-handlers/src/context.rs`.
4. Implement `Has*` on `AppState` in `livrarr-server/src/state.rs`.
5. Write the handler in `livrarr-handlers`: validate → call trait → map result.
6. Register the route in `livrarr-server`.

### Adding a New Background Job

1. Define the trigger method on a trait in `livrarr-jobs`.
2. Implement it in `livrarr-server`.
3. Handlers spawn via `tokio::spawn` + `state.clone()`.
4. All sleeps must use `tokio::select!` with a `CancellationToken`.

### Adding a New Database Table

1. New migration file in `crates/livrarr-db/migrations/`. Never edit existing migrations.
2. Define the `*Db` trait in `livrarr-domain`.
3. Implement on `SqliteDb` in `livrarr-db`.
4. Wire through `AppState` in `livrarr-server`.

---

## Naming Conventions

| Thing | Convention |
|---|---|
| Crates | `livrarr-{name}` (hyphenated) |
| Persistence traits | `{Resource}Db` |
| Service traits | `{Domain}Service` |
| Capability traits (handlers) | `Has{Service}` |
| Handler files | one file per resource group |
| DB enums (single word) | `lowercase` |
| DB enums (multi-word) | `snake_case` |
| API enums | `#[serde(rename_all = "snake_case")]` or explicit renames |

---

## Data Layer

SQLite with WAL mode. Single write connection, multiple readers. Per-connection pragmas: `foreign_keys = ON`, `busy_timeout = 5000`. Migrations via sqlx — embedded, run at startup before serving traffic. Once a migration ships in any release, it is immutable.

---

## Deployment

Single-container Docker on Linux. Multi-stage build (rust:bookworm builder, debian:bookworm-slim runtime). PUID/PGID user creation in entrypoint. Target hardware floor: Raspberry Pi 4. Runtime data mapped to `/config` by the user at deploy time.

---

## When to Update This Document

Update `ARCHITECTURE.md` when:
- A new crate is added or removed
- A dependency rule changes
- A new class of provider or background job is introduced
- The compile wall boundary moves
- A new canonical pattern is established

Update `PRINCIPLES.md` when:
- A universal engineering rule is added or refined
- The conflict resolution ladder changes

---

## Detailed Documentation

When a wiki page conflicts with `ARCHITECTURE.md`, `ARCHITECTURE.md` is authoritative. Update the wiki page to match, not the other way around.

- Domain entities: `wiki/domain/`
- Subsystem deep-dives: `wiki/architecture/`
- Patterns reference: `wiki/patterns/`
- Key decisions: `wiki/decisions/`
- Integration quirks: `wiki/integrations/`
- Crate reference: `wiki/crates/`
- Active learnings: `wiki/insights.md`
