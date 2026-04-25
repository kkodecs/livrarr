# Roadmap

## Current Status: Alpha 3 (released 2026-04-25)

### What's in Alpha 3

- Full metadata enrichment pipeline with provenance tracking and merge engine
- Series monitoring via Goodreads
- Readarr library import with preview and undo
- List imports (Goodreads/Hardcover CSV)
- Built-in EPUB reader, PDF viewer, audiobook player
- OPDS catalog for reader apps
- Send to email (Kindle)
- Foreign language support (10+ languages)
- RSS sync with auto-grab
- Handler compile-time isolation (livrarr-handlers crate)
- Docker image optimized to ~76MB

### Alpha 4 (next)

| Item | Description |
|------|-------------|
| Cover architecture overhaul | Trust model (User > Validated > Unvalidated), quality gate, download-then-decide, EPUB cover extraction |
| Cover picker UI | Browse and select covers from multiple providers |
| Audiobook cover support | Separate cover slot for audiobook art |
| Readarr import enrichment | Safe post-import enrichment with trust model protection |

### Alpha 5+

| Item | Description |
|------|-------------|
| Author monitoring improvements | Auto-add from monitored authors with better dedup |
| Mobile-responsive UI | Touch-friendly layout for phones/tablets |
| PUID/PGID support | Configurable container user/group |
| ARM Docker image | linux/arm64 support |

## Deferred to Beta

| Item | Rationale |
|------|-----------|
| Cursor-based pagination | Replaces offset-based |
| HttpOnly cookie sessions | Security hardening |
| SSRF validation + resolver pinning | Security hardening |
| `livrarr doctor` CLI | Read-only integrity scanner |

## Future Ideas

- Request system (user requests for works)
- Shared collections across users
- Notification integrations (Discord, Telegram, etc.)
