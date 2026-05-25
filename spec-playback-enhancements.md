---
feature: playback-enhancements
stage: spec
status: draft
version: 3
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-010, REQ-011, REQ-012, REQ-013, REQ-014, REQ-015, REQ-016, REQ-017, REQ-018, REQ-019]
---

# Spec: playback-enhancements

## 0a. Fundamental Design Principles

- **KASH-forward compatibility.** All data models (progress, chapters, bookmarks) must accommodate future cross-format synchronization (EPUB CFI ↔ M4B timestamp mapping) with at most additive schema changes (new columns, new tables). No breaking migrations. KASH is explicitly out of scope for this feature, but nothing we build should make it harder.
- **Format B progress display.** Ebook progress is shown as percentage; audiobook progress is shown as time remaining. The visual bar always uses percentage for width.
- **ABS-style progress lifecycle.** Single position field drives both resume and display. All lifecycle thresholds use normalized `progress_pct` (0.0–1.0), which works identically for ebooks (from `location.start.percentage`) and audiobooks (from `currentTime / duration`). Hysteresis: `finished_at` is set when `progress_pct >= 0.98` and cleared when `progress_pct < 0.95`. No high-water mark. `finished_at` timestamp records completion history.
- **M4B-only chapter extraction.** Chapter metadata is parsed from MP4 atoms. Multi-file audiobooks (folder of MP3s) are not supported — no multi-file player exists today.
- **User-scoped personal state.** Progress and bookmarks are per-user. Chapters are per-library-item (shared metadata). All progress/bookmark APIs require authenticated user context. User A cannot see or mutate user B's progress or bookmarks.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | MP4/M4B container spec (ISO 14496-12) | Chapter metadata is stored in `chpl` atom (Nero chapters) or QuickTime chapter track (`trak` with `text`/`sbtl` handler). When both exist, QuickTime takes precedence (richer metadata). | Assuming a single chapter format — must check both locations | Documented |
| ST-002 | epub.js (react-reader) | Location is expressed as EPUB CFI string; `location.start.percentage` provides 0.0–1.0 progress | Storing epub position as page number or raw percentage without CFI — CFI is required for accurate resume | Documented |
| ST-003 | HTML5 Audio API | `currentTime` is a float in seconds; `duration` may be `Infinity` or `NaN` for unseekable streams | Using `duration` without `isFinite()` check | Documented |
| ST-004 | SQLite foreign keys | `ON DELETE CASCADE` on foreign keys deletes child rows when parent is deleted | Manual orphan cleanup for bookmarks/progress when library items are removed | Documented |

## 1. Problem Statement

Livrarr's ebook reader and audiobook player lack features that users of Audiobookshelf and Audible expect: chapter navigation in audiobooks, visible progress on the library page, bookmarks, and proper dark mode in the ebook reader. Mobile users cannot access read/listen buttons because they rely on hover state. These gaps make Livrarr feel like a catalog, not a reading/listening app.

**Who:** Any Livrarr user who reads ebooks or listens to audiobooks through the built-in reader/player.

**If we don't solve it:** Users will continue to use external apps (Audiobookshelf, Moon+ Reader) instead of Livrarr's built-in player, undermining the vision of a unified book management + consumption app.

## 2. Requirements

### Progress Display

