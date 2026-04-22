# Library Management

File organization, import, and library item tracking. Primarily `livrarr-library` with `livrarr-tagwrite`.

## Filesystem Layout (Opinionated, Enforced)

```
{root_folder}/
  {user_id}/
    {Author}/
      {Title}.epub          # ebooks: flat layout
      {Title}/              # audiobooks: directory layout
        chapter01.m4b
        chapter02.m4b
```

- Per-user subdirectories within shared root folders
- Author/Title sanitized (illegal chars, length limits, fallbacks)
- Layout is non-negotiable (Principle 7)

## Import Pipeline

1. Download completes (detected by download poller)
2. Files enumerated and classified by extension
3. **Copy** to library (never move or link — Principle 8)
4. Tag writing on the library copy (temp-file-then-rename for safety)
5. CWA downstream copy (hardlink-first, copy fallback)
6. Library item record created in DB

### Tag Writing

- EPUB: rbook (Dublin Core + Calibre series + cover)
- M4B: mp4ameta (iTunes atoms + cover, preserves chapters)
- MP3: id3 (ID3v2 frames + cover, two-pass atomic for multi-file audiobooks)

All tag operations use temp-file-then-rename. Multi-file MP3 audiobooks: write all temps, then rename individually. State machine pattern makes partial completion recoverable.

## File Validation

Uses expected size from grab record. Hash validation deferred (too slow on Raspberry Pi with large audiobooks).

## Manual Scan

Directory traversal for manual import. `confirm_scan` on ImportWorkflow (not FileService) — single import orchestration surface.

## CWA Integration

Calibre-Web Automated downstream copy. Hardlink-first with copy fallback. CWA copy is identical to tagged library copy and is never modified after creation.
