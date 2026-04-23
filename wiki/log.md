# Wiki Change Log

## 2026-04-23 — Architecture-excellent sprint review (fifteenth pass)

**Updated pages:**
- wiki/log.md only — no insights changed; all 41 insights verified accurate against actual code

**Context:** Read `readarr_import_workflow.rs` (OnceLock elimination, explicit field injection confirmed), `monitor.rs` (trigger_monitor stub confirmed), `import_pipeline.rs` (HTTP calls via explicit client confirmed). Nothing new found beyond the 14 prior passes. One stale item flagged that hasn't been noted yet: `wiki/index.md` line 44 says "28 active learnings" — the count is now 41. Also, `wiki/index.md` has no `wiki/crates/` section (insight 38 documents this as a known navigation gap). Both are fixable by a session that can modify wiki/index.md.

## 2026-04-23 — Architecture-excellent sprint review (fourteenth pass)

**Updated pages:**
- wiki/log.md — documented two stale errors in wiki/crates/server.md (no changes to insights.md — all 41 insights verified accurate)

**Context:** Full read of changed files confirms insights 9d, 9g, 9h, 9i, 36, 37, 38, 39, 40, 41 are all accurate. Two stale errors found in `wiki/crates/server.md` (not fixable in this pass — constraint: only insights.md and log.md):
1. `server.md` line 102 says `LiveSettingsService` "Implements eight service traits" but lists 7 (insight 39 is correct with 7). Typo introduced when the credential traits were split — the count wasn't updated.
2. `server.md` lines 196-205 says `import_pipeline.rs` contains "Pure helper functions for the import pipeline (no DB or network calls)" — this is wrong. `fetch_qbit_content_path` and `fetch_sabnzbd_storage_path` both make HTTP calls via an explicitly-passed client (insight 40 is correct). A future session should correct server.md at these two locations.

## 2026-04-23 — Architecture-excellent sprint review (thirteenth pass)

**Updated pages:**
- insights.md — amended 9g (trigger_monitor is a dead stub — use tokio::spawn + run_monitor)

**Context:** `AuthorMonitorWorkflow::trigger_monitor()` is defined in the domain trait and has an empty stub implementation in `AuthorMonitorWorkflowImpl` (comment: "Stub — server wires this up"). It is never called anywhere in the codebase. The actual on-demand monitor trigger from handlers uses `tokio::spawn + run_monitor` directly (9g pattern). A future session seeing this trait method could waste time calling it or wondering why handlers bypass it. Added explicit warning to 9g.

## 2026-04-23 — Architecture-excellent sprint review (twelfth pass)

**Updated pages:**
- insights.md — corrected 1 (crate count 11 → 13; documented livrarr-jobs, livrarr-cli, livrarr-behavioral)

**Context:** Cargo.toml lists 13 workspace members; insight 1 said 11. The count was correct as of the Phase 5 (April 19) session; 3 crates were added since. `livrarr-jobs` is the non-obvious one — it defines `JobService` (trigger bulk enrichment, author search, folder scan), which handlers bind via `HasJobService` to reach background jobs without depending on livrarr-server. This is the compile-wall-safe pattern for handler→job communication. `livrarr-cli` is an empty stub; `livrarr-behavioral` is the behavioral test harness.

## 2026-04-23 — Architecture-excellent sprint review (eleventh pass)

**Updated pages:**
- insights.md — corrected 40 (import_pipeline.rs does make network calls)

**Context:** Code read of `infra/import_pipeline.rs` found that insight 40's claim of "no network" is factually wrong. `fetch_qbit_content_path` and `fetch_sabnzbd_storage_path` both make HTTP calls — they are async functions that use an explicitly-passed `HttpClient`. The correct boundary is "no AppState access, no service trait calls, no DB" — not "no network." The corrected text distinguishes between service-layer access (banned) and explicit-parameter I/O (permitted). A future session reading "no network" would be confused when they see the HTTP calls in those functions.

## 2026-04-23 — Architecture-excellent sprint review (tenth pass)

**Updated pages:**
- insights.md — added 41 (module-level composite context traits)

**Context:** Code read of `opds.rs` and `manual_import.rs` found that both modules define their own composite context trait (`OpdsHandlerContext`, `ManualImportHandlerContext`) from `Has*` traits directly — without extending `AppContext`. This is a middle ground between individual per-function narrow bounds (insight 9h) and the full `AppContext` supertrait. Not captured in any of the nine prior passes. Non-obvious: someone extending one of these modules needs to know the module's own composite trait is the bound, not `AppContext`; someone adding a new high-handler-count module needs to know this is the established pattern. These traits do NOT extend `AppContext` — they select only the `Has*` traits the module actually uses.

## 2026-04-23 — Architecture-excellent sprint review (ninth pass)

**Updated pages:**
- insights.md — amended 9d (OnceLock fully eliminated; zero instances remain; do not reintroduce)

