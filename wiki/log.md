# Wiki Change Log

## 2026-07-25 — wiki/patterns verified: three of four conventions had drifted from the code

**Updated pages:** all four of `wiki/patterns/` — `async-service.md`, `error-handling.md`,
`migration-pattern.md`, `test-doubles.md`. Eighth documentarian pass, and the first on
*convention* pages, where the failure mode is "the codebase no longer does this" rather than a
misspelled name. **13 claims corrected, 4 blocks deleted.**

- **async-service.md** — the page opened "Every service in Livrarr follows the trait + impl +
  stub pattern." The stub leg is the exception: `livrarr-behavioral/src/stubs.rs` holds seven
  doubles, and `WorkService`, `AuthorService`, `FileService` and most others have none. Its code
  sample was also fiction in four places — it pointed at `livrarr-domain/src/services.rs` (no
  such file; `services/` is a directory), used method names `get_work`/`add_work` (the trait has
  `add`/`get`), and named two types that exist nowhere in the repo, `DomainError` and
  `StubWorkService`. Sample corrected against the real trait; errors are per-service.
- **migration-pattern.md** — rule 4 required `PRAGMA foreign_key_check` before and after every
  migration, fatal on new violations. **No such check exists anywhere in the codebase** — zero
  hits. Rule deleted. Also: the backup filename has no `vN` component, and the backup is skipped
  when the DB file does not yet exist, not when there is nothing to migrate.
- **test-doubles.md** — documented three test-DB helpers (default, shared-memory with
  `cache=shared`, temp-file with `tempfile`). `livrarr-db::test_helpers` contains exactly one
  function, `create_test_db`. The other two were removed rather than rewritten.
- **error-handling.md** — the HTTP table listed validation as 400; the real `Validation` /
  `Unprocessable` variants return **422**. `StorageError → 503` is wrong: `DbError::Io` maps to
  500, and `SQLITE_BUSY` produces no 503 at all — it is absorbed by `busy_timeout`. `Timeout →
  504` has no variant and nothing returns 504. Table now names the `ApiError` variant behind each
  row.

**Context:** documentarian edit pass #8, scoped by `build/reviews/DOC-EDIT-PACKET-5.md`.
Per-edit citations: `build/reviews/docs-edit-patterns-changelog.md`. One convention left
unverified and flagged: "the codebase uses zero `dyn` for service traits" — an absolute that
needs a real sweep to confirm or refute.

## 2026-07-25 — domain.md Entities verified: the "newtype IDs" are type aliases; domain.md complete

**Updated pages:**
- crates/domain.md — seventh documentarian pass. The Entities section (ID aliases, entity
  structs, enums) audited against `entities.rs`, `enrichment_types.rs` and `infra_config.rs`,
  all three read in full. **19 claims corrected, 2 deleted, 2 omission caveats added.**
  **domain.md is now fully verified end to end.**
  The headline: **the section called "Newtype IDs" documents type aliases.** Every one is
  `pub type X = i64` — `WorkId` and `AuthorId` are literally the same type, so passing one where
  the other belongs compiles cleanly. The page promised a compile-time guarantee the crate does
  not provide. Renamed to "ID Type Aliases" with the caveat stated.
  Deleted: `SourceKind` (zero hits anywhere — which also settles pass #6's open question: there
  was no `SourceKind::is_foreign()` to redirect the deleted `is_foreign_source` to) and
  `WorkField::normalization_class()` (zero hits).
  Phantom enum variants corrected: `GrabStatus` was documented with `queued` and `downloading`,
  neither of which exists — the real seven are Sent/Confirmed/Importing/Imported/ImportFailed/
  Removed/Failed. `EnrichmentStatus` was documented with `pending` and `skipped`, neither of
  which exists — it has four variants and `Thin` was missing.
  Added: `IdentityStatus` and `TagStatus`, both entirely absent. `IdentityStatus` is the identity
  half of the two-state split (identity outcomes moved off `EnrichmentStatus` in migration 055),
  and the rest of the wiki leans on it heavily.
  Also: `PlaybackProgress` is not audiobook-only (CFI for EPUB, page number for PDF, seconds for
  audio); `ProvenanceSetter` has six variants and `Imported` (CSV) and `Import` (Readarr) are
  distinct; `MergeResolved<T>` carries no conflict information; `IndexerConfig` has a third
  field. The section header's claim that those three files account for the section **holds** —
  verified type by type.

**Context:** documentarian edit pass #7, governed by `build/reviews/DOC-EDIT-PACKET-4.md`.
Per-edit citations: `build/reviews/docs-edit-domain-md-entities-changelog.md`.

## 2026-07-25 — domain.md region 1, lower half: normalize_for_matching is production-dead

**Updated pages:**
- crates/domain.md — sixth documentarian pass. Region 1 split again: this pass covers
  **Utility Functions, Settings, Readarr Import Types, Torznab and Keyed Mutex**; the Entities
  section (newtype IDs, entity structs, enums — page lines 9–74) is **still unaudited** and is
  now the only unverified part of the page.
  Deleted: `is_foreign_source` — zero hits anywhere in the repo.
  The headline correction: **`normalize_for_matching` still exists but is production-dead.** Its
  own doc says it is superseded by `identity_matching::identity_key` (REQ-014), that no
  production call site uses it, and that it survives only because test fixtures build
  `normalized_title`/`normalized_author` with it. The page presented it as a live utility, which
  would send a reader to the wrong normalizer — the exact mismatch that motivated the
  replacement.
  Also: `sanitize_path_component` takes two arguments, not one; `TorznabParseResult` has two
  variants, not three (there is no "empty"); `MetadataConfig` also carries Google Books
  settings; `KeyedMutex` hard-caps resident keys at 256 and `sweep()` is a backstop, not the
  primary reclaim path — per-guard drop already prunes. Utility Functions is 6 of 12 functions
  and the Readarr bullets cover 12 of 14 types; both now say so.

**Context:** documentarian edit pass #6, still governed by `build/reviews/DOC-EDIT-PACKET-4.md`.
Per-edit citations: `build/reviews/docs-edit-domain-md-region1a-changelog.md`.
**Still unaudited:** the Entities section only.

## 2026-07-25 — domain.md region 2 verified: 29 config/settings methods do not take a user_id

