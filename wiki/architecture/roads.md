# Roads — the whole-app one-road map

**What this is.** The flow-side contract for Livrarr: every operation that changes state, the ONE
canonical pipeline ("road") it must ride, and EVERY entry point ("door") into it. The structure-side
contract (entities, crate seams, system invariants) is `docs/canonical-model.yaml` — the two are
companions. A door that is not listed here does not exist; a second implementation of a listed road
is a defect, not a variant.

**How it is consumed.** Whole, in one read. Any feature design that touches a road must enumerate
that road's full door list (not just the door being edited) and show each door still converges.
Adding a door or a road requires updating this file in the same change.

**Door ID format.** `path::function` (function names survive edits; line numbers don't).
`@ spawn` marks a background task spawned inside that function.

**Status legend.** CLEAN = all doors converge · DEBT = a second road exists (fix or accept
explicitly) · DECISION = converges mechanically but a product call is open.

**Status:** authored 2026-07-04 from live-code traces; adversarially verified the same day by
cross-family review (Gemini + Codex, instructed to refute; record:
`docs/roads-md-adversarial-verification-2026-07-04.md`). Codex's two refutations — the
scan-adoption door and the user-cover fork — were verified at source and folded in below.
Where an older wiki page disagrees with this file, this file wins; where code disagrees with
both, code wins — fix the map.

**Summary: 14 roads. 9 CLEAN · 4 DEBT (R7 manual import, R8 Readarr import, R9 scan adoption,
R3 user-cover fork) · 1 DECISION (R2 background convergence ships disabled). 8 dead-code items
queued for deletion. File→LibraryItem creation universe: 4 sites (R6 canonical + 3 debt).**

---

## R1 — Work creation

- **Operation:** a book enters the library as a Work.
- **Road:** `resolve_identity` → seed factory (`livrarr-domain/src/seed.rs`) → `WorkService::add`
  → `ensure_identity_and_enrichment` → R2 (enrichment) → R3-adjacent materialize (cover).
- **Doors (6):**
  | Door | Entry | Converges |
  |---|---|---|
  | Direct add (search / GR link) | `crates/livrarr-handlers/src/work.rs::add` | yes |
  | Manual import (scan review) | `crates/livrarr-handlers/src/manual_import.rs::import` | yes (work creation; file handling is R7) |
  | List import (CSV) | `crates/livrarr-handlers/src/list_import.rs::confirm` | yes |
  | Author monitor | `crates/livrarr-server/src/author_monitor_workflow.rs::run_monitor` | yes |
  | Series monitor | `crates/livrarr-server/src/series_query_service.rs` (monitor worker) | yes (seeds Pending by design, per M9) |
  | Readarr import | `crates/livrarr-server/src/readarr_import_workflow.rs::start` | yes (work creation; file handling is R8) |
- **Invariant:** every door funnels into the single enrichment pipeline with a seed; no path writes
  enrichable metadata, covers, or tags by any other route (canonical-model invariant #1).
  Batch doors MAY seed identity-Pending; those works must converge later via R2 — never silent limbo (M9).
- **Forbidden:** creating a Work row outside the seed factory; enriching outside R2.
- **Deep docs:** [work-creation-pipeline](work-creation-pipeline.md), [metadata-pathway](metadata-pathway.md).
- **Status:** CLEAN.

## R2 — Enrichment / refresh

- **Operation:** fill or re-fill a Work's metadata from providers.
- **Road:** `WorkService::refresh` (or `add` via `ensure_identity_and_enrichment`) →
  `run_unified_enrichment` → `EnrichmentWorkflowImpl::enrich_work` → `EnrichmentServiceImpl::enrich_work`
  → `DefaultProviderQueue::dispatch_enrichment` → `MergeEngine` → `apply_enrichment_merge` (CAS)
  → materialize (cover + tag projection).
- **Doors (7):**
  | Door | Entry | Converges |
  |---|---|---|
  | Single refresh | `crates/livrarr-handlers/src/work.rs::refresh` | yes |
  | Bulk refresh | `crates/livrarr-handlers/src/work.rs::refresh_all @ spawn` — loops `WorkService::refresh` | yes |
  | Retry incomplete | `crates/livrarr-handlers/src/work.rs::retry_all_incomplete @ spawn` | yes |
  | Anchor affirm | `crates/livrarr-handlers/src/work.rs::affirm_pending_anchor @ spawn` | yes |
  | Post-add backfill (5s delay) | `crates/livrarr-handlers/src/work.rs::add @ spawn` | yes |
  | Post-manual-import (5s delay) | `crates/livrarr-handlers/src/manual_import.rs::import @ spawn` | yes |
  | Background convergence sweep | `crates/livrarr-server/src/jobs/convergence.rs::convergence_tick` → `WorkService::converge_work` | yes — but **ships disabled** (`config.rs` `ConvergenceConfig`, `enabled=false` default) |
- **Invariant:** all provider HTTP rides the one outbound queue (`livrarr-http/src/outbound_queue.rs`);
  merge respects provenance order User > Provider > System; null never overwrites populated.
- **Forbidden:** calling `enrich_work` / the provider queue / `apply_enrichment_merge` from anywhere
  but this chain; direct UPDATEs to enrichable columns.
- **Deep docs:** [metadata-pathway](metadata-pathway.md), [enrichment-pipeline](enrichment-pipeline.md).
- **Status:** DECISION — road is clean, but with convergence disabled by default, identity-Pending
  works from batch doors sit unresolved on default installs unless a user manually hits
  retry-incomplete. Open PO call: enable by default / leave off / off-but-surfaced.

## R3 — Cover change — **DEBT (fork)**

- **Operation:** a Work's cover artifact changes on disk.
- **Today — two write paths, not one:**
  - Enrichment covers ride the cover write gate (`cover_write_gate::run_cover_write_gate`,
    called from enrichment materialization) — proxy-validated, two slots, crash-recovery marker.
  - User covers do NOT: `LiveCoverService::select_cover` calls
    `livrarr_materialize::download_cover_to_disk` directly, and `::upload_cover` does its own
    tmp-write-then-rename (`crates/livrarr-server/src/cover_service.rs`), each followed by direct
    cover-metadata DB updates with User trust.
- **Doors (4):** `crates/livrarr-handlers/src/cover.rs::select_cover_handler` (→ direct path),
  `cover.rs::upload_cover_handler` (→ direct path), enrichment materialize step (R2 → gate),
  startup passes `crates/livrarr-server/src/jobs/cover_startup.rs::run` (layout migration →
  crash recovery → provenance backfill, strictly sequenced).
- **Invariant (intent):** user-selected covers lock against enrichment overwrite (trust order);
  ALL disk writes share one gate mechanics.
- **Target:** route the two user doors through the same write-gate mechanics, keeping User trust
  and the override-lock product behavior unchanged.
- **Status:** DEBT (small) — user doors bypass the gate mechanics; product semantics intact.
  (Dead sibling: `jobs/cover_backfill.rs` — superseded by cover_startup, see Dead code.)

## R4 — Identity state changes

- **Operation:** a Work's identity is affirmed, disputed, resolved, or merged.
- **Road:** identity engine (`livrarr-identity` via `WorkService`) — one-way contract: identity
  feeds enrichment via `CapturedIdentity`; enrichment never writes identity state.
- **Doors (6):** `identity_conflicts.rs::resolve`, `identity_conflicts.rs::dismiss`,
  `identity_review.rs::resolve`, `identity_review.rs::dismiss`, `work.rs::affirm_pending_anchor`,
  `work.rs::merge` (work merge, all in `crates/livrarr-handlers/src/`).
- **Invariant:** anchors are monotonic — ADD appends, CONFLICT raises to the user, only a user EDIT
  mutates an established anchor.
- **Status:** CLEAN.

## R5 — Release grab

- **Operation:** a chosen release is handed to a download client and tracked as a Grab.
- **Road:** `ReleaseService::grab` (`crates/livrarr-download/src/release_service.rs::grab`):
  SSRF check → resolve client → protocol dispatch (qBit / SAB / Transmission) →
  **`upsert_grab` (the one grab-creation chokepoint)** → history event.
- **Doors (3):**
  | Door | Entry | Converges |
  |---|---|---|
  | Manual grab | `crates/livrarr-handlers/src/release.rs::grab` | yes (`GrabSource::Manual`) |
  | RSS auto-grab (scheduled) | `crates/livrarr-server/src/jobs/rss_sync.rs::rss_sync_tick` → `RssSyncWorkflowImpl::run_sync` | yes (`GrabSource::RssSync`) |
  | RSS "sync now" | `crates/livrarr-handlers/src/config.rs::trigger_rss_sync @ spawn` → same workflow | yes |
- **Invariant:** grab record created only AFTER the client confirms; hash ownership — pollers touch
  only downloads matching an active Grab's hash for that client.
- **Notes:** no re-grab path exists anywhere (failed grabs are re-imported via R6, never re-grabbed).
  The two RSS doors carry duplicated guard glue around the same `Arc<AtomicBool>` — same lock,
  cosmetic duplication only.
- **Deep docs:** [grab-system](grab-system.md), [rss-sync](rss-sync.md), [usenet-pipeline](usenet-pipeline.md).
- **Status:** CLEAN.

## R6 — Import from grab

- **Operation:** a completed download becomes organized library files + a LibraryItem.
- **Road:** `ImportService::import_grab` (`crates/livrarr-server/src/import_service.rs::import_grab`)
  → `ImportWorkflow::import_grab` (`crates/livrarr-library/src/import_workflow.rs::import_grab`:
  per-(user,work) lock → enumerate/classify → `atomic_copy`, untagged → orphan adoption branch →
  LibraryItem create) → post-steps: retag if enriched (`TagService::retag_library_items`) →
  CWA copy (`infra/import_pipeline.rs::cwa_copy`) → auto-email.
- **Doors (5):**
  | Door | Entry | Converges |
  |---|---|---|
  | Poller, qBittorrent | `jobs/download_poller.rs::poll_qbittorrent` → `spawn_import` | yes |
  | Poller, SABnzbd | `jobs/download_poller.rs::poll_sabnzbd` → `spawn_import` | yes |
  | Poller, Transmission | `jobs/download_poller.rs::poll_transmission` → `spawn_import` | yes |
  | Auto retry w/ backoff (max 5) | `jobs/download_poller.rs::retry_failed_imports` → `spawn_import` | yes |
  | Manual "Retry Import" | `crates/livrarr-handlers/src/queue.rs::retry_import` | yes |
- **Invariant:** copy-for-import, never move; tags written to the library copy only, via the retag
  step — the import copy itself is untagged-then-deferred.
- **Forbidden:** creating LibraryItem rows or materializing library files outside `ImportWorkflow`.
- **Deep docs:** [import-pipeline](import-pipeline.md), [library-management](library-management.md).
- **Status:** CLEAN — the road itself holds. R7, R8, and R9's scan adoption violate the forbidden
  clause from outside: the file→LibraryItem creation universe is 4 sites, not 1.

## R7 — Manual import (file handling) — **DEBT**

- **Operation:** user-picked on-disk files become library files + LibraryItems.
- **Today:** a full second implementation. `manual_import.rs::import` →
  `ImportService::import_single_file` → `do_import_single_file`
  (`crates/livrarr-server/src/import_service.rs::do_import_single_file`): raw `std::fs::copy`
  (not `atomic_copy`), inline `write_tags` on the .tmp (not defer-to-retag), own LibraryItem create,
  own CWA + email steps. Only chapter extraction is shared with R6.
- **Divergence that bites:** different copy semantics, different tag timing, two code paths to keep
  in sync for every import fix.
- **Target:** converge file materialization on R6's `ImportWorkflow`; manual import keeps only its
  match/confirm UX.
- **Status:** DEBT — accepted second road until the Phase-2 consolidation decision; no new callers.

## R8 — Readarr import (file handling) — **DEBT**

- **Operation:** bulk catalog migration from Readarr — files hardlinked/copied in, LibraryItems created.
- **Today:** a third implementation. `readarr_import_workflow.rs::start` → spawned
  `ImportRunner::run` → `process_files` → own `materialize_file` (hardlink-first) → direct
  `create_library_item` with `tag_status: Pending`. Never calls `ImportWorkflow` / `ImportService` /
  `TagService`; skips CWA copy and email entirely; relies on R10's tag_convergence job to
  eventually tag.
- **Target:** same as R7 — file materialization through `ImportWorkflow` (its hardlink-first mode
  is a legitimate config of the road, not a reason for a separate road).
- **Status:** DEBT — accepted second road until the Phase-2 consolidation decision; no new callers.

## R9 — Library scan — **DEBT**

- **Operation:** walk a root folder, adopt what's on disk.
- **Doors (2):** `crates/livrarr-handlers/src/root_folder.rs::scan` (per-rootfolder),
  `root_folder.rs::scan_path` (unmapped scan). Both synchronous (`spawn_blocking` inside).
- **Today:** scan adoption creates LibraryItem rows directly in the handler — a matched file goes
  `root_folder.rs::scan` → `ImportIoService::create_library_item` → `db.create_library_item`,
  never touching `ImportWorkflow`. This is the fourth file→LibraryItem creation site
  (alongside R6 canonical, R7 manual, R8 Readarr).
- **Note:** `ImportWorkflow::confirm_scan` — the fn `library-management.md` calls "the single import
  orchestration surface" — is dead: zero HTTP callers, its only test is `#[ignore]`d. See Dead code
  and Wiki corrections.
- **Target:** scan adoption joins the Phase-2 import consolidation (adopt-in-place becomes a mode
  of the one road, not a separate road).
- **Status:** DEBT — no new callers of the direct path.

## R10 — Tag write / sync

- **Operation:** metadata is projected into file tags (EPUB load-bearing; audio writers disabled — OOM).
- **Road:** `TagService` / materialize tag projection; recovery loop
  `jobs/tag_convergence.rs::tag_convergence_tick` (60s) sweeps `tag_status: Pending`.
- **Doors:** R2 materialize step; R6 retag post-step; tag_convergence job. No direct HTTP door.
- **Forbidden:** calling `livrarr_tagwrite::write_tags` outside `TagService`/materialize —
  R7 currently violates this inline.
- **Status:** CLEAN as a road; R7's inline call is tracked under R7's DEBT.

## R11 — Author monitoring

- **Operation:** watch an author, pull new works into the library.
- **Road:** `AuthorMonitorWorkflow::run_monitor` → seeds via R1's author-monitor door.
- **Doors (4):** `jobs/author_monitor.rs::author_monitor_tick` (24h), manual trigger
  `crates/livrarr-handlers/src/work.rs::author_search @ spawn`, bibliography refresh route
  `author.rs::refresh_bibliography`, post-create delayed spawns (`work.rs::add`, `author.rs::add`).
- **Status:** CLEAN.

## R12 — Series monitoring

- **Operation:** watch a series, discover and add its books.
- **Road:** `run_series_monitor_worker` → seeds via R1's series-monitor door.
- **Doors (5):** `crates/livrarr-handlers/src/series.rs::monitor_series @ spawn`,
  `series.rs::promote_series @ spawn`, `series.rs::refresh_series`, `series.rs::resolve_gr`,
  startup repair `jobs/series_backfill.rs::run_series_backfill`.
- **Status:** CLEAN.

## R13 — RSS sync

- **Operation:** poll indexers, match releases to monitored works, auto-grab.
- **Road:** `RssSyncWorkflowImpl::run_sync` (fetch → parse → score → filter → grab via R5).
- **Doors (2):** scheduled `jobs/rss_sync.rs::rss_sync_tick` (cadence DB-configured, 0=off),
  manual `config.rs::trigger_rss_sync @ spawn`. Same workflow instance, same run guard.
- **Deep doc:** [rss-sync](rss-sync.md).
- **Status:** CLEAN.

## R14 — Playback progress & cross-format resume

- **Operation:** reading/listening position moves; positions sync across formats via kash links.
- **Doors (3):** `crates/livrarr-handlers/src/workfile.rs::update_progress`,
  `cross_format.rs::post_sync_to_here`, `cross_format.rs::post_decline`.
- **Invariant:** positions never move backward automatically; a cross-format jump happens only on
  explicit user confirmation; strictly per-user.
- **Deep doc:** [cross-format-resume](../domain/cross-format-resume.md).
- **Status:** CLEAN.

---

## Not roads (single-door writes — no convergence contract needed)

Work update/delete · author CRUD · series row update · bookmarks CRUD · notifications
dismiss/read · queue remove · workfile delete · send-to-Kindle · config/indexer/download-client/
remote-path CRUD + connectivity tests · auth/setup/user/session · log level. One door each; the
generic invariants (provenance order, per-user isolation, no SQL outside livrarr-db) still apply.
A second door onto any of these promotes it to a road row in this file.

## Dead code queued for deletion (found during mapping, 2026-07-04)

| Item | Where | Why dead |
|---|---|---|
| `run_repair` | `jobs/repair.rs` | zero call sites (only its own behavioral test) |
| ~~`run_cover_backfill`~~ | ~~`jobs/cover_backfill.rs`~~ | DONE — file deleted in N2 (merge `9f1f61e`, 2026-07-04) |
| `ImportWorkflow::confirm_scan` | `livrarr-library/src/import_workflow.rs` | zero HTTP callers; test `#[ignore]`d |
| `ImportWorkflow::retry_import` | `livrarr-library/src/import_workflow.rs` | production retry uses `ImportService::import_grab` |
| `DownloadService` trait + `GrabResult` | `livrarr-download/src/lib.rs` | zero implementors; trap next to the real chokepoint |
| `copy_to_cwa` | `livrarr-library/src/lib.rs` | zero production callers; live path is `infra/import_pipeline.rs::cwa_copy` (known debt, `scripts/recovery-advice.py`) |
| `GrabSource::AutoAdd` | `livrarr-domain/src/services/release.rs` | never constructed (planned auto-grab-on-add, never wired) |
| `ReadarrImportService::update_work_enrichment` | `crates/livrarr-server/src/readarr_import_service.rs` | reported caller-less by cross-family review — an unused enrichment-write wrapper (standing R2 bypass risk); verify callers before delete |

## Wiki corrections queued (stale statements this mapping falsified)

- `overview.md` → canonical model lives at `docs/canonical-model.yaml`, not `architecture/canonical-model.yaml`.
- `library-management.md` — `confirm_scan` is NOT the import orchestration surface; manual import
  runs through `ImportService::import_single_file` (currently R7 DEBT).
- `grab-system.md` — missing Transmission (third client) and the automatic import-retry-with-backoff.
- `metadata-pathway.md` / `work-creation-pipeline.md` — the background convergence job now EXISTS
  (`jobs/convergence.rs`), disabled by default; "removed / zero production callers" is stale.
