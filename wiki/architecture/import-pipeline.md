# Import and file recovery

Livrarr organizes files into separate ebook and audiobook roots:

```
{root}/{user_id}/{Author}/{Title}.epub
{root}/{user_id}/{Author}/{Title}/{audio files}
```

The [root product policy](../../ARCHITECTURE.md) requires copies into the library,
leaving downloads available for seeding. CWA uses hardlink-first with copy fallback
for its downstream artifact. On September 18 the user confirmed that Readarr
imports must create independent copies; there is no Readarr hardlink exception.
The September 18 [source check](../../build/reviews/wiki-follow-ups-2026-09-18/readarr-copy/REVIEW.md)
found hardlink-first imports and linked retry targets in the recorded deployment.
The [correction was deployed](../../build/reviews/readarr-independent-copies/deployment-20260918T063034Z/DEPLOYMENT.md)
on September 18: Readarr requests Copy, and the shared core separates existing
links on Copy retries before success, preserving the library file's current
contents and permissions. [Implementation/test evidence](../../build/reviews/readarr-independent-copies/RESULT.md)
records the reviewed source in the separate `readarr-independent-copies` worktree.
No existing-library inventory or repair was performed; files never revisited by an
import remain outside this correction. [Decision record](../../build/reports/wiki-decisions-2026-09-18.md).

## Workflow and ownership

The documented shared file-entry authority is `ImportWorkflow::import_file` in
`livrarr-library`. Grab imports, manual files, Readarr files and scan adoption feed
that core with their own policy and evidence. Server composition supplies tag,
CWA and notification services. `livrarr-library` has no direct tagwrite dependency;
chapter extraction crosses a domain contract.

An automatic import starts when the poller has an import-safe download, resolves
its client path and remote mapping, and atomically claims the Grab. Import is
serialized by `(user_id, work_id)`, not Grab ID. Files are enumerated, classified,
validated against the destination and imported. Each LibraryItem records its
relative path, owning Work, user, root and tag state.

The shared-core documentation records row creation before tag post-processing;
older pages placing tags, CWA and row creation in a different universal order are
historical. Trace the selected door's actual post-steps: grab/manual/Readarr/scan
policies are not interchangeable. See [workflow guide](roads.md).

## Collision and recovery rules

- A destination already recorded for this Work can be skipped. A destination owned
  by another Work must surface a collision.
- A file without a row may be adopted on retry only under the core's validation,
  including expected size. Existence alone does not establish identity or ownership.
- For Copy imports, both a recorded-file skip and an orphan adoption
  ensure independent storage before success. Another Work's ownership and orphan
  size checks precede mutation; separation copies the current target, preserving edits.
- Copy/tag changes use temporary files and atomic replacement. On tag failure,
  deleting the temporary copy leaves the original library file intact.
- Refresh file size after a successful tag rewrite. Metadata changes can alter it.
- Keep filesystem/network operations outside SQLite transactions and blocking work
  off the async executor. Cancellation must retain ownership through admitted work.
- Path mapping must match a component boundary: `/downloads` cannot match
  `/downloads2`. Sanitize names and preserve valid UTF-8 when limiting components.
- The "files not fully synced" size pre-check (on-disk total under 90% of the
  indexer's advertised `grab.size`) exists for one setup: downloads on a remote
  machine copied over by an outside tool. Since 2026-09-30 (`c48f9b02`) it runs
  only when the grab's download client is a torrent client; usenet grabs skip it,
  because the advertised usenet size counts repair files that unpacking removes
  (GitHub #178). The protocol comes from the client row, never from a URL or
  title; if the client cannot be read, the check runs. Error text and the retry
  schedule are unchanged.

Historical tag support explicitly disabled MP3/M4B writes because the upstream
writers could buffer multi-GB media. Unfinished work changes those surfaces. Do
not advertise audio-tag success based on an Unsupported-as-success return or old
method inventory; check the deployed source. EPUB tags are the established path.

## Manual import and scan

Manual import identifies candidate Works, lets the user review/correct them and
imports the selected files. A title/author override deliberately drops stale
match-derived keys. Minimum-only import uses the shared atomic identity coordinator;
defer must leave no partial Author/Work writes and must explain recovery.

Do not send paths, filenames or checksums to an LLM. Cleanup/metadata assistance is
limited by the root privacy boundary; older text saying files go to an LLM is not
an authorization. The described scan limits are 50 media files and 10,000 traversed
entries; check the actual entry point before changing or relying on those limits.

Library scan parses ebook title from the filename and audiobook title from the
Work directory. Preview is read-only; adoption is a separate shared-core operation.
Cross-format sidecars have additional identity and rescan rules in
[cross-format resume](../domain/cross-format-resume.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/architecture/import-pipeline.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="import-pipeline"></a>
<a id="auto-import-flow"></a>
<a id="file-classification"></a>
<a id="tag-writing-detail"></a>
<a id="manual-import"></a>
<a id="manual-scan"></a>
<a id="import-lock"></a>
<a id="name-sanitization"></a>
