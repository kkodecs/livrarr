# LibraryItem

A file on disk in the organized library. User-scoped.

## Lifecycle

1. **Imported** — file copied from download directory, tags written
2. **Active** — file exists, accessible to downstream tools
3. **Missing** — file expected but not found on disk

## Key Properties

- Always scoped to user_id
- File path follows opinionated layout (Principle 7)
- Metadata is embedded in the file (Principle 5: the file is the artifact)
- CWA downstream copy created after import (hardlink-first)
- Files in the library are not modified without explicit user action

## Import Path

Copy, never move or link (Principle 8). Original stays in download directory for torrent seeding. Tag writing modifies only the library copy.
