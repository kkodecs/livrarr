# Design/packet: door-gate suite (pipeline-hygiene item 2)

Status: v2 after review r1 (codex P1 folded: handler layer added, goal made precise;
gemini P3 folded: reuse existing helper conventions). Authored 2026-07-12 from
live-source traces (every expected value below carries the `path:line` it was read
from; the working tree at trace time == commit `8c9c4ab5` + the suppression deletion).

## Goal

One table-driven behavioral suite, `tests/behavioral/test_door_gate.rs`, pinning the R1
(work-creation) and R2 (enrichment/refresh) doors of `wiki/architecture/roads.md` at THREE
layers: job-door→`WorkService::add` (Layer A), service→enrichment-seam (Layer B), and
handler-door→service (Layer C — the layer the 2026-06 add-from-search incident lived at).
Covered exhaustively: all 6 R1 doors' service halves; all 7 R2 doors; the 5 `work.rs`
handler doors. Excluded, by name, with reasons (§Exclusions): the `manual_import.rs::import`
and `list_import.rs::confirm` HANDLER BODIES (their work-creation and refresh-chain halves
ARE covered at Layers A/B) and the private Readarr `ImportRunner` (documented stand-in).
Closes wiki insight 46's recurring class at the covered layers. BUILD-LIGHT per PO decision
2026-07-12: behavioral table, not type-system enforcement. Pattern generalized from
`work_service_doors_thread_expected_freshness` (tests/behavioral/test_responsiveness_cache.rs:559).

## Traced system facts the table rests on (verified in source, 2026-07-12)

- F1. `ensure_identity_and_enrichment` is a PRIVATE inherent method on `WorkServiceImpl`
  (crates/livrarr-metadata/src/work_service.rs:2111), absent from the `WorkService` trait.
  The observable seam for tests is the `EnrichmentWorkflow::enrich_work` call it (and the
  refresh/convergence paths) make — which `StubEnrichmentWorkflow` records
  (crates/livrarr-behavioral/src/stubs.rs:277-296).
- F2. The ensure seam dispatches a CONSTANT triple for every caller:
  `(EnrichmentMode::Background, RequestPriority::High, Freshness::PreferCache)`, threading
  `source_provider_data` + `candidate_id` (work_service.rs:2193-2201). The six R1 doors are
  NOT distinguished at this seam — the sameness IS the contract.
- F3. The ensure seam is gated: no enrichment call when the work doesn't need it
  (work_service.rs:2186-2188) and identity Pending/Conflict/NeedsReview blocks enrichment
  (doc, work_service.rs:2106-2108; behavior already pinned in test_responsiveness_add.rs).
- F4. `WorkService::refresh` maps `RefreshSurface::Interactive → RequestPriority::Normal`,
  `Bulk → Low`, both with `EnrichmentMode::Manual`, `Freshness::Bypass`, `candidate_id: None`
  (work_service.rs:1619-1633), gated on `identity_permits` (work_service.rs:1613).
- F5. `retry_all_incomplete` routes each incomplete work through
  `refresh(user, work, RefreshSurface::Bulk)` (crates/livrarr-metadata/src/convergence_service.rs:299-303).
- F6. `converge_work` BYPASSES ensure: direct `settle_identity(Background, Convergence)` only
  when a chaseable anchor remains (convergence_service.rs:104-113), then direct
  `run_unified_enrichment(None, EnrichmentMode::Background, None, RequestPriority::Low,
  Freshness::PreferCache)` only when identity permits AND enrichment is
  Unenriched|Failed (convergence_service.rs:124-143).
- F7. `WorkService::add` = `add_fast` + awaited `complete_add` for created works
  (work_service.rs:320-334); identity mode derives from the candidate's `provenance_setter`:
  `Import|Imported → IdentityMode::Background`, everything else (incl. None and `AutoAdded`)
  `→ Interactive` (work_service.rs:309-318). `add_fast`'s dedup branches call ensure directly
  (work_service.rs:619-628 and the adopt/normalized-match twins).
