# Grab

A download action for a release. User-scoped — each grab belongs to a specific user.

## Lifecycle — the `GrabStatus` state machine

Seven states, not five (`crates/livrarr-domain/src/entities.rs:70-78`), persisted as the
lowercase/camelCase strings in `crates/livrarr-db/src/sqlite_grab.rs:85-94`:

1. **Sent** — the row is created with this status when the release is dispatched to the
   download client (`crates/livrarr-download/src/release_service.rs:465`), whether the user
   clicked grab or RSS sync auto-grabbed
2. **Confirmed** — still with the download client. The queue view treats `Sent` and
   `Confirmed` alike and asks the client for live progress
   (`crates/livrarr-handlers/src/queue.rs:43`)
3. **Importing** — the poller found an import-safe download whose files exist locally and
   claimed the grab; only the claiming tick spawns the import
   (`crates/livrarr-server/src/jobs/download_poller.rs:288-298`)
4. **Imported** — the import pipeline processed the files
5. **ImportFailed** — the import errored (`download_poller.rs:831`); retryable from the queue,
   which re-sets this status if the retry also fails (`crates/livrarr-handlers/src/queue.rs:125`)
6. **Failed** — the download client reported failure (`download_poller.rs:538`)
7. **Removed** — the user removed the grab (`crates/livrarr-download/src/grab_service.rs:88`)

There is no `Downloading` or `Completed` state on a grab; those are download-client queue
states (`QueueStatus`, `crates/livrarr-domain/src/entities.rs:242-249`), a separate enum.

## Key Properties

- Always scoped to a user_id
- Import lock key: `(user_id, work_id)` — prevents filesystem races
- Download poller checks status every 60 seconds
- Orphan file adoption: if target exists but no DB record, adopt instead of re-import
- Stale grabs reset on startup (startup recovery)

## Queue Visibility

- Admin: sees all queue items
- User: sees all items (prevents duplicate grabs), but "grabbed by" is redacted for non-admin users
