# D1 — qBittorrent state truth table (DRAFT for PO ratification)

Status: DRAFT authored 2026-07-13 (quality-waves overnight run). Sources: qBittorrent 5.0
WebUI API documentation (state table, fetched 2026-07-13), `map_qbit_state`
(`crates/livrarr-download/src/lib.rs:334-348`), `is_completed_state`
(`crates/livrarr-server/src/jobs/download_poller.rs:828-839`). Cross-family verification:
see the review record referenced in the journey doc. **Not ratified — Wave 2 group 2a
(one shared classifier consumed by both the queue UI and the import trigger) implements
only after PO sign-off.**

## The two live classifiers disagree today

| state | UI status (`map_qbit_state`) | import trigger (`is_completed_state`) | qBit docs meaning |
|---|---|---|---|
| `checkingResumeData` | Queued | **completed → triggers import** | "Checking resume data on qBt startup" — a startup transient that applies to torrents in ANY completion state |
| `checkingUP` | Completed | not completed | "finished downloading and is being checked" |
| `stoppedUP` (qBit 5.x rename of pausedUP) | **Warning** (unmatched fallback) | completed | finished + stopped |
| `stoppedDL` (5.x rename of pausedDL) | **Warning** (unmatched fallback) | not completed | paused, NOT finished |

The dangerous one is `checkingResumeData`: on a qBt restart with a half-downloaded torrent,
the poller can classify it completed and trigger import of incomplete files. The `stopped*`
gaps are 5.x renames `map_qbit_state` never learned — affected torrents show Warning in the
queue UI.

## Proposed truth table (one table, two projections)

"Import-safe" = data fully downloaded AND files at their final, stable path.

| state | docs meaning | UI status | import-safe |
|---|---|---|---|
| `downloading` | active download | Downloading | no |
| `stalledDL` | downloading, no peers | Downloading | no |
| `forcedDL` | forced download | Downloading | no |
| `metaDL` | fetching metadata | Queued | no |
| `forcedMetaDL` | forced metadata fetch | Queued | no |
| `allocating` | allocating disk space | Queued | no |
| `queuedDL` | queued for download | Queued | no |
| `checkingDL` | checking, NOT finished | Queued | no |
| `checkingResumeData` | startup resume check (any completion state) | Queued | **no** (fixes the live bug) |
| `pausedDL` / `stoppedDL` | paused, not finished | Paused | no |
| `uploading` | seeding, transferring | Completed | **yes** |
| `stalledUP` | seeding, no connections | Completed | **yes** |
| `forcedUP` | forced seeding | Completed | **yes** |
| `queuedUP` | queued for upload | Completed | **yes** |
| `pausedUP` / `stoppedUP` | finished + paused/stopped | Completed | **yes** |
| `checkingUP` | finished, being verified | Queued | **no — wait for the check to finish** (Readarr precedent: a failing check must not read as completed; UI Queued keeps the pair consistent) |
| `moving` | relocating to another path | Downloading | **no — path unstable; import next tick** (Readarr groups moving with in-progress; both review families flagged Completed as misleading) |
| `missingFiles` | data files missing | Warning | no |
| `error` | error state | Error | no |
| `unknown` / unmatched | unknown | Warning | no |

## Cross-family verification (2026-07-13, review-design-*-r1.json + dispositions)

Both families returned real verdicts (gemini FAIL / codex PASS); three findings FOLDED into
the table above: `forcedMetaDL` row added (codex — real API state the docs table omits);
`checkingUP` UI moved Completed→Queued (codex — Readarr precedent, kills the split-brain);
`moving` UI moved Completed→Downloading (both families independently — Readarr groups it
with in-progress). Refuted/covered: gemini's P0 ("pausedUP/stoppedUP can be incomplete")
contradicts the API doc's explicit definitions (the UP/DL suffix IS the completion axis:
pausedUP = "paused and has finished downloading"; an incomplete stopped torrent is
stoppedDL); gemini's bare-`checking` state is not in the modern API enumeration and any
unenumerated state already lands in the unmatched fallback row (Warning, not import-safe).

CAVEAT for ratification (real qBit nuance, pre-existing, not introduced here): "complete"
in all UP states means the SELECTED files are complete — a torrent with deselected files
reads complete while unselected pieces are absent. The import path already operates on
what exists at content_path; the table does not change that exposure.

## Remaining PO decision

Ratify the table as folded (the two former open picks — checkingUP and moving — now carry
cross-family-agreed values). On ratification, Wave 2 group 2a implements: ONE shared
classifier producing (UI status, import-safe) consumed by both the queue endpoint and the
download poller.
