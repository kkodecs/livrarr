---
feature: "cross-format-resume"
stage: spec
status: delivered
version: 4
# As-built 2026-06-09: all 18 REQ delivered; PO-validated live (DC Carl).
# Operational deviations are recorded in ir-v2 (4 AS-BUILT amendments).
# Deferred (logged, not silently dropped):
#   - Link establishment for ALREADY-imported audiobooks (sidecar added later)
#     has no rescan path — links form only on import. Follow-up: rescan/backfill
#     reconciliation.
#   - kash generation MUST use the library's epub as input (livrarr rewrites
#     EPUB tags on import; pre-import sources never hash-match) — kash_gen-side.
#   - Audiobook "Sync to here" is always visible (server no-ops on unlinked)
#     rather than link-gated — accepted UX deviation.
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-010, REQ-011, REQ-012, REQ-013, REQ-014, REQ-015, REQ-016, REQ-017, REQ-018]
---

# Spec: cross-format-resume

Two QoL features under one theme — **don't lose your place**:
- **2A. Cross-format resume** — switch between reading and listening; the format you open offers to jump to your furthest place in the other (Amazon Whispersync model). Symmetric in both directions.
- **2B. Sleep-timer auto-bookmark** — turning on the sleep timer drops a bookmark at where you were, so you can jump back to the last point you were awake.

Both lean on the existing KASH sidecar (2A) and the existing bookmark + sleep-timer plumbing (2B).

> **v4** resolves round-3 review: accept honors the exact target shown (no silent re-target); audio drift uses **container duration only** (file size dropped — benign tag edits change it); boundary resolves to the **last anchor**, never "the end" (no skipped tail); "sync to here" stores the nearest anchor; per-user isolation AC added; Q-001 no longer implies the existing per-file progress API already does per-link storage.

## 0a. Design Principles

- **Whispersync parity (2A).** Furthest-position high-water mark per user/link; prompt-to-jump when the opened format is behind; never auto-move backward. **Symmetric** — both directions (ebook→audiobook *and* audiobook→ebook).
- **Progress, not raw position (2A).** The furthest mark advances only from genuine reading/listening progress, not from manual scrubs/jumps; a manual "sync to here" is the explicit override.
- **Approximate-but-instant (2A).** Resume lands at the nearest KASH anchor (a paragraph / a few seconds). Sub-anchor precision is a non-goal; imprecision must never cause a backward jump or skip unanchored content.
- **Cheap-validate, never block (2A).** File identity uses cheap metadata only; no whole-file audio hashing, ever, in v1.
- **Reuse existing infrastructure.** Build on the existing server-side playback/reading progress and the existing bookmark store. Introduce no new bookmark persistence.
- **Non-disruptive automation (2B).** The sleep-timer bookmark never interrupts playback and is fully corrigible via the existing bookmark UI (rename/delete).

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | KASH format | Anchors map EPUB CFI ↔ audio timestamp, ordered and monotonic in reading order; sampled (~1 per 4–18s, **not** every paragraph) | Claiming sub-anchor precision; assuming every paragraph/second has an anchor | High |
| ST-002 | KASH file | Carries `epub_hash` + `audio_hash` (SHA-256) + `duration_seconds`. `epub_hash` identifies the ebook (small file, cheap to verify); `audio_hash` is **provenance only** — v1 does not recompute it. | Recomputing the m4b SHA-256 to validate identity | High |
| ST-003 | `EpubReader.tsx` (react-reader = epub.js) | Ebook reader addresses position by EPUB CFI | Assuming a shared coordinate between formats without translation | High |
| ST-004 | `AudioPlayer.tsx` | Audiobook position (seconds) is persisted **server-side** via the playback-progress API, keyed **per `libraryItemId`** (not per link) | Assuming the existing API already stores a per-link value; client-only/localStorage position | High |
| ST-005 | `AudioPlayer.tsx` | A bookmark store exists: CRUD over `/workfile/{id}/bookmarks` & `/bookmarks/{id}`; fields `position`, `sortKey`, `name`, `chapterTitle` | Building a new bookmark store | High |
| ST-006 | `AudioPlayer.tsx` | The audiobook sleep timer has **two** activation paths: timed durations via `startSleepTimer(minutes)` **and** end-of-chapter via the `sleepAtChapterEnd` state. Position is known at activation in both. | Hooking only `startSleepTimer` (would miss end-of-chapter) | High |
| ST-007 | Browser platform | `Date` provides device-local wall-clock time + timezone with no permission prompt | Assuming authoritative/server time for the bookmark label | High |
| ST-008 | Library files | m4b files are large (multi-GB); full SHA-256 is expensive and has caused OOM-adjacent pain | Hashing the m4b at any point in v1 | High |
| ST-009 | MP4/ISO-BMFF format | An `.m4b` has **no whole-file checksum** of itself, but its **container duration** is readable without reading the whole file. | Expecting the m4b to self-identify by hash; reading the full file to confirm identity | High |

