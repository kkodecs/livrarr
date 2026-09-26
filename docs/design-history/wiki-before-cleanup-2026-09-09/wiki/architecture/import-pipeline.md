# Import Pipeline

How files get from download directory to organized library. File operations live in
`livrarr-library`; **tag writing does not**. `livrarr-library` depends only on
`livrarr-domain` and `livrarr-db` (`crates/livrarr-library/Cargo.toml`) — it has no
`livrarr-tagwrite` edge. The tag step runs in `livrarr-server`
(`crates/livrarr-server/src/tag_service.rs`), which calls `livrarr-tagwrite`; the cover +
tag projection path goes through `livrarr-materialize`
(`crates/livrarr-materialize/src/lib.rs:15`, `:341`).

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

**Only `.epub` is written.** `write_tags_sync` dispatches `epub` to `write_epub` and returns
`TagWriteStatus::Unsupported` for **`m4b` and `mp3`**, exactly as it does for any other
extension (`crates/livrarr-tagwrite/src/lib.rs:168-179`). The reason is in the code: the
upstream writers (`mp4ameta`, `id3`) buffer the shifted media region in RAM when metadata
atoms grow, which OOMs on multi-GB audiobooks; audiobook players read their own metadata DBs,
so embedded tags there are not load-bearing (`:173-176`). `write_m4b` and `write_mp3` survive
as dead code, kept for a possible revival (`:1026-1029`, `:1095-1097`). Unsupported formats
import without tags — not an error.

**Per-file flow** (`crates/livrarr-server/src/tag_service.rs:34-83`):
1. Copy the in-place library file → `{file}.tmp` (`:41-49`)
2. Write tags on `.tmp` (`:51`)
3. `Written` → fsync, then rename `.tmp` → final (`:54-71`)
4. `Unsupported` / `NoData` → delete `.tmp`, return Ok (`:73-77`)
5. Failure → delete `.tmp`, return the error (`:78-81`)

**There is no "re-copy source → final (untagged)" step, and no window where the library file
is missing.** Tagging works on a *copy* of the file already in place; the original is only
ever replaced by a successful rename, so a failure needs no repair.

**Multi-file MP3 audiobooks (TAG-006)** (`tag_service.rs:145-214`): copy every item →
`.tmp`, one `write_tags_batch` call over the `.tmp` set, then rename each into place with
per-file failure handling. A copy failure deletes all `.tmp` files and marks those items
failed (`:180-187`) — again, no re-copy, because the originals were never removed.

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
