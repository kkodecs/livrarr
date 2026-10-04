# Livrarr — current handoff

## Executive summary

**Nothing is in progress.** prerelease-trust-pass is closed (2026-10-03), and nothing is unpushed in Livrarr: main equals origin/main at `fdd54cc6`. On 2026-10-04 the PO adopted a nine-chunk [work plan](build/plans/todo-chunks-2026-10-04.md#executive-summary) and approved **chunk 1** (five tool fixes in kk-build) and **chunk 2** (five silent failures in Livrarr) to start together in a fresh PM session. Neither has started. The next PM reads [the chunks 1 and 2 handoff](build/state/handoff-workplan-chunks-1-2.md). Nothing else is approved; scope additions need the PO's explicit word. See [current work](#current-work), [git and push](#git-and-push), [what is delivered](#what-is-delivered), [open items](#open-items) and [where to read next](#read-next).

## Current work

None in progress. Approved and not started (PO "yes go", 2026-10-04):

- **Chunk 1, tool fixes (kk-build):** the tests-gate timeout, the seat-checker false alarm, the close-gate background-cap note, the review recorder's partly-fixed count, and a close-gate check that every expected test binary ran.
- **Chunk 2, silent failures round 2 (Livrarr):** two pop-ups on a failed book delete, audiobook bookmark failures, ignored audio playback failure, the bell badge ignoring failed polls, and the EPUB reader's saved-position load failure.

Nothing is dispatched for either: no state file, packet or seat. Open PO decision: who writes chunk 2's tests (asked once at its start). The PO approved the wisdom-loop guard update, and the PM applied it in kk-build, uncommitted (session log entry 2026-10-04T04:04Z). Chunks 3–9 are not approved; adopting the plan set only the order. Source: [work plan](build/plans/todo-chunks-2026-10-04.md) §1, §2 and [parallel lanes](build/plans/todo-chunks-2026-10-04.md#parallel-lanes); [next PM handoff](build/state/handoff-workplan-chunks-1-2.md).

## Git and push

Livrarr main equals origin/main at `fdd54cc6` (the prerelease-trust-pass retro lessons in `CLAUDE.md`); nothing is unpushed. This steward pass is uncommitted until the PM commits it, and a push waits for the PO's word. The kk-build state is in the [next PM handoff](build/state/handoff-workplan-chunks-1-2.md#current-state).

## What is delivered

- **prerelease-trust-pass** (closed 2026-10-03) — corrected user docs (including the in-app AI help context), the dead `[auth]` keys removed with config warnings in the log, and a one-time setup token on first run. Pushed `59b7e9db..3ccf7db9`, then the lessons commit `fdd54cc6`. Source: [status](build/state/STATUS-prerelease-trust-pass.md), [retro](build/state/retro-prerelease-trust-pass.md), [spec v6](spec-prerelease-trust-pass.md#executive-summary). The [feature handoff](build/state/handoff-prerelease-trust-pass.md) is history.
- **Push and issues, 2026-10-01** — livrarr `43b369c2..59b7e9db` (small-bugs-pass, errors-and-delete-pass, retro commit `59b7e9db`); GitHub #178, #180, #182 and #183 closed.
- **errors-and-delete-pass** (2026-09-30) and its retro follow-ups (2026-10-01), **small-bugs-pass** (2026-09-30) and **identity-upgrade-inplace** (2026-09-29) — delivered and pushed. Earlier features: see the [steward continuation](build/ops/document-steward/CONTINUE.md).
- Standing PO directions: Gemini and Grok benched, Astra sole reviewer (2026-09-24); let the user edit what they want (2026-09-25); no push without the PO's word; the steward pass runs at every feature close.

## Open items

Unapproved candidates are in [TODO](build/plans/TODO.md), grouped by the [work plan](build/plans/todo-chunks-2026-10-04.md), including the PDF reader fix, the delete-with-files folder-swap race (Security) and "Make url_base work" (GitHub #119). Also open: files on the parked branch `wip/identity-conflict-authority-stopped` ([proposal](build/reviews/steward-upkeep-2026-09-26/WIP-BRANCH-PROPOSAL.md)).

## Read next

PM: `~/Projects/kk-build/templates/claude-md/pm-context.md`, then [CLAUDE.md](CLAUDE.md), then [the chunks 1 and 2 handoff](build/state/handoff-workplan-chunks-1-2.md). Document upkeep: [steward continuation](build/ops/document-steward/CONTINUE.md) and this pass's [receipt](build/reviews/steward-upkeep-2026-10-04/RECEIPT.md), which lists contradictions for the PM.

The previous root handoff is preserved unchanged at `build/reviews/steward-upkeep-2026-10-04/predecessors/HANDOFF.md`.
