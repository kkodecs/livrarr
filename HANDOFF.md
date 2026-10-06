# Livrarr — current handoff

## Executive summary

**Nothing is in progress.** Chunks 1 to 4 of the work plan are done. Chunk 4, the feature `settings-honesty`, is delivered and closed: the settings pages show only what works, normal users no longer see admin pages, and the server requires 8-character passwords. No PO decision is open. Nothing else is approved; chunk 5 needs the PO's word. The live app on port 8789 runs this feature's build. Livrarr main equals origin/main. The next PM reads `pm-context.md`, [CLAUDE.md](CLAUDE.md) (note the three new settings-honesty lessons), this handoff and the [work plan](build/plans/todo-chunks-2026-10-04.md#executive-summary). See [current work](#current-work), [git and push](#git-and-push), [what is delivered](#what-is-delivered), [open items](#open-items) and [where to read next](#read-next).

## Current work

**None.** `settings-honesty` is delivered and closed (`stage: done`; `verify.py close` passed at 2026-10-06T00:46:05Z, [close log](build/reviews/settings-honesty/pm-verify-close.log)); [status](build/state/STATUS-settings-honesty.md).

**Retro lessons:** the PO approved all [three](build/state/retro-settings-honesty.md#recommendations); they are in `CLAUDE.md` ("Lessons (from settings-honesty retro").

**Next work:** chunk 5 of the [work plan](build/plans/todo-chunks-2026-10-04.md#5-operations-hygiene) is not approved. Scope additions need the PO's explicit word.

## Git and push

- **Livrarr:** main equals origin/main after the PM's push (the PM records the final hash in the commit). This feature's commits are `fcbcb080` (code and tests) and the docs commit that follows it (spec v6 as built, changelog, wiki notes, retro lessons, this handoff). Untracked: `crates/livrarr-server/livrarr.db` and a design-history wiki copy predate this work; two coverage `default_*.profraw` files at the repo root are not ignored (unapproved to-do).
- **kk-build:** unchanged by this feature. `arch-step-dogfood` equals origin at `d52b915`. Five tracked kk-build files are modified and uncommitted (`build/state/escaped-defects.jsonl`, `skills/kk-handoff/SKILL.md`, `wiki/framework/seat-context-check.md`, `wisdom/journal.md`, `wisdom/weaknesses.md`). They predate this work and are not the PM's; do not commit them blind.

## What is delivered

- **settings-honesty** (work-plan chunk 4, 2026-10-06) — the PO's decisions from a sitting, one change each: six placeholder menu items and their pages removed (General, Notifications and Tags stay greyed); the not-found screen says "This page does not exist"; the Light theme button removed; Media Management's Naming and File Management boxes replaced by one accurate "File locations" sentence; Interactive Search enabled on indexers and Automatic Search removed; Test buttons on the Metadata page for Hardcover, Audnexus and the AI connection (they test saved values); an admin-only Unmapped Files menu link and two fixed root-folder links; Manual Import, Readarr Import, Media Management, Status and Logs hidden from normal users, who get the real language list; an 8-character password minimum on the server. The setup wizard hint was dropped: the wizard's later steps are never reached. Astra was the sole reviewer (the PO accepted the spec after round 3; tests round 3, code round 2). Server and web pages were deployed to port 8789 at 2026-10-05T23:58:41Z ([deployment](build/reviews/settings-honesty/deploy-20261005T235801Z/DEPLOYMENT.json)); the PO's live check: "ok looks good". Source: [decisions](build/state/chunk-4-decision-sitting.md#decisions), [spec v6 as built](spec-settings-honesty.md#8-as-built), [non-requirements](spec-settings-honesty.md#4-non-requirements), [retro](build/state/retro-settings-honesty.md#executive-summary), [reviews](build/reviews/settings-honesty/), [settings pages wiki page](wiki/architecture/settings-pages.md).
- **security-before-release** (work-plan chunk 3, closed 2026-10-05) — secrets kept out of every log, Readarr import undo deletes only inside the library, manual import Retry reaches a local download client. Source: [spec v5 as built](spec-security-before-release.md#7-as-built-corrections-and-limits), [retro](build/state/retro-security-before-release.md#executive-summary).
- **silent-failures-2** (work-plan chunk 2, closed 2026-10-05) — six silent failures in the web app now show an error. Source: [spec v6 as built](spec-silent-failures-2.md#7-as-built-corrections-and-limits), [retro](build/state/retro-silent-failures-2.md#executive-summary).
- **Work-plan chunk 1** (2026-10-04) — five kk-build tool fixes, pushed as `d52b915`. Evidence: [review round 3](build/reviews/tooling-fixes-2026-10-04/REVIEW-astra-r3.md).
- Earlier features: see the [steward continuation](build/ops/document-steward/CONTINUE.md).
- Standing PO directions: Gemini and Grok benched, Astra sole reviewer (2026-09-24); let the user edit what they want (2026-09-25); no push without the PO's word; the steward pass runs at every feature close.

## Open items

- GitHub #76 is still open on GitHub; the PM asked the PO whether to close it and has no answer.
- Unapproved candidates are in [TODO](build/plans/TODO.md), grouped by the work plan. They include the [items noticed during settings-honesty](build/plans/TODO.md#noticed-during-settings-honesty-2026-10-05-unapproved) (normal users part 2, the red frontend lint, a possibly flaky log-sink test, the dead indexer hook and others), the unreachable setup-wizard steps, light theme, the type-based secret design, and the audiobook that stays stopped after a stream refresh fails and recovers.
- Files on the parked branch `wip/identity-conflict-authority-stopped` ([proposal](build/reviews/steward-upkeep-2026-09-26/WIP-BRANCH-PROPOSAL.md)).

## Read next

PM: `~/Projects/kk-build/templates/claude-md/pm-context.md`, then [CLAUDE.md](CLAUDE.md), then this handoff and the [work plan](build/plans/todo-chunks-2026-10-04.md#executive-summary) (next move: ask the PO about chunk 5). Feature history: the settings-honesty [session log](build/state/session-log-settings-honesty.md) and [state file](build/state/settings-honesty.yaml); its [decision-sitting handoff](build/state/handoff-chunk-4-settings-honesty.md) is history. Document upkeep: [steward continuation](build/ops/document-steward/CONTINUE.md) and this pass's [receipt](build/reviews/steward-close-settings-honesty-2026-10-06/RECEIPT.md), which lists contradictions for the PM.

The previous root handoff is preserved unchanged at `build/reviews/steward-close-settings-honesty-2026-10-06/predecessors/HANDOFF.md`.
