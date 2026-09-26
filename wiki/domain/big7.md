# Core entities

| Entity | Meaning | Persistence |
|---|---|---|
| [Work](work.md) | A book across formats and editions; the primary entity | User-scoped |
| [Author](author.md) | A credited person, associated with Works | User-scoped |
| [Series](series.md) | An ordered grouping and its monitoring policy | User-scoped |
| [Release](release.md) | An indexer offer for a downloadable copy | Transient; may be cached |
| [Grab](grab.md) | The user's download action and import lifecycle | User-scoped |
| [LibraryItem](library-item.md) | A concrete file in an organized root | User-scoped |
| [List](list.md) | An import session with preview, confirmation and undo | User-scoped |

A Work can have ebook and audiobook files simultaneously. Monitoring is separate
per format; missing a file is different from wanting a download. A Work has a
primary Author and can carry additional contributor evidence. The documented
series assignment uses one `series_id`; do not infer implemented many-to-many
membership from old overview drawings.

Root folders, indexers, download clients and remote mappings are shared
administrator-managed infrastructure. User-owned record operations must enforce
user scope even when a scheduled job enumerates multiple users.

The seven names describe core product concepts, not every type in the system.
Captured identity, Editions, provider routes, review/history records and playback
state supply additional contracts. See [identity workflow](../architecture/work-creation-pipeline.md)
and the [intended model](../../docs/canonical-model.yaml).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/domain/big7.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="big7-entities"></a>
<a id="key-relationships"></a>
<a id="scoping-rules"></a>
