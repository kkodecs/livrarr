# roads.md — adversarial verification record (2026-07-04)

Backing record for the verification claim in `wiki/architecture/roads.md` ("adversarially
verified the same day by cross-family review"). Session: the roads-map authoring session
(metadata-remediation, 2026-07-04). Raw CLI transcripts were session-scratchpad files
(not retained); this record preserves the verdicts and the source-verification trail.

## Setup

Both reviewers (Gemini `gemini-3.5-flash`, Codex `gpt-5.5`) received the draft map plus an
explicit refutation brief: re-verify the "three parallel import implementations" finding
independently (Direction A), and mechanically attack the completeness claim "everything else
is fine" (Direction B) by enumerating call sites — grab creation (`CreateGrabDbRequest` /
`upsert_grab`), LibraryItem creation (`CreateLibraryItemDbRequest` / `create_library_item`),
tag writes (`livrarr_tagwrite::write_tags`), enrichment writes (`apply_enrichment_merge`,
`update_work_enrichment`, enrichable-column writes), work creation outside the seed factory,
and spot-checks of the "Not roads" single-door list.

## Verdicts

| Question | Gemini | Codex |
|---|---|---|
| Three parallel import roads real? | CONFIRMED | CONFIRMED (with scope caveat: not the full files→LibraryItem universe) |
| "Everything else is fine"? | CONFIRMED ("exactly 3 LibraryItem sites, no fourth door") | **REFUTED** (two counterexamples below) |

## Codex refutations — both verified at source by the authoring session before folding in

1. **Fourth file→LibraryItem creation door: scan adoption.**
   `root_folder::scan` creates LibraryItem rows directly in the handler via
   `ImportIoService::create_library_item` → `db.create_library_item`, never touching
   `ImportWorkflow`. Verified: `crates/livrarr-handlers/src/root_folder.rs:265-275`
   (handler call site), `crates/livrarr-server/src/import_io_service.rs:135-153`
   (straight DB delegation). → roads.md R9 re-marked DEBT; creation universe corrected to 4 sites.
2. **User cover doors bypass the cover write gate.**
   `LiveCoverService::select_cover` calls `livrarr_materialize::download_cover_to_disk`
   directly; `::upload_cover` does its own tmp-write-then-rename. Verified:
   `crates/livrarr-server/src/cover_service.rs:220-230` (direct download), `:342-355`
   (direct tmp/rename). The write gate (`run_cover_write_gate`) is used only by enrichment
   materialization. → roads.md R3 re-marked DEBT (fork); invariant restated as intent.

False alarm cleared: Codex's session log showed `create_work`/`create_library_item` hits in
`api_secondary_impl.rs`; verified test-support only ("Secondary API implementations for
testing", `create_test_library_item` helpers, `crates/livrarr-server/src/api_secondary_impl.rs:1-3,883`),
correctly excluded from both verdicts.

Gemini unique find: `ReadarrImportService::update_work_enrichment` — an enrichment-write
wrapper it reports as caller-less (dead-code candidate; standing R2 bypass surface).
Single-family claim, not independently re-verified (caller grep was sandbox-denied) —
recorded in roads.md dead-code table as "verify callers before delete".

## What held under attack (both families)

Grab creation (sole chokepoint `ReleaseServiceImpl::grab` → `upsert_grab`,
`crates/livrarr-download/src/release_service.rs:351-367`); enrichment writes (merge chain
only); work creation (seed factory only); tag writes (TagService/materialize + convergence
+ R7's already-flagged inline call); "Not roads" spot-checks (work update, workfile delete,
queue remove — one routed door each).

## Family asymmetry note

Gemini confirmed the exact claim Codex disproved ("exactly 3 sites"). Consistent with the
recorded pattern that the agreeing family masks; the refutation brief is what surfaced the
counterexamples. Keep adversarial framing for future map verifications.

## Disposition

All corrections folded into `wiki/architecture/roads.md` same day (summary line: 9 CLEAN /
4 DEBT / 1 DECISION, 4-site creation universe). Earlier same-session verifications also on
record there: convergence job exists but ships disabled (`crates/livrarr-server/src/config.rs:149`,
registration `jobs/mod.rs:128-134`), `refresh_all` loops canonical refresh
(`crates/livrarr-handlers/src/work.rs:714-721`), canonical model lives at
`docs/canonical-model.yaml`.
