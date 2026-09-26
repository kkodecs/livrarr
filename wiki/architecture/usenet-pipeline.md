# SABnzbd and Usenet

Usenet uses the same release/grab model with a different download client protocol.
The parsed enclosure type identifies NZB; the resulting typed protocol chooses
SABnzbd. See [downloads](grab-system.md) for the common path.

Livrarr fetches the NZB through the indexer transport, uploads it with SABnzbd's
multipart `mode=addfile` request, and records the returned `nzo_id` as the download
ID. A false status is an error, even when HTTP succeeded.

The poller examines the active queue and completed history, then resolves the
reported `storage` path through remote mapping before import. **The history
`search` parameter matches names, not `nzo_id`.** Never use an ID-shaped search as
proof that a download is absent. Scope results to the actual download client.

Queue state is separate from [Grab state](../domain/grab.md). A failed status write
must not produce an already-completed history fact that will duplicate next tick.
Transient path unavailability during seedbox transfer needs recovery, not a claim
that the library file was permanently lost.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/architecture/usenet-pipeline.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="usenet-pipeline"></a>
<a id="how-it-differs-from-torrents"></a>
<a id="protocol-routing"></a>
<a id="sabnzbd-grab-flow"></a>
<a id="sabnzbd-polling"></a>
<a id="gotcha"></a>
