# Livrarr — Architecture

This document has two parts:

- **Part 1 — Product Principles:** what Livrarr is and what it stands for. The *why* that makes the structural decisions make sense.
- **Part 2 — Structure:** the crates, dependencies, patterns, and conventions. The *how*.

For universal engineering principles that apply regardless of product, see [`PRINCIPLES.md`](PRINCIPLES.md). If this document and `PRINCIPLES.md` conflict, `PRINCIPLES.md` wins.

If the code and this document diverge, stop and resolve the mismatch. If the architecture changes intentionally, update this document in the same commit.

For a review, distinguish a requirement from evidence that it is implemented. Record
the source, consequence and uncertainty of a mismatch; do not repair code merely to
make it match this document. The approved scan method is in the
[architecture review plan](build/reviews/architecture-review-2026-09-07/PLAN.md).

**Structural reference checked 2026-09-09:** the reviewed source is
`/mnt/opt/livrarr-worktrees/merge-containment` at
`af865bdb76d3cd1e42302f9791830479a26220f5`, including its reviewed containment patch.
The document home is `/mnt/opt/livrarr`; its parked implementation is not the review
baseline. The [document review](build/reviews/architecture-review-2026-09-07/context-docs-017/REVIEW.md)
records targeted checks and unresolved wording questions. This is not an audit of
every implementation against every principle.

---

# Part 1 — Product Principles

## What Livrarr Is

Livrarr is a self-hosted book library manager for ebooks and audiobooks. It downloads books automatically from torrent and Usenet, organizes them, and lets users read and listen from a single app. It tracks authors and series, handles foreign languages, imports from external sources, and keeps metadata accurate and up to date.

The hard problem is metadata. Book metadata is fragmented across hostile sources that actively resist automated access. Livrarr's job is to get it right anyway — reliably, automatically, without getting banned, and without leaking user data to do it.

## Core Product Commitments

### Work-First, Not Author-First

The user wants a book. Authors are metadata, not the entry point. The Work — a title, independent of format, edition, or packaging — is the primary entity in the data model, the UI, and every workflow. Users search for works, add works, then find and grab releases in whichever media type (ebook or audiobook) they want.

This is why Livrarr manages both formats as one app instead of two: a Work spans formats. Fragmenting the same book across separate ebook and audiobook records would scatter the same title across multiple places in the UI.

### User Intent Is Final — and Nothing Happens Without It

When a user makes a decision — affirm a book, resolve a conflict, select from search — the system must act on it immediately and completely. No silently ignored actions. No badge that never updates. No state the user cannot escape without deleting and re-adding.

User intent governs product decisions within the data-safety priority defined in
`PRINCIPLES.md`. If the system cannot safely carry out an action, it must explain why;
silently ignoring it is a bug.

The corollary: Livrarr does not take actions on a user's files or data without a triggering user action. No silent writes outside that action's scope, no behind-the-scenes modifications to files already settled in the library. Tag writing at import time is part of adding the book — the user's add *is* the consent, not a separate automatic step done with no triggering action at all. A file already in the library is not modified again without a new, explicit user action — except to complete a workflow the user already started (e.g., a tag rewrite once an async metadata resolution fills in what was missing at add time).

### Uncertainty Is Visible, Not Silent

When Livrarr cannot identify a book with confidence, it must say so. Works that cannot be identified go into a review queue for the user to resolve. There are no stuck states, no silent limbo, no permanent terminal states the user cannot clear.

An honest "I don't know" is better than a confident wrong answer.

Enrichment follows the same visibility rule. An interactive Add creates or selects
the local work using available evidence, then returns while provider capture,
enrichment and a conditional delayed refresh continue in the background. Creation
minimum, sufficient identity evidence and descriptive completeness are different
questions; a returned record does not mean all provider work has finished.

Each entry path must expose uncertainty and failures and provide recovery. The
legacy `identity_status` badge is frozen after the identity-layer cutover; current
identity presentation and review come from the captured-identity workflow. Do not
use an old `identity-pending` badge as evidence that current work is still running.

