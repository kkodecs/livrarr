# Design note: sprint-d-seeds-doors

One-page architecture note (trimmed-middle process, PO 2026-06-12) — replaces IR
v1/contract/IR v2 for this feature. Settles the structural decisions; everything
behavioral is governed by `spec-sprint-d-seeds-doors.md` (v3, final). All file:line
citations below were read directly on 2026-06-12.

## D1 — SeedBuilder home and shape

New module **`crates/livrarr-domain/src/seed.rs`**. Domain is the only legal home:
the six construction sites span `livrarr-handlers` (compile-walled from
db/metadata/tagwrite/download), `livrarr-metadata`, and `livrarr-server`; all three
already depend on `livrarr-domain`, which depends on nothing — no new seam edges.

Shape: **one constructor function per door** (`seed_add_box`, `seed_manual_import`,
`seed_list_import`, `seed_author_monitor`, `seed_series_monitor`,
`seed_readarr_import`), each taking domain-level primitives (title, author, the
door's language input per D2, anchors/identity, door-specific options) and returning
`WorkCandidate`. The builder owns: `normalize_language` + the single `"en"`
last-resort default, and writes the SAME language value into `WorkSeedFields.language`
and the door's identity-harvest input (`RawHarvest.language` /
`CapturedIdentity.language`) — REQ-001(b)'s coherence guarantee, closing the ST-004
incoherence. Doors keep computing their own `IdentityState` (resolver results,
Pending, monitor's Confirmed) and pass it in — the builder unifies field assembly,
not identity policy.

**Enforceable line:** `WorkSeedFields {` / `WorkCandidate {` literals are legal only
in `seed.rs` and `#[cfg(test)]` code (AC-001; grep-checkable at review). Types
verified at `livrarr-domain/src/identity.rs:455-486`.

## D2 — Language settings storage + NULL semantics

**Migration 063** (next free; `crates/livrarr-db/migrations/` ends at 062):
`ALTER TABLE authors ADD COLUMN monitor_language TEXT` and
`ALTER TABLE series ADD COLUMN monitor_language TEXT`, both NULL-able, no backfill. (`Author` verified language-less at `livrarr-domain/src/lib.rs:455-468`.)

**NULL semantics (r1 fix — spec-conformant, no amendment):** NULL means "user never
chose" → at the construction point the builder's `"en"` last-resort fires, exactly
as REQ-002 specifies. The smart-default rule (Q-001) exists as the **UI pre-fill**
on the rich surfaces. The name is `monitor_language` (it governs monitor-created
works), not a general preference.

**Backend monitor-enable guarantee (code-r3/r4 rework — supersedes the per-surface
threading):** "a monitored author/series never carries a NULL `monitor_language`"
is enforced **structurally at the DB write chokepoints**, not per UI surface
(reviewers found 4 monitor-enabling surfaces; threading each is fragile). When a
write leaves the row monitored with a NULL language, the chokepoint persists the
smart default = `seed::dominant_language(entity's works)` else `"en"`:
- `update_author`: resolve `chosen` (explicit set/clear wins, else preserve), then
  `monitored && chosen.is_none() → default`. **Airtight (PO 2026-06-13):** this
  fires for BOTH an unset field AND an explicit clear (`Some(None)`) — closes the
  r4 R-5 hole.
- `update_series_flags` + `upsert_series`: post-write `ensure_monitored_series_language`
  heal (monitored && NULL → default). The series request can't express an explicit
  clear, so the heal is sufficient.
- `seed::dominant_language` is the one shared rule (unique-max, tie/empty → None);
  `series_query_service.suggested_language` and the frontend pre-fills mirror it.
Bare UI toggles (AuthorsPage list, SeriesDetailPage) send no language and rely on
this guarantee. Verified no other writer sets `monitored`/`monitor_ebook/audiobook`
true (`create_author` inserts unmonitored; the gr_key-adoption UPDATE touches no
flags).

## D3 — Threading through the series monitor / promotion flow

`MonitorSeriesRequest` (handlers) and `MonitorSeriesServiceRequest`
(`livrarr-domain/src/services/series.rs:130-134`, verified: gr_key + 2 flags today)
gain `language: Option<String>`. The promote road (`POST /series/{id}/promote`,
sprint-c REQ-009) gains the same field; the frontend holds the choice in
row-level state so it rides every step (author resolution → picker → completion).
**As built (code r1):** the monitor/promote completion persists via the
`upsert_series` road (`monitor_language = COALESCE(excluded, existing)`); the
**change road for an already-monitored series** is `update_series_flags`, extended
with `monitor_language: Option<String>` (COALESCE; `None` = flag-only toggle never
clears it), carried by `UpdateSeriesRequest.language` on `PUT /series/{id}` and a
persisted-truth selector on monitored rows. Cancel at any step persists nothing
(spec AC-013). The sprint-c silent-resolution road is untouched.

## D4 — List import language rides confirm; the preview is transient (r2 fix, verified)

The entire list-import flow is transient frontend state: `ListImportPage.tsx:33-39`
holds phase/preview/selection in `useState`, with no preview_id in the URL — a page
refresh drops the WHOLE preview (the user re-uploads), not just the language
(verified by direct read; Gemini r2 R-3). The r1 design (preview column + setter +
restore-on-mount) was premised on a refresh-survivable preview that does not exist,
and is reverted. Final design: `ListService::confirm`
(`livrarr-domain/src/services/list.rs:127-133`) gains `language: Option<String>`;
the frontend passes the selector value at confirm; `None` → builder's `"en"`
(D2-consistent). Refresh semantics are coherent: list and language are lost
together — no silent wrong-language confirm path exists, so the project's
restore-on-mount rule is not in play. **Future note:** if list import is ever made
resumable (preview_id in URL + server-side restore), the language choice moves into
preview storage as part of that redesign.

## D5 — Quality screen placement (#53)

Pure function in `livrarr-metadata` next to the workflow (`screen.rs` or in
`author_monitor_workflow.rs`): `screen_entry(&OlWorkEntry, author_name) ->
Option<JunkClass>`, applied inside the existing eligibility filter (before the
auto-add/notification fork). `OlWorkEntry` (verified 3 fields,
`author_monitor_workflow.rs:19-26`) gains capture of the `authors` array — count
only, threshold 6 as a named constant (spec REQ-004a). `MonitorReport`
(`livrarr-domain/src/services/monitor.rs:6-11`, verified) gains
`entries_screened: usize` (behavioral stubs updated to match). Unit fixtures = the
ST-007 verbatim titles, as raw OL JSON (spec AC-008/009).

## D6 — F8 deletion set

Delete `crates/livrarr-metadata/src/llm_scraper.rs` (380 lines) + its `pub mod`
declaration (metadata `lib.rs:19`) + the `MetadataProvider` trait (`lib.rs:56-66`) +
the `#[cfg(test)]` impl on `OpenLibraryProvider` (`lib.rs:341-342`).
`ProviderSearchResult` / `ProviderAuthorResult` / `ProviderWorkDetail` are deleted
**iff** the post-trait reference trace shows them orphaned (checked at code time
with `find_referencing_symbols`); if live lookup code uses them, they stay (spec
REQ-006).

## D7 — work_service.rs shrink (REQ-007)

The builder lands in domain (not work_service.rs), and the seed-adjacent free
functions in work_service.rs (`iso639_1_to_3`, `lookup_term_to_seed`,
`seed_carries_identifier`) move to `seed.rs` where dependency-clean — if any is
entangled with metadata-crate types it stays, and the shrink comes from the other
moves; AC-012 enforces the outcome (< 3,616 lines), not a specific move list.

## D8 — Frontend

No language selector exists in `frontend/src/components/` (survey, confirmed
absence). One new `LanguageSelect` component (shadcn Select primitive, normalized
ISO codes + display names), used at the three surfaces: ListImportPage, the author
monitor settings (AuthorDetailPage), the series monitor action (SeriesPage
`SeriesRow`).

## Per-door wiring table (door → builder → language)

| Door | Construction site today (verified) | Language input to builder | Identity input |
|---|---|---|---|
| Add-box search add | `livrarr-handlers/src/work.rs:234` | `req.language` (provider lookup result) | resolver output (`resolve_identity`, harvest carries same language) |
| GR-link add | same handler (`work.rs:234`) | `req.language` (usually absent → builder default) | resolver output |
| Manual import | `livrarr-handlers/src/manual_import.rs:1101` | file `dc:language` → picked candidate language | resolver output |
| List import | `livrarr-metadata/src/list_service.rs:121` | confirm-time user choice (D4); None → builder "en" | row-derived identity (unchanged) — harvest language now matches seed (ST-004 fix) |
| Author monitor | `livrarr-metadata/src/author_monitor_workflow.rs:321` | `authors.monitor_language`; NULL → None → builder "en" (D2) | pre-stamped Confirmed w/ ol_key (unchanged) |
| Series monitor | `livrarr-metadata/src/series_query_service.rs:779` | `series.monitor_language`; NULL → None → builder "en" (D2) | Pending + `seed_anchors` pass-through **unchanged** (spec ST-008 — Sprint E owns convergence) |
| Readarr import | `livrarr-server/src/readarr_import_workflow.rs:1237` | `edition.language` | seeded CapturedIdentity (language now coherent) |

## Canonical model statement

No new entities, no seam changes: the builder adds zero dependency edges
(`livrarr-domain` depends on nothing; all constructing crates already depend on it).
`Author`/`Series` gain a persisted field — spine entities are concepts; fields are
not spine-gated. **No amendment row required.** The deletion (D6) removes
unsanctioned pub surface, moving surface_sanction_ratio the right direction.

## Riders for the combined code review

- ST-008 pass-through honored at the series door (no convergence semantics change).
- `UpdateAuthorRequest` does not carry `monitor_since` (set server-side) — the author
  `monitor_language` rides the author-update road like the other monitor settings;
  confirm at code time which request surface the settings UI actually uses.
- Behavioral stubs (`livrarr-behavioral/src/stubs.rs`) updated for the
  `MonitorReport` field, the `ListService::confirm` signature change, and any other
  trait-signature changes.
- List-import refresh drops the whole preview by design (transient flow, verified
  D4) — no restore path this sprint; the language selector simply starts fresh with
  the re-upload.
