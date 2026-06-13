---
feature: sprint-d-seeds-doors
stage: spec
status: final
version: 3
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007]
---

# Spec: sprint-d-seeds-doors — Seeds & doors (F2) + quality screen (#53) + cleanup (F8)

## 0a. Design Principles

- **One construction point for work seeds.** Every door builds its `WorkCandidate`
  through a single shared construction point; per-door language sourcing is declared
  there as data, never inline at a door. The seed fields and the identity harvest of
  a candidate always carry the same language value.
- **Language is never silently invented.** A seed's language comes from the door's
  source data or from an explicit user choice. `"en"` appears only as the documented
  last-resort default, applied in exactly one place (the construction point).
- **The monitor adds only things that look like real books.** The quality screen runs
  before anything is created — rejects produce neither a work nor a notification.
  Rejects are silent in the UI (PO disposition, 2026-06-12) but logged and counted.
- **Behavior-preserving cleanup.** F8 deletions and any `work_service.rs`
  restructuring change no observable behavior. Unreachable code is deleted, not
  preserved — git history is the archive.
- **ST-012 continuity.** No GR `/search` anywhere; this sprint deletes the last
  latent `/search` URL template in the tree.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | Door survey, all sites read directly 2026-06-12. Seven work-creation doors exist: Add-box/GR-link (one shared handler, `work.rs:234`), list import (`list_service.rs:121`), author monitor (`author_monitor_workflow.rs:321`), series-monitor worker (`series_query_service.rs:779`), Readarr import (`readarr_import_workflow.rs:1237`), manual import (`manual_import.rs:1101`). Every door without exception enters via `WorkService::add` (`work_service.rs:471`), and every add outcome routes through `ensure_identity_and_enrichment` (`work_service.rs:2695`, doc :2686-2694). | The funnel is already one road; this sprint changes how seeds are BUILT, not where they GO. `add()` already runs `clean_title` and rejects empty titles (:478-481). | Inventing a new add road; assuming any door bypasses `add()`. | high |
