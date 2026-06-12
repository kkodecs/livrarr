---
feature: sprint-c-series
stage: spec
status: final
version: 11
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-010]
---

# Spec: sprint-c-series — Series reconcile

## 0a. Design Principles

- **Back-fill is the foundation.** All Series page improvements depend on works having
  `series_id` populated. Back-filling from `series_name` is the prerequisite for
  everything else.
- **Back-filled series are unmonitored by default.** The user opts in to monitoring;
  the system does not auto-monitor series it discovers from metadata.
- **No GR /search calls in the series path.** The anti-bot forbidden pattern
  (ST-012 from Sprint B contract) extends to series: all GR calls must use the
  autocomplete endpoint or the author's GR page; bare `/search?q=...` is banned.
- **Monitoring is a per-series, per-media-type setting** (ebook + audiobook toggles),
  not a binary.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | **[Baseline as of spec time; RESOLVED 2026-06-12 — the function is deleted per REQ-005.]** `fetch_series_from_book_search` (then at `series_query_service.rs:916-924`, read directly) constructed `format!("https://www.goodreads.com/search?q={}&search_type=books&page={}", ...)` and scraped series names from title patterns `(Series Name, #N)`. Produced `gr_key = ""` stubs only. | This was one of the two GR `/search` call sites in the series path (ST-005 is the complete scope authority) — the reason REQ-005 exists. | Reintroducing any such path — it is the ST-012 anti-bot pattern and produces unusable keys. | high |
| ST-002 | `sqlite_series.rs:158-186` (`link_work_to_series`), `work_service.rs:692-697` (create_work): `series_id` is written onto works in exactly two cases — (a) at create time when the work comes from the series monitor worker (`candidate.series_id != None`); (b) via `link_work_to_series` called by the monitor worker after creation. For all other add paths (manual import, search add, author monitor, enrichment), `series_id = None` even when `series_name` is populated. | No enrichment or refresh path writes `series_id`. | Assuming enrichment populates `series_id`. | high |
| ST-003 | **[Baseline as of spec time; the drop-behavior is RESOLVED 2026-06-12 — unmatched DB rows are now appended per REQ-003.]** Two distinct series-list roads (both read directly). Global `/series`: `list_enriched` (series_query_service.rs:40-103) iterates **DB rows only** (`list_all_series`) and counts `works_in_library` by FK (`works_by_series` keyed on `series_id`) — works carrying only an orphan `series_name` were invisible there because no DB row existed. Author page: `build_merged_series_list` is **cache-entry-driven** — a GR cache entry with no DB match still renders, with `works_in_library` falling back to `series_name` string-compare, while DB rows matching no cache entry were dropped (now appended). Read-only display logic on both roads — neither creates rows nor writes FKs. | Creating stub rows makes orphaned series appear on the global page automatically (it is DB-driven and already FK-counts); the author page additionally needed the unmatched-DB-row append (REQ-003, delivered). | Assuming one road's behavior describes the other (the global page never string-counts; the author page never FK-counts cache-only entries). | high |
| ST-004 | `migrations/023_series_monitoring.sql:3-14` + live PRAGMA (read directly): series unique key is `(user_id, author_id, gr_key)`; `gr_key` is `TEXT NOT NULL` with **no DEFAULT** (PRAGMA dflt_value=None). Any stub writer must therefore supply a gr_key value explicitly, and two stubs sharing `gr_key=""` under one author would collide. | Stub series (no real gr_key) require normalized name as the dedup key per author — a schema migration must establish that uniqueness (REQ-008); the column semantics for stub keys (empty string vs NULL-after-migration) are an implementation choice recorded at code time. | Inserting two stubs with same author_id and the same placeholder gr_key — the UPSERT will merge them silently. Assuming a schema DEFAULT exists. | high |
| ST-005 | **[Baseline as of spec time; RESOLVED 2026-06-12 — both sites removed per REQ-005; autocomplete is the only road.]** `resolve_gr_candidates` (then at :244-268, read directly): the primary path used the JSON autocomplete endpoint (WAF-free); when autocomplete returned empty it fell back to scraping `https://www.goodreads.com/search?q=<author_name>&search_type=authors`. The series path then had TWO `/search` call sites: that authors fallback and the books fallback in `fetch_series_from_book_search` (ST-001). | Both `/search` sites had to be removed for ST-012 compliance — the autocomplete path is compliant, the fallbacks were not. | Reintroducing either site; assuming any GR `/search` URL is reachable-safe — Sprint B's **sampled** ST-012 record (`spec-metadata-correctness.md` GR row: live 202-WAF observations, autocomplete cutover commits a21c643/33ba983) establishes that `/search` serves WAF interstitials, and that same record forbids fresh re-probes of GR `/search`. The evidence is that prior sample, not a new probe. | high |
| ST-006 | `frontend/src/pages/series/SeriesPage.tsx:36-131, 254-360` (read directly) + `MonitorSeriesRequest` (series.rs:227-238): monitoring is per-media-type — `monitor_ebook` + `monitor_audiobook` independent booleans; the existing UI renders "monitored" as (ebook OR audiobook), shows per-type chips, and the monitor action offers an Ebook / Audiobook / Both choice. | The toggle model for REQ-004 already exists and is per-media-type. | Inventing a new combined-toggle model. | high |
| ST-007 | `sqlite_series.rs:158-186` (`link_work_to_series`, read directly): the worker's link runs under an assignment guard — `UPDATE works SET series_id=... WHERE ... (series_id IS NULL OR (SELECT work_count FROM series WHERE id = works.series_id) > ?)` — implementing "most specific (fewest books) wins" (`wiki/domain/series.md`, Work Assignment). `work_count` is the **GR roster size**: written by the monitor worker as `all_books.len()` (`series_query_service.rs:583`), surfaced as `book_count` in `SeriesListView`, while `works_in_library` is computed dynamically from FKs (`list_enriched` :40-103). | `work_count` is authoritative GR data feeding cross-series arbitration; the in-library number is a separate, already-computed value. | Redefining `work_count` as a library count (breaks the guard). Stub rows whose `work_count` value lets them win the guard against GR-backed series. | high |
| ST-008 | `run_series_monitor_worker` (series_query_service.rs, read directly): the series roster comes from `https://www.goodreads.com/series/{gr_key}` pages — paged (`?page=N`, ≤10 pages, 1s sleep between), parsed by `parse_series_detail_html` (goodreads.rs:981) into `GoodreadsSeriesBook { title, gr_key, position: Option<f64>, year: Option<i32> }` (:935-942), then filtered to **primary works** (integer positions > 0) — the same filtered set whose length the worker writes as `work_count`. Works the worker creates carry `normalize_gr_key(&book.gr_key)` as their seed anchor, and worker dedup uses `livrarr_matching::work_dedup::find_matching_work` with the GR key + title/author. No roster is persisted anywhere today — every monitor run re-fetches. | The series pages are an already-trodden road (not `/search` — ST-012 untouched); primary-work filtering keeps roster display consistent with `book_count`; `find_matching_work` is the matching authority shared by linking and display. | Sourcing rosters from any GR `/search` URL. Persisting the unfiltered entry list (display would disagree with `book_count`). Inventing a second roster↔work matching algorithm. | high |

