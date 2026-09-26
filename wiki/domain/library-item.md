# LibraryItem

A LibraryItem is a user-scoped file linked to a Work, with root folder, relative
path, media type, file size and tag/revision state. It is not the Work's identity.
One Work may have files in both formats or multiple audiobook files.

The [import authority](../architecture/import-pipeline.md) owns creation and
destination validation. Imports copy from downloads; originals remain available
for seeding. CWA's downstream hardlink/copy is a separate artifact policy.

Stored paths need the root joined before file access. Relative paths passed to a
writer against the server's working directory have caused false FileNotFound
failures. Record size after successful writes and preserve the existing file on
failed temporary-file replacement.

Tag synchronization stays within the user's authorized workflow. Already settled
files require a new action; unfinished import completion may finish its work.
File absence, tag state and identity uncertainty are separate conditions.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/domain/library-item.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="lifecycle"></a>
<a id="key-properties"></a>
<a id="import-path"></a>
