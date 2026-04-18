# Grab

A download action for a release. User-scoped — each grab belongs to a specific user.

## Lifecycle

1. **Created** — user clicks "grab" or RSS sync auto-grabs
2. **Downloading** — torrent/NZB sent to download client
3. **Completed** — download client reports completion
4. **Imported** — import pipeline processed the files
5. **Failed** — download or import failed

## Key Properties

- Always scoped to a user_id
- Import lock key: `(user_id, work_id)` — prevents filesystem races
- Download poller checks status every 60 seconds
- Orphan file adoption: if target exists but no DB record, adopt instead of re-import
- Stale grabs reset on startup (startup recovery)

## Queue Visibility

- Admin: sees all queue items
- User: sees all items (prevents duplicate grabs), but "grabbed by" is redacted for non-admin users