**Context:** Code search confirmed zero `OnceLock` instances in the crate tree. Insight 9d framed OnceLock as the "escape hatch when full refactoring is impractical," implying it might still be present. Added explicit note that `LiveImportService` and `LiveReadarrImportWorkflow` were both refactored to explicit constructor injection, leaving zero OnceLocks in the project. Future sessions should not reintroduce them.

## 2026-04-23 — Architecture-excellent sprint review (eighth pass)

**Updated pages:**
- insights.md — amended 36 (added CancellationToken cooperative-sleep requirement)

**Context:** Code read of `author_monitor_workflow.rs` found that insight 36 described the AtomicBool guard and AlreadyRunning behavior but omitted the CancellationToken pattern. All sleeps in the workflow (inter-author 1s delay, 429 backoff 60s) use `tokio::select! { sleep, cancel.cancelled() => return Ok(partial_report) }`. A bare `tokio::time::sleep()` would block graceful shutdown for the full duration — particularly painful with a 60s 429 backoff. The token is threaded scheduler → job tick → per-user `run_monitor`. Future sessions adding new background workflows need this pattern; checking `cancel.is_cancelled()` only at iteration boundaries is insufficient.

## 2026-04-23 — Architecture-excellent sprint review (seventh pass)

**Updated pages:**
- insights.md — added 40 (import_pipeline.rs is pure utilities, not orchestration)

**Context:** Five previous passes captured Phases 1, 2, and 4 patterns but missed Phase 3 (import_pipeline.rs migration). The file name "pipeline" implies it orchestrates the import flow, but after Phase 3 it contains only pure free functions — no service calls, no DB, no network. New import orchestration goes in `LiveImportService`, not here. Non-obvious enough to cause a future session to add service coordination to import_pipeline.rs.

## 2026-04-23 — Architecture-excellent sprint review (sixth pass — correction)

**Updated pages:**
- insights.md — corrected 36 (AlreadyRunning aborts entire tick, not skip-and-continue)

**Context:** Code read of `author_monitor.rs` and `author_monitor_workflow.rs` found that insight 36 was factually wrong. The AtomicBool is global (not per-user), and `AlreadyRunning` triggers `return Ok(())` from the entire tick function — not "continues to the next item" as the prior text stated. The distinction matters: future sessions could write a per-user guard expecting the job to skip just that user, but the actual design bails the whole tick to avoid queuing work behind an already-running scan.

## 2026-04-23 — Architecture-excellent sprint review (fifth pass)

**Updated pages:**
- insights.md — amended 39 (added single-struct impl principle + 4-step pattern for extending settings)

**Context:** Pass 4 documented the 7 trait names but not the server-side constraint: `LiveSettingsService<DB>` implements all 7 on one struct; AppState holds one `Arc<LiveSettingsService>`. A future session might split the impl into 7 separate structs, creating unnecessary wiring. Added the "don't split the impl" rule and the 4-step extension pattern (new domain trait → new impl block → new Has* → new AppState impl).

## 2026-04-23 — Architecture-excellent sprint review (fourth pass)

**Updated pages:**
- insights.md — added 39 (SettingsService 7-trait split; Prowlarr config in IndexerSettingsService not AppConfigService)

**Context:** Passes 1–3 captured the structural patterns but not the specific trait inventory from the SettingsService split. The non-obvious piece: Prowlarr config was explicitly moved from AppConfigService to IndexerSettingsService as a post-review fix — future sessions might put it back in the wrong place. Added full trait list so sessions adding new settings know which trait to extend.

## 2026-04-23 — Architecture-excellent sprint review (third pass)

**Updated pages:**
- insights.md — added 38 (per-crate wiki pages in wiki/crates/ not linked from index)

**Context:** Session created four per-crate reference docs (wiki/crates/handlers.md, domain.md, server.md, db.md) but did not link them from wiki/index.md. Future sessions following CLAUDE.md's "read wiki/index.md" instruction would miss these entirely. Added insight 38 as a direct navigation pointer.

## 2026-04-23 — Architecture-excellent sprint review (second pass)

**Updated pages:**
- insights.md — added 37 (http_client_safe for user-supplied URLs)

**Context:** First pass missed one security-critical pattern: AppState carries two HTTP clients and choosing the wrong one is an SSRF vulnerability. `http_client_safe` must be used for any URL that comes from user configuration (download clients, indexers, cover URLs); `http_client` is for hardcoded public endpoints only. Enforced as of the qBit SSRF fix commit.

## 2026-04-23 — Architecture-excellent sprint review

**Updated pages:**
- insights.md — added 9h (narrow `Has*` handler bounds), 9i (credential trait isolation), amended 9d (prefer explicit injection over OnceLock), added 36 (AtomicBool execution guard + user-scoped job pattern)