## 1. Problem Statement

Users consume the same book in both formats and move between reading and listening. Two gaps today:

1. Each format remembers only its own place. Switching means manually finding your spot in the other format.
2. When you fall asleep listening, the saved position is wherever the audio happened to stop — not where you were last awake — so you lose your place.

This feature (2A) provides Whispersync-style cross-format resume, and (2B) auto-creates a bookmark when you turn on the sleep timer.

## 2. Requirements

### 2A. Cross-format resume

- **REQ-001**: On library scan, when an audiobook file has a sibling `.kash` (same stem), the system records its presence and path, associated with that audiobook LibraryItem.
- **REQ-002**: A `.kash` binds exactly **one** audiobook LibraryItem to exactly **one** ebook LibraryItem (`epub_hash` identifies the ebook). A work may contain multiple such links (multiple editions); each link is independent. If no on-disk ebook in the work matches `epub_hash`, no link is established.
- **REQ-003**: For an established `.kash` link, the system maintains, **per user per link**, a "furthest position" across that link's ebook and audiobook, ordered using the `.kash` alignment. It advances **only from genuine reading/listening progress** (playback ticks / page turns); a manual seek, scrub, or navigation jump never advances it.
- **REQ-004**: When the user opens either format and the furthest position resolves to an anchor **strictly ahead of** the current position, the system presents a prompt offering to jump to that anchor, showing a **human-readable** target (audiobook timestamp, or ebook chapter + percentage — never a raw CFI). No prompt is shown when the resolved target is at or behind the current position. Position is not changed without explicit confirmation.
- **REQ-005**: If the user accepts, the opened format resumes at the resolved target anchor. If the user declines, the current position is unchanged and the furthest position is not reset.
- **REQ-006**: The system never moves a format's saved position backward automatically.
- **REQ-007**: Cross-format resume is offered only for a **validated** `.kash` link: both items present, `epub_hash` matches the ebook, and the audiobook passes the cheap identity check (REQ-014). Otherwise each format retains its own independent position and no cross-format prompt appears.
- **REQ-008**: A link whose files no longer match on disk — `epub_hash` mismatch, or audio drift per REQ-014 — is treated as absent for translation; the affected pair falls back to independent per-format positions.
- **REQ-009**: Library scanning does not read whole audio files or compute any audio hash — scan performance is not degraded by per-file multi-GB work.
- **REQ-014**: Audio-side identity and drift for a `.kash` link are determined by **cheap metadata only** — the m4b's container duration (read from the header) compared against `.kash` `duration_seconds`. v1 **does not** compute the m4b's SHA-256, and does not use file size (which benign tag/cover edits would change).
- **REQ-015**: Position translation resolves to the nearest anchor at or before the target; before the first anchor resolves to the start; at or beyond the last anchor resolves to the **last anchor** (never skipping unanchored tail content). The **resolved jump target is never behind the opened format's current position** — if the nearest qualifying anchor would be behind current, no forward jump is offered.
- **REQ-016**: The furthest position is per-user, per-link, and advances **monotonically from progress only**. Accepting a jump goes to the **exact target anchor shown in the prompt** (always a valid forward move); the prompt target is not silently re-pointed. Only a manual "sync to here" (REQ-018) may move the furthest to a non-advancing position.
- **REQ-017**: After the user declines a jump for a format, the prompt does not reappear for that format until the furthest position advances beyond its value at decline time.
- **REQ-018**: The user can manually **"sync to here"** — set the link's cross-format position to the **nearest anchor at or before** their current spot (consistent with how furthest positions are compared). This overrides the furthest position (including downward) and is the escape valve for skipping to the end or deliberately re-reading.

