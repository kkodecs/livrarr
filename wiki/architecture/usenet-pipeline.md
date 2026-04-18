# Usenet Pipeline

Usenet support runs alongside the torrent pipeline. Same search results, different download path.

## How It Differs From Torrents

| Aspect | Torrent (qBittorrent) | Usenet (SABnzbd) |
|--------|----------------------|------------------|
| File format | .torrent / magnet | NZB (XML manifest) |
| Auth | Cookie-based session | API key |
| Grab | Upload .torrent or send magnet URL | Download NZB, push via multipart |
| ID tracking | Torrent hash (`download_id`) | `nzo_id` from SABnzbd |
| Completion detection | qBit `content_path` | SABnzbd history `storage` |
| Default client | Per-protocol default | Per-protocol default |

## Protocol Routing

Automatic — determined by release `enclosure type`:
- `application/x-bittorrent` → torrent client
- `application/x-nzb` → Usenet client

One default client per protocol enforced via partial unique index.

## SABnzbd Grab Flow

1. Livrarr downloads the NZB from the indexer (handles Prowlarr-proxied URLs, Cloudflare, cookies)
2. Pushes NZB to SABnzbd via `POST /api` with `mode=addfile` (multipart)
3. SABnzbd returns `nzo_id` → stored as `download_id`
4. If SABnzbd returns `status: false`, grab fails with SABnzbd's error message

## SABnzbd Polling

Each poller tick:
1. Fetch queue (`mode=queue`) for active downloads
2. For each active grab whose `nzo_id` is NOT in queue: search history via `mode=history&search=<nzo_id>`
3. Completed → trigger import using `storage` path (after remote path mapping)
4. Failed → update grab to `failed` with `fail_message`
5. Orphan detection: grab in `sent` status for >24h, nzo_id not in queue or history, SABnzbd reachable → mark `failed`

## Gotcha

SABnzbd's `search` parameter in history API searches by **name**, not by `nzo_id`. This was discovered during prototyping — the spec originally assumed ID-based search.