**Updated pages:**
- crates/domain.md — fifth documentarian pass, completing the service-trait half of the page.
  Region 2 (`### ReadarrImportWorkflow` → `### Common Error`, 18 traits) audited against source
  at `de5f1be7`; every trait read in full. The region carried 84 method bullets: **roughly 65
  corrected, 2 deleted, 4 missing methods added, 6 partial-list caveats added — exactly one
  bullet needed nothing.** Combined with pass #4, the whole `## Service Traits` block is verified;
  only the top of the page (entities/enums/utility functions, settings, Readarr types, Torznab,
  KeyedMutex — lines 1–137) remains unaudited.
  **The dominant defect: 29 methods across seven traits were shown taking a `user_id` they do
  not take** — `RootFolderService` (4), `DownloadClientSettingsService` (5),
  `DownloadClientCredentialService` (1), `IndexerSettingsService` (9),
  `IndexerCredentialService` (1), `AppConfigService` (7), `RemotePathMappingService` (5), plus
  `ManualImportService::list_root_folders` and two `ImportIoService` list methods. These are
  admin/global config surfaces; the page implied a per-user access model that does not exist.
  Deleted because they are not on the trait claimed: `ManualImportService::create_history_event`
  and `ImportIoService::create_library_item`.
  Other corrections: `EmailService::send_file` takes `(file_bytes, filename, extension)` — not a
  user and item id; it performs no lookup. `ListService::preview` takes raw bytes. `confirm`
  takes five arguments, `validate_metadata_languages` six, `update_progress` six.
  `ReadarrImportWorkflow::progress/preview/start/undo` all take `user_id`. `HistoryService` has
  a second, infallible `record` method. `HttpFetcher` has four methods, not two — and
  `fetch_no_redirect` **defaults to `fetch`, which follows redirects**, so a test double that
  overrides only `fetch` silently gets redirect-following behavior.

**Context:** documentarian edit pass #5, still governed by `build/reviews/DOC-EDIT-PACKET-4.md`.
Per-edit citations: `build/reviews/docs-edit-domain-md-region2-changelog.md`.
**Still unaudited:** page lines 1–137 (region 1).

## 2026-07-25 — domain.md service traits verified against source (partial pass — see boundary)

**Updated pages:**
- crates/domain.md — fourth documentarian verify-and-correct pass. **Partial by design:** the
  page is 417 lines over ~150 KB of source, so this pass covers the service-trait block from
  `### WorkService` through `### RssSyncWorkflow` only. 22 claims corrected, 5 deleted.
  Deleted because they exist nowhere in the repo: `WorkService::finish_bulk_refresh`,
  `AuthorMonitorWorkflow::trigger_monitor`, `ImportWorkflow::confirm_scan`. Deleted because
  they are not what the page said they were: `WorkService::refresh_all` (bulk refresh has no
  service method — it lives at the handler layer, and the trait carries only a commented-out
  placeholder) and `ImportWorkflow::retry_import` (a route handler in `livrarr-handlers`, not
  a trait method). `lookup` / `lookup_filtered` were moved off `WorkService` into a new
  `DiscoveryService` section, where they actually live — and `lookup` takes no `user_id`.
  The recurring defect, as on db.md: invented request structs and invented or dropped
  parameters — `enrich_work` takes six arguments not three, `refresh` takes a `surface`,
  `run_monitor` takes `(user_id, cancel)`, `update_flags` takes five, and two functions
  (`spawn_bibliography_refresh`, `BibliographyTrigger::trigger`) take `(author_id, user_id)`
  in that order, the reverse of what the page said.

**Context:** documentarian edit pass #4, scoped by `build/reviews/DOC-EDIT-PACKET-4.md`.
Per-edit citations and the exact resume boundary: `build/reviews/docs-edit-domain-md-changelog.md`.
**Not yet audited:** everything above `## Service Traits` (entities, enums, utility functions,
settings, Readarr types, Torznab, KeyedMutex) and everything from `### ReadarrImportWorkflow`
to the end of the page.

## 2026-07-25 — db.md verified against source: signatures were systematically wrong about user scoping

