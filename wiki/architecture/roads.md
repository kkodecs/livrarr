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

**Updated 2026-07-04 (import-consolidation):** R7/R8/R9 file-handling debts absorbed into R6's one
core (`ImportWorkflow::import_file`), R3's user-cover fork closed through the write gate, R2's open
decision resolved (convergence default ON, `1697bc7`). Design + cross-family confer record:
`build/plans/design-import-consolidation.md` (untracked build/).

**Summary: 14 roads, all CLEAN — R7/R8/R9 absorbed into R6, R3 unified through the gate, R2
decision resolved (2026-07-04). Dead-code queue: executed, see table. File→LibraryItem creation
universe: 1 site (`ImportWorkflow`).**

---

## R1 — Work creation

- **Operation:** a book enters the library as a Work.
- **Road:** `resolve_identity` → seed factory (`livrarr-domain/src/seed.rs`) → `WorkService::add`
  → `ensure_identity_and_enrichment` → R2 (enrichment) → R3-adjacent materialize (cover).
- **Doors (6):**
  | Door | Entry | Converges |
  |---|---|---|
  | Direct add (search / GR link) | `crates/livrarr-handlers/src/work.rs::add` | yes |
  | Manual import (scan review) | `crates/livrarr-handlers/src/manual_import.rs::import` | yes (work creation; file handling: R6 manual-file door) |
  | List import (CSV) | `crates/livrarr-handlers/src/list_import.rs::confirm` | yes |
  | Author monitor | `crates/livrarr-server/src/author_monitor_workflow.rs::run_monitor` | yes |
  | Series monitor | `crates/livrarr-server/src/series_query_service.rs` (monitor worker) | yes (seeds Pending by design, per M9) |
  | Readarr import | `crates/livrarr-server/src/readarr_import_workflow.rs::start` | yes (work creation; file handling: R6 Readarr door) |
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
  | Background convergence sweep | `crates/livrarr-server/src/jobs/convergence.rs::convergence_tick` → `WorkService::converge_work` | yes — enabled by default since 2026-07-04 (`1697bc7`; `[convergence] enabled = false` opts out) |
- **Invariant:** all provider HTTP rides the one outbound queue (`livrarr-http/src/outbound_queue.rs`);
  merge respects provenance order User > Provider > System; null never overwrites populated.
- **Forbidden:** calling `enrich_work` / the provider queue / `apply_enrichment_merge` from anywhere
  but this chain; direct UPDATEs to enrichable columns.
- **Deep docs:** [metadata-pathway](metadata-pathway.md), [enrichment-pipeline](enrichment-pipeline.md).
- **Status:** CLEAN — decision resolved 2026-07-04 (step1-code Unit 1): background convergence is
  enabled by default; batch-door identity-Pending works converge without user action.

## R3 — Cover change

- **Operation:** a Work's cover artifact changes on disk.
- **Road:** ONE commit protocol in `cover_write_gate.rs` — slot lock keyed (user, work, slot) →
  bytes → tmp + meta sidecar → DB commit → atomic rename → meta cleanup; crash recovery converges
  from the sidecar, whose `url` is `Option<String>` (None for uploads). Two entries, one mechanics:
  - `run_cover_write_gate` — enrichment candidates: User-incumbent NoOp, trust ladder, same-URL
    short-circuit, keep-or-replace comparator (semantics unchanged from N2).
  - `run_user_cover_write` — user select (URL) / upload (bytes): no trust guard, no comparator —
    a user choice is absolute, including replacing their own earlier pick. Upload validation
    (5MB cap, magic-byte sniff, 8000×8000 cap, JPEG re-encode) lives in the gate module: the
    single validation site.
- **Doors (4):** `crates/livrarr-handlers/src/cover.rs::select_cover_handler` and
  `cover.rs::upload_cover_handler` (→ `LiveCoverService` → `run_user_cover_write`), enrichment
  materialize step (R2 → `run_cover_write_gate`), startup passes
  `crates/livrarr-server/src/jobs/cover_startup.rs::run` (layout migration → crash recovery →
  provenance backfill, strictly sequenced).
- **Invariant:** user covers lock against enrichment overwrite (trust order, unchanged); ALL disk
  writes share the one gate mechanics and the one recovery protocol.
- **Status:** CLEAN (2026-07-04, import-consolidation).

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

## R6 — Import (file → LibraryItem, one road for all doors)

- **Operation:** a file becomes an organized library file + a LibraryItem — from a completed
  download, a user-picked file, a Readarr migration, or a scan adoption.
