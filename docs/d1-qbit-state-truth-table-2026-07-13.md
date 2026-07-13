# D1 — qBittorrent state truth table (RATIFIED)

Status: RATIFIED by the PO 2026-07-13 (morning session), as folded. IMPLEMENTED by
quality-waves group 2a (same day): `classify_qbit_state`
(`crates/livrarr-download/src/lib.rs`) is the ONE classifier producing both projections
(`QbitStateClassification { ui_status, import_safe }`); consumers are the download
poller's import gate (`poll_qbittorrent`) and `queue_service::fetch_qbit_progress`
(which now serves the canonical `ui_status` vocabulary in `download_status` instead of
the raw qBit state string). Both legacy classifiers are deleted. Table-driven pins:
`tests/behavioral/test_qw2_class_a_pins.rs` (`qw2a_*`).

Sources at authoring: qBittorrent 5.0 WebUI API documentation (state table, fetched
2026-07-13), `map_qbit_state`, `is_completed_state`. Cross-family verification: see the
review record referenced in the journey doc.

> **Implementation-time correction (2026-07-13):** `map_qbit_state` turned out to have
> ZERO production callers — the queue UI never consumed any state classification; it
> received the raw state string via `QueueProgress.download_status`, which the frontend
> declares but does not read. The table below remains accurate about the two *functions*;
> the live wrong-behavior exposure was `is_completed_state`'s `checkingResumeData` row
> (import trigger). 2a gave the UI projection its first live consumer.

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

## PO decision — resolved

Ratified as folded 2026-07-13 (the two former open picks — checkingUP and moving — carry
the cross-family-agreed values). Group 2a implemented the single shared classifier the
same day; see the Status block at the top for the as-built shape.