- F8. Seed constructors' `provenance_setter` (crates/livrarr-domain/src/seed.rs):
  `seed_add_box` None (:118) · `seed_manual_import` None (:142) · `seed_list_import`
  Some(Imported) (:165) · `seed_author_monitor` Some(AutoAdded) (:185) · `seed_series_monitor`
  Some(AutoAdded) (:211) · `seed_readarr_import` Some(Import) (:237).
- F9. `StubEnrichmentWorkflow` records (mode, priority) pairs, freshness, work_ids, call
  counts (stubs.rs:211-296) but currently DROPS `candidate_id` (`_candidate_id`, stubs.rs:283).
- F10. `EnrichmentWorkflow::enrich_work` does NOT carry `source_provider_data` — that value is
  consumed inside `run_unified_enrichment` before the workflow call. Not observable at this seam.
- F11. Readarr's `ImportRunner`/`process_works` is private with one production construction
  site (crates/livrarr-server/src/readarr_import_workflow.rs:207); behavioral tests use the
  documented stand-in: Readarr-shaped candidate driven through `WorkService::add`
  (tests/behavioral/test_wcc_path_seams.rs:8-10, test_author_dedup.rs:10-13).
- F12. `AuthorMonitorWorkflowImpl` IS constructible against a per-file recording
  `StubWorkService` (tests/behavioral/test_consolidation_author_monitor.rs:26-53, :350-352).

## Deliverables

1. **Stub addition (behavioral crate, additive only):** `StubEnrichmentWorkflow` gains a
   `candidate_ids: Arc<Mutex<Vec<Option<CandidateId>>>>` recorder + `candidate_ids()` accessor,
   populated in `enrich_work` (rename `_candidate_id` → `candidate_id`). No existing test changes.
2. **`tests/behavioral/test_door_gate.rs`** — registered in `crates/livrarr-behavioral/Cargo.toml`
   AND `git add -f`'d in the same change (registering and force-adding are ONE change — CLAUDE.md
   lesson 2026-07-12). Three layers below (A: job-door→add · B: service→seam · C: handler→service).
3. **Convention line** added to `wiki/architecture/roads.md` (R1/R2 sections) and the tail of wiki
   insight 46: *a new R1/R2 door is not done until its row exists in `test_door_gate.rs`.*
4. Suite header documents the residual NOT covered (see §Residuals).

## Layer B — service→seam rows (real `WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>`,
constructed as in test_author_dedup.rs:64-71; fresh `:memory:` DB per test; spy assertions on the
recorded (mode, priority) + freshness + candidate_id + work_ids + exact call count)

