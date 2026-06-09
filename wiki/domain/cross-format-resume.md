# Cross-Format Resume (kash links)

Whispersync-model position sync between an ebook and its audiobook. Delivered 2026-06-09 (commit d6b869d). Spec/IR at repo root (`spec-cross-format-resume.md`, `ir-v{1,2}-cross-format-resume.yaml`).

## Model

- **Coordinate = audio timestamp (f64 seconds).** Never percentage (ebook% ≠ audio%), never CFI-vs-seconds comparison. Ebook positions map CFI→ts client-side via served `.kash` anchors + epub.js `EpubCFI.compare`.
- **`.kash` sidecar** sits next to the m4b (`<stem>.kash`): JSON with `version:1`, `epub_hash` (sha256 of the ebook), `audio_hash` (provenance only — NEVER recomputed), `duration_seconds`, `chapters[]`, `alignment[]` (cfi↔ts anchors, ~1 per 4–18s).
- **1:1 links** (`kash_links`: UNIQUE audio_item_id, UNIQUE ebook_item_id). Per-(user, link) state in `cross_format_state`: monotonic `furthest_ts` + per-format decline thresholds. Migration 058.
- **Furthest advances only from genuine progress** (`ProgressKind::Progress` + finite `cross_format_ts`), atomically inside `PlaybackProgressDb::upsert_progress` (one SQLite tx, `MAX()` in SQL — race-safe). `Seek` never advances; serde default for a missing `kind` is `Seek` so stale clients can't poison the mark. Manual "sync to here" is the explicit override (may decrease; clears declines).
- **Validation is read-side only** (prompt/anchors/sync re-validate per open: stored duration vs sidecar ±2.0s, epub re-hash) and **scan-side reconciled**: a rescan with the sidecar absent or duration-mismatched DELETES the link (state cascades) — closes the stale-mark poison window. The m4b is NEVER read or hashed (audio identity = stored container duration only).

## Key code

- `livrarr-domain/src/kash.rs` — pure parse/lookup/resolve. Parse NORMALIZES equal-ts anchor runs (final anchor wins; real generators emit same-second ties — 15/4003 in the first production sidecar); only ts DECREASES reject.
- `livrarr-library/src/cross_format_service.rs` — `CrossFormatServiceImpl<D, F>` (validation, prompt, anchors, decline, sync). Audio-side kash path is derived by root-join (NOT `resolve_path` — it canonicalizes, which stats the m4b).
- `livrarr-library/src/import_workflow.rs` — `establish_kash_link` (free fn) + `extract_chapters_and_kash` hook on ALL THREE import paths (grab ×2, manual via `extract_chapters_for_item`).
- `livrarr-handlers/src/cross_format.rs` — 4 routes under `/workfile/{id}/cross-format/*`; all link-absent/stale errors map to 404 (the reader treats 404 as "no cross-format here").
- Frontend: `utils/kashAnchors.ts` (CFI→ts binary search), `components/ResumePromptBanner.tsx`, both readers integrate. Ebook seek-vs-progress classification uses a TIME SETTLE WINDOW (`jumpUntilRef`: mount+3s, programmatic jumps +1.5s) — epub.js fires multiple `relocated` per jump and react-reader's own arrows bypass our handlers, so consume-once flags don't work.
- Behavioral suite: `tests/behavioral/test_cross_format_resume.rs` (49 tests) — GITIGNORED like all tests/; lives locally, `[[test]]` entry deliberately NOT committed.

## Gotchas (hard-won)

1. **livrarr rewrites EPUB tags on import → a kash generated from the pre-import epub NEVER hash-matches the library copy.** kash_gen must take the library's file as input. (First production sidecar needed a hash patch for this reason.)
2. **No rescan path establishes links for already-imported audiobooks** — drop a sidecar next to an existing m4b and nothing links it until a re-import. Open product gap; dev workaround was a direct `kash_links` insert.
3. **Generator anchors can tie on ts and slightly overshoot duration_seconds** (last anchor past the end). Parse handles both; don't re-strictify.
4. The audiobook "Sync to here" button is always visible (server 404→no-op when unlinked); the ebook one is anchors-gated.

## Sleep-timer bookmark (2B, frontend-only)

Both activation paths (timed + end-of-chapter) drop a bookmark `Sleep Timer / <local date> @ <time>` at the activation position, deduped within 60s (bookmark list + in-flight ref), sonner toast, never blocks playback. Ordinary bookmark — renamable/deletable, no auto-cleanup.