**Updated pages:**
- crates/db.md — third documentarian verify-and-correct pass, audited against all 22 trait
  modules in `crates/livrarr-db/src/api/` at `b33e8fe8`. **4 methods deleted** (they exist
  nowhere in the repo: `reset_pending_enrichments`, `set_enrichment_status_skipped`,
  `list_works_for_retry`, `increment_retry_count`), and roughly **70 bullet-level claims
  corrected** across every trait section except `AuthorDb`, `HistoryDb`, `ConfigDb` and
  `ProvenanceDb`, which were already accurate.
  The systematic defect: the page invented `user_id` parameters on traits that are deliberately
  **not** user-scoped — `RootFolderDb`, `RemotePathMappingDb`, `DownloadClientDb` and
  `IndexerDb` are all shared, admin-managed infrastructure ("Not user-scoped — indexers are
  global"), and `list_active_grabs` / `list_retriable_grabs` are cross-user by design. It also
  invented request structs for methods that take explicit parameters (`create_root_folder`,
  `save_bibliography`, `save_series_cache`, `create_list_import_record`,
  `insert_list_import_preview_row` — that last one takes 14 explicit params), and dropped real
  parameters elsewhere (`update_grab_status`'s `import_error`, `update_series_flags`'s
  `monitor_language`, `list_notifications`' `unread_only`, `upsert_progress`'s three trailing
  args). Other corrections: `create_work` is **not** on `WorkDb` — it lives on the separate
  `WorkDbCreate` trait, split deliberately so only `WorkServiceImpl` can create Works
  (compile-time enforcement of M2); `search_works` is a paginated `LIKE` match, not full-text
  search; `EnrichmentRetryDb` has one method, not three; `ApplyEnrichmentMergeRequest` carries
  no external-ID field; `get_*_with_credentials` returns the same data as its plain sibling and
  exists to signal intent, not because the plain one omits credentials.
  Added a standing caveat that the method lists are partial and `src/api/` is the full contract.

**Context:** documentarian edit pass #3, scoped by `build/reviews/DOC-EDIT-PACKET-3.md`.
Per-edit citations: `build/reviews/docs-edit-db-md-changelog.md`. The packet expected staleness
from the identity-edit merge; there was none to correct — this page has never documented the
identity surface at all (no `WorkIdentityRepository`, no `apply_identity_clear`, no startup
backfill). That is an absence, not a stale claim, and adding it would be authorship.

## 2026-07-25 — handlers.md verified against source: AppContext is not a union of everything

**Updated pages:**
- crates/handlers.md — second documentarian verify-and-correct pass, audited against
  `crates/livrarr-handlers/` at `84db7a44`. 14 claims corrected, 1 renamed, 0 deleted.
  The load-bearing correction: **`AppContext` does NOT union all `Has*` capability traits.**
  `HasDiscoveryService`, `HasWorkIdentityRepository`, and `HasHttpFetcher` are deliberately
  outside it, so handlers needing them bind them directly — the page asserted the opposite in
  three places (intro, table row, blanket-impl note), and the same error sat in the crate
  header ("Generic over `AppContext`"; in fact the only `AppContext` bound in the crate is
  `system::routes`, the router-composition function). Also corrected: the two module composite
  traits (`ManualImportHandlerContext`, `OpdsHandlerContext`) do **not** extend `AppContext` —
  they are plain 9- and 6-trait unions; `WorkService` is 24 methods, not 17 (the identity-edit
  merge added three); `LiveMetadataConfigAccessor` writes rather than reads; `SystemAccessor`
  is log tail + log level, not "uptime, hostname"; `CoverProxyCacheAccessor` fronts a TTL
  cache, not an LRU one; `work::author_search` is an admin-only author-monitor trigger, not an
  add-flow author search; `work::list` is paginated with no monitored filter; `refresh_all`
  takes four filters. Renamed `is_allowed_cover_source` → `is_allowed_host` (the documented
  name exists nowhere in the repo).

**Context:** documentarian edit pass #2, scoped by `build/reviews/DOC-EDIT-PACKET-2.md`.
Per-edit citations: `build/reviews/docs-edit-handlers-md-changelog.md`. Undocumented
handlers were deliberately left out — including the three identity-edit routes and
`work::merge` — because adding them is authorship, not correction; they are listed as a
decision in the changelog.

## 2026-07-24 — server.md verified against source: 21 wrong claims corrected or cut

**Updated pages:**
- crates/server.md — first documentarian verify-and-correct pass. Every factual claim on the
  page was checked against `crates/livrarr-server/` source; ~3 in 4 held. Deleted: the whole
  `## Disk (disk.rs)` section (no such file), four phantom functions under `release_helpers.rs`
  (`search_indexer`, `build_torznab_url`, `clean_search_term`, `fetch_and_parse` — none exist
  anywhere in the repo), and a `data_dir` field on `LiveImportService` that isn't a field.
  Corrected: `validate_llm_endpoint_startup` validates URL *shape*, never reachability;
  `CoverProxyCache` is TTL + oldest-inserted eviction, **not LRU**; `ImportIoServiceImpl` does
  **no file I/O** (ten DB methods only); `LiveTagService` is not EPUB-only (explicit MP3 batch
  path); `LiveMatchingService` returns match clusters, never works; `SecondaryApiImpl` is
  `#[cfg(test)]` and is **not** a production API surface; `fetch_all_readarr_data` is mostly
  sequential and fetches no editions; `rss_sync_tick` does not check already-running;
  `AppConfig` has five sections, not three; three `services/*.rs` paths pointed at files that
  don't exist (two live at the crate root, one is in `livrarr-download`). Added: the 13
  `AppState` fields the three field tables were missing (of 61 total), including
  `discovery_service`, `identity_resolver`, and `cover_service`; the Phase 5 table's column
  header now matches what its rows carry.

**Context:** documentarian role, edit pass #1, scoped by `build/reviews/DOC-EDIT-PACKET.md`.
Audit report: `build/reviews/docs-audit-server-md.md`. Change log with per-edit citations:
`build/reviews/docs-edit-server-md-changelog.md`. Undocumented files/job modules were left
alone — that is authorship, deliberately out of scope for this pass.

**Caveat for future sessions:** Serena's reported line numbers drifted from the real file in
`main.rs` (it placed `load_config` at 1069/1070; the function is at 1062). Every citation in
the change log was confirmed by opening the line. Do not cite a Serena `body_location`
without reading it.

## 2026-07-14 — Settle-road title trust: cause-aware grey + one trust policy; flm colon-truncation killed

**Updated pages:**
- insights.md — new insight 72 (the settle-road trust unit: `GreyCause` on `TitleVerdict::Grey`, `title_id_trust` as the ONE text-corroborated-trust policy, `flm_title`/`canon_author` deleted, try-again dead-end clearing, fixture-faithfulness lessons); insight 59 amended (grey-never-absorbs now names its one ratified exception — AC-004 anchor trust at the two settle-road seats).

**Context:** identity-fix unit (quality-waves). The WWZ GR-key drop and the flm containment gate were two strictness bugs on one road; both seats now consume the matching authority. Contract: `design-settle-road-matching.md` (3-round design review + test review + r8 code review, both families PASS).

## 2026-06-14 — Work-creation pipeline mapped; M9 convergence gap documented

**New page:**
- architecture/work-creation-pipeline.md — the five phases (identify → seed → create → enrich → materialize, with file:line anchors), the per-door identify matrix (which doors resolve at the door vs seed Pending), and the **M9 convergence gap**: series-monitor + Readarr-import seed `identity-pending` BY DESIGN (M9 permits it), but the automatic convergence M9 mandates was removed (`enrichment_retry_tick` gone; `retry_all_incomplete` user-triggered only; `bulk_resolver::resolve_bulk` unused) → Pending works sit in the "silent limbo" M9 forbids → REQ-022 violation. Flagged as the Sprint E prerequisite (#144 remainder).

**Updated pages:**
- architecture/metadata-pathway.md — § "Background Retry Job" was stale (pointed at `jobs/enrichment.rs`, which no longer exists). Corrected: the recurring retry job was removed; convergence is now user-triggered via `retry_all_incomplete` (no recurring loop) — which is the M9 gap.
- insights.md — new insight 54 (the M9 convergence gap; seeding-Pending is by design, the missing auto-convergence is the regression).

**Context:** session tracing why series-monitor + Readarr works land unidentified and empty. Corrected a mid-thread "it's an oversight" verdict against principle M9 — seeding Pending is deliberate; the missing automatic convergence is the real (binding-principle) regression. Grounded in code (door call sites, `identity.rs`, `work_service.rs`) + M9.

## 2026-06-10 — Canonical model authored; 13→17 crate corrections

**New artifact:**
- architecture/canonical-model.yaml — the authored entity spine (16 entities), intended seams (17 crates, full `livrarr-*` names), data_flow + invariants, amendments log. Activates kk-build's forward gate (IR `domain_entities` vs spine; crate edges vs seams), reverse gate (pub-type staleness), and amendments-log enforcement. Deliberate non-conformances on record: `Release` (no pub type yet — rename queued as issue #141); live `library→tagwrite` edge off-model (intent: via `materialize`, decision S1).

**Updated pages:**
- wiki/architecture/overview.md — 13→17 crates; dependency graph now includes external-data / identity / enrichment / materialize; metadata/download/library dep lines corrected against Cargo.toml; pointer to the canonical model as the intended topology.
- wiki/insights.md — insight 1 corrected to the 17-crate layout; new insight 48 (canonical model location + gate rules a future session needs).
- (kk-build repo) wiki/livrarr/crate-architecture.md — full 17-crate rewrite; wiki/framework/verify-gate-behaviors.md 13→17 reference. Committed there as 103f9ab.
- (addendum, post-merge) wiki/insights.md insight 48 extended: audit_canonical.py + first-audit baseline (entity_coverage 0.9375 / seam_conformance 0.9787 / surface_sanction_ratio 0.024) + conformance issues #141/#143 + the gate-friction-reporting directive for the first gated feature. PR #140 merged to main as ab28699 (merge commit via admin bypass; rebase impossible — branch carries the 2cf112d internal merge).

**Context:** canonical-model authoring session with the PO (kk-build architecture-hardening step). Crate facts verified against `Cargo.toml` members + per-crate dependency extraction; entity names verified against `pub struct/enum` declarations. Model file written; livrarr commit pending PO word.

## 2026-05-29 — M9 amended: fully-formed by path tier (work-creation-consistency)

**Updated pages:**
- wiki/domain/metadata-principles.md — M9 amended. Was "enrichment is synchronous; no deferred enrichment, no eventually-consistent metadata." Now binds **by path tier**: interactive paths stay synchronous/fully-formed; batch + monitor paths may create `identity-pending` works that converge via the async resolver, with a terminal `needs-review` state for the unresolvable. "Consistency" redefined as eventual convergence (same destination, not same clock). Binding invariant: all paths converge on the same identity (REQ-022).

**Context:** PO-approved carve-out during work-creation-consistency IR v1. Strict M9 was an ideal, not an architectural necessity, and was already violated in practice (async cover download; Goodreads anti-bot/LLM-gated; REQ-025 timeout=abstain forces eventual convergence). **Flagged for harmonization, not changed:** `build/foundation/principles.md` principle 6 ("Metadata enrichment is synchronous at add time") carries the same phrasing tension.

## 2026-05-29 — Crate-count + provider-stack corrections (work-creation-consistency prep)

**Updated pages:**
- wiki/architecture/overview.md — corrected "10-crate" → **13-crate** workspace; added `livrarr-jobs` and `livrarr-cli` to the dependency graph; fixed `livrarr-handlers` deps to `domain, http, matching` (was `domain, http`).
- wiki/architecture/enrichment-pipeline.md — Provider Stack was missing **Google Books** and **Audible** (both are registered `ProviderClient` queue providers). Rewrote to 6 network providers + Readarr (synthetic) + LLM validator; documented the language applicability rule (English excludes Google Books; foreign excludes OpenLibrary + Hardcover); fixed the flow step-3 line; cross-linked metadata-pathway.md as authoritative.
- build/foundation/module-map.md — added stale-content banner (`librarr`→`livrarr`, `librarr-organize`→`livrarr-library`, and `livrarr-jobs` is now a thin `→domain` trait crate, not the orchestrator described). Verified against `livrarr-jobs/Cargo.toml` (domain only).
- build/foundation/ir-pattern.md — added naming-note banner (`librarr`→`livrarr`; structural conventions still current).
- (kk-build) wiki/livrarr/crate-architecture.md — full rewrite; prior version listed 4 non-existent crates (`livrarr-enrichment`/`-api`/`-core`). Real 13-crate layout verified against `Cargo.toml`.

**Context:** Architecture-stage prep for the `work-creation-consistency` feature. Verified against code: `Cargo.toml` members (13 crates), `find_implementations(MetadataProvider)` (only OL + LlmScraper — the queue dispatches via the `ProviderClient` enum, not that trait), and the three provider-registration sites in `livrarr-server/src/main.rs` (enrichment queue / cover service / pre-add cover picker). Flagged to PO separately: (a) kk-build `config.yaml` may still encode the old fake crate paths; (b) known-but-unfixed errors in `wiki/crates/server.md` ("eight service traits" should be 7 per insight 39; `import_pipeline.rs` "no network calls" is wrong per insight 40).

## 2026-05-14 - Metadata pathway explainer

**Updated pages:**
- wiki/architecture/metadata-pathway.md - added current add/enrich/merge/cover/tag pathway, entry points, pseudocode, risk areas, and speed/accuracy improvement backlog
- wiki/index.md - linked the new metadata pathway explainer and corrected the
  Metadata Principles count from M1-M7 to M1-M10

**Context:** The current metadata pipeline has diverged from older enrichment
notes. This pass documents the active code path through `WorkService::add`,
`run_unified_enrichment`, `EnrichmentServiceImpl::enrich_work`,
`DefaultProviderQueue::dispatch_enrichment`, merge application, cover caching,
and tag sync. It also captures the main convergence risk: manual import starts
with much sparser seed metadata than Readarr import.

## 2026-04-23 — Architecture-excellent sprint review (sixteenth pass)

**Updated pages:**
- wiki/log.md only — no insights changed; all 43 insights verified accurate against actual code

**Context:** Spot-checked `context.rs` (Has* capability traits), `settings_service.rs` (7-trait split, single struct), and the plan file. Nothing new found beyond the 15 prior passes. Two known stale errors in `wiki/crates/server.md` remain (server.md says "eight service traits" — should be seven; says "no network calls" in import_pipeline.rs — incorrect per insight 40). Both are outside the scope constraint (only insights.md and log.md editable here).

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
- 2026-06-10: insight 49 added — speed baseline + serial-scatter finding, a6 release gate (B + parallelization), F1 live-confirmation + revert. Sprint A closed in ROADMAP.
- 2026-06-10: insight 16 corrected (metadata_source is a dead column; works.language drives foreign routing) + insight 50 added (Sprint B evidence round: F1 root cause = wrong-book adoption, anchors triple-stored, file logging alive / stale livrarr.txt pointer, no 24h cache). Source: spec-metadata-correctness.md §0b.

## 2026-07-02 — Phase 3 foundation build complete (overnight session)
Rewrote insight 30: the "rate limiter must reject" rule described the enrichment
TokenBucket, deleted in Phase 3 stage C. Replaced with the outbound-queue transport
architecture (pacing/cap/breaker/priority at livrarr-http, R-11 pause semantics,
reporter split). Insight 49's description of TokenBucket+Semaphore in
dispatch_enrichment is now historical — the scatter still uses JoinSet but transport
control lives at the outbound queue. Stages B0..C at commits 19af4d5, 7e76dec,
556e327, f03b537, 1657d26, f557e07, 97963cf; every stage dual-family reviewed (PASS).

## 2026-07-02 — Phase 4 (data completeness + convergence) built + dual-family reviewed
All five units built on 97963cf, orchestrator-gate-verified (1094 tests, 10 new),
Gemini+Codex PASS across 3 review batches, 0 findings; work UNCOMMITTED pending PO go.
Units: dead `pacing_queue` module deleted (submit was `todo!()`, zero production
callers); M-013 empty-list guard at extract time (`non_empty_vec` — HC/GB/Readarr all
emitted `Some(vec![])`, which won the priority walk and ERASED stored genres via the
un-COALESCEd `genres = ?` bind); M-012 GR cover gate moved into `merge()` (was
network-path-only; the cached-reuse path also stamped every payload Success→Validated
trust, amplifying the miss); M-014 `merge_generation` predicate + rows_affected check
on both `apply_enrichment_merge` UPDATEs; M-017 `converge_outcome` pure fn (Completed
now requires zero chaseable anchors) + job error-arm backoff. Insights 56-58 added
(merge chokepoint policies; Completed contract; GR-breaker test flake).
`livrarr-domain/services/pacing.rs` (PacingLane/ProviderCallOutcome) deliberately left
— PO decision pending. Packet + unit ledger: `build/plans/packet-phase4.md`.

## 2026-07-03 — Phase 5 (one matching authority) COMPLETE: built, reviewed, merged, deployed
All Phase-5 units landed on metadata-remediation (overnight autonomous run, PO
gates pre-approved): H default-language setting (2a12960), A authority module +
46 trap tests (3454b7a), C decision-diff harness (7ffc697) whose v2 report the
PO approved as the cutover basis, J AC-012 pin (99c7bd7), I merge-two-works
(5908fd5), D identity-engine rewire + candidate persistence + refresh gate
(5801200), F recognition fixes + harness old-side freeze (49648c1), G GR unlock
+ HC Tier-2 delete + dead scaffolding (0ef07d1), E identity key + adopt/dedup +
recompute (6fa00e9), J2 review surface + conflicts wiring + RSS language-skip
notifications (a06ba33). Every unit dual-family reviewed to PASS+PASS (codex
caught 6 real P1s across the night: harness live-call old sides, backfill
catch-all error masking, resolve/dismiss missing parked-state guards ×2 + the
orchestrator caught the sibling-key-collision P1 and the scan-flatten
regression pre-review). Final gate 1221/0 (129 suites); startup recompute
backfilled all 133 works' stored keys zero-collision (identity_key_generation=1);
3 works carry segmented keys. Insights 13 (rewritten — GR needs no LLM), 59-61
added. Hygiene backlog recorded in the phase report: GateReason::
DeterministicSkipNoLlm rename, orphaned tests/behavioral/test_metadata.rs, two
stale "askllm" stub docstrings, normalize_for_matching removal once fixtures
migrate. Spec REQ-013 (per-install) and REQ-015 (c)/(d) amendments applied.
P11's parenthetical Hardcover example still needs the PO's one-line edit.

## 2026-07-03 — N1: GR series-page parser rewrite (React layout)

GR redesigned /series/<id> to React (books in data-react-props JSON; no
position labels; old h3/JSON-LD layout gone) → parser silent-empty →
series-add created 0 works. Rewrote parse_series_detail_html (per-entry
tolerant parse, loud drift warns, pagination counter blob), roster = header's
"N primary works" cutoff (live probe 43318 disproved blob-membership==primary:
27 listed for 3 primaries), positions only from same-series decorations.
Emptiness never persisted: stored-empty heals on open, monitor never writes an
empty/partial fetch, every roster save pairs with work_count. Review: r1 2
findings (collision-branch heal bypass P1, mount-order test gap P2), r2 codex
partial-pagination P1, r3 PASS×2. 1239 tests green. Fixtures:
crates/livrarr-external-data/fixtures/. Updated goodreads.md (stale
"GR requires LLM" + series-page section), series-matching.md, series.md
(roster amendments), insight 62 added. Gemini note for retro: r1 retry
findings were verbatim codex copies (verdict-file bleed suspected); 0 unique
gemini P1s again.

## 2026-07-03 — N4: GR picker through the matching authority

The 2026-07-03 refresh residue (8-9 GR misses, all bare-seed vs subtitled
GR record) traced to gr_best_match's whole-title jaccard. The picker now
routes through parse_title/title_verdict (Same or Grey picks, decorated hit
title parsed, junk filter + author-token guard unchanged, Same>Grey ranking,
earliest-on-ties). Review r1 codex P1: a picked one-sided-volume Grey could
self-corroborate downstream because apply_bare_title stripped the decoration
and agree() ignored series_position — fixed by making the evidence travel
(autocomplete parses decoration into series fields; candidate/payload carry
them; agree folds positions into the volume VETO only, incl. blocking the
edition-bridge rescue). r2 PASS×2. 1250 tests green. Insight 59 amended.

## 2026-07-03 — N3: dead-URL phase1 cover fast-fail

Import profiling showed dead embedded cover URLs burning phase1's full 3s
budget per book (~40% of machine time), same host repeating across the batch.
New HttpFetcher::fetch_ssrf_safe_fast_connect (600ms connect budget via a
third pre-built client sharing the SSRF preflight; default-bodied trait
method, pre-desugared for trait_variant) + a per-import-run task-local
negative host cache (manual-import loop only, fail-open everywhere else).
download_cover_to_disk got a typed error so only connect-class failures mark
a host dead. Built by a Sonnet agent in a worktree (hybrid with N4);
PASS+PASS round 1, zero findings. Merged 9e97a13; merged-tree gates 1263/0.

## 2026-07-04 — N2 cover pipeline consolidation
- insights.md: NEW insight 63 (one cover rank / write gate / layout); amended 52(4) (#153 upgrade half now live) and 51(8) (audiobook dims writer closed).
- architecture/metadata-pathway.md: corrected the stale "OL emits no cover URL" claim; risk section rewritten — cover writes decoupled from the generic merge (write gate).
- Source: unit N2, merge 9f1f61e (design confer r1/r2 + code review r1 FAIL+FAIL → fix round → r2 PASS+PASS; record at build/reviews/n2-cover-consolidation/).

## 2026-07-04 — architecture-review prep
- overview.md: canonical-model path corrected (docs/, not architecture/) + roads.md companion link.
- crates/server.md: LiveSettingsService trait count corrected (seven, verified against settings_service.rs impls); import_pipeline "no network calls" claim corrected (two fetchers use an explicit HttpClient).
- roads.md (untracked, provenance pending PO confirm): stale cover_backfill dead-code row marked DONE.
- NEW docs/architecture-review-briefing.md — review entry point (reading order, doc trust table, intentional-debt register, day-one items).

## 2026-07-10 — audit-the-audit + god-object design session
- insights.md 9g: corrected stale `trigger_monitor` claim — stub + trait method were DELETED (af709f01, M-006); trait has only `run_monitor` (verified against monitor.rs:31-37).
- docs/metadata-audit-2026-06-28.md: added "Status Reconciliation — re-audited 2026-07-10" section (6 sonnet+Serena verify agents + orchestrator re-reads): 17 findings fixed, M-008 closed-intentional, M-005 god object OPEN (grew to 3,742), M-021 + 2 minor guards latent-open. Original bodies preserved as history.
- NEW (untracked, repo root): responsiveness-recommendations.md, design-work-service-split.md, reviews-work-service-split.md — see handoff-work-service-split.md in kk-build state.

## 2026-07-11 — work-service-split executed (insights 64, 65; handlers.md Has* table)

- Added insight 64 (DiscoveryService split: new domain trait + DiscoveryServiceImpl; WorkServiceImpl<D,E,H> lifecycle-only) and 65 (30/129 tests/behavioral files are unregistered orphans — check Cargo.toml registration before trusting/editing).
- handlers.md capability table: +HasDiscoveryService row; HasWorkService row now notes the 17-method trait.
- Source: work-service-split series 2734fd02, 7c1de013, 3521c940, 0094e805 + cross-family review (Codex PASS; Gemini fact-checked — orphan discovery real & pre-existing, attribution refuted).

## 2026-07-12 — suppression machinery deleted (pipeline-hygiene item 1)

- insights.md 30: tail rewritten — suppression machinery DELETED (variants, config fields, record_suppressed, SuppressionExhausted; migration 073 drops the columns); breaker-pause wording trimmed to "no retry budget". Insight 65 amended earlier same day: manifest guard does NOT cover registered-but-untracked test files (2 uip files were missing from origin until 8c9c4ab5).
- architecture/metadata-pathway.md: dispatch_enrichment pseudocode rewritten to the live seam order (applicability → anchor derivation → terminal-skip → U-B1 cache consult → JoinSet; pacing/breaker at the outbound queue, not in-queue — the old block still showed pre-Phase-3 breaker-at-dispatch + persist-Suppressed). Outcome-class list and speed-controls list updated. Fixed on sight: provider_client.rs path (external-data, not metadata) and the pre-Phase-5 "GR search requires LLM disambiguation" claim (deterministic picker per insight 13). Page not otherwise re-verified — other sections may carry similar era-drift.
- crates/db.md: ProviderRetryStateDb API list corrected — record_suppressed removed, record_will_retry_paused added, record_terminal_outcome signature fixed.

## 2026-07-13 — docs-sync: quality-waves Wave 1 dead-code/rename fallout

- crates/server.md: removed the `rate_limiter.rs` subsection (file deleted — `OlRateLimiter`/`GoodreadsRateLimiter` gone); AppState field table — dropped the `provider_health` and `refresh_in_progress` rows (fields don't exist, verified against state.rs) and the `provider_health_accessor` row (no such accessor trait); corrected the `provider_queue`/`enrichment_service` rows, which called themselves "Phase 1.5 plumbing (not yet on live enrichment path)" — both are on the live path per state.rs's own field comments.
- crates/handlers.md: dropped the `HasProviderHealth` capability-trait row and the `ProviderHealthAccessor` accessor-trait row — same falsification as server.md, neither exists in context.rs/accessors.rs (found while verifying the server.md AppState table).
- architecture/roads.md: R6 Forbidden clause — `create_test_library_item` was documented as an "exempt, deletion candidate"; it's now actually deleted (quality-waves Wave 1), rule text updated to "no exemptions." Dead-code table: added `QueueItem`/`QueueResponse`/`QBitTorrent` (`livrarr-download/src/lib.rs`) as a NEW candidate — verified zero live consumers (`grab_service.rs`'s `QueueItem` resolves to the same-named but distinct `livrarr_domain::services::grab::QueueItem` via glob import, not this one).
- architecture/metadata-pathway-pseudocode.html: added a stale-render banner at the top. The Goodreads LLM-disambiguation section, the LLM Identity Validator section, and the `record_suppressed` branch in the dispatch pseudocode all predate Phase 5 matching, the suppression-machinery deletion, and this wave's llm_validator.rs deletion. Not rewritten — metadata-pathway.md is the maintained sibling and is current on all three points.
- Found but out of scope (reported, not edited): insights.md 15 ("LLM validator confirms provider match when work is added") is stale — llm_validator was unwired from enrichment at Sprint E (REQ-005), before this wave deleted the file; metadata-pathway.md's own "Candidate Selection" and "Validation Strictness" sections describe the same pre-Phase-5 LLM-selection flow and are stale for the same reason, but that drift predates this wave and is a larger rewrite than this pass covers.

## 2026-07-13 — docs-sync: quality-waves Wave 2 behavior-fix audit (no wiki fixes required)

- Verified all 8 Wave 2 behavior-fix changes against live code (`provider_client.rs`, `services/series.rs` + `series_query_service.rs` + `handlers/series.rs`, `jobs/download_poller.rs` + `jobs/maintenance.rs` + `jobs/rss_sync.rs`, `livrarr-http/src/outbound_queue.rs`, `handlers/cover.rs`, `sqlite_import.rs` + `sqlite_library_item.rs`, `m4_scoring.rs` — all uncommitted working-tree changes, cross-checked against `docs/quality-remediation-plan-2026-07-12.md` items #2, #3, #9(part 1), #23, #33, #34, #35): OpenLibrary `fetch(&Work)` ISBN/ol_key tiers now return on transient failure instead of degrading to the title+author fuzzy tier; `OpenLibraryClient` generic over `HttpFetcher`; `SeriesMonitorWorkerParams` carries a `CancellationToken` and both GR pagination loops (`fetch_series_roster_pages`, `fetch_author_series_pages`) select sleeps against it when given one (previously always a bare `tokio::time::sleep`); `download_poller_tick`/`rss_sync_tick`/`call_record_retention_tick` check `cancel.is_cancelled()` at loop/run boundaries (previously the param was named `_cancel` and ignored); every `outbound_queue.rs` lock is now poison-tolerant (`unwrap_or_else(|poisoned| poisoned.into_inner())`), matching the dispatcher guard's pre-existing discipline; the three `cover.rs` handlers (alternatives/select/upload) return `ApiError` instead of bare `StatusCode`/a private `ErrorBody`; the Readarr-import read path now uses the canonical `sqlite_library_item::row_to_library_item` (real `tag_status`/`tagged_at_generation` columns) instead of `sqlite_import.rs`'s own copy, which hardcoded `Pending`/`0`; `m4_scoring::normalize` strips combining marks via `unicode_normalization::char::is_combining_mark` instead of a hand-rolled partial Unicode range table (scores can shift for non-Latin scripts).
- Swept every wiki page plausibly touching these eight mechanics — `domain/metadata-sources.md`, `integrations/openlibrary.md`, `architecture/enrichment-pipeline.md`, `architecture/metadata-pathway.md`, `domain/series.md`, `architecture/series-matching.md`, `integrations/goodreads.md`, `crates/db.md`, `crates/handlers.md`, `crates/domain.md`, `crates/server.md`, `patterns/error-handling.md`, `architecture/roads.md`, `architecture/import-pipeline.md`, `architecture/rss-sync.md`, `architecture/grab-system.md`, `domain/metadata-principles.md`, `architecture/work-creation-pipeline.md`, `decisions/key-decisions.md` — plus a full-text scan of `wiki/` and `docs/` for the touched identifiers/behaviors. **No page asserted the pre-wave behavior as fact**, so nothing was falsified and no content page needed an edit: these are internal hardening fixes (lock poisoning, HTTP error-envelope shape, a duplicate DB row-mapper's hardcoded defaults, a per-tier transient-vs-no-match distinction, Unicode combining-mark coverage, cancellation-token plumbing) below the granularity the wiki documents. Confirmed no wiki page names `OpenLibraryClient`, `ErrorBody`, `SeriesMonitorWorkerParams`, `row_to_library_item`, or `unicode_is_combining_mark` at all.
- Found but out of scope (reported, not edited): `architecture/enrichment-pipeline.md:10` ("Open Library ... Does not emit a `cover_url` in normalized output") contradicts `architecture/metadata-pathway.md:470-473`, which already carries the correction ("Emits a cover URL in the normalized detail when the OL record carries a cover id... an earlier version of this page wrongly said it emitted none") — `OpenLibraryClient::build_payload` (`provider_client.rs`) does emit `cover_url` and this wave didn't touch that function. The 2026-07-04 N2 log entry above fixed metadata-pathway.md's copy of this claim but not enrichment-pipeline.md's; that drift predates Wave 2 and is unrelated to it. Also noted: `crates/handlers.md`'s Route Handlers section has no `cover.rs` subsection at all (`get_cover_alternatives`/`select_cover_handler`/`upload_cover_handler` are undocumented) — a pre-existing omission, not a falsified claim, so left alone.

## 2026-07-13 — quality-waves 2a: shared qBit classifier (insights 70, 65 amendment; roads row; D1 doc)

- insights.md: added insight 70 — `classify_qbit_state` (livrarr-download) is the ONE qBit state classifier, both projections from one table row per the ratified D1 doc; consumers = poller import gate + `fetch_qbit_progress` (canonical `download_status`); `map_qbit_state` + `is_completed_state` deleted (the former had zero production callers — the queue UI had been receiving the raw state string, unread by the frontend).
- insights.md 65: amended with the directory-wide orphan instance — `tests/implementation/` (6 files) is registered by nothing and never compiles; `test_manifest_guard` covers `tests/behavioral/` only. Do not author pins there; triage pending.
- architecture/roads.md: `create_test_notification` re-opened as a NEW dead-code candidate — Wave 1's KEEP cited a caller in `test_impl_secondary.rs:377`, which never compiles (orphan directory above).
- docs/d1-qbit-state-truth-table-2026-07-13.md: status header DRAFT → RATIFIED + IMPLEMENTED (as-built shape recorded); implementation-time correction block added (the map_qbit_state consumer premise was dead at authoring); "Remaining PO decision" section closed.
- Wiki sweep for falsified claims: full-text scan of wiki/ for `map_qbit_state`, `is_completed_state`, and the qBit state strings — zero hits; `crates/server.md`'s `poll_qbittorrent` / `fetch_qbit_progress` prose lines remain true as written. No content-page edits needed beyond the above.

## 2026-07-13 — quality-waves 2d + #36: swallowed-writes sweep audit (no wiki fixes required)

- Verified the D2 sweep (15 warn-arm sites across `import_workflow.rs`, `author_service.rs`, `series_query_service.rs`, `import_service.rs` — probe #19–#22 plus same-class neighbors in the same functions) and the #36 one-transaction change (`insert_work_row` shared helper + `confirm_anchor_in_tx` inside one tx in `create_work_with_anchor`; conflict arm delegates to `create_work`, preserving no-anchor-write semantics) against wiki claims: full-text scan for `create_work_with_anchor`, `resolve_ol_key`, `cwa_copy`, `update_grab_status`, `link_work_to_series` — only neutral API-listing lines (crates/db.md:102/:204, crates/domain.md:217, crates/server.md:193), none asserting the pre-sweep swallow or the two-commit creation as fact. No content-page edits needed.

## 2026-07-13 — docs-sync: quality-waves Wave 3 pure moves

- integrations/goodreads.md: fixed 5 stale citations caused by the `goodreads.rs` → `goodreads/{mod,client,parsers,llm_repair}.rs` split — parser location (`goodreads.rs` → `goodreads/parsers.rs`), `GOODREADS_USER_AGENT` definition (wrong crate `livrarr-metadata` + old file → `crates/livrarr-external-data/src/goodreads/client.rs:26`, verified), the test-fixture UA string (`goodreads.rs:977` → `goodreads/parsers.rs:934`, verified still inside `#[cfg(test)] mod tests`), the `/search` audit mention (`goodreads.rs` → `goodreads/`), and the "Existing client" pointer (wrong crate + old file → `crates/livrarr-external-data/src/goodreads/`). The wrong-crate part of the two explicit citations was a pre-existing error (never `livrarr-metadata`), folded into the same fix since it shared the citation with the now-moved file.
- architecture/metadata-pathway-pseudocode.html: same split — `goodreads.rs` → `goodreads/` in the Goodreads Search section's Source line. Left the adjacent `crates/livrarr-metadata/src/provider_client.rs` mention untouched (real path is `livrarr-external-data/src/provider_client.rs`; that file never moved, so the wrong-crate name there predates and is unrelated to this wave).
- insights.md 55: `lib.rs:1611` → `lib.rs:735` for the REQ-005 LLM-validator-removal comment — the line shifted because `merge_engine.rs`'s ~912 lines were extracted out of `livrarr-enrichment/src/lib.rs` (845 lines now vs. 1757 combined). The `provider_queue.rs:524` citation earlier in the same insight is a different, untouched file — left as-is.
- insights.md 16: `main.rs ≈311-322` → `main.rs ≈908-919` for the `with_applicability_rule` registration — shifted because `main()`'s body was extracted into named private init fns (`ensure_data_dir`, `init_tracing_and_config`, `init_database`, `build_db_and_auth`, `build_http_clients`, `run_startup_passes`, `serve_until_shutdown`, …) earlier in the same file; content (GR/Audnexus/GB/Audible dispatch for non-`en` works) re-verified unchanged.
- crates/domain.md: two section headers corrected for the `lib.rs` → `entities.rs`/`enrichment_types.rs`/`infra_config.rs`/`util.rs` split — "Entities (lib.rs)" → "Entities (entities.rs, enrichment_types.rs, infra_config.rs)" (Core Entity Structs/Enums now span all three — e.g. `DownloadClient`/`Indexer`/`IndexerConfig` are in `infra_config.rs`, `FieldProvenance`/`MetadataProvider`/`WorkField`/`OutcomeClass` etc. are in `enrichment_types.rs` — not itemized per-symbol this pass) and "Utility Functions (lib.rs)" → "Utility Functions (util.rs)" (6 of 7 listed fns verified present there).
- crates/db.md and crates/server.md checked for layout claims from the same wave (db traits/request-structs → `src/api/*.rs`; server `main()` → named init fns) — no fixes needed. db.md never cited a file location for the DB traits/structs (only lists methods/fields), and server.md's "Entry Point (main.rs)" section is a same-file, summary-level description that the intra-file init-fn extraction doesn't falsify.
- Found but out of scope (reported, not edited): insights.md 50's `UpsertExternalIdRequest` emission citation (`livrarr-enrichment/src/lib.rs ≈952-980`) is stale, but not from this wave — the construct no longer exists anywhere in `livrarr-enrichment` (confirmed zero hits; superseded by REQ-007's identity-track-only anchor writes per insight 51); `git diff --cached -G"UpsertExternalIdRequest"` against the lib.rs split shows the symbol was already absent, i.e. this predates Wave 3. Same class: crates/domain.md's `SourceKind`/`is_foreign_source` entries reference symbols that don't exist anywhere in `livrarr-domain` any more (zero hits repo-wide, and absent from the lib.rs split diff) — pre-existing, left untouched. docs/metadata-audit-2026-06-28.md and docs/matching-inventory-2026-07-02.md carry now-stale `enrichment/lib.rs` merge-symbol line cites from before this wave — historical audit records, intentionally left untouched.
- `library-management.md`'s "failed CWA copy logs a warning while the main import still succeeds" remains true and is now fully honored — config/work/task-result read failures in the CWA/email block also warn instead of silently skipping.

## 2026-07-14 — indexer-citizenship unit (insight 71; server.md GrabSearchCache refs; contract doc committed)

- insights.md: added insight 71 — origin-keyed `RateBucket::Indexer` everywhere (configured-indexer origin at grab time, one `normalized_origin` authority in livrarr-http), indexer buckets breaker-tracked (429 → 30-min immediate trip; book-provider signals unchanged), no delegation bypass on RateLimited/CircuitOpen grabs, `ReleaseSearchCache` semantics (cache_only/default/refresh; all-miss = successful empty), Releases-tab cache-only mount, `fetch_no_redirect` default caveat, and the package-local tests/ gitignore trap.
- crates/server.md: the implementing agent removed the two stale `GrabSearchCache` references (the type + AppState field were deleted with the dead cache; the surviving `infra/cache.rs` content is the unrelated manual-import scan state).
- New committed docs: `design-indexer-rate-limits.md` (the 4-round-reviewed contract) + dispositions under build/reviews/quality-waves/ (local, untracked by convention).
- Verified no other wiki page asserts the pre-unit behavior (searched: GrabSearchCache, RateBucket::None grab claims, release search caching, indexer 429/backoff): grab-system.md and rss-sync.md describe flows above this granularity; rss-sync.md's known stale points (sequential/category-only) predate this unit and are unchanged by it.

## 2026-07-14 — matching-conformance unit (insight 59 amended, 13 corrected; design doc committed)

- insights.md: amended insight 59 — the shared provider hit-picker is now `pick_best_candidate` (identity_matching); the loose 0.75-jaccard `score_provider_candidates`/`score_candidates` are deleted; the 5 provider search sites route through it (`accept_grey=false` for Audible/OL/HC/GB, `true` for GR); author bar = `author_verdict` everywhere; OL index-back mapping fixed. Corrected insight 13's two stale "shared 0.75 picker" phrasings.
- New committed doc: `design-provider-picker-conformance.md` (both-family design review PASS r10; IMPLEMENTED header). Per-round dispositions under build/reviews/quality-waves/ (local, untracked by convention).
- Removed a stale SFC TDD canary (`tests/behavioral/test_sfc_audible_provider.rs`) that referenced the deleted `score_provider_candidates`; the rest of that stalled search-fallback-chain file is left for the separate orphan-test triage (insight 65 backlog).
- Codex code-review r1 caught a PRE-EXISTING OpenLibrary wrong-key bug (a compacted candidate index used against the unfiltered `docs` array) — folded (original-index `kept` mapping, mirrors HC/GR); r2 both families PASS.

## 2026-07-19 — work-history architecture gate closed (spine amended to 17; insight 48 corrected)

- docs/canonical-model.yaml: HistoryEvent added to the entity spine (17th entity) + amendments row — work-history architecture review r1, both families independently flagged the omission; altitude precedent PlaybackProgress/Bookmark.
- insights.md: corrected insight 48 — canonical model path is `docs/canonical-model.yaml` (not `architecture/`; moved 2026-06-29) and the spine is now 17 concepts.
- Ground-truth found during review folds (recorded in ir-v1-work-history.yaml + contract, spec ST-006 left untouched per closed-spec discipline): `retag_library_items` has THREE production callers — import_service.rs:111 (import-time), :284 (post-enrichment), :522 (reorganize_work_files, r2 Codex find; absent from ST-006's ":111/284" cite). Also: `supersede_anchor` has zero production callers; `HistoryServiceImpl` is the struct, `LiveHistoryService` its SqliteDb alias (state.rs:94).
- Architecture artifacts live at repo root: ir-v1-work-history.yaml + contract-work-history.yaml (r3 both-family PASS, zero findings; verify.py architecture + review both PASS on file).
- 2026-07-19: insights.md +76 (work_update echo / NoChange has no producers / content_changed is the truthful changed signal) — work-history Stage 5 session.
- 2026-07-20: insight 77 added (work-history event system + backfill semantics, feature aa7f6985); insight 48's amendments note already covers the HistoryEvent spine row.

- 2026-07-23: insight 30 — corrected outbound-queue wait semantics: bounded per-priority admission with typed QueueFull → budget-exempt WillRetry{QueueFull} plumbed through all 4 provider adapters (alpha-hardening fixes, audit #6).
- 2026-07-24: insights.md +78 (anchor uniqueness truth: 042 dropped 041's global, 044's per-user ALL-type index is live; GT6 falsification RCA pointers; 076 delta = bridge freedom). Source: identity-edit dual-suite red-run divergence.

- 2026-07-25 — added insight 79 (outbound-queue dispatcher wedge). Evidence is unusually strong: three of four independent implementations of an unrelated feature converged on the same fix to `outbound_queue.rs`, plus a single-variable A/B that turned a 7-minute hang into 36.82s. Also corrected a claim carried earlier in that session that the fix was one entry's idiosyncratic scope creep — it was not; three entries made it.
