# Import Pipeline

How files get from download directory to organized library. Spans `livrarr-organize` and `livrarr-tagwrite`.

## Auto-Import Flow

1. Download poller detects completed torrent/NZB (60s interval)
2. Grab status set to `importing` (atomic UPDATE prevents concurrent duplicate imports)
3. Source path resolved: qBit `content_path` or SABnzbd `storage` + remote path mapping
4. Files enumerated and classified by extension
5. Each file routed to root folder by media type
6. File copied to organized path (never moved — Principle 8)
7. Tag writing on library copy (temp-file-then-rename)
8. File size measured AFTER tag writing (tags change file size)
9. CWA downstream copy if configured (hardlink-first)
10. Library item record created in DB
11. Grab status updated (imported or importFailed)

## File Classification

| Extension | Media Type |
|-----------|-----------|
| `.epub`, `.mobi`, `.azw3`, `.pdf` | Ebook |
| `.mp3`, `.m4a`, `.m4b`, `.flac`, `.ogg`, `.wma` | Audiobook |
| Other | Skipped with warning |

## Tag Writing Detail

Supported formats only (`.epub`, `.m4b`, `.mp3`). Unsupported formats import without tags — not an error.

**Per-file flow:**
1. Copy source → `{target}.tmp`
2. Write tags on `.tmp` in place
3. Success: rename `.tmp` → final
4. Failure: delete corrupted `.tmp`, re-copy source → final (untagged), log warning

**Multi-file MP3 audiobooks (TAG-006):**
1. Copy all source MP3s → `.tmp` files
2. Pass 1: write tags into all `.tmp` files
3. If any fails: abort, delete ALL `.tmp` files, re-copy ALL sources → final (untagged)
4. Pass 2: rename all `.tmp` → final (all-or-nothing)

## Manual Import

User points at a filesystem path. Files sent to LLM for title/author extraction. OL searched for matches. User reviews and confirms. Same import pipeline for file operations.

Cap: 50 media files per scan, 10,000 total filesystem entries traversed.

## Manual Scan

Walks `{root}/{user_id}/` directory. Matches files to works by normalized title+author from path structure. Creates library items for matches.

**Path parsing:**
- Ebook (depth 2): `{author}/{file}` — title from filename stem
- Audiobook (depth 3+): `{author}/{title}/{files}` — title from directory name
- Normalization: strip control chars, replace illegal chars with spaces, collapse whitespace, case-insensitive match

## Import Lock

`(user_id, work_id)` — not per-grab. Serializes concurrent imports for the same work.

## Name Sanitization

- Illegal chars (`\ / : * ? " < > |`) → underscores
- Control characters stripped
- `.`/`..` → fallback values
- Trailing dots/spaces trimmed
- Path components limited to 255 bytes (truncate at UTF-8 boundary, append ellipsis)
