# Series

An ordered collection of related works. Globally shared per user. Sourced from Goodreads.

## Data Source

Series data comes exclusively from Goodreads (GR). A series cannot exist without a `gr_key`. This is a scoped exception to Principle 6 (degrade gracefully) — the author detail page simply doesn't show the series section if the author has no `gr_key`.

## Identity

- Identity is `gr_key`, not name. Name is display-only, updated on refresh.
- Uniqueness: `(user_id, author_id, gr_key)` — a co-authored series (same `gr_key`) can exist independently under each author.

## Monitoring

Per-media-type monitoring: `monitor_ebook` and `monitor_audiobook` are independent booleans.

When a series is monitored:
- Missing works are created and their corresponding `monitor_ebook`/`monitor_audiobook` flags set
- Unmonitoring clears monitoring flags on associated works
- Everything downstream (search, grab, import) uses existing per-work monitoring

## Work Assignment

- A work links to at most one series via `series_id`
- Display-only `series_name`/`series_position` may exist independently (legacy data, bibliography adds)
- When a work appears in multiple GR series, assigned to the most specific (fewest books)
- Assignment guard: only update `series_id` if current is NULL or new series has smaller `work_count`

## Cache

`author_series_cache` stores series list per author with `fetched_at` timestamp. Invalid JSON → cache miss → re-fetch from GR.

## Non-Goals (v1)

- Foreign language series
- Hardcover series data
- Series-level indexer search
- Manual series editing/assignment
- Overlapping/meta-series support
- Auto-merge of duplicate works