## 1. Problem Statement

The Series entity and FK exist and function correctly — but only for series explicitly
monitored by the user. Works enriched with `series_name` metadata never get `series_id`
populated (confirmed: 3 series visible in the current library with orphaned strings:
Dresden Files, Uplift Saga, Green Bone Saga).

Result: the Library → Series page is effectively a "Monitored Series" list, not a
true series catalog. Users who have books in a series don't see that series unless they
independently discover and add it via the monitor flow.

The fix: auto-create unmonitored series stubs from `series_name` metadata, link the
works via `series_id`, and redesign the Series page as a complete catalog with
per-series monitoring controls.

A secondary issue: the series path still carries two GR `/search` call sites (ST-005)
— the books-search fallback in `fetch_series_from_book_search` (which also produces
unusable empty-key stubs) and the authors-search HTML fallback in
`resolve_gr_candidates`. Both are the ST-012 anti-bot forbidden pattern. Remove both.

## 2. Requirements

- **REQ-001**: When a work with a non-empty `series_name` is created or has its
  `series_name` set/changed, the system ensures a series row exists for that
  (user, author, normalized series name) combination. If no series row exists, one is
  created as an unmonitored stub (`monitor_ebook=false`, `monitor_audiobook=false`,
  no gr_key). The work's `series_id` is then linked. Symmetrically: when a work's
  `series_name` is cleared, its `series_id` is unlinked (NULLed); when it changes to a
  different series, the work is relinked to the new series' row (created as a stub if
  absent). The unlink rule applies to EVERY path by which a work ceases to be linked —
  `series_name` cleared, `series_name` changed away, or the work itself deleted from
  the library. An **unmonitored stub** left with zero linked works after any unlink is
  deleted; a **monitored** series is never auto-deleted regardless of work count.
  A work whose `author_id` is NULL (author deleted — `works.author_id` is
  `ON DELETE SET NULL` while `series.author_id` is `NOT NULL`) is **skipped** by stub
  creation and linking: its `series_name` remains a display-only string, never an
  error. Such works heal through the recurring startup back-fill (REQ-002) once an
  author is assigned. (Grounding: no code path rewrites `works.author_id` after
  creation other than ON DELETE SET NULL — user edits bind the `author_name` string
  only, `update_work_user_fields` sqlite_work.rs:481-520 — so author reassignment is
  not a live trigger today; if such a door is added later, the recurring back-fill
  heals the links without further changes.)
  Link arbitration: an explicit **user edit** of `series_name` always relinks per the
  above. A **non-user** (enrichment/merge) `series_name` change on a work already
  linked to a GR-backed (non-stub) series updates the string only — it never moves
  the FK away from GR-grounded assignment (consistent with ST-002: enrichment has
  never written `series_id`). On stub-linked or unlinked works the ongoing path
  applies normally regardless of who wrote the change.

