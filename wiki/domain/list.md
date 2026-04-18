# List

A mechanism for bulk-importing works from external sources. User-scoped.

## What Lists Are

Lists are import sessions, not persistent collections. A user provides a CSV (Goodreads export) or URL (OpenLibrary list), the system parses it, proposes matches, and the user confirms which works to add.

## Lifecycle

1. **Preview** — parse list source, match entries to works, return candidates with match status
2. **Confirm** — bulk-add matched works via WorkService. Each row has independent error handling — partial success is valid
3. **Undo** — remove works added by a specific import session (identified by `import_id`)

## Key Properties

- Import sessions are tracked so undo can target specific imports
- Matching uses the same enrichment pipeline as normal work addition
- Partial success: if 8 of 10 works import, the 8 succeed and 2 report errors