| Row | Drive (real entry) | Expected at the seam |
|---|---|---|
| B1 | `add` w/ `seed_add_box(…, Confirmed identity, candidate_id: Some(X), false)` | exactly 1 call: (Background, High), PreferCache, candidate_id Some(X), right work_id [F2, F7] |
| B2 | `add` w/ `seed_manual_import(…, candidate_id: Some(X))` | same as B1 [F2, F8] |
| B3 | `add` w/ `seed_list_import(…, candidate_id: None)` | 1 call: (Background, High), PreferCache, candidate_id None [F2, F8] |
| B4 | `add` w/ `seed_author_monitor(…)` | 1 call: canonical triple, candidate_id None [F2, F8] |
| B5 | `add` w/ `seed_series_monitor(…, series_id, flags)` | 1 call: canonical triple; work row carries series_id + monitor flags [F2, F8] |
| B6 | `add` w/ `seed_readarr_import(…, source_provider_data, …)` — the F11 stand-in for the private ImportRunner seam | 1 call: canonical triple, candidate_id None [F2, F8, F11] |
| B7 | `add` (any seed) of a work whose anchor-dedup target is already **Enriched** | **0** enrichment calls — the needs-gate [F3] |
| B8 | `add` anchor-dedup onto an existing **Unenriched** work | 1 call: canonical triple (ensure via add_fast's dedup branch) [F7] |
| B9 | `refresh(user, work, RefreshSurface::Interactive)` | 1 call: (Manual, Normal), Bypass, candidate_id None [F4] |
| B10 | `refresh(user, work, RefreshSurface::Bulk)` | 1 call: (Manual, Low), Bypass, candidate_id None [F4] |
| B11 | `retry_all_incomplete` with exactly one incomplete work seeded | 1 call: (Manual, Low), Bypass — rides refresh(Bulk) [F5] |
| B12 | `converge_work(user, work, 3)` on identity-permitting + Unenriched work | 1 call: (Background, Low), PreferCache, candidate_id None [F6] |
| B13 | `converge_work` on an **Enriched** work | **0** enrichment calls [F6] |
| B14 | `add` of a candidate resolving identity-Pending (bridge-only seed) | **0** enrichment calls (identity gate; overlaps the pin in test_responsiveness_add.rs — kept so THIS table is the complete door contract) [F3] |

Table mechanics: one `#[tokio::test]` per row is acceptable, but prefer a shared
`assert_door(row)` helper taking a row struct (drive closure + expected struct) so a new door
is literally one new row. Seeding helpers may reuse the suite-local patterns from
test_responsiveness_cache.rs (create_user/seed_work/confirmed_candidate shapes).
CAUTION (fixture trap, wiki insight 68 tail): a second enrichment pass on the same work
terminal-skips unless retry state is reset — rows drive FRESH works; B7/B8's dedup targets are
seeded via a first add, and any re-drive must call `reset_all_retry_states` (NOT
`reset_enrichment_for_refresh`).

## Layer A — job-door→`WorkService::add` rows (recording StubWorkService)

| Row | Drive | Expected recorded `add` call |
|---|---|---|
| A1 | `AuthorMonitorWorkflowImpl::run_monitor` with one eligible bibliography entry (construction per F12; OL fixture via StubHttpFetcher as in test_consolidation_author_monitor.rs) | exactly the works the fixture makes eligible; each candidate has provenance_setter Some(AutoAdded), identity carrying the entry's ol_key, candidate_id None [F8; D4 trace] |
| A2 | series-monitor worker (`run_series_monitor_worker`) — **conditional**: IF constructible in the behavioral crate against a recording StubWorkService + stub HTTP (verify at authoring time), one roster-gap drive | add called with provenance Some(AutoAdded), series_id Some, monitor flags from the series row [F8; D5 trace]. IF NOT constructible: pin `seed_series_monitor`'s output shape directly (unit-style row) AND document the visibility gap in the suite header exactly like the Readarr precedent (F11) — do NOT change production visibility to force it. |
| A3 | list import (`ListServiceImpl::confirm`) — **conditional**, same rule as A2: if constructible with a recording StubWorkService, drive one confirmed row; expected: add called with provenance Some(Imported) [F8; D3 trace]. Fallback identical to A2's. |
| A4 | Readarr | NO row — F11; covered by B6 + the documented stand-in. The suite header says so. |

A-row conditionals are resolved by the AUTHOR (Codex) at authoring time by attempting the
construction; whichever arm applies, the suite header records which was taken and why, citing
the blocking type if not constructible. Both arms are fully specified above — no improvisation.

## Layer C — handler-door→service rows (NEW in v2, folding review r1 codex P1)

Precedent: behavioral tests already drive handler FUNCTIONS directly with a suite-local state
type implementing the needed `Has*` traits + a constructed `AuthContext { user, auth_type,
session_token_hash }` (crates/livrarr-handlers/src/types/auth.rs:6-11). Live examples:
`test_id_completeness.rs:34-36,259-260` (imports and drives `work::affirm_pending_anchor`
with an `auth_context()` helper), `test_author_dedup.rs:914-944` (AuthContext + RouteState),
`test_responsiveness_bulk.rs:267` (drives `bulk_refresh_sweep` directly). REUSE those helper
conventions verbatim (review r1 gemini P3) — same helper names/shapes where they exist.

Harness: a recording `StubWorkService` implementing the full `WorkService` trait (pattern:
tests/behavioral/test_consolidation_author_monitor.rs:26-100 — record what each row asserts,
`todo!()` the rest), wrapped in a suite-local state implementing the handler's bound
(`HasWorkService` + `HasNotificationService`/`HasTagService` where the handler requires them).
Spawned work: `#[tokio::test(start_paused = true)]` + advance time past the 5s chains; await
assertions via a bounded wait-for-call-count helper polling the stub recorder with
`tokio::time::advance`/`yield_now` — NEVER bare wall-clock sleeps (tests review r1 flake lesson).

| Row | Drive (handler fn, direct call) | Expected recorded service calls, in order |
|---|---|---|
| C1 | `work::add` with a created:true stubbed `add_fast` (crates/livrarr-handlers/src/work.rs:216-276). Bounds (review r2 codex P2, verified at work.rs:170-177): the C1 state must implement `HasWorkService + HasAuthorService + HasSeriesQueryService + HasEnrichmentWorkflow + HasIdentityResolver + HasAppConfigService` — all except WorkService as INERT compile-time stubs; `app_config_service().get_default_language()` is called pre-candidate (work.rs:186-187), so the AppConfig stub must answer it. `add_fast` must return `author_created: false` so the author/series follow-up spawns (work.rs:276-335) stay un-exercised. Only `WorkService` calls are asserted. | `resolve_identity_local(..)`, `add_fast(user, candidate)`, then spawned: `complete_add(uid, wid, None, candidate_id, IdentityMode::Interactive, ConflictSource::ManualAdd)` (work.rs:262-269), then AFTER the 5s advance: `refresh(uid, wid, RefreshSurface::Interactive)` (work.rs:271-275) — chained, never before complete_add returns |
| C2 | `work::refresh` (work.rs:648-658) | exactly one `refresh(user, id, RefreshSurface::Interactive)` |
| C3 | `work::refresh_all` (work.rs:711-775) | `try_start_bulk_refresh` guard consulted; spawned sweep issues `refresh(user, work, RefreshSurface::Bulk)` for EVERY listed work (via `bulk_refresh_sweep`, work.rs:673-688) |
| C4 | `work::retry_all_incomplete` (work.rs:777-800) | spawned `retry_all_incomplete(user)` service call |
| C5 | `work::affirm_pending_anchor` (work.rs:1003-1060) | the affirm service call, then spawned `refresh(user_id, work_id, RefreshSurface::Interactive)` (work.rs:1050-1058) |

If a handler's exact `Has*` bound set makes a row disproportionate (e.g. drags in unrelated
service stubs), the author reports the bound list and stops for a scope call — do NOT stub
half the app to force a row.