The same honesty applies to failure: a provider timing out, an external dependency failing, or a downstream integration erroring degrades what Livrarr can offer — it never corrupts stored state or silently drops the user's original intent. A failed CWA copy logs a warning while the main import still succeeds; a failed provider fetch still creates the work with whatever data is available.

### Identity Has One Confidence Hierarchy

Book identity is resolved in this order, highest confidence first (amended 2026-08-05 by identity-layer-rewrite, matching spec P5's evidence ladder):

1. **User selection** — the user picked it; this is final and cannot be overridden
2. **The user's own file** — embedded identifiers (ISBN/ASIN) and embedded cover from a file the user owns; outranks any provider answer
3. **Known provider ID** — a provider route; trust it as a route to the work
4. **Title + author** — the universal minimum; every book has these fields, and a work is fully creatable from them alone with zero provider routes
5. **Unidentified** — goes to the review queue; never silent, never stuck

ISBN identifies an edition, not a work. Under the routes model it is an edition-scoped lookup key: a shared one may confirm sameness, a differing one proves nothing, and nothing may require or veto on it (spec P6 — this supersedes the pre-F2 "hint, not an authority" wording with an enforceable rule).

This hierarchy must be implemented once, in one place, not re-derived per entry point.

### Providers Are Interchangeable

A metadata provider (Goodreads, Hardcover, OpenLibrary, Google Books, Audnexus, Audible) is an implementation of a trait. The rest of the system does not care which provider runs.

Adding a new provider means implementing the trait contract — nothing else changes. Provider-specific behavior (auth, parsing, quirks) lives inside the provider and does not leak into the identity engine, the enrichment orchestrator, or the merge layer.

This is the intended separation, not a literal inventory of today's edit sites.
Current provider registration also involves enum dispatch and configured policies;
see the provider pattern below. Whether those connections meet this commitment is
a review question. A primary-provider policy remains an unapproved opportunity.

### LLM as Metadata Advisor

LLMs assist with metadata repair — for example, extracting fields from provider HTML that deterministic parsing misses. An LLM never selects a match, never triggers a download, never mutates library state, and never auto-accepts a result. Deterministic matching decides; ambiguity goes to the user, not an LLM.

Livrarr is fully functional with no LLM configured. LLMs are probabilistic; they advise, they don't decide.

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

The book file on disk is what Livrarr manages. The file is the thing the user cares about — not Livrarr's database representation of it. A correctly tagged EPUB or M4B is self-contained: it works in any tool without Livrarr.

Livrarr owns the layout it writes into: ebooks flat (`{root}/{user_id}/{Author}/{Title}.ext`), audiobooks in their own directory (`{root}/{user_id}/{Author}/{Title}/{files}`), separate roots per media type. Downstream tools adapt to Livrarr's output, not the other way around.

Import copies files from the download directory into the organized library — the original stays in the download directory for torrent seeding, and the library copy is independent of the source. The one exception is the CWA downstream integration, which hardlinks first (falling back to copy) because that copy is never modified. No other path hardlinks into the library.

A library managed by Livrarr must remain useful if Livrarr is uninstalled. The files are the user's; Livrarr is the manager, not the owner.

### Automated Discovery, Automated Organization

Author monitoring auto-adds new works. RSS sync auto-grabs matching releases for monitored works. After the grab, the system handles everything: download, import, organize, tag. The user sets policy — what to monitor, match thresholds — the system executes it. Manual intervention is reserved for genuinely ambiguous cases.

### Ecosystem Citizen

Livrarr integrates with the tools self-hosted users already run: Prowlarr, qBittorrent, Audiobookshelf, Kavita, Calibre-Web Automated. It follows Servarr conventions for API shape and terminology rather than inventing its own. Self-hosted users already have a stack; Livrarr fits into it, it does not replace it.

### Privacy by Default

Only publicly available information leaves the machine.

External calls — to metadata providers (Goodreads, Hardcover, OpenLibrary, etc.) and to LLMs — send only what is already on the public internet: title, author, provider keys. This is not a privacy violation because any person could look this up themselves.

What never leaves the machine: file paths, filenames, checksums, reading history, reading position, user preferences, credentials, or any information that could identify the user or their specific copy of a file.

No telemetry. No tracking. Livrarr does not phone home.

### Secure by Default

Self-hosted doesn't mean insecure. Passwords are hashed with argon2id. Session tokens and API keys are stored as SHA-256 hashes — shown once in plaintext, never retrievable again. There is no anonymous access and no network-based auth bypass. The one exception is download-client passwords, stored plaintext per Servarr convention and redacted in API responses.

Self-hosted users are exposed to their local network; secure defaults protect them without requiring configuration.

---

# Part 2 — Structure

## System Overview

The workspace contains 17 crates. This is a responsibility sketch, not a complete
Cargo dependency graph. In particular, `livrarr-domain` does not depend on the
database; `livrarr-db` implements contracts defined in domain.

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
        livrarr-domain ◄── livrarr-db (SQLite/sqlx)
    (types, traits, enums)

Supporting: livrarr-matching (release parsing/scoring)
            livrarr-jobs (job trigger traits, compile-wall safe)
            livrarr-behavioral (test harness + stubs)
            livrarr-cli (stub)
```

## Dependency Rules

**All dependency arrows point toward `livrarr-domain`. No cycles.**

| Crate | May depend on |
|---|---|
| `livrarr-domain` | No other workspace crate; external dependencies are listed in its Cargo manifest |
| `livrarr-db` | domain |
| `livrarr-http` | domain |
| `livrarr-matching` | domain |
| `livrarr-external-data` | domain, http |
| `livrarr-identity` | domain, http, external-data (no db — persistence via domain traits) |
| `livrarr-enrichment` | domain, http, db, external-data |
| `livrarr-materialize` | domain, http, tagwrite (no db) |
| `livrarr-metadata` | domain, http, db, matching, identity, enrichment, materialize, external-data |
| `livrarr-download` | domain, http, db |
| `livrarr-library` | domain, db, materialize |
| `livrarr-tagwrite` | domain |
| `livrarr-jobs` | domain only |
| `livrarr-handlers` | **domain, http, matching, jobs only — COMPILE WALL** (never db/metadata/tagwrite/download) |
| `livrarr-server` | everything (composition root) |

Verify the compile wall: `cargo tree -p livrarr-handlers`

Nothing depends on `livrarr-server`.

The table states allowed boundaries, not an assertion that every allowed edge is
used. In the reviewed manifests, library depends on domain and db; handlers depend
on domain, http and matching; server has 13 direct production workspace dependencies.
CLI has no dependencies. The behavioral harness directly depends on domain, db and
tagwrite and has additional test dependencies. The
[manifest inventory](build/reviews/architecture-review-2026-09-07/context-docs-017/SOURCE-BASELINE.json)
records all 17 members and their production workspace dependencies. Manifest checks
do not establish runtime use of an allowed dependency.

---

## Crate Responsibilities

### `livrarr-domain`
Shared entities, ID newtypes, enums, errors, service contracts and deterministic
domain rules. Shared identity comparison lives in `identity_matching` and
`identity_layer/services`; `text_norm` supplies text-normalization primitives.
Use these shared authorities rather than reproducing their rules elsewhere.
This crate has no dependency on another workspace crate; its Cargo manifest is the
source of truth for external libraries.

**Non-responsibilities:** persistence implementations, HTTP and application workflow orchestration.

### `livrarr-db`
All SQL queries and migrations. `SqliteDb` implements the `*Db` traits defined in domain. No SQL anywhere else.

### `livrarr-http`
The canonical HTTP transport. Owns rate limiting, SSRF protection, retry, 429 handling, user-agent injection. All outbound HTTP goes through here. See Principle: The Canonical Transport Is the Only Transport.

### `livrarr-external-data`
Provider clients: Goodreads, Hardcover, OpenLibrary, Google Books, Audnexus, Audible. Each implements the provider trait. Transport via `livrarr-http`. Provider-specific auth, parsing, and quirks are isolated here.

### `livrarr-identity`
Provider identity capture and deterministic identity decisions. `capture_identity_routes`
collects provider evidence; `DeterministicIdentityEngine` evaluates evidence, conflicts
and interaction using shared domain comparison rules. The metadata crate's
`IdentityRoadServiceImpl` coordinates settlement and persistence through repository
contracts. The old `settle_identity`/quorum/anchor-writing workflow was removed;
do not use it as current navigation.

### `livrarr-enrichment`
Enrichment pipeline. Fetches full payloads from providers, merges into one record. `DefaultMergeEngine` is the sole merge authority: deterministic, priority-ordered, null-guarded, single language-incompatibility chokepoint.

### `livrarr-materialize`
Covers, dimensions, atomic file writes. Downloads and decodes cover images, persists dimensions, performs atomic disk writes.

### `livrarr-metadata`
Application orchestration: work lifecycle and refresh, discovery/search, identity
settlement, background convergence and monitoring. These responsibilities span
services including `WorkService`, `DiscoveryService` and `IdentityRoadServiceImpl`;
the crate is not one service with a single universal call sequence. **Must not grow
into a god object** — keep ownership clear and split by coherent concern.

### `livrarr-matching`
Release title parsing, candidate scoring, M1–M4 matching pipeline, embedded metadata extraction. Used by import flows.

### `livrarr-download`
Download client integrations (qBittorrent, SABnzbd) and Torznab indexer search.

### `livrarr-library`
Import workflow, file layout enforcement, CWA downstream copy. Owns where files live on disk.

### `livrarr-tagwrite`
EPUB, M4B, and MP3 metadata tag writing. Format-specific heavy dependencies isolated here.

### `livrarr-handlers`
Axum route handlers and DTOs. Handlers declare the narrow `Has*` capabilities they
need; the broader `AppContext` aggregate also exists. **Compile wall.** The intended
boundary is input validation, calls through service contracts and response mapping,
without business logic, SQL or direct file I/O. The Add handler also coordinates
background continuations; whether particular handlers do too much is a review
question, not settled by the compile wall alone.

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

It defines `JobService`, `DownloadPoller`, `AuthorMonitor` and their result types.
The reviewed handler manifest does not currently depend on this crate; its intended
role does not prove that a particular trigger is wired through it.

### `livrarr-cli`
Command-line workspace member with no declared dependencies in the reviewed manifest.

### `livrarr-behavioral`
Behavioral-test harness, fixtures and stubs. It is support for testing production
paths, not part of the server's production dependency graph.

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
- Tag writing (EPUB/M4B/MP3) stays within a user-authorized workflow, including import and its unfinished background completion; settled files require a new user action as described in Part 1
- No telemetry, no analytics, no external reporting of any kind

## Current Conventions

These are established patterns. Follow them; deviate with a reason.

- Identity matching and selection are deterministic; LLM assistance is limited to metadata repair/cleanup and never selects or confirms a match
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
2. Extend the adapter surface and `ProviderClient` enum dispatch in `livrarr-external-data/src/provider_client.rs`. `ProviderClient` is not a trait in domain.
3. Wire the provider into the relevant discovery/enrichment dispatch and explicit provider policies. Determine which identity routes and metadata capabilities it supplies; do not assume all providers supply the same information.
4. Register the required transport/rate policy in the canonical `livrarr-http` path. The limiter is process-global and shared — do not create a local one.
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

SQLite with WAL mode and a four-connection pool. SQLite still admits one writer at a time; every write-bearing transaction reserves that slot through the shared `BEGIN IMMEDIATE` authority so concurrent writers wait under `busy_timeout` instead of failing during a deferred upgrade. Per-connection pragmas include `foreign_keys = ON` and `busy_timeout = 5000`. Migrations via sqlx are embedded and run at startup before serving traffic. Once a migration ships in any release, it is immutable.

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
