# Release

A downloadable version of a work found via search. Not persisted — releases are transient search results from indexers.

## What a Release Is

A release is a search result from a Torznab/Newznab indexer. It has quality attributes (format, size, seeders) and a download URL. Releases are never stored in the database.

They do outlive one request, though: search results are held in a process-local in-memory cache keyed by `(title, author, indexer_id)` with a 24-hour TTL and lazy 5-minute eviction (`crates/livrarr-download/src/release_search_cache.rs:16-25`). A search serves a fresh cache entry with zero HTTP unless `refresh` is set, and rewrites the entry on every live fetch (`crates/livrarr-download/src/release_service.rs:107-126`, `:255-262`). That cache — not the database — is why a repeated search can return releases an indexer no longer lists.

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
2. Every indexer with interactive search enabled is queried in parallel, 30s timeout each
   (`crates/livrarr-download/src/release_service.rs:73-77`, `:132-141`, `:178`)
3. **One search per indexer, not two tiers:** `t=search&q={title} {author-last-name}`, plus
   `&apikey=` and `&cat=` when configured (`:83-93`, `:157-169`). There is no `t=book`
   structured tier — the query carries only the author's **last** whitespace-separated token
4. Torznab XML parsed per Newznab spec; an item missing a `guid` or a download URL is dropped
   with a warning (`:213-230`). Protocol is decided here: an enclosure type containing `nzb`
   is Usenet, **everything else — including a missing enclosure type — is Torrent**
   (`:232-240`)
5. Results merged and deduplicated by **`(guid, indexer)`**, so the same GUID from two
   indexers survives twice; no indexer-priority tiebreak runs here (`:295-297`)
6. Sorted torrents-before-usenet; torrents by seeders desc then size desc, usenet by publish
   date desc then size desc (`:299-319`)

## Protocol Routing

The MIME type is read once, at parse time, and only to look for `nzb` (see step 4 above) —
`application/x-bittorrent` is never matched by name. What routes the grab is the resulting
`DownloadProtocol` (`crates/livrarr-download/src/release_service.rs:373-376`):

- `Torrent` → the default torrent client (`qbittorrent`; Transmission is also checked as a
  torrent default, `:399-403`)
- `Usenet` → the default Usenet client (`sabnzbd`)
- No client configured for the protocol → `NoClient` error; an explicitly chosen client whose
  protocol doesn't match → `ClientProtocolMismatch` (`:385-396`)

## RSS Sync

RSS sync uses the same search infrastructure but without query parameters — it fetches recent releases and matches them against monitored works using fuzzy scoring. The base weights are title 0.45, author 0.40, year 0.10, series 0.05 (`crates/livrarr-matching/src/m4_scoring.rs:25-28`), **renormalized over whichever fields are actually present** (`:30-38`) — with no year and no series, title and author alone carry the score at 0.529/0.471. The 0.80 threshold is a schema default, not a constant: it is the operator-settable `rss_match_threshold` (`crates/livrarr-db/migrations/015_rss_sync.sql:22`), read per run at `crates/livrarr-metadata/src/rss_sync_workflow.rs:271`.