### 2B. Sleep-timer auto-bookmark

- **REQ-010**: When the user turns on the audiobook sleep timer — **either** a timed duration **or** end-of-chapter mode (ST-006) — the system automatically creates a bookmark at the playback position at the moment of activation.
- **REQ-011**: The auto-created bookmark is named `Sleep Timer / <date> @ <time>`, using the user's local date and time at creation.
- **REQ-012**: Auto-bookmark creation does not interrupt playback. It is **deduplicated**: no new sleep-timer bookmark is created if one already exists for the same workfile (name prefixed `Sleep Timer / `) whose position is within 60 seconds of the current position.
- **REQ-013**: The auto-created bookmark is an ordinary bookmark — it appears in, and is renamable/deletable via, the existing bookmark UI, and selecting it seeks to its position. There is **no automatic cleanup** in v1; sleep-timer bookmarks are user-prunable, and REQ-012 prevents same-position accumulation.

## 3. UI/Interface Design

- **Cross-format resume (2A):** a single **non-modal, dismissible** "jump to furthest position" prompt shown on opening the format that is behind, mirroring Amazon Whispersync's resume dialog — shows the human-readable target and offers **Jump** / **Stay**. Declining ("Stay") suppresses the prompt for that format until the furthest position advances again (REQ-017). A **"sync to here"** action (REQ-018) resets the link's position to the current spot. No bespoke mockup required (mirror Whispersync); flag if one is wanted.
- **Sleep-timer bookmark (2B):** no new UI surface — reuses the existing bookmark side-panel. A subtle, non-blocking toast confirms creation.

## 4. Non-Requirements