- **REQ-001**: The works page must show a progress bar overlay on each work card (poster, table, overview views) for any work with non-zero reading or listening progress. The bar shows the higher progress when both ebook and audiobook progress exist.
- **REQ-002**: The progress text badge must use format B: percentage for ebooks ("47%"), time remaining for audiobooks ("3h 12m left"). When audiobook duration is unknown or non-finite, fall back to percentage. Time remaining formatting: hours + minutes when ≥1h, minutes only when <1h, "<1m" when under 60s. Playback speed does not affect the displayed remaining time on the works page (it's raw remaining, not speed-adjusted).
- **REQ-003**: Progress at or above 98% must display as "Complete" with a full bar.
- **REQ-004**: Progress is tracked per library item, not per work. Each library item has exactly one media type. The `WorkDetailResponse.libraryItems[]` array provides separate entries per format, each carrying its own progress. The `LibraryItemResponse` API payload must include `progress_pct` (0.0–1.0, nullable), `duration_seconds` (nullable float — for audiobooks, sourced from the file's parsed media duration when available, falling back to the work-level `durationSeconds` metadata), and `media_type` (already present) so the works page can compute format-appropriate display text without additional API calls. The works list endpoint must include these fields. The frontend selects the higher `progress_pct` across library items for the card overlay bar, and uses the winning item's `media_type` to determine display format (percentage vs time remaining).
- **REQ-005**: The work detail page must show separate progress bars for ebook and audiobook when both exist.

### Progress Lifecycle

- **REQ-006**: A `finished_at` timestamp must be recorded when `progress_pct >= 0.98`. If `progress_pct` subsequently drops below `0.95`, `finished_at` must be cleared (ABS pattern). This logic uses `progress_pct` for both ebook and audiobook — no format-specific branching. When audiobook duration is non-finite, `progress_pct` cannot be computed from `currentTime/duration`; in that case, do not set `finished_at` automatically.
- **REQ-007**: Deleting a library item must cascade-delete all associated progress records and bookmarks.

### M4B Chapter Extraction

- **REQ-008**: At import time, M4B files must be scanned for chapter metadata. Check QuickTime chapter track first (precedence), then Nero `chpl` atom. Extracted chapters (title + start time in seconds) must be stored in the database, associated with the library item. Chapter end time is derived as the next chapter's start time (or file duration for the last chapter). Chapters must be sorted by start time ascending. Empty titles are stored as "Chapter N" (1-indexed). If chapter metadata is corrupt or unparseable, log a warning and skip (no chapters stored — player falls back to flat timeline). Chapters with non-monotonic or out-of-range start times are discarded with a warning.
- **REQ-009**: A backfill job must extract chapters from all existing M4B library items that haven't been scanned. The job must run asynchronously in the background after server startup. Per-item scan status tracks terminal outcomes: 'scanned' (chapters extracted), 'no_chapters' (valid file, no chapters), 'parse_error' (corrupt — not retried). Items with transient I/O errors remain unscanned (status NULL) and are retried on next startup. No global flag — the job queries for unscanned items directly. Future M4B imports set scan status at import time (REQ-008).
- **REQ-010**: Re-importing an M4B must replace its chapter data. Existing bookmarks must not be deleted (positions are in seconds, independent of chapter metadata).

### Chapter Navigation UI

- **REQ-011**: The audiobook player must display a chapter title row showing the current chapter name, with a button to open the chapter list panel. When no chapters exist, the chapter row, chapter progress bar, chapter tick marks, prev/next chapter buttons, and end-of-chapter sleep option must all be hidden.
- **REQ-012**: The seek bar must display chapter boundary tick marks. The player must show a second, non-interactive chapter progress bar below the main seek bar showing position within the current chapter.
- **REQ-013**: Previous/next chapter buttons must flank the skip buttons. Previous: if >3s into current chapter, jump to chapter start; if ≤3s, jump to previous chapter start. If ≤3s into the first chapter, jump to 0s (start of file). Next: jump to next chapter start. If on the last chapter, the next button is disabled (grayed out).
- **REQ-014**: The chapter list panel must slide in from the right on desktop and appear as a bottom sheet on mobile. It must show completion state (checkmark for chapters whose end time ≤ current position, play icon for the chapter containing current position, blank for future chapters), chapter number, name, and start time. Tapping a chapter jumps to its start. The panel auto-scrolls to the current chapter on open. Chapter completion is position-derived — seeking backward changes checkmarks.

### End-of-Chapter Sleep Timer

- **REQ-015**: The sleep timer popover must include an "End of chapter" option as the first item, shown only when chapter data exists. When active, playback pauses when `currentTime` reaches or exceeds the current chapter's end time (next chapter start, or file duration for last chapter), checked on each `timeupdate` event. If the user skips to a different chapter, the timer follows to the end of the new chapter. On the last chapter, behavior is equivalent to stopping at end of file. The active indicator shows "Sleeping at chapter end" instead of a countdown. When the timer triggers and pauses playback, it is automatically deactivated — the user must re-enable it for the next session.

### Bookmarks

- **REQ-016**: Both the ebook reader and audiobook player must support named position bookmarks, scoped per user. Tapping the bookmark button creates a bookmark at the current position with an auto-generated name (chapter title + position). Interaction: single-click/tap on a bookmark row jumps to that position. Rename via tap-and-hold (mobile) or dedicated pencil icon on the row (desktop). Bookmarks are sorted by position in the book — audiobook bookmarks sort by numeric seconds; ebook bookmarks store `progress_pct` at creation time and sort by that value (EPUB CFI strings are not lexically sortable). Deleting is via hover-reveal X button (desktop) or swipe-left (mobile). The bookmark data model must include `user_id` with `ON DELETE CASCADE` to the users table.
- **REQ-017**: The bookmark data model must include `paired_bookmark_id` (nullable, self-referencing FK with `ON DELETE SET NULL`, bidirectional) for future KASH cross-format pairing. Deleting one bookmark in a pair unpairs the other (sets its `paired_bookmark_id` to NULL) rather than cascade-deleting it. PDF bookmarks are not supported.

### Bug Fixes

- **REQ-018**: The ebook reader's dark mode must apply dark background and text colors to the reader frame/chrome (top bar, surrounding container), not just the epub content. The epub content already renders correctly in dark mode — the bug is that the frame stays light.
- **REQ-019**: Read/listen icons on work cards must be visible on touch devices without hover. Detection uses `@media (pointer: coarse)` (touch-primary devices): always-visible compact pill at bottom-right of cover with backdrop blur, minimum 44px touch targets. On non-touch devices (`pointer: fine`): existing hover overlay preserved.

## 3. UI/Interface Design

- Mockups: `ui/playback-enhancements-mockup.html`
- Key interactions:
  - **Progress overlay**: 3px bar at bottom of poster cover, brand color fill. Text badge in metadata area. Table view: 60px inline bar + text. Overview: bar on cover + inline bar.
  - **Audiobook player**: Chapter title row → chapter panel. Dual progress bars (overall + chapter). Prev/next chapter buttons. Bigger secondary icons (18px in 36px touch targets). Center text between seek timestamps shows chapter position and overall percentage.
  - **Chapter panel**: Desktop right-slide, mobile bottom sheet. Completed/current/future states. Completion is position-derived.
  - **Sleep timer**: "End of chapter" as first option in existing popover. Auto-deactivates after triggering.
  - **Bookmarks**: Panel matching chapter panel design language. Auto-name + tap-and-hold rename (mobile), pencil icon rename (desktop). Single-click jumps.
  - **Mobile icons**: Always-visible pill with bg-black/60 + backdrop-blur. Detected via `pointer: coarse`, not viewport width.
  - **Progress text**: Format B — "47%" for ebook, "3h 12m left" for audiobook.

## 4. Non-Requirements

- **KASH generation and cross-format sync** — explicitly deferred (#23 Part 2). Data models are forward-compatible but no KASH logic is built.
- **Multi-file audiobook playback** — no continuous player for folder-of-MP3s. Chapter extraction is M4B only.
- **PDF bookmarks** — only epub bookmarks supported.
- **PDF reader dark mode** — out of scope (only epub reader fix).
- **Chromecast / external playback** — out of scope.
- **Progress reset UI** — no explicit "mark as unread" button. Seeking backward naturally clears finished state.
- **Chapter tick density adjustment** — no hiding of ticks for high chapter counts.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | No-chapter audiobook behavior | resolved | Flat timeline, all chapter UI hidden |
| Q-002 | Which audio formats for chapter extraction | resolved | M4B only |
| Q-003 | Multi-file audiobook chapters | resolved | Out of scope — no multi-file player |
| Q-004 | Re-import chapter behavior | resolved | Chapters replaced, bookmarks survive |
| Q-005 | Duplicate chapter names | resolved | Display as-is |
| Q-006 | Chapter tick density for 200+ chapters | resolved | No adjustment for now |
| Q-007 | Chapter update on skip across boundary | resolved | On next timeupdate tick (whatever's easier) |
| Q-008 | Re-read progress handling | resolved | ABS pattern: seek backward clears finished_at, no high-water mark |
| Q-009 | Complete threshold | resolved | 98%+ |
| Q-010 | Work detail with both formats | resolved | Shows both bars separately |
| Q-011 | Null audiobook duration | resolved | Fall back to percentage |
| Q-012 | Bookmark cascade on item delete | resolved | Cascade delete |
| Q-013 | Bookmark sort order | resolved | Position in book |
| Q-014 | End-of-chapter on last chapter | resolved | Stop at end of file |
| Q-015 | End-of-chapter + chapter skip | resolved | Timer follows to new chapter |
| Q-016 | Paired bookmark directionality | resolved | Bidirectional — both point to each other |
| Q-017 | Multi-file audiobook support | resolved | Out of scope — no multi-file player |
| Q-018 | Chapters for existing library items | resolved | One-time backfill job |
| Q-019 | PDF bookmarks | resolved | Not supported |
| Q-020 | Work detail progress display | resolved | Two bars with labels |
| Q-021 | Progress API approach | resolved | Include progress_pct + duration_seconds on LibraryItemResponse |
| Q-022 | Progress deletion mechanism | resolved | Cascade delete only |
| Q-023 | Bookmark click vs rename conflict | resolved | Single-click jumps, pencil icon renames (desktop), tap-and-hold renames (mobile) |
| Q-024 | Paired bookmark deletion | resolved | ON DELETE SET NULL — unpair, don't cascade |
| Q-025 | Chapter completion semantics | resolved | Position-derived: chapters ending before currentTime show checkmark |
| Q-026 | Sleep timer after trigger | resolved | Auto-deactivates after pausing playback |
| Q-027 | Chapter boundary button edges | resolved | First chapter ≤3s → jump to 0s; last chapter next → disabled |
| Q-028 | Touch detection mechanism | resolved | @media (pointer: coarse), not viewport width |
| Q-029 | Chapter extraction precedence | resolved | QuickTime > Nero; corrupt → skip with warning |
| Q-030 | Backfill job lifecycle | resolved | Async after startup, flag prevents re-run |
| Q-031 | Progress lifecycle format-specific logic | resolved | All thresholds use normalized progress_pct, no format branching |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Works page poster/table/overview views show a progress bar overlay for works with non-zero progress. Bar width reflects percentage. When both formats have progress, the higher value is used.
- [ ] **AC-002** (REQ-002, REQ-003): Ebook progress text shows "47%". Audiobook shows "3h 12m left" (≥1h), "23m left" (<1h), "<1m" (<60s). Unknown duration falls back to percentage. 98%+ shows "Complete".
- [ ] **AC-003** (REQ-004): Works list and work detail API responses include `progress_pct`, `duration_seconds`, and `media_type` on each `LibraryItemResponse`. Null `progress_pct` when no progress exists.
- [ ] **AC-004** (REQ-005): Work detail page shows separate ebook and audiobook progress bars when both exist.
- [ ] **AC-005** (REQ-006): Setting `progress_pct >= 0.98` records `finished_at`. Subsequently setting `progress_pct < 0.95` clears `finished_at`. Non-finite audio duration prevents automatic `finished_at`.
- [ ] **AC-006** (REQ-007): Deleting a library item removes all associated `playback_progress` and `bookmark` rows via cascade.
- [ ] **AC-007** (REQ-008): Importing an M4B with Nero or QuickTime chapters populates the `audiobook_chapters` table with title + start_time_secs for each chapter, sorted ascending. QuickTime chapters take precedence over Nero. Corrupt metadata results in zero chapters and a logged warning.
- [ ] **AC-008** (REQ-009): Backfill job runs async after startup, extracts chapters from all unscanned M4B items (chapter_scan_status IS NULL, media_type = audiobook). Sets per-item scan status on terminal outcomes. Transient failures remain unscanned for retry.
- [ ] **AC-009** (REQ-010): Re-importing an M4B replaces chapter rows. Bookmark rows for the same library item are unaffected.
- [ ] **AC-010** (REQ-011): Audiobook player with chapters shows chapter title row, chapter progress bar, chapter ticks, prev/next buttons, end-of-chapter sleep option. Without chapters, all chapter-specific UI is hidden.
- [ ] **AC-011** (REQ-012): Seek bar displays vertical tick marks at each chapter boundary. A thin non-interactive progress bar below shows position within the current chapter.
- [ ] **AC-012** (REQ-013): Previous chapter button: >3s into chapter → chapter start; ≤3s → previous chapter (or 0s if first chapter). Next chapter button → next chapter start; disabled on last chapter.
- [ ] **AC-013** (REQ-014): Chapter panel: right slide-in on desktop, bottom sheet on mobile. Shows checkmark (end ≤ currentTime), play-icon (current), blank (future). Tap jumps to chapter. Auto-scrolls to current.
- [ ] **AC-014** (REQ-015): Sleep timer "End of chapter" option appears only when chapters exist. Pauses at chapter end. Follows on chapter skip. Shows "Sleeping at chapter end" label. Auto-deactivates after triggering.
- [ ] **AC-015** (REQ-016): Bookmark button creates a named bookmark. Single-click jumps. Pencil icon (desktop) or tap-and-hold (mobile) renames. Sorted by position. Deletable. All bookmarks scoped to authenticated user.
- [ ] **AC-016** (REQ-017): Bookmark table includes `paired_bookmark_id` (nullable FK to self, ON DELETE SET NULL). Not populated by this feature but schema supports bidirectional pairing.
- [ ] **AC-017** (REQ-018): In ebook dark mode, the reader frame (top bar, container div) uses dark background (#18181b), matching the epub content.
- [ ] **AC-018** (REQ-019): On `pointer: coarse` devices, read/listen icons appear as an always-visible pill (≥44px touch targets) at bottom-right of the cover. On `pointer: fine`, existing hover overlay is preserved.
- [ ] **AC-019** (REQ-016, REQ-007): User A's bookmarks and progress are not visible to user B. Deleting user A's account cascades to their bookmarks and progress.