- **REQ-002**: At server startup (after migrations), a one-time back-fill pass runs
  over all works with `series_name != ""` and `series_id IS NULL`, applying the
  REQ-001 logic for each. Idempotent — safe to run on subsequent restarts.

- **REQ-003**: The Series list endpoint (`GET /series`) returns all series in the
  library — both explicitly monitored and unmonitored stubs — with their
  `works_in_library` count derived from the FK-linked works (not series_name
  string-compare). The same completeness applies to the author-scoped series list
  (`GET /author/{id}/series`): DB series rows (including stubs) that match no GR cache
  entry are appended to the merged list rather than dropped (today
  `build_merged_series_list` iterates cache entries only — ST-003), and any series
  with a DB row counts `works_in_library` by FK. This endpoint must also work in a
  **degraded mode when the author has no gr_key** (today it errors at
  `series_query_service.rs:286-292` before ever loading DB rows): the GR cache/fetch
  leg is skipped and the DB-backed series list (with FK counts) is still returned —
  a back-filled stub is never invisible on its author's page.

- **REQ-004**: The Series list page (`/series`) displays all series with:
  - Series name and linked author
  - `works_in_library` count
  - Monitored status — per-media-type ebook / audiobook controls (ST-006); any
    combined rendering is display-only aggregation, never a binary control
  - A way to start monitoring an unmonitored series (which triggers the GR series
    monitor worker to resolve the gr_key and discover new works)
  - Each row expands to list the series' FK-linked works with the existing per-work
    library-presence indication (files present vs not, per media type)

- **REQ-005**: BOTH GR `/search` call sites in the series path are removed
  (ST-005): (a) `fetch_series_from_book_search` (series_query_service.rs:916) —
  callers fall back to the gr_key-less stub path (series names already come from work
  metadata; synthesizing them from GR book search is redundant and anti-bot-risky);
  (b) the authors-search HTML fallback in `resolve_gr_candidates`
  (series_query_service.rs:250-268) — autocomplete becomes the only road; an empty
  autocomplete result surfaces as an honest "author not found on Goodreads" outcome
  to the caller, never a scrape.

- **REQ-006**: `work_count` keeps its existing meaning — the series' GR roster size,
  written by the monitor worker (ST-007) — and is NOT redefined or recomputed from
  the library. The in-library number shown anywhere is the FK-computed
  `works_in_library` (REQ-003); no new stored count is introduced. Stubs have no GR
  roster: a stub's displayed count is its FK-linked count, and a stub must never win
  the ST-007 assignment guard — a GR-backed series may claim a work away from a stub,
  a stub never displaces a GR-backed link. (As built: stubs store the sentinel
  `work_count = i32::MAX` — any real roster size beats it under the guard — masked to
  0 at the API boundary.)