- Live read-along / synchronized highlighting (explicitly superseded by this feature).
- Tap-to-seek during playback.
- KASH generation (`kash_gen.py`, GPU) — deferred; v1 consumes existing `.kash` only.
- Sub-anchor / sentence-level precision.
- Exact audio-content verification (v1 trusts the cheap container-duration identity check; no m4b SHA-256, no file-size check).
- Real-time multi-device sync (v1 handles races via monotonic furthest + honoring the shown target — REQ-016).
- Automatic cleanup/expiry of sleep-timer bookmarks (user-prunable; REQ-013).
- Syncing positions or bookmarks to external services (Amazon/Audible/etc.) — livrarr-local only.
- mp3 audiobook alignment (KASH targets m4b; mp3 deprioritized).
- A sleep-timer bookmark on the ebook side (sleep timer is audiobook-only).

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Server-side storage for the per-user, per-link furthest position. | resolved | Architecture must define it — the existing playback-progress API is **per-LibraryItem and does not cover per-link high-water** (ST-004); new storage (likely adjacent to that API) is required, preserving existing per-LibraryItem behavior. |
| Q-002 | Does the ebook reader persist reading position server-side the way the audiobook player does? | resolved | Assumed server-side, mirroring ST-004; Architecture confirms `EpubReader.tsx` and adapts if it differs. |
| Q-003 | Ebook-side display of the jump target (CFI is opaque to users). | resolved | Show chapter + percentage (REQ-004); never the raw CFI. |
| Q-004 | Stale-`.kash` UX — surface "alignment outdated" vs silently fall back. | resolved | v1: silent fallback to independent positions; regeneration is deferred scope. |
| Q-005 | Sleep-timer bookmark label format (locale/format of "date @ time"). | resolved | Device-local time; exact format flexible, not load-bearing. |
| Q-006 | What counts as "genuine progress" vs a manual jump for advancing the furthest mark, including consumption right after a forward seek? | resolved | Architecture defines the signal (progress from playback ticks / page-turn events, not seek/scrub); a forward seek does not advance the mark, and the "sync to here" override (REQ-018) backstops any misclassification. |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Given an `.m4b` with a sibling `Title.kash`, after a scan the work reports the kash present with its path.
- [ ] **AC-002** (REQ-002): Given a `.kash` whose `epub_hash` matches an on-disk ebook in the work, a link is established to that specific ebook item; given no matching ebook, no link is established.
- [ ] **AC-003** (REQ-002, REQ-003): In a work with two independent links, progress in one edition pair does not trigger a prompt in the other (furthest marks are isolated per link).
- [ ] **AC-004** (REQ-003, REQ-004): After listening (genuine progress) to position T, opening the ebook where T resolves to an anchor ahead of the current spot shows a jump prompt to that anchor.
- [ ] **AC-005** (REQ-005): Accepting the prompt moves the ebook to the resolved target anchor (ahead of its prior position).
- [ ] **AC-006** (REQ-005, REQ-017): Declining leaves the position unchanged; reopening with no change to the furthest position shows **no** prompt; once the furthest position advances, the prompt returns.
- [ ] **AC-007** (REQ-004, REQ-006, REQ-015): When the furthest position resolves to an anchor at or behind the opened format's current position, **no** prompt is shown and the position is not moved (no backward jump), including the in-gap case where current sits past the nearest preceding anchor.
- [ ] **AC-008** (REQ-007): A work with only an ebook (no audiobook, or no validated `.kash` link) shows no cross-format prompt; its position persists independently.
- [ ] **AC-009** (REQ-008, REQ-014): When the linked `.m4b`'s container duration no longer matches `.kash` `duration_seconds`, the link is invalidated, no cross-format prompt appears, and positions remain independent.
- [ ] **AC-010** (REQ-009, REQ-014): A library scan over a directory of large `.m4b` files completes without reading the whole files or computing any audio hash.
- [ ] **AC-011** (REQ-015): A furthest position before the first anchor resolves to the start; at or beyond the last anchor resolves to the **last anchor** (no skipped tail content).
- [ ] **AC-012** (REQ-016): If the furthest position advances (e.g. from another device) after the prompt is shown, accepting still jumps to the **exact target shown** in the prompt — never a silently different location, never backward.
- [ ] **AC-013** (REQ-004): The prompt shows a human-readable target — audio direction a formatted timestamp, ebook direction a chapter label + percentage; the raw CFI is never displayed.
- [ ] **AC-014** (REQ-004, REQ-005): Reverse direction is symmetric — after **reading** ahead in the ebook to position P, opening the **audiobook** where P resolves ahead shows a jump prompt; accepting seeks the audio to the resolved anchor.
- [ ] **AC-015** (REQ-003): A manual forward seek/scrub (e.g. to the end) does **not** advance the furthest position; only genuine progress does.
- [ ] **AC-016** (REQ-018): Invoking "sync to here" sets the link's furthest position to the nearest anchor at or before the current spot, even if that is behind the prior furthest.
- [ ] **AC-017** (REQ-010, REQ-011): Turning on a 30-minute sleep timer at position T creates a bookmark at T named `Sleep Timer / <today's local date> @ <current local time>`.
- [ ] **AC-018** (REQ-010): Enabling **end-of-chapter** sleep at position T creates exactly one `Sleep Timer / …` bookmark at T, and playback continues until chapter end.
- [ ] **AC-019** (REQ-012): Turning the timer on twice within 60 seconds (position) at ~the same spot creates only one sleep-timer bookmark, and playback is not interrupted.
- [ ] **AC-020** (REQ-013): The created bookmark appears in the bookmark panel, seeks to its position when selected, and is renamable/deletable.
- [ ] **AC-021** (REQ-003, REQ-016): Two users on the same established `.kash` link are isolated — user A's progress does not trigger a prompt for, or alter the positions of, user B.
