# Roadmap

## Current Status: Alpha 4 (released 2026-04-29)

Post-alpha4 work has landed on `main` but is not yet tagged as a release.

### Shipped since Alpha 4

- Trust-aware multi-cover system with picker UI (ebook + audiobook slots, quality gate, reject filter)
- English Work Lifecycle refactor (identity anchors, cover gate, conflict resolution)
- Recently-downloaded sort + URL protocol dropdowns
- Server-side media type filter across all layers
- Various bug fixes (import, UI, version display)

### What was in Alpha 4

- SSRF trusted origins for private-IP indexers and download clients
- Manual import dedup improvements
- Download poller fix for private-IP qBittorrent

### What was in Alpha 3

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
- Mobile-responsive UI (all 27 pages)
- Docker image optimized to ~76MB

## Enhancement Roadmap

Prioritized list of planned features. See `build/plans/enhancement-roadmap.md` for full details and difficulty estimates.

| Priority | Issue | Title | Size |
|----------|-------|-------|------|
| 1 | — | ~~Quick UI polish (#30, #32, #41)~~ | ~~S-M~~ Done |
| 2 | #17 | Transmission download client | L |
| 3 | #26 | Collapse series toggle | L |
| 3 | #27 | Lists / bookshelves | L |
| 4 | #20 | ~~Multi-cover harvest + selection~~ | ~~XL~~ Done |
| 4 | #33 | Separate audiobook cover | L |
| 5 | #22 | M4B chapter navigation | M |
| 5 | #23 | Progress memory + KASH sync | XXL |
| 6 | #28 | Auto-tagging / genre curation (LLM) | XL |
| 6 | #31 | Discovery / suggestions (LLM) | XL |
| 7 | #16 | Watch folder | L |
| 8 | #34 | rclone seedbox integration | XL |
| 9 | #21 | Push metadata to OpenLibrary | L |
| 10 | #29 | Third-party security audit | — |

### Parked (no timeline)

| Issue | Title |
|-------|-------|
| #25 | Magazine support |
| #24 | Anime / manga support |

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