## Exclusions (documented verbatim in the suite header)

- `manual_import.rs::import` and `list_import.rs::confirm` handler bodies — composite handler
  contexts (multipart/preview machinery, many services). Their work-creation halves are covered
  at Layers A/B (B2/B3, A3-conditional); the manual-import +5s refresh chain (manual_import.rs:886-895)
  has the same shape as C1's chain tail. A future incident in those handler bodies is NOT caught
  by this suite — named residual.
- Readarr `ImportRunner` — private, one production construction site (F11); covered by the
  documented stand-in (B6). No handler/job row possible without a production visibility change.
- `source_provider_data` — not observable at the workflow seam (F10). Pinning it would need a
  deeper spy at `run_unified_enrichment` (a WorkServiceImpl-internal seam) or a full
  ProviderQueue-level fixture; out of BUILD-LIGHT scope. Recorded, not asserted.
- `IdentityMode`/`ConflictSource` per door — identity-side parameters (settle_identity), invisible
  to the enrichment spy at Layer B. Layer C's C1 row pins the literals the add handler threads
  (work.rs:262-269); Layer A pins the provenance values that drive the derivation (F7/F8) for the
  batch doors.

## Process

Codex authors the suite from this packet (cross-family: OpenAI writes tests). Expected result:
suite lands GREEN (it pins current wiring); any red row is a REAL door defect — report it, do not
bend the row to pass (if a row contradicts live behavior, the packet or the code is wrong — stop
and escalate, cite the row's F-fact). Gemini reviews the authored suite. fmt/clippy/full-workspace
gates as always.