| ST-002 | Read directly: `list_service.rs:125`, `author_monitor_workflow.rs:325`, `series_query_service.rs:783` each hardcode `language: "en".into()`. Their source payloads carry **no language field at all**: `ListImportPreviewRow` = title/author/isbn_13/isbn_10/goodreads_book_id/year (`livrarr-db/src/lib.rs:1839-1846`); `OlWorkEntry` = key/title/first_publish_date (`author_monitor_workflow.rs:19-26`); `GoodreadsSeriesBook` = title/gr_key/position/year (sprint-c ST-008, `goodreads.rs:935-942`). | Exactly 3 doors hardcode `"en"`, and for them "derive from the source" is impossible as a field read — language must come from user choice or library context. | Speccing language derivation from a source field on these doors (the field does not exist). | high |
| ST-003 | Read directly. The other doors already derive: Add-box/GR-link `req.language` → `normalize_language` → `"en"` fallback (`work.rs:179-183`, also threaded into the identity `RawHarvest` :211); Readarr `edition.language` (`readarr_import_workflow.rs:1133-1136`, fallback :1241, also in `CapturedIdentity` :1213); manual import file `dc:language` → picked candidate language → `"en"` (`manual_import.rs:1072-1076`, into `RawHarvest` :1090). | The deriving doors need no new UI — their `"en"` is a fallback, not a stamp. | Adding language pickers to doors that already have a real signal. | high |
| ST-004 | Read directly: the list door's seed says `"en"` (`list_service.rs:125`) while the same candidate's identity `CapturedIdentity` carries `language: None` (`list_service.rs:112`). | Seed fields and identity harvest can disagree today; consistency is a new guarantee this sprint introduces, not a property to preserve. | Assuming one language value flows coherently through a candidate today. | high |
| ST-005 | `author_monitor_workflow.rs`, read directly. Eligibility today = has OL key (:275) + not already known (:276-281) + parseable year (:282-292) + year ≥ monitor_since (:293). No quality screen of any kind. A title-less entry becomes a work titled "Unknown" (:296). Identity is pre-stamped `Confirmed` at construction (:334) with the OL key as anchor — no resolver pass. `monitor_new_items=false` produces notification-only entries (insight 42). | Everything OL lists with a date lands as a Confirmed work (or a notification); junk and real books are indistinguishable to the system. | Assuming any existing filter catches junk; assuming monitor adds pass identity resolution. | high |
| ST-006 | Serena full reference traces + direct reads, 2026-06-12. `MetadataProvider` trait (`livrarr-metadata/src/lib.rs:56-66`) has exactly two impls: `LlmScraperProvider` (`llm_scraper.rs:252`) and a `#[cfg(test)]` impl (`lib.rs:341-342`). `LlmScraperProvider` has **zero construction sites outside its own file** (its `new` :87 has no external callers); `build_llm_scraper_configs` (:285) is called only by its own unit test (:358); doc-comment :284 — "Deferred until render proxy is available". The module contains a GR `/search?q={query}` URL template (:286) — the ST-012-banned pattern, never executed. | The entire scraper road is unreachable from production; deleting it changes no behavior. The roadmap's "zero impls" claim was wrong in letter, right in effect. | Treating the trait as live surface; wiring anything to the scraper "since it's there"; keeping the `/search` template. | high |
| ST-007 | **SAMPLED 2026-06-12**: one live `authors/OL5152266A/works.json` fetch (Jim Butcher) on the app's own UA (`KkodecsBookBot/0.1.0`, `livrarr-http/src/lib.rs:162-166`) — the exact feed the monitor reads. 130 works, first 100 sampled: **18 junk / ~72 real / ~10 ambiguous**. Junk anatomy: **multi-author anthologies dominate** (e.g. "Blood Lite" 23 authors, "Dangerous Women" 23, "Urban Enemies" 6 — every observed junk anthology ≥6 credited authors; max observed on a clean work: 5); the entries' `authors` array is in the SAME payload we already fetch and currently ignore (`OlWorkEntry`, ST-002). Bundle keywords observed verbatim: "…Omnibus Volume 1/2", "Jim Butcher Box Set", "…Series 5 Books Collection Set", "…Books 1-4". Self-titled bundles: "Jim Butcher Set". Whitespace-corrupted titles: `"Ghost Story\n…\nDresden Files"`. **NOT observed**: any "Summary of"/"Study Guide" entry (0/100 — second-author sample pending, Q-004). Uncatchable by title text: novel-titled anthologies (author-count only), foreign-language edition duplicates (real works, not junk), retailer/signed edition variants. The live library held zero `auto_added` provenance rows at sample time (cleared 2026-06-12) — the OL feed is the grounding. **Second sample, same UA/road: Orwell `OL118077A` + Austen `OL21594A`, 100 entries each.** Summary/study-guide class: exactly **1/200** ("1984 SparkNotes Literature Guide" — the Q-004 exemplar); anthologies 0; **author-name-in-title occurs on REAL works** ("Persuasion by Jane Austen", "Mansfield Park (Jane Austen Novels Book 5)") — bare name-containment is NOT a junk signal; bundle form "Jane Austen Collection Volume 1 : Three Books in One" observed; cross-author bind-ups (~4, e.g. "Lorna Doone, Pride and Prejudice") remain residual/unscreened. | Every REQ-004 screen class carries ≥1 verbatim observed exemplar; anthology junk is detectable only via the author-count field, not title text. | Shipping an unsampled pattern class as fact; claiming title patterns suffice (the dominant class needs author count); screening on bare author-name containment (observed on real works). | high (three authors, n=300; the claim is the classes, not their frequencies) |
| ST-009 | Across all three sampled authors: **96/100 (Butcher), 100/100 (Orwell), 100/100 (Austen) entries carry no `first_publish_date`**, and the works endpoint is paged at `limit=100` (insight 43) — Butcher's 130 works truncate to 100 today. Today's eligibility (ST-005) already drops every date-less entry. | The screen's live exposure is bounded to date-bearing entries; the date requirement is doing most of the accidental junk suppression today. | Using date-absence as a junk signal (it is near-universal, not discriminative). Touching the date requirement or the page cap this sprint — that is monitor redesign, out of scope. | high |
| ST-008 | `series_query_service.rs:792-797`, read directly: worker-created works seed `IdentityState::Pending` with the comment "the background resolver converges it" — that resolver no longer exists (`enrichment_retry_tick` removed; `retry_all_incomplete` is user-triggered only). The #144-remainder row on Sprint E owns this, including a pending PO decision. | Sprint D must pass `seed_anchors` through the new construction point unchanged; the dead-convergence semantics are NOT this sprint's to fix. | Absorbing Sprint E scope (background convergence, anchor-completion unification) into the seed refactor. | high |

