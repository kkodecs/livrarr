# Design: #112 — foreign-language editions leak onto English author pages

Status: cross-family reviewed (2 rounds — initial FAIL x2 fixed below, pre-fix plan
check PASS/FAIL-with-refinements x2, all applied). Signed off by the product owner;
implemented, tested, and shipped.
Branch: `feature/112-foreign-editions-author-page`.
Reviews: `build/reviews/issue-112-foreign-editions/` (Gemini + Codex, 4 rounds total).

## Problem (recap)

An English author's page (bibliography list + series list) can show foreign-language
editions/translations as if they were regular entries. Reported case: Neal Stephenson's
page listed *Cryptonomicón I/II/III* (the Spanish translation) as a "3-book series."
Monitoring/adding from these paths creates Works stamped with the install's default
language, not the edition's real language.

Two independent leak paths, both in scope:
1. **Bibliography list** (`author_service.rs`) — sourced from OpenLibrary (primary) /
   Google Books (fallback-only, rarely triggered for well-known authors).
2. **Series list** (`series_query_service.rs`) — sourced from Goodreads series pages.

## Decisions (confirmed with the product owner)

- **Toggle**, not hard filter: default view = "author's language only," switch to
  "all languages." Reuses the existing `monitor_language` / `dominant_language`
  concept already on Author/Series (wiki insight #53) as "author's language" —
  falls back to the install default if unset.
- **Unknown language** (can't determine either way): shown by default, labeled plainly
  ("unknown"), no "needs review" wording — we're not asking the user to act.
- **Both leak paths fixed** in this pass, not bibliography-only.
- When an entry is added/monitored, its **real detected language** is used instead of
  defaulting to the install/author language — closes the mis-tagging half of the bug.

## What each source actually gives us (verified, not assumed)

| Source | Endpoint in use today | Language data? |
|---|---|---|
| OL author bibliography | `/authors/{key}/works.json` (`fetch_ol_bibliography`) | None — confirmed absent from the payload. |
| OL work editions (per-work) | `/works/{key}/editions.json` (already called elsewhere: `query_ol_detail`, for ISBN) | Yes, per-edition `languages` field — but incomplete (~69% populated in a real sample: 24/35 editions of *Cryptonomicon*, missing exactly the Spanish edition from the bug report). **Rejected as the mechanism** — one call per bibliography entry is exactly the "hundreds of single-record round-trips" anti-pattern wiki insight #45 tells us to avoid. |
| OL search, batched by author | `search.json?q=author_key:{ol_key}&fields=key,title,language&limit=100` | Yes — confirmed live: one call returns ALL of an author's works with a per-work `language` aggregate array in one round trip. Tested against the real Neal Stephenson OL author key (`OL19430A`): 70 works, 47 with a language array, `Cryptonomicon` → `["ger","eng","fre","spa"]` — same ~67% completeness as the per-edition approach, at 1/70th the calls. **This is the mechanism the design uses.** |
| GB bibliography fallback | `inauthor:"..."` search (`fetch_gb_bibliography`) | Yes, `volumeInfo.language` — currently fetched and discarded. |
| GB title search (hypothetical use for language tagging) | `intitle:...` | Structurally blind to translations: searching an English title only matches English-titled results. Confirmed live — a GB search for "Cryptonomicon" returned 0 Spanish results; a direct ISBN lookup for the real Spanish edition also returned 0. GB simply doesn't have that specific edition. |
| GR series list | `goodreads.com/series/list` scrape | None. |

Net: cross-referencing OL bibliography entries against GB **by title** doesn't work
(title mismatch across languages) and would burn GB's 1,000/day quota for a fallback
that's incomplete anyway. OL's own edition data, reused rather than cross-provider
searched, is the right primary source for the bibliography path.

## Design

### 1. Bibliography path — one batched OL search call, no GB call needed

Issue `search.json?q=author_key:{ol_key}&fields=key,title,language&limit=100` per author,
**paginating via `offset` until `numFound` is exhausted** (not a fixed one-page call —
review round 1 caught that a single `limit=100` page silently drops overflow entries to
"Unknown" → shown by default, recreating the leak for any author with 100+ works). Cap
pagination at a defensive maximum (e.g. 10 pages / 1000 works) as an **error guard, not
a normal path**: hitting the cap logs a warning and marks the result partial, rather than
classifying uncovered entries as ordinary Unknown. Match returned `key` values against
bibliography entries' `ol_key`, then classify each entry from its `language`
aggregate array:

- **Aggregate includes the target language** (author's language, e.g. `eng`) →
  `language: Some("en")` — this Work has a real edition in that language, show it.
- **Aggregate present, target language absent** → `language: Some("<other>")` —
  genuinely foreign, filter/label.
- **No `language` field on this work at all** → defaults to the target language rather
  than `None` (Unknown) — revised post-launch after live testing showed a literal
  "language unknown" label on the majority of an English author's own catalog (OL's
  aggregate only tags ~67-69% of even a well-known author's Works) read as broken, not
  cautious. Absence of evidence isn't evidence of a foreign language; `None` is now a
  genuinely rare edge case (see `classify_ol_language` / `classify_one_series_language`).

One call, same completeness as the per-edition approach (~67-69%), no anti-pattern.

This is a presence check ("does this Work have an English edition"), not a single
classification — correctly keeps a merged multi-language OL Work (like the real
*Cryptonomicon*, which has 13 tagged English editions) visible, while correctly
flagging an unmerged foreign-only duplicate Work.

Entries sourced from the **GB fallback path** (`fetch_gb_bibliography`, rare — only
fires when OL has nothing) already carry `volumeInfo.language` per entry — stop
discarding it, map it straight into the same field.

**No new Google Books calls, no quota impact, no cross-provider title-matching risk.**

### 2. Series path — GB title search, cached per series

Goodreads series entries have no OL work-key to check editions against, and the
series name Goodreads gives us (e.g. "Criptonomicón") is already in whatever language
the series actually is — so a GB title(+author) search doesn't have the translation
title-mismatch problem the bibliography path would have had. Confirmed live: GB's
tagging is reliable on what it does return (100% tagged in the one real sample tested,
19/19 editions).

- Query GB (`intitle:"<series name>" inauthor:"<author name>"`, `fields=` restricted,
  small `maxResults`).
- **Confidence gate: reuse the existing matching authority** (`livrarr-domain/src/identity_matching.rs`
  `title_verdict`/`author_verdict` — the one place "is this the same book/author" is
  decided everywhere else in the codebase, wiki insight #59) instead of inventing new
  fuzzy-match logic. Only trust a GB volume's `volumeInfo.language` if `title_verdict`
  (series name vs. volume title) returns Same or Grey (not Different/VetoVolume) and
  `author_verdict` doesn't veto.
- **Known limitation (reviewed and accepted):** `title_verdict` compares main titles;
  series-marker text isn't part of that comparison, so a series whose volume titles
  don't literally contain the series name can fail the gate and resolve to Unknown even
  when GB has the right data. This is a safe direction (Unknown → shown by default,
  never a false foreign-hide) — a coverage gap, not a correctness risk. Acceptable for
  this pass; a future enhancement could add GB's `seriesInfo` field as a second signal
  when populated.
- No qualifying match → `language: None` (Unknown), shown by default.

Series lists are much smaller than full bibliographies (single digits to low dozens
per author vs. dozens-to-hundreds of bibliography entries), so the GB quota cost here
is modest — and it's cached (see below), so it's a one-time cost per series, not
per page view.

### 3. Data model — no DB migration needed, but a real mapping chain to thread through

`BibliographyEntry` and `SeriesCacheEntry` are stored as serialized JSON blobs
(`author_bibliography.entries`/`raw_entries`, `author_series_cache.entries`/`raw_entries`
— both keyed by `author_id`, no per-field columns). Adding `language: Option<String>`
to each Rust struct is picked up by serde automatically; no migration required.

Computed once at fetch/refresh time (same point where OL/GB/GR are already called),
persisted in the existing cache row — so toggling the view or reloading the page never
re-triggers a provider call. Only `refresh_bibliography` / `refresh_author_series`
recompute it.

**However** (review round 2 caught this — verified myself, confirmed real): the DB/cache
struct is NOT what actually reaches the frontend. `language` has to be threaded through
every hop on the wire, or it's silently dropped partway:

- Bibliography: `livrarr_db::BibliographyEntry` (DB/cache) → `livrarr_domain::services::BibliographyEntry`
  (domain) → `author_service.rs`'s bibliography-building/enrichment mapping (~line 625-632)
  → `bibliography_to_json` (`crates/livrarr-handlers/src/author.rs:280-300`, confirmed —
  this hand-builds a `serde_json::json!{}` with an explicit field list; a struct-level
  field added upstream is silently dropped here unless this literal map is also updated)
  → frontend `BibliographyEntry` TS type.
- Series: `livrarr_db::SeriesCacheEntry` (DB/cache) → the series domain/service view
  (`livrarr-domain/src/services/series.rs`) → **both** `list_series` and
  `refresh_series` handler mappings (`crates/livrarr-handlers/src/series.rs` — confirmed
  `list_series` at line ~166-190 hand-maps every field into `SeriesResponse`; `refresh_series`
  has its own separate mapping and must be updated too) → frontend `SeriesResponse` TS type.

### 4. Frontend — toggle + labels + real language on add/monitor

- Author-language-only / all-languages toggle on both the Books and Series tabs of
  `AuthorDetailPage.tsx`. Default state = author-language-only (reuses the resolved
  `monitor_language` ?? install default, same value already computed server-side for
  series suggestions — insight #53's `dominant_language`).
- Filter rule: show if `language` is `None` (Unknown) or matches the effective default;
  hide (or show-with-label, in "all languages" mode) otherwise.
- "All languages" mode: label each non-default entry with its language; Unknown entries
  get a quiet "unknown" tag, no call-to-action wording.
- **Add mutation** (`AuthorDetailPage.tsx` bibliography add): `AddWorkRequest.language`
  already exists as a field on both the handler and domain structs (`work.rs:105-134`,
  `services/work.rs:25-59`) — it's just never populated from the author page today.
  Send the entry's detected `language` instead of leaving it null.
- **Monitor mutation** (`AuthorDetailPage.tsx` series monitor button) — **corrected
  after review**: the actual current behavior is a single `language` state for the
  *entire* series section (`SeriesSection`, `AuthorDetailPage.tsx:454`), pre-filled from
  `author.monitorLanguage ?? "en"`, and every "Monitor" click on every series row sends
  that same section-wide value — there is no per-series override today. (My first draft
  of this doc mischaracterized this as an automatic per-series prefill; it verified as
  wrong when checked directly.)
  - Fix: when a series entry has a *known* detected language, that row's Monitor action
    sends the detected language, overriding the section dropdown for that one call only.
    Series with Unknown detected language keep today's behavior (the shared dropdown).
  - **No silent override** (review caught this): the existing `HelpTip` on the dropdown
    claims it controls the language for every monitor created in the section, which
    would become false for foreign-tagged rows. Show a visible marker on those rows
    (e.g. a small "Auto: ES" badge next to the series name) and update the HelpTip copy
    to note that a row with a detected language uses its own value instead of the
    dropdown.

## Explicitly out of scope for this pass

- Bibliography **completeness** (does OL/GB's combined footprint miss legitimate books
  entirely) — a real, separate question, not this bug.
- Per-edition granularity within a series (assumes a series' volumes share one language
  — reasonable for translated multi-volume works, the reported case).
- Any change to the OL-primary/GB-fallback *discovery* order for the bibliography list.

## Open implementation questions (not product calls, just need picking)

- Exact `maxResults` cap for the series GB query — low-stakes, settle during implementation.
- Cache invalidation: language tags recompute on the same cadence as the existing
  bibliography/series refresh — confirm no separate TTL is needed.
- Exact pagination page cap for the OL author search (design says 10 pages / 1000 works
  as a defensive guard) — confirm this is generous enough in practice.

## Next step

Done — implemented, cross-family reviewed, live-tested against the real reported case
(Neal Stephenson / Cryptonomicon), and shipped. See commit history on
`feature/112-foreign-editions-author-page` for the two post-launch refinements (the
"no signal defaults to target language" change above, and the Add-flow language fix).