- **Road:** `ImportWorkflow::import_file` (`crates/livrarr-library/src/import_workflow.rs`) —
  per-(user,work) lock → target validation → adopt/dedup outcome matrix (row for this work →
  Skipped · row for another work, or a size-mismatched orphan → PathCollision, surfaced per-file ·
  size-matched orphan → Adopted, no I/O) → materialize per `Materialization` mode (`Copy` =
  `atomic_copy` · `HardlinkFirst` = hardlink with atomic-copy fallback · `AdoptInPlace` = no I/O)
  → LibraryItem create (`tag_status: Pending`) → optional chapter extraction. Grab imports drive
  the same core per file via `import_grab` (grab resolution, enumeration, size pre-check, format
  filter, grab status + history; holds the one lock for its whole run — the core is non-reentrant
  by design). Post-steps are per-door POLICY in `LiveImportService`: grab = retag-if-enriched →
  CWA → email · manual = retag unconditional → CWA → email · Readarr = none (tags ride R10
  convergence) · scan = none.
- **Doors (8):**
  | Door | Entry | Mode |
  |---|---|---|
  | Poller, qBittorrent | `jobs/download_poller.rs::poll_qbittorrent` → `spawn_import` | Copy |
  | Poller, SABnzbd | `jobs/download_poller.rs::poll_sabnzbd` → `spawn_import` | Copy |
  | Poller, Transmission | `jobs/download_poller.rs::poll_transmission` → `spawn_import` | Copy |
  | Auto retry w/ backoff (max 5) | `jobs/download_poller.rs::retry_failed_imports` → `spawn_import` | Copy |
  | Manual "Retry Import" | `crates/livrarr-handlers/src/queue.rs::retry_import` → `ImportService::import_grab` | Copy |
  | Manual import (file) | `manual_import.rs::import` → `ImportService::import_single_file` | Copy |
  | Readarr migration | `readarr_import_workflow.rs::ImportRunner::process_files` | HardlinkFirst |
  | Scan adoption | `root_folder.rs::scan` → `ImportService::adopt_scanned_file` | AdoptInPlace |
- **Invariant:** copy-for-import on grab/manual, hardlink-first on Readarr, adopt-in-place on scan;
  tags written to the library copy only, via retag/R10 — never inline during materialization.
- **Forbidden:** creating LibraryItem rows or materializing library files outside `ImportWorkflow`
  — now true at exactly 1 site. (Exempt, documented: `api_secondary_impl.rs::create_test_library_item`
  — test scaffolding with zero callers anywhere (LSP + text scan, 2026-07-04); deletion candidate.)
- **Deep docs:** [import-pipeline](import-pipeline.md), [library-management](library-management.md)
  (both describe the pre-consolidation shape — correction queued below).
- **Status:** CLEAN.

## R7 — Manual import (file handling) — **ABSORBED into R6 (2026-07-04)**

- Manual import keeps its match/confirm UX (`manual_import.rs`); file materialization is R6's
  `import_file(Copy)` via `ImportService::import_single_file`. The old second pipeline
  (raw `std::fs::copy` + inline `write_tags` on the .tmp + own LibraryItem create) is deleted;
  tags land via the unconditional retag post-step, synchronously BEFORE CWA and email — closing
  the historical R10 forbidden-clause violation.
- Behavior deltas vs the old fork: atomic copy; per-(user,work) lock + dedup/collision handling;
  a tag-write failure now self-heals via R10 convergence instead of permanently landing untagged.

## R8 — Readarr import (file handling) — **ABSORBED into R6 (2026-07-04)**

- `ImportRunner::process_files` keeps discovery/path-translation/progress; per file it calls R6's
  `import_file(HardlinkFirst)`. Its private `materialize_file` and direct `create_library_item`
  wrapper are deleted. Policy unchanged: no tags at import time (R10 tag_convergence), no CWA, no
  email, no chapters.
- Behavior delta: a re-run with file-present-but-row-missing now ADOPTS the orphan (size-checked
  by the core) instead of skipping — closes the crashed-partial-migration hole. Cross-work path
  collisions surface as per-file errors in the progress report; the bulk run never aborts on one
  file.

## R9 — Library scan — **ABSORBED into R6 (2026-07-04)**

- `root_folder.rs::scan` no longer creates LibraryItem rows in the handler: a matched untracked
  file goes `ImportService::adopt_scanned_file` → R6 `import_file(AdoptInPlace)`. Path collisions
  land in the scan's error list and the walk continues.
- **Door-list correction:** `scan_path` was never an adoption door — it is a read-only preview
  (matches against `identity_key_flat`, zero DB writes; verified at source 2026-07-04). The old
  "Doors (2)" row overstated it.
