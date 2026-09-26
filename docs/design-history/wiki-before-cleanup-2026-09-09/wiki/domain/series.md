# Series

An ordered collection of related works. User-scoped: every series row carries a `user_id`
(`crates/livrarr-domain/src/entities.rs:456`) and is fenced by it
(`crates/livrarr-db/src/cross_user_isolation_tests.rs:607`). GR-backed rows are
sourced from Goodreads; **stub** rows are sourced from work metadata (sprint-c-series,
2026-06-12).

## Data Source

Two kinds of rows:
- **GR-backed:** `gr_key` is a real (numeric) Goodreads series key. Created by the
  monitor flow or by stub promotion/silent resolution.
- **Stub:** created from a work's `series_name` metadata (back-fill or ongoing
  reconcile). `gr_key = "stub:" + identity_key(name, "").0` — the shared identity
  normalizer (`crates/livrarr-metadata/src/series_link.rs:28-32`), **not**
  `normalize_for_matching`: that one is superseded and has no production call site left
  (`crates/livrarr-domain/src/util.rs:85-90`), and the two recipes disagree on stopwords
  and accents, so computing a stub key with it yields a different key. Internal marker,
  **masked to `""` at the API boundary** (UI hides GR links for keyless series).
  `work_count = i32::MAX` sentinel (see Work Assignment), masked to 0 at the API.

## Identity

- GR-backed identity is `gr_key`; stub identity is the normalized name (encoded in
  the stub key). Uniqueness: `(user_id, author_id, gr_key)` — gives per-name stub
  uniqueness with no schema change.
- Promotion (stub → monitored) resolves a real gr_key (exact normalized-name match
  on the author's GR series; picker on ambiguity; author auto-link at ≥0.90 name
  similarity first) and adopts it **in place** — row id and work links survive. A
  gr_key collision with an existing row merges the stub into it (works relinked,
  stub deleted).
- Silent resolution: the first expansion of a stub runs the same exact-match road
  (no picker, no modals, monitoring untouched) and adopts key + real roster size
  together — never adopts on an empty roster parse.

## Monitoring

Per-media-type: `monitor_ebook` / `monitor_audiobook`. Stubs are created unmonitored;
**monitoring is never enabled without a resolved gr_key** (the flag-toggle road
rejects stubs; promotion is the road). When monitored: missing works created (cap 50)
and flags propagate to linked works; unmonitoring clears them.

## Work Assignment (REQ-001 reconcile, series_link.rs)

- A work links to ≤1 series via `series_id`. The reconcile runs at create, user edit,
  post-enrichment, and the idempotent startup back-fill (`jobs/series_backfill.rs`).
- Arbitration: a **user edit** of `series_name` always relinks; a **system** write
  (enrichment/merge/back-fill) never displaces a GR-backed link (string-only update).
- The worker's guarded link is unchanged: "most specific (fewest books) wins" via
  `work_count`. `work_count` remains the **GR roster size** (never a library count —
  see spec ST-007); the stub sentinel (`i32::MAX`) means GR-backed rows can claim
  works away from stubs, never the reverse.
- GC: an **unmonitored stub** left with zero linked works is deleted (unlink, relink
  away, or work deletion); monitored series are never auto-deleted.
- NULL-author works (author deleted) are skipped — `series_name` stays display-only;
  the recurring back-fill heals them if an author appears.
- Q-002 normalization: `"X, Book 3"` → stub `"X"`, work's `series_name` rewritten,
  `series_position` filled only when absent (`split_series_suffix` in livrarr-domain).
- Error semantics: user-edit reconcile failures **propagate** (no self-heal exists);
  create-path failures warn only (startup back-fill self-heals links).

## Roster (REQ-010, series_roster table; amended by N1 2026-07-03)

Persisted GR roster per series (parsed primary works: title/gr_key/position/year),
migration 062, FK CASCADE. Written by the monitor worker (write-through of the fetch
it already does) and on expansion when no usable roster is stored. **Emptiness is
never truth (N1):** an empty parse is never persisted, never overwrites stored data
(monitor skips write-through AND the work_count update), and a stored-empty row
reads as absent — the next expansion refetches, so rosters damaged during the
2026-07 GR-layout break heal on open. A pagination walk that collects fewer books
than the header's declared primary count yields EMPTY (drift), never a partial
roster (`crates/livrarr-metadata/src/series_query_service/gr_fetch.rs:107-119`).
**A missing primary count is the second EMPTY path** — page 1 parsing books while the
header states no count means the header drifted, and the walk returns nothing rather
than adopt GR's full edition soup (`:97-106`). Both route into the same no-write
guards. Inside a good window the roster is truncated to the declared count and then
collection-shaped titles are screened out (`:120-129`). Every roster save pairs with a `work_count` update (count IS the GR roster
size, ST-007). Non-empty stored rosters still never refetch. On the 2026-07 GR
layout the roster = the first "N primary works" entries (see
[goodreads.md](../integrations/goodreads.md) § Series pages); positions come only
from same-series title decorations — umbrella-series rosters ride unnumbered.
`GET /series/{id}/books` merges roster ↔ linked works: normalized GR key first,
then `find_matching_work` (the same matcher as linking — one authority),
claim-once, linked works never dropped. Display-only road: never creates works,
never writes FKs; an empty fetch degrades to linked-only rows with
`roster_available: false`.

## Anti-bot (ST-012)

The series path makes **zero** GR `/search` requests — autocomplete JSON for author
candidates, `/series/{key}` + `/series/list?id=` pages for rosters/lists. The old
books-search synthesis and authors-search HTML fallback are deleted; do not
reintroduce.

## Cache

`author_series_cache` (per-author series list, LLM-cleaned + raw) and `series_roster`
(per-series roster). Invalid JSON → cache miss → refetch. Author list endpoint runs
degraded for key-less authors (DB rows still served); DB rows matching no cache entry
are appended, never dropped.

## Non-Goals (v1)

- Foreign language series; cross-name series dedup (#112 — e.g. Enderverse variants)
- Hardcover series data; series-level indexer search
- Overlapping/meta-series; auto-merge of duplicate works
