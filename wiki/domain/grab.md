# Grab

A Grab records a user's download action and its import lifecycle. The documented
states are Sent, Confirmed, Importing, Imported, ImportFailed, Failed and Removed.
Downloading/Completed are client queue concepts, not additional Grab states.

Sent/Confirmed consult the actual client for progress. The poller atomically claims
an import-safe download as Importing; only the claimant starts import. Imported
means the import path completed. ImportFailed can be retried; Failed records a
download failure; Removed is the user removal path.

Match download IDs within their owning client. Serialize file import per
`(user_id, work_id)`, not per Grab. On restart, interrupted claims need recovery;
on retry, destination orphan adoption needs the shared validation rather than
blind copying. [Downloads](../architecture/grab-system.md) ·
[import recovery](../architecture/import-pipeline.md).

Queue visibility and user redaction are API policy distinct from ownership of a
Grab. Do not infer permission to mutate another user's record from a shared queue
view; verify the real authenticated route when changing this surface.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/domain/grab.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="lifecycle--the-grabstatus-state-machine"></a>
<a id="key-properties"></a>
<a id="queue-visibility"></a>