- `ImportWorkflow::confirm_scan` (the dead fn `library-management.md` mislabeled "the single import
  orchestration surface") was deleted in step1-code Unit 2 (`5e427d8`).

## R10 — Tag write / sync

- **Operation:** metadata is projected into file tags (EPUB load-bearing; audio writers disabled — OOM).
- **Road:** `TagService` / materialize tag projection; recovery loop
  `jobs/tag_convergence.rs::tag_convergence_tick` (60s) sweeps `tag_status: Pending`.
- **Doors:** R2 materialize step; R6 retag post-step; tag_convergence job. No direct HTTP door.
- **Forbidden:** calling `livrarr_tagwrite::write_tags` outside `TagService`/materialize —
  holds everywhere since 2026-07-04 (the manual door's historical inline call now rides the
  retag post-step).
- **Status:** CLEAN.

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

| Item | Where | Status |
|---|---|---|
| `run_repair` | `jobs/repair.rs` | DONE — step1-code Unit 2 (`5e427d8`) |
| `run_cover_backfill` | `jobs/cover_backfill.rs` | DONE — N2 (merge `9f1f61e`) |
| `ImportWorkflow::confirm_scan` | `livrarr-library/src/import_workflow.rs` | DONE — step1-code Unit 2 (`5e427d8`) |
| `ImportWorkflow::retry_import` | `livrarr-library/src/import_workflow.rs` | DONE — import-consolidation 2026-07-04 (trait method, impl, stub; 2 tests deleted, 1 repointed at `import_grab`) |
| `DownloadService` trait + `GrabResult` | `livrarr-download/src/lib.rs` | DONE — step1-code Unit 2 (`5e427d8`) |
| `copy_to_cwa` | `livrarr-library/src/lib.rs` | DONE — step1-code Unit 2 (`5e427d8`) |
| `GrabSource::AutoAdd` | `livrarr-domain/src/services/release.rs` | DONE — step1-code Unit 2 (`5e427d8`) |
| `ReadarrImportService::update_work_enrichment` | `crates/livrarr-server/src/readarr_import_service.rs` | DONE — step1-code Unit 2 (`5e427d8`) |
| dead `ImportService` trait + orphan result structs | `livrarr-library/src/lib.rs` | DONE — import-consolidation 2026-07-04 (name-collision trap beside the live domain trait) |
| `ImportIoService::create_library_item` + `CreateLibraryItemRequest` | `livrarr-domain/src/services/import_io.rs` (+ impl/stub) | DONE — import-consolidation 2026-07-04, dead after the R7/R9 rewires |
| `ReadarrImportService::create_library_item` | `crates/livrarr-server/src/readarr_import_service.rs` | DONE — import-consolidation 2026-07-04, dead after the R8 rewire |
| `materialize_file` | `crates/livrarr-server/src/readarr_import_workflow.rs` | DONE — logic lives in the core as `materialize_hardlink_first` (livrarr-library) |
| `create_test_library_item` + sibling test helpers | `crates/livrarr-server/src/api_secondary_impl.rs` | NEW candidate — zero callers (LSP + text scan, 2026-07-04); test scaffolding, exempt from R6's forbidden clause, delete next sweep |
| `build_tag_metadata` / `read_cover_bytes` | `crates/livrarr-server/src/infra/import_pipeline.rs` | NEW candidates — dead after the R7 rewire; near-duplicate private fns already live in `tag_service.rs` |

## Wiki corrections queued (stale statements this mapping falsified)

- `overview.md` → canonical model lives at `docs/canonical-model.yaml`, not `architecture/canonical-model.yaml`.
- `library-management.md` — `confirm_scan` is NOT the import orchestration surface; manual import
  runs through `ImportService::import_single_file` (currently R7 DEBT).
- `grab-system.md` — missing Transmission (third client) and the automatic import-retry-with-backoff.
- `metadata-pathway.md` / `work-creation-pipeline.md` — the background convergence job now EXISTS
  (`jobs/convergence.rs`), disabled by default; "removed / zero production callers" is stale.
- `metadata-pathway.md` / `work-creation-pipeline.md` (again, 2026-07-04): convergence is now
  ENABLED by default (`1697bc7`) — the "disabled by default" wording above is itself stale.
- `library-management.md` / `import-pipeline.md` — describe the pre-consolidation manual/Readarr/
  scan pipelines; all file materialization now routes through `ImportWorkflow::import_file`
  (2026-07-04).
- `crates/domain.md:225` — still documents the deleted `ImportWorkflow::retry_import`.