**New wiki pages:**
- wiki/crates/handlers.md — livrarr-handlers crate: route handlers, AppContext, Has* traits, compile wall
- wiki/crates/domain.md — livrarr-domain crate: service traits, domain types, BIG7 model
- wiki/crates/server.md — livrarr-server crate: composition root, AppState, service impls, jobs
- wiki/crates/db.md — livrarr-db crate: SQLite impls, migration patterns, SqliteDb

**Context:** Architecture-excellent sprint split the monolithic `AppContext`/`SettingsService` into granular `Has*` capability traits and isolated credential access behind separate traits. Four things non-obvious from code alone: (1) individual handler functions should bind narrow `Has*` traits, not the full `AppContext` supertrait — AppContext is only for route-layer composition; (2) `DownloadClientCredentialService` and `IndexerCredentialService` are intentionally split from their settings siblings as compile-time RBAC groundwork; (3) `OnceLock<Box<AppState>>` is now the last-resort escape hatch — explicit constructor injection (passing `Arc<ServiceImpl>`) is the preferred approach, as demonstrated by `LiveImportService` and `LiveReadarrImportWorkflow`; (4) background workflows callable from both scheduled job and handler hold an `AtomicBool running` guard — `swap(true, AcqRel)` returns old value, return `Err(AlreadyRunning)` if true; scheduled job treats AlreadyRunning as Ok() and continues to next user.

## 2026-04-19 — Compile wall 100% second review pass

**Updated pages:**
- insights.md — added 9g (handler-level spawning for background work)

**Context:** Independent consult found one uncaptured pattern: handlers are the only layer that can clone AppContext and tokio::spawn, because services only have `&self`. Three instances in work.rs (add→bibliography, refresh_all→bulk loop, author_search→monitor). Complement to 9d — 9d is for when services must hold state, 9g is the default for everything else.

## 2026-04-19 — Compile wall 100% post-session review

**Updated pages:**
- insights.md — amended 9e (WorkId/UserId are domain-native, not banned), added 9f (accessor newtype wrappers for orphan rule)

**Context:** Cross-agent review of compile-wall-100pct session. 2 of 6 independent agents identified the orphan-rule accessor pattern as the top uncaptured insight (6 wrappers in state.rs). 9e was corrected — prior version implied WorkId/UserId were banned livrarr-db types, but they're defined in livrarr-domain.

## 2026-04-19 — Compile wall 100% wiki consult

**Updated pages:**
- insights.md — added trait signature type safety rule (9e)

**Context:** Reviewed all session artifacts from 100% handler extraction. Both cross-family reviewers (Gemini + GPT) independently flagged P0 that service trait signatures are the compile wall's transitive boundary. The banned-types audit and substitution map from the plan were not yet captured in insights. Other patterns (Arc, OnceLock, orphan-rule wrappers) either already captured or derivable from code.

## 2026-04-19 — Phase 5 compile wall documentation

**Updated pages:**
- architecture/overview.md — added livrarr-handlers crate, compile wall section, Arc<ServiceImpl> pattern, renamed livrarr-organize → livrarr-library, added livrarr-matching crate
- insights.md — updated crate count to 11, added compile wall insight (9b), added Arc service pattern insight (9c)

**Context:** Phase 5 extracted all 40 route handlers from livrarr-server to livrarr-handlers behind a compile wall. The wiki previously didn't document this crate, the AppContext pattern, or the service wiring conventions.

- insights.md — added OnceLock<Box<AppState>> circular dep pattern (9d)

## 2026-04-18 — Full ingest from build artifacts

Processed all 17 specs chronologically (v2 through consolidation), 4 policies, cross-cutting decisions, and 3 build analyses. Later specs overwrote earlier knowledge per conflict rules.

**New pages (9):**
- domain/release.md — transient search results, protocol routing
- domain/series.md — Goodreads-sourced series, monitoring, assignment
- domain/list.md — bulk import mechanism
- domain/metadata-sources.md — all providers, foreign language pipeline, gotchas
- architecture/rss-sync.md — automated matching, gap detection
- architecture/usenet-pipeline.md — SABnzbd integration
- architecture/import-pipeline.md — detailed scan → tag → track flow
- architecture/ui-architecture.md — React stack, Readarr mimicry
- patterns/migration-pattern.md — SQLite migration rules

**Updated pages (4):**
- insights.md — expanded to 28 items, added cross-references to wiki pages
- index.md — added all new pages
- architecture/enrichment-pipeline.md — enrichment modes detail from consolidation spec
- domain/work.md — per-media-type monitoring detail

**Sources:** spec-librarr-v2.md through spec-consolidation.md (17 specs), 4 policy files, cross-cutting-decisions.md, 3 build analyses

## 2026-04-18 — Initial wiki scaffold

- Initial wiki scaffold created (17 pages)
- Ingested domain knowledge from high-level build artifact review
