# Downloads and grabs

Indexers accept direct Torznab/Newznab URLs; Prowlarr is optional. Release search
and RSS feed fetching are distinct paths that converge at the grab service.
[Release semantics](../domain/release.md) · [RSS matching](rss-sync.md).

Interactive search queries enabled indexers in parallel, parses releases, drops
items missing required identifiers/URLs, deduplicates by `(guid, indexer)` and sorts
results. The documented query is `t=search` with title plus author surname; there
is no second `t=book` tier. Search itself does not apply RSS matching scores.
The process-local release cache has a 24-hour TTL; cache-only opens spend no HTTP,
and explicit Search requests fresh results.

A selected release goes through `ReleaseService::grab`: URL trust checks, client
selection, protocol dispatch, then the shared Grab persistence and history path.
Torrent clients include qBittorrent and Transmission; Usenet uses SABnzbd. The
selected client's protocol must match; a default is chosen per protocol.

Indexer requests share origin pacing. Rate-limit breakers are per configured
indexer so one Prowlarr backend's 429 does not silence its siblings. Transport
failure protection is per origin. Rate-limited torrent downloads must not fall
back to handing the URL to qBittorrent, which would bypass the queue.

The poller scopes downloads to the actual client and Grab. qBittorrent UI status
and import safety come from one classifier: resume checking, checking completed
data and moving are not stable import states. A completed remote download can
still be in transit to the local mount. Failed imports use bounded retry/recovery;
they are not a second release-grab operation.

[Grab states](../domain/grab.md) · [Import and recovery](import-pipeline.md) ·
[SABnzbd details](usenet-pipeline.md) · [Transport lessons](../insights/providers-and-transport.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/architecture/grab-system.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="grab-system"></a>
<a id="components"></a>
<a id="indexer-system"></a>
<a id="release-search"></a>
<a id="download-clients"></a>
<a id="grab-flow"></a>
<a id="import-lock"></a>
<a id="orphan-file-adoption"></a>
