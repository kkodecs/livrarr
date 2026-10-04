# Livrarr — current handoff

## Executive summary

**Chunk 2 of the work plan is in progress; chunk 1 is done.** Chunk 1 (five tool fixes in kk-build) passed Astra's review on round 3 and is committed in kk-build as `d52b915`, not pushed. Chunk 2 (silent failures, round 2) is the bugfix feature `silent-failures-2`, at the spec stage: spec v2 passed its gate check and Astra's first spec review is running. Livrarr main equals origin/main at `457dc333`; the chunk-2 spec and this steward pass are not committed yet. The next PM reads the [chunk 2 session log](build/state/session-log-silent-failures-2.md) and [state file](build/state/silent-failures-2.yaml). Nothing else is approved; scope additions need the PO's explicit word. See [current work](#current-work), [git and push](#git-and-push), [what is delivered](#what-is-delivered), [open items](#open-items) and [where to read next](#read-next).

## Current work

**Chunk 2, `silent-failures-2` (Livrarr, stage spec).** Scope (PO "yes go", 2026-10-04): two pop-ups on a failed book delete, audiobook bookmark failures, ignored audio playback failure, the bell badge ignoring failed polls, and the ebook reader's saved-place load failure. The PO added the same saved-place fix for the audiobook player. PO decisions: Opus writes the tests and code and Astra reviews; a failed saved-place load shows an error with Retry plus a way out that opens at the start and saves no progress that session. Spec: [`spec-silent-failures-2.md`](spec-silent-failures-2.md#executive-summary) v2 (sha256 `5e9c358c…`), `verify.py spec` PASS; Astra spec review round 1 running. Source: [state](build/state/silent-failures-2.yaml), [session log](build/state/session-log-silent-failures-2.md), [spec packets and reviews](build/reviews/silent-failures-2/).

**Chunk 1, tool fixes (kk-build): done.** The tests-gate timeout, the seat checker after a reset, the review tally, the close-gate background note, and a close-gate check that every test target ran. Astra review: round 1 FAIL, round 2 FAIL, round 3 PASS. Evidence: [build/reviews/tooling-fixes-2026-10-04/](build/reviews/tooling-fixes-2026-10-04/REVIEW-astra-r3.md). The tests-gate fix means chunk 2 should not need a gate bypass.

Chunks 3–9 are not approved; adopting the [work plan](build/plans/todo-chunks-2026-10-04.md#executive-summary) set only the order. The [chunks 1 and 2 handoff](build/state/handoff-workplan-chunks-1-2.md) is history.

## Git and push

- **Livrarr:** main equals origin/main at `457dc333` (the previous root handoff). Uncommitted: `spec-silent-failures-2.md` (in progress) and this steward pass, which the PM commits. Two other untracked files predate this work.
- **kk-build:** `arch-step-dogfood` is one commit ahead of origin (`d52b915`, chunk 1). Other uncommitted kk-build files were left untouched by the PM (session log 2026-10-04T05:15Z). Any push waits for the PO's word.

## What is delivered

- **Work-plan chunk 1** (2026-10-04) — five kk-build tool fixes, committed locally, unpushed.
- **prerelease-trust-pass** (closed 2026-10-03) — corrected user docs (including the in-app AI help context), the dead `[auth]` keys removed with config warnings in the log, and a one-time setup token on first run. Pushed `59b7e9db..3ccf7db9`, then the lessons commit `fdd54cc6`. Source: [status](build/state/STATUS-prerelease-trust-pass.md), [retro](build/state/retro-prerelease-trust-pass.md), [spec v6](spec-prerelease-trust-pass.md#executive-summary).
- **errors-and-delete-pass** (2026-09-30) and its retro follow-ups (2026-10-01), **small-bugs-pass** (2026-09-30) and **identity-upgrade-inplace** (2026-09-29) — delivered and pushed. Earlier features: see the [steward continuation](build/ops/document-steward/CONTINUE.md).
- Standing PO directions: Gemini and Grok benched, Astra sole reviewer (2026-09-24); let the user edit what they want (2026-09-25); no push without the PO's word; the steward pass runs at every feature close.

## Open items

Unapproved candidates are in [TODO](build/plans/TODO.md), grouped by the [work plan](build/plans/todo-chunks-2026-10-04.md), including four items noticed during chunk 2, the PDF reader fix, the delete-with-files folder-swap race (Security) and "Make url_base work" (GitHub #119). Also open: files on the parked branch `wip/identity-conflict-authority-stopped` ([proposal](build/reviews/steward-upkeep-2026-09-26/WIP-BRANCH-PROPOSAL.md)).

## Read next

PM: `~/Projects/kk-build/templates/claude-md/pm-context.md`, then [CLAUDE.md](CLAUDE.md), then the chunk 2 [session log](build/state/session-log-silent-failures-2.md) and [state file](build/state/silent-failures-2.yaml). Document upkeep: [steward continuation](build/ops/document-steward/CONTINUE.md) and this pass's [receipt](build/reviews/steward-upkeep-2026-10-04b/RECEIPT.md), which lists contradictions for the PM.

The previous root handoff is preserved unchanged at `build/reviews/steward-upkeep-2026-10-04b/predecessors/HANDOFF.md`.