- **REQ-007**: Work detail pages: when a work has no `series_name`, no series line
  is rendered (not a blank line). (#109 display nit.)

- **REQ-008**: Stub series for the same author with different names never collide or
  silently merge — distinct stubs per (user, author, normalized series name) must
  coexist. The plain placeholder scheme could not satisfy this (ST-004: the
  `UNIQUE(user_id, author_id, gr_key)` key merges same-placeholder stubs on upsert).
  As built: stubs carry `gr_key = "stub:" + normalize_for_matching(name)` — the
  existing unique key then yields per-name stub uniqueness with no schema change;
  real GR keys are numeric so the prefix cannot collide, and stub keys are masked to
  `""` at the API boundary. Promoting a stub to monitored (acquiring a real gr_key)
  preserves the row's identity: its id and all `series_id` work links survive the
  promotion (an in-place `gr_key` update).

- **REQ-009**: Promoting an unmonitored stub to monitored resolves its gr_key first,
  using only the compliant autocomplete road (REQ-005). If the stub's **author** has no
  gr_key (common for import-created authors — the series-list fetch hard-requires one,
  `series_query_service.rs:286-289`), the promotion flow first resolves the author via
  the existing author-candidate flow (`resolve_gr`); if the author cannot be resolved
  or the user cancels, the stub is left unmonitored and unchanged. With the author key
  in hand, the author's series list is fetched, and an exact normalized-name match
  adopts that series' gr_key onto the stub row, then the existing async monitor flow
  runs (202 + background worker, unchanged). If no match or multiple matches: the
  existing author-series picker is surfaced for the user to choose (or cancel — cancel
  leaves the stub unmonitored and unchanged). If the resolved gr_key already belongs to
  another series row for the same author, the stub is **merged into that row**: the
  stub's works relink to the existing row, the stub is deleted, and the requested
  monitoring flags are applied to the surviving row — never a constraint collision or
  a silent duplicate. Monitoring is never silently enabled without a resolved gr_key.

- **REQ-010** (PO amendment, 2026-06-12): The Series page expansion shows the series'
  **full roster** — every primary work of the series — not only the FK-linked library
  works. Because GR must not be hit repeatedly (PO directive), rosters are
  **persisted**: a `series_roster` store (one row per series: parsed entries JSON +
  `fetched_at`, FK CASCADE on series delete) is written (a) by the monitor worker as a
  write-through of the fetch it already performs on every run, and (b) once, on first
  expansion of a GR-backed series that has no stored roster (same pages, same parser —
  ST-008). Subsequent expansions serve from the store only. The read endpoint
  (`GET /series/{id}/books`) merges the stored roster with the series' FK-linked
  works — matched by normalized GR book key first, then by the shared work-matching
  authority (ST-008) — and returns each roster entry as either *in library* (work id +
  the existing presence indication data) or *not in library* (title/position/year
  only); linked works absent from the roster are appended. **Stubs resolve on
  expand** (PO amendment 2, 2026-06-12 — "show everything whether it's monitored or
  not"): a stub's first expansion attempts **silent identity resolution** — the
  REQ-009 exact-match road (author gr_key present, exactly one normalized-name match
  among the author's GR series, never a picker and never the author-resolution
  modal). On success the stub **adopts the gr_key and the real roster size in one
  step** (so the sentinel count never leaks once GR-backed), the roster is fetched
  and stored, and the full list is served — monitoring stays OFF. If the resolved key
  already belongs to another row for the same author, that row's roster is used for
  display only (no row merge outside promotion). On any resolution or fetch failure
  the expansion falls back to linked works with an honest "couldn't auto-match —
  monitor to pick the right series" hint, and the stub is left unchanged (no key
  adoption without a stored roster). This road is otherwise **display-only**: it
  never creates works, never writes `series_id`, and never changes monitoring (work
  creation stays solely with the monitor worker — M2 single creation gate).

  *Design (merge pseudocode, the risk concentration):*
  ```
  books(series):
    works = FK-linked works of series (+ library items)
    if series is stub: return { rosterAvailable: false, rows: works }
    roster = roster_store.get(series.id)
    if roster is none:
        entries = fetch+parse series pages (ST-008 road, primary filter)
        roster_store.put(series.id, entries)   # even when entries == []
        roster = entries
    by_key = { normalize_gr_key(w.gr_key) -> w  for linked works }
    rows = []
    for e in roster (position order):
        w = by_key[normalize_gr_key(e.gr_key)]
            else find_matching_work(works, e.title, author, gr_key=e.gr_key)
        rows.push(w ? InLibrary(e.position, w) : Missing(e.title, e.position, e.year))
        mark w used
    rows += unused linked works (appended, position order)
    return { rosterAvailable: true, rows }
  ```

## 3. UI/Interface Design

Series list page — all series, sorted by name:

```
[ Series name ]          [ Author ]     [ N books ]  [ ○ Monitor ]
The Dresden Files        Jim Butcher        17         ○ Not monitoring
The Green Bone Saga      Fonda Lee           5         ○ Not monitoring
The Uplift Saga          David Brin          6         ○ Not monitoring
The Wheel of Time        Robert Jordan      14         ● Monitoring
```

For a stub row the books cell shows the FK-linked library count (no GR roster is
known); GR-backed rows keep the existing display.

The monitor control reuses the existing per-media-type model (ST-006): the action
offers Ebook / Audiobook / Both; a row reads "monitored" when either flag is set and
shows per-type chips. Clicking the monitor control on an unmonitored stub runs the
REQ-009 promotion flow: silent gr_key adoption on an exact name match, the existing
author-series picker on no/ambiguous match (preceded by the author-candidate picker
when the author itself has no gr_key).

External Goodreads links for a series render only when the series has a real gr_key —
a stub shows no GR link (no dead `goodreads.com/series/` URLs).

Each series row is **expandable** — expanding reveals the series' **full roster**
(REQ-010), ordered by position: books in the library render as links with the same
library-presence indication the UI already uses elsewhere (files present per media
type vs. metadata-only / no files — no new indicator vocabulary); roster books not in
the library render muted (title, position, year) with no action in this sprint.
Stubs resolve silently on first expand (REQ-010) so the full list shows for
unmonitored series too; only a stub that cannot be auto-matched falls back to its
linked library works plus a "monitor to pick the right series" hint.

## 4. Non-Requirements

- Series detail page redesign — existing layout stays.
- Series discovery UI (the GR author-search modal) — unchanged.
- Foreign-language series deduplication (#112) — deferred. Stub dedup is by exact
  `series_name` per author for this sprint. Cross-language merging is future work.
- Changing `work_count` semantics — it remains the GR roster size maintained by the
  monitor worker (ST-007); the library-presence number is the computed
  `works_in_library`. No new stored count is introduced.
- Series position display / ordering within the detail page — unchanged.
- Bulk monitoring toggle — not in scope.
- `llm_scraper.rs:287` (GR `/search?q={query}` URL template) — considered and excluded:
  it belongs to the foreign-language discovery scraper configs (fr/de/es/…), not the
  series flow. Its ST-012 disposition is a foreign-language-sprint call. (Noted: since
  GR `/search` is 202-WAF'd, those configs likely return interstitials — flagged for
  that sprint, not changed here.)

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | When a stub series is promoted to monitored (user clicks Monitor), should the GR resolution run inline (blocking the toggle) or async (202 + background worker, current behavior)? | resolved | The gr_key resolution step (REQ-009) runs before monitoring is enabled (it gates the toggle); the discovery worker that follows keeps the existing async 202 behavior unchanged. |
| Q-002 | If `series_name` contains a positional suffix like `"The Wheel of Time, Book 3"` — should the back-fill normalize it to `"The Wheel of Time"` before creating the stub? | resolved | Yes — strip `, Book N` / `, #N` suffixes during stub creation (same normalization as the matching layer). The work is updated coherently: its `series_name` is rewritten to the clean form, and the extracted number populates `series_position` **only when the work has none** (an existing position is never clobbered). |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Add a work via manual import with `series_name = "Test Series"` and no existing series row for that author — a series stub is created and the work's `series_id` is non-null after import.
- [ ] **AC-002** (REQ-001): Adding a second work in the same series links it to the existing stub (no duplicate series row created).
- [ ] **AC-003** (REQ-002): With pre-existing works that have `series_name != ""` and `series_id IS NULL`, a server restart creates stubs and links all such works. Subsequent restart does not create duplicate rows.
- [ ] **AC-004** (REQ-002/REQ-003): Fixture-driven — seed works carrying orphan `series_name` values (`series_id IS NULL`, ≥2 distinct series across ≥2 authors), run the back-fill, and `GET /series` returns a row per seeded series with correct author attribution. (Manual corroboration on the live library: Dresden Files, Green Bone Saga, and Uplift Saga appear — previously invisible.)
- [ ] **AC-005** (REQ-003): `works_in_library` for each series matches the actual FK-linked work count, not a string-compare estimate.
- [ ] **AC-006** (REQ-004): Series page lists monitored and unmonitored series. An unmonitored stub row reads "Not monitored"; its monitor control offers the existing Ebook / Audiobook / Both choice (ST-006); after promotion the row shows the chosen per-type chips.
- [ ] **AC-007** (REQ-004/REQ-009): Promoting a stub whose name exactly matches one author series adopts that gr_key and starts the existing background worker (202); the stub's row id and work links are unchanged after promotion.
- [ ] **AC-008** (REQ-005): No HTTP request to `goodreads.com/search` (any `search_type`) is made by the series path under any condition — including autocomplete-empty fallback scenarios.
- [ ] **AC-009** (REQ-006): Linking or unlinking a work changes `works_in_library` on the list endpoints while the series row's stored `work_count` (GR roster size) is untouched by the link operation.
- [ ] **AC-010** (REQ-007): A work detail page for a work with no `series_name` renders no series line in the metadata section.
- [ ] **AC-011** (REQ-008): Creating stubs for two different series under the same author yields two distinct rows (no silent merge); promoting one to monitored leaves the other intact.
- [ ] **AC-012** (REQ-001): Clearing a work's `series_name` NULLs its `series_id`; deleting the work has the same unlink effect; in either case, if that was the stub's last linked work and the stub is unmonitored, the stub row is deleted; a monitored series in the same situation survives at count 0.
- [ ] **AC-013** (REQ-009): Promoting a stub with no exact name match among the author's series surfaces the picker; cancelling leaves the stub unmonitored with no gr_key and monitoring flags unchanged.
- [ ] **AC-014** (REQ-009): Promoting a stub whose author has no gr_key surfaces the author-candidate flow first; cancelling (or a failed author resolution) leaves the stub unmonitored and unchanged.
- [ ] **AC-015** (REQ-003): `GET /author/{id}/series` includes a DB stub that matches no GR cache entry (e.g., GR fetch returns nothing for the author), with FK-derived `works_in_library`.
- [ ] **AC-016** (REQ-003): `GET /author/{id}/series` for an author with NO gr_key returns the author's DB-backed series (stubs included, FK counts) instead of the current "Author has no Goodreads key" error.
- [ ] **AC-017** (REQ-001/Q-002): Back-filling, creating, or **updating** a work whose `series_name` is (or becomes) `"X, Book 3"` yields a stub named `"X"`, rewrites the work's `series_name` to `"X"`, and sets `series_position = 3` when the work had none; a work with an existing `series_position` keeps it.
- [ ] **AC-018** (REQ-009): Promoting a stub whose name resolves to a gr_key already held by another series row for the same author merges the stub into that row — the stub's works end up FK-linked to the surviving row, the stub row is gone, and the surviving row carries the requested monitoring flags.
- [ ] **AC-019** (REQ-001/REQ-002): A work with `series_name` set but `author_id` NULL is skipped by both the ongoing path and the startup back-fill — no stub created, no error, `series_name` still displayed; after an author is assigned, the next startup back-fill creates the stub and links the work.
- [ ] **AC-020** (REQ-004/REQ-010): Expanding a GR-backed series lists the full stored roster in position order — in-library entries as links with the standard presence indication, not-in-library entries muted with title/position — and library-linked works missing from the roster are appended, never dropped.
- [ ] **AC-022** (REQ-010): The first expansion of a GR-backed series with no stored roster performs exactly one GR series-page fetch sequence and persists the result; a second expansion (and any later one) serves entirely from the store with zero GR requests — including the case where the fetch parsed zero entries.
- [ ] **AC-023** (REQ-010): A monitor-worker run persists the roster it fetched (write-through); a subsequent expansion of that series triggers no GR fetch.
- [ ] **AC-024** (REQ-010): Expanding a stub whose name exact-matches one of its author's GR series silently adopts that gr_key (monitoring still off, sane `work_count` written with it), stores the roster, and serves the full list; expanding a stub that cannot be silently resolved (no author key, no/ambiguous match, or fetch failure) returns its FK-linked works flagged `rosterAvailable: false` with the can't-auto-match hint and leaves the stub row unchanged. Deleting a series removes its stored roster (FK cascade).
- [ ] **AC-021** (REQ-006/REQ-001): A work linked to a GR-backed series is never displaced by stub machinery — an enrichment-driven `series_name` change updates the string but keeps `series_id`; conversely the monitor worker CAN claim a stub-linked work (the stub never wins the ST-007 guard against a GR-backed series).
