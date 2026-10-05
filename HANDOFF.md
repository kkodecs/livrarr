# Livrarr — current handoff

## Executive summary

**Nothing is in progress.** Chunks 1 to 3 of the work plan are done. Chunk 3, the feature `security-before-release` (as trimmed by the PO), is delivered and closing: the PM runs `verify.py close` and then pushes. No PO decision is open. Nothing else is approved; chunk 4 needs the PO's word. The live app on port 8789 runs this feature's build. After the PM's push, Livrarr main equals origin/main. The next PM reads `pm-context.md`, [CLAUDE.md](CLAUDE.md), this handoff and the [work plan](build/plans/todo-chunks-2026-10-04.md#executive-summary). See [current work](#current-work), [git and push](#git-and-push), [what is delivered](#what-is-delivered), [open items](#open-items) and [where to read next](#read-next).

## Current work

**None.** `security-before-release` is delivered and closing: the state records `stage: deploy` until the PM's `verify.py close` passes; [status](build/state/STATUS-security-before-release.md).

**Retro lessons:** the PO approved all [three](build/state/retro-security-before-release.md#recommendations); they are in `CLAUDE.md` ("Lessons (from security-before-release retro").

**Next work:** chunk 4 of the [work plan](build/plans/todo-chunks-2026-10-04.md#executive-summary) is not approved. Scope additions need the PO's explicit word.

## Git and push

- **Livrarr:** after the PM's push, main equals origin/main (the PM records the final hash in the commit). This feature's commits are `2987ba08` (the PII commit hook allows reserved example domains), `497b391e` (code and tests) and `1dc08796` (spec v5 as built, changelog, wiki notes, retro lessons). Untracked: `crates/livrarr-server/livrarr.db` and a design-history wiki copy predate this work; coverage `*.profraw` files at the repo root appeared on 2026-10-05 and are not ignored.
- **kk-build:** `arch-step-dogfood` equals origin at `d52b915`. Five tracked kk-build files are modified and uncommitted (`build/state/escaped-defects.jsonl`, `skills/kk-handoff/SKILL.md`, `wiki/framework/seat-context-check.md`, `wisdom/journal.md`, `wisdom/weaknesses.md`). They predate this work and are not the PM's; do not commit them blind.

## What is delivered

- **security-before-release** (work-plan chunk 3, 2026-10-05) — three fixes: secrets are kept out of every log by a pattern filter at every log sink (GitHub #76; the AI connection test blanks the key it sent from the provider's reply); undoing a Readarr import deletes only inside each item's own library folder, and the page says how many files were left; Retry on a failed manual import reaches a download client on the local network. Astra was the sole reviewer (spec PASS round 3, tests round 2, code round 3). The PO accepted that the filter misses a few rare secret shapes. Server and web pages were deployed to port 8789 at 2026-10-05T17:53:13Z ([deployment](build/reviews/security-before-release/deploy-20261005T175224Z/DEPLOYMENT.json)); the PO's live check is done, and the PM's log scan found none of the 7 saved keys and passwords. Source: [spec v5 as built](spec-security-before-release.md#7-as-built-corrections-and-limits), [accepted limits](spec-security-before-release.md#4-non-requirements), [retro](build/state/retro-security-before-release.md#executive-summary), [reviews](build/reviews/security-before-release/), [log redaction wiki page](wiki/patterns/log-redaction.md).
- **silent-failures-2** (work-plan chunk 2, closed 2026-10-05) — six silent failures in the web app now show an error. Source: [spec v6 as built](spec-silent-failures-2.md#7-as-built-corrections-and-limits), [retro](build/state/retro-silent-failures-2.md#executive-summary).
- **Work-plan chunk 1** (2026-10-04) — five kk-build tool fixes, pushed as `d52b915`. Evidence: [review round 3](build/reviews/tooling-fixes-2026-10-04/REVIEW-astra-r3.md).
- Earlier features: see the [steward continuation](build/ops/document-steward/CONTINUE.md).
- Standing PO directions: Gemini and Grok benched, Astra sole reviewer (2026-09-24); let the user edit what they want (2026-09-25); no push without the PO's word; the steward pass runs at every feature close.

## Open items

Unapproved candidates are in [TODO](build/plans/TODO.md), grouped by the work plan. They include the type-based secret design under Security (the code declares what is sensitive), the items noticed during security-before-release, and the audiobook that stays stopped after a stream refresh fails and recovers. `TODO.md` records that GitHub #76 was not closed by the PM. Also open: files on the parked branch `wip/identity-conflict-authority-stopped` ([proposal](build/reviews/steward-upkeep-2026-09-26/WIP-BRANCH-PROPOSAL.md)).

## Read next

PM: `~/Projects/kk-build/templates/claude-md/pm-context.md`, then [CLAUDE.md](CLAUDE.md), then this handoff and the [work plan](build/plans/todo-chunks-2026-10-04.md#executive-summary) (next move: ask the PO about chunk 4). Feature history: the security-before-release [session log](build/state/session-log-security-before-release.md) and [state file](build/state/security-before-release.yaml); its [code-stage handoff](build/state/handoff-security-before-release.md) is history. Document upkeep: [steward continuation](build/ops/document-steward/CONTINUE.md) and this pass's [receipt](build/reviews/steward-close-security-before-release-2026-10-05/RECEIPT.md), which lists contradictions for the PM.

The previous root handoff is preserved unchanged at `build/reviews/steward-close-security-before-release-2026-10-05/predecessors/HANDOFF.md`.
