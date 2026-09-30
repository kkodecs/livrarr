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

## Removing files from disk

As of 2026-09-30 (errors-and-delete-pass), one module decides where an item lives
and when it may be removed: `crates/livrarr-domain/src/library_path.rs`.
`resolve_for_read` (`:38`) serves every read (stream, download, email, OPDS,
cross-format). `remove_library_file` (`:108`) serves both removal doors. The remove
rule never follows a link at the item's own entry. It requires the resolved parent
to be inside the resolved root, removes only a regular file, and never removes a
folder. An absent file counts as already gone. The check-then-remove race is an
accepted limit; closing it needs a new dependency.

- **Deleting a Work** keeps its files unless the user ticks "Also delete files from
  disk" (`DELETE /work/{id}?deleteFiles=true`; the box is unticked on every open).
  Root folders are read before the records go. Files that cannot be removed come
  back as warnings. The `workDeleted` history event records the real count removed.
  List-import undo always keeps files.
- **"Delete File"** on a book's files tab removes the file first, then the record. A
  refused removal keeps the record and answers 409 with the reason. This follows
  Readarr's single-file delete; there is no checkbox.

Before this, the delete path passed the relative stored path to the filesystem, so
no file was ever found, and history counted every item as removed.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/domain/library-item.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="lifecycle"></a>
<a id="key-properties"></a>
<a id="import-path"></a>