## 1. Problem Statement

Three of the seven work-creation doors stamp every new work `language="en"` no matter
what it is — list import, author monitor, series monitor (ST-002). A French author's
new release, a Korean series' missing volumes, a Spanish CSV import: all land as
"English" works, which mis-routes metadata providers (the applicability rule
dispatches by `works.language`) and produces the wrong-language metadata class the
project has been fighting since F1. The doors can't read a language field — their
sources don't have one — so the fix is to ask the user at the three setup surfaces,
with a smart default, and to centralize seed construction so no door can invent a
language silently again (today each door assembles its own seed by hand, and one of
them disagrees with itself — ST-004).

Separately (#53), the author monitor adds whatever OpenLibrary lists under an author.
The sampled feed (ST-007) shows what that means in practice: multi-author
anthologies, omnibus volumes, box sets, self-titled publisher bundles, and
whitespace-corrupted titles (the anatomy behind the #53 Dead Beat report) — all
pre-stamped identity-Confirmed (ST-005). There is no quality screen at all.

And the metadata crate still carries a never-wired LLM web-scraper road — including a
banned GR `/search` URL template — that confuses every reader and reviewer who finds
it (this sprint's own roadmap row got its facts wrong, which proves the point). It is
unreachable from production (ST-006) and goes away.

## 2. Requirements

- **REQ-001**: All seven work-creation doors construct their `WorkCandidate` through
  one shared construction point (the roadmap's "SeedBuilder"). No door assembles
  `WorkSeedFields` directly. The construction point guarantees: (a) the language
  sourcing rule for each door is declared in one place, as data; (b) the candidate's
  seed fields and its identity harvest (`RawHarvest` / `CapturedIdentity`) carry the
  same language value; (c) the `"en"` last-resort default is applied here and only
  here. Existing non-language behavior of every door (anchors, `seed_anchors`
  pass-through per ST-008, provenance, identity state) is preserved unchanged.

- **REQ-002**: Per-door language sourcing:
  - Add-box / GR-link: the request's language (provider lookup result), normalized —
    current behavior, now routed through the construction point.
  - Readarr import: `edition.language` — current behavior, routed through.
  - Manual import: file `dc:language`, then the picked candidate's language —
    current behavior, routed through.
  - List import: the import's user-chosen language (REQ-003).
  - Author monitor: the author's persisted monitor language (REQ-003).
  - Series monitor: the series' persisted monitor language (REQ-003).
  In every case, a missing/empty source value falls back to `"en"` at the
  construction point.

- **REQ-003**: The three signal-less doors gain a user-facing language choice with a
  smart default ("ask the user", PO 2026-06-12):
  - **List import**: the preview screen gets a language selector applying to the
    whole import (every work its confirm creates). Default `"en"`.
  - **Author monitor**: enabling/configuring monitoring on an author offers a
    language choice, persisted on the author. Default = the dominant language among
    the author's existing library works; `"en"` when the author has none.
  - **Series monitor**: the existing monitor action (Ebook/Audiobook/Both — sprint-c
    ST-006) gains a language choice, persisted on the series. Default = the dominant
    language among the series' FK-linked works; `"en"` when there are none. The
    monitor/promotion road today carries only `gr_key` + the two monitor flags
    (`MonitorSeriesServiceRequest`, series.rs:242-246, read directly) — that request
    surface gains the language field. For a stub, promotion is a multi-step
    always-200 flow (author resolution → series picker — sprint-c REQ-009): the
    language chosen at the initial monitor action rides every step of that flow and
    is persisted by whichever step completes monitoring; cancelling at any step
    persists nothing. The sprint-c silent-resolution road (stub expand) is
    display-only and carries no language choice.
  The persisted setting governs works added by that monitor from then on. Changing
  it later affects only future adds — already-created works are never re-stamped.

- **REQ-004**: The author monitor applies a deterministic quality screen to every
  bibliography entry at eligibility time, BEFORE the auto-add/notification fork: a
  screened entry produces neither a work nor a notification. Screen classes (each
  grounded by an ST-007 verbatim exemplar):
  - (a) **Multi-author anthology** — the entry is credited to **≥ 6** authors.
    The threshold is pinned at 6 from the sample (every observed junk anthology had
    ≥6 credited authors, the max observed on a clean work is 5 — ST-007) and is a
    named constant, not a tunable. The `authors` array is in the HTTP response
    the monitor already fetches but is NOT currently deserialized (`OlWorkEntry`
    keeps three fields — ST-002/ST-005): this requirement includes extending the
    monitor's deserialization to capture it; zero new network requests. Counting
    rule: the length of the entry's `authors` array; a missing or empty array
    counts as 1 (this class never fires on it).
  - (b) **Bundle keywords** — title contains "Omnibus", "Box Set" / "Boxed Set",
    "Collection Set", "Series Set", "Books in One", or a "Books N–M" range form.
  - (c) **Self-titled bundle** — deterministic rule: normalize the title (lowercase,
    punctuation stripped), then remove the monitored author's normalized name, the
    bundle vocabulary (set, box set, collection, books, novels, omnibus, volume),
    connective stopwords (by, the, of, a, and), and digits; if nothing remains,
    screen the entry. "Jim Butcher Set" → remainder empty → screened. "Persuasion
    by Jane Austen" → remainder "persuasion" → passes (real work, observed —
    ST-007). "Definitive Jane Austen" → remainder "definitive" → passes (accepted
    residual).
  - (d) **Malformed title** — embedded newline or collapsed multi-space runs.
    Malformed entries are screened, not repaired — no guessing a plausible title
    (the #53 Dead Beat lesson).
  - (e) **Summary / study-guide keywords** — grounded by the sampled exemplar
    "1984 SparkNotes Literature Guide" (rare: 1 in 300 sampled entries, but real).
    High-precision keywords only: "Summary of", "Study Guide", "SparkNotes",
    "CliffsNotes", "Workbook", "Quotes from". "Notes on" and "Analysis of" are
    deliberately EXCLUDED — they match real literary titles ("Notes on a Scandal").
  Rejection is silent in the UI (PO disposition) but each reject is debug-logged
  with entry title + matched class, and the monitor's run report carries a screened
  count. The screen uses no network and no LLM.

- **REQ-005**: Title sanity for monitor adds: an entry with no title is rejected by
  the screen — never created as a work named "Unknown" (today's behavior, ST-005).
  An entry whose title survives cleaning as empty is likewise rejected (this is
  `add()`'s existing floor, ST-001 — the screen rejects it earlier, without burning
  an add attempt).

- **REQ-006**: The unreachable scraper road is deleted: the `MetadataProvider` trait,
  `LlmScraperProvider`, `LlmScraperConfig`, `build_llm_scraper_configs`, the
  `#[cfg(test)]` impl on `OpenLibraryProvider`, and their exclusively-owned helper
  types. Types the trait references that have other live users (e.g.
  `ProviderSearchResult`) are retained. After deletion the workspace compiles with
  zero references to the deleted items and no GR `/search` URL template remains in
  `livrarr-metadata`.

- **REQ-007**: The `work_service.rs` reduction begins with this sprint's extraction:
  the seed construction point of REQ-001 does not live in `work_service.rs`, and
  `work_service.rs` ends the sprint strictly smaller than its start size (3,616
  lines, measured 2026-06-12). No behavior change. (The full god-object split is
  explicitly NOT this sprint — see Non-Requirements.)

## 3. UI/Interface Design

Three small deltas to existing surfaces; no new pages, no new control vocabulary
(full mockups skipped — PO may reinstate):

- **List import preview**: one language selector above the row table, defaulted to
  English, labeled as applying to the whole import.
- **Author monitor**: the monitor configuration (where `monitored` /
  `monitor_new_items` / `monitor_since` live today) gains a language selector,
  pre-filled per REQ-003's smart default.
- **Series monitor action**: the existing Ebook/Audiobook/Both choice gains the same
  selector, pre-filled from the series' linked works.

The selector reuses the language vocabulary the app already uses elsewhere
(normalized ISO codes with display names).

## 4. Non-Requirements

- No language picker on Add-box, GR-link, Readarr, or manual import — they have a
  real signal (ST-003).
- No "unknown language" state — an absent signal resolves to `"en"`; the
  provider-applicability gate's semantics are untouched.
- No re-stamping of existing works when a monitor language setting changes.
- No LLM and no network calls in the quality screen.
- No replacement for the deleted scraper — LLM-driven discovery is Alpha 8 (#31),
  and would be built on LLM web-search APIs, not HTML scraping (PO, 2026-06-12).
- No background-convergence fix for Pending-seeded monitor works — that is the
  Sprint E #144-remainder row, with its own pending PO decision (ST-008).
- No full `work_service.rs` split — only REQ-007's extraction this sprint.
- Author dedup and cross-name series dedup (#112) remain out of scope (standing).
- Cross-author bind-ups ("Lorna Doone, Pride and Prejudice" — ST-007) stay
  unscreened: no deterministic title rule separates them from real comma-titled
  works without a known-works list. Accepted false-negative; the anthology class
  catches the ≥6-author ones.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | "Dominant language" tie-breaking: an author/series whose works are evenly split across languages, or all language-less? | resolved | Most-common language wins; on tie or no works, default `"en"`. The user sees and can change the pre-fill either way — the default never silently decides anything final. |
| Q-002 | Does the list-import language selector apply per-import or per-row? | resolved | Per-import. Rows carry no language signal to differ on (ST-002), and a per-row column is UI noise for the common one-language list. |
| Q-003 | Where do screened-reject counts surface? | resolved | Debug log (title + matched class) + a screened-count field on the monitor's run report struct; no UI surface this sprint. |
| Q-004 | Include the summary/study-guide keyword class with no observed exemplar? | resolved | Include — exemplar found in the second sample ("1984 SparkNotes Literature Guide", Orwell feed; 1/200 entries). High-precision keywords only; "Notes on" / "Analysis of" excluded (they match real titles, e.g. "Notes on a Scandal"). |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Workspace-wide, `WorkSeedFields` is constructed only at
  the shared construction point (and in tests) — a code search finds no door-local
  construction remaining.
- [ ] **AC-002** (REQ-001/ST-004): For every door, a created candidate's seed
  language equals its identity-harvest language — including the list door, whose
  seed/harvest disagree today.
- [ ] **AC-003** (REQ-002/REQ-003): An author whose library works are French gets
  monitor default `fr`; a monitor run adding a new work creates it with
  `language="fr"` in both seed and harvest.
- [ ] **AC-004** (REQ-003): A series whose linked works are French gets the same
  treatment for worker-created missing works.
- [ ] **AC-005** (REQ-003): A list import with the selector set to `de` creates every
  confirmed work with `language="de"`.
- [ ] **AC-006** (REQ-002): Deriving doors are pinned unchanged: Add-box with
  provider language `pt` → `pt`; Readarr edition `nl` → `nl`; manual import file
  `dc:language` `ja` → `ja`; each with the source signal absent → `"en"`.
- [ ] **AC-007** (REQ-003): Changing an author's OR a series' monitor language
  re-runs nothing: existing works keep their language; the next monitor-created
  work uses the new setting. Both settings survive restart.
- [ ] **AC-013** (REQ-003): Monitoring a stub through the multi-step promotion flow
  (author resolution and/or series picker) with a language chosen at the first step
  ends with that language persisted on the surviving series row; cancelling at the
  picker leaves no language persisted (stub unchanged, per sprint-c REQ-009).
- [ ] **AC-008** (REQ-004): A fixture set of ST-007's verbatim junk (incl. "Blood
  Lite" @ 23 authors, "Urban Enemies" @ exactly 6 authors — the threshold
  boundary, "Jim Butcher Box Set", "…Omnibus Volume 2", the
  newline-corrupted "Ghost Story…" title) is fully rejected: no works created, no
  notifications created, rejects counted in the run report with matched classes.
  The anthology fixtures are raw OL JSON entries in the sampled shape (with
  `authors` arrays), exercising the extended deserialization — not pre-parsed
  structs.
- [ ] **AC-009** (REQ-004): A fixture set of real titles from the sampled feeds
  passes the screen untouched — including a co-authored work at the threshold
  boundary (≤5 credited authors) and a real title containing the author's own name
  ("Persuasion by Jane Austen", observed — ST-007). The false-positive guard.
- [ ] **AC-010** (REQ-005): An OL entry with `title: None` (or cleaning to empty) is
  screened out — no work titled "Unknown" is created.
- [ ] **AC-011** (REQ-006): `MetadataProvider`, `LlmScraperProvider`, and
  `build_llm_scraper_configs` no longer exist; the workspace compiles; no
  `goodreads.com/search` URL template remains in `livrarr-metadata`.
- [ ] **AC-012** (REQ-007): `WorkSeedFields` construction is absent from
  `work_service.rs`, and the file is smaller than 3,616 lines.
