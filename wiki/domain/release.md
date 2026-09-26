# Release

A Release is an indexer search result, not a stored library record. It carries the
indexer's title, GUID, size, download URL, protocol, categories, date and optional
peer counts. A process-local search cache can retain it beyond one request;
cached results may outlive availability at the indexer.

Interactive search filters malformed offers, deduplicates `(guid, indexer)` and
sorts torrent results before Usenet; torrents use seeders then size, Usenet uses
publication date then size. The documented parser recognizes an NZB enclosure;
other/missing enclosure types fall into Torrent. Routing thereafter uses the typed
protocol and requires a compatible download client.

RSS applies its own candidate-aware parsing, hard gates, scoring and monitoring
policy. Those scores do not belong to ordinary search sorting. A release becomes
a [Grab](grab.md) when sent through the shared download action.
[Search and downloads](../architecture/grab-system.md) ·
[RSS policy](../architecture/rss-sync.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/domain/release.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="what-a-release-is"></a>
<a id="fields"></a>
<a id="search-flow"></a>
<a id="protocol-routing"></a>
<a id="rss-sync"></a>
