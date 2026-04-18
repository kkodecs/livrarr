# Release

A downloadable version of a work found via search. Not persisted — releases are transient search results from indexers.

## What a Release Is

A release is a search result from a Torznab/Newznab indexer. It has quality attributes (format, size, seeders) and a download URL. Releases are never stored in the database — they exist only in search results and grab requests.

## Fields

- `title` — release title from the indexer
- `guid` — unique identifier from the indexer
- `size` — file size in bytes
- `downloadUrl` — from `<enclosure url>` in Torznab XML
- `seeders`, `leechers` — peer counts
- `publishDate` — when posted
- `categories` — Newznab categories (7020 = ebook, 3030 = audiobook)
- `indexer` — which indexer returned it
- `protocol` — `torrent` or `usenet`

## Search Flow

1. User triggers release search for a work
2. All enabled indexers queried in parallel (30s timeout each)
3. Two-tier search per indexer:
   - Tier 0: `t=book&author={author}&title={title}` (structured, if supported)
   - Tier 1: `t=search&q={title} {author}` (freetext fallback)
4. Torznab XML parsed per Newznab spec
5. Results merged, deduplicated by GUID (highest-priority indexer wins)
6. Sorted by seeders descending

## Protocol Routing

When a user grabs a release, protocol determines which download client handles it:
- `application/x-bittorrent` → default torrent client (qBittorrent)
- `application/x-nzb` → default Usenet client (SABnzbd)
- No client configured for protocol → error

## RSS Sync

RSS sync uses the same search infrastructure but without query parameters — it fetches recent releases and matches them against monitored works using fuzzy scoring (title 0.45, author 0.40, year 0.10, series 0.05, threshold 0.80).
