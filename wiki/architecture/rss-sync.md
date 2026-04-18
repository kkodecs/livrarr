# RSS Sync

Automated release discovery and grabbing for monitored works. Runs as a background job.

## How It Works

Every 15 minutes (configurable, min 10min, 0=disabled):

1. **Fetch** — poll all RSS-enabled indexers in parallel. No search query — just `t=book` (or `t=search` fallback) with categories and `limit=100&extended=1`. 30s timeout per indexer.
2. **Parse** — extract title+author from each release title using `m3_string::parse_string()`
3. **Match** — score extractions against all monitored works (all users) using `m4_scoring::score_candidate()`. Score >= threshold (default 0.80) required.
4. **Filter** — skip if: not monitored for this media type, active grab exists, already in library
5. **Grab** — auto-grab best eligible release per user+work+media type per cycle

## How It Differs From Readarr

| Aspect | Readarr | Livrarr |
|--------|---------|---------|
| Quality profiles | Full (delays, tiers, proper/repack) | None — books don't have quality variation |
| Pending releases | DB-backed queue | None — no delay profiles needed |
| Matching | Deterministic (Goodreads ID) | Fuzzy (title 0.45, author 0.40, year 0.10, series 0.05) |
| Multi-user | Single-user | Feed fetch global, matching/grabbing per-user |
| Monitoring | Single boolean | Per-media-type (ebook/audiobook) |
| Filters | 6 specifications | 3 binary checks |

## Per-Media-Type Monitoring

Works have `monitor_ebook` and `monitor_audiobook` flags (split from single `monitored` boolean). RSS filter checks release categories: 7020 = ebook, 3030 = audiobook. A release with both categories is matched twice.

## Eligibility Ranking

Among eligible releases for a user+work+media type: score desc → indexer priority asc → seeders desc → size asc → GUID asc (stable tiebreaker).

## Gap Detection

Per-indexer state tracks `last_publish_date` + `last_guid`. If oldest item in a batch is newer than stored date and stored GUID is not in batch, log a gap warning (once). First sync for any indexer records state only — no grabs (prevents flood on setup).

## Duplicate Handling

If download client rejects as duplicate (torrent already added): create Grab record linked to existing download ID if provided. If only error returned, create Grab without download_id — poller does best-effort matching on completion (only when exactly one unlinked grab matches).

## Configuration

`indexer_config` singleton: `rss_sync_interval_minutes` (default 15), `rss_match_threshold` (default 0.80). Admin-only via `GET/PUT /api/v1/config/indexer`. Job reads from DB each tick — no restart needed.
