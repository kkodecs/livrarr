# Livrarr — current handoff

## Executive summary

**prerelease-trust-pass is delivered and pushed. Only the PM's retrospective and close remain.** Main equals origin/main (`dff80a06`); nothing is unpushed. The PO has no open decision for this feature. Nothing beyond its scope is approved; scope additions need the PO's explicit word. See [the current feature](#current-feature), [git and push](#git-and-push), [what is delivered](#what-is-delivered), [open items](#open-items) and [where to read next](#read-next).

## Current feature

**prerelease-trust-pass** — delivered 2026-10-03; retrospective and `verify.py close` (with its long coverage run) remain for the PM. What shipped:

- the wrong claims in the user docs are corrected (15 of the scan's 16, plus 7 lines the PO added), including the context file that feeds the in-app AI help; the Help page now serves the corrected file;
- the dead `[auth]` proxy-login keys are removed, and config warnings reach the log; the reverse-proxy sub-path (`url_base`) stays in the backlog;
- a fresh install asks for a one-time setup token, so it cannot be claimed by whoever reaches it first.

Astra passed the code on round 2. Deployed by hand on port 8789; the PO's live check passed ("tested both all good"). **Open PO decisions for this feature: none.** Source: [status](build/state/STATUS-prerelease-trust-pass.md), the [session log](build/state/session-log-prerelease-trust-pass.md) entries dated 2026-10-02 and 2026-10-03, [spec v6](spec-prerelease-trust-pass.md#executive-summary), [setup-token page](wiki/deployment/first-run-setup.md). The [feature handoff](build/state/handoff-prerelease-trust-pass.md) is from the code stage; read it as history, not current state.

## Git and push

Pushed `59b7e9db..dff80a06` on 2026-10-03: `d77f690a` (setup token, `[auth]` keys removed, config warnings), `c70dca4e` (user-doc corrections) and `dff80a06` (spec v6, changelog, wiki). Main equals origin/main. Any commit after this one (this steward pass, the spec's last tick) waits for the PO's word to push.

## What is delivered

- **prerelease-trust-pass** (2026-10-03) — above.
- **Push and issues, 2026-10-01** — livrarr `43b369c2..59b7e9db` (small-bugs-pass, errors-and-delete-pass, retro commit `59b7e9db`); GitHub #178, #180, #182 and #183 closed.
- **errors-and-delete-pass** (2026-09-30) and its retro follow-ups (2026-10-01), **small-bugs-pass** (2026-09-30) and **identity-upgrade-inplace** (2026-09-29) — delivered and pushed. Earlier features: see the [steward continuation](build/ops/document-steward/CONTINUE.md).
- Standing PO directions: Gemini and Grok benched, Astra sole reviewer (2026-09-24); let the user edit what they want (2026-09-25); no push without the PO's word; the steward pass runs at every feature close.

## Open items

Unapproved candidates are in [TODO](build/plans/TODO.md), including the PDF reader fix, the delete-with-files folder-swap race (Security) and "Make url_base work" (GitHub #119). Also open: files on the parked branch `wip/identity-conflict-authority-stopped` ([proposal](build/reviews/steward-upkeep-2026-09-26/WIP-BRANCH-PROPOSAL.md)).

## Read next

PM: `~/Projects/kk-build/templates/claude-md/pm-context.md`, then [CLAUDE.md](CLAUDE.md), then the [status](build/state/STATUS-prerelease-trust-pass.md) and the latest dated [session log](build/state/session-log-prerelease-trust-pass.md) entries for the retrospective and close. Document upkeep: [steward continuation](build/ops/document-steward/CONTINUE.md) and this pass's [receipt](build/reviews/steward-upkeep-2026-10-03/RECEIPT.md), which lists contradictions for the PM.

The previous root handoff is preserved unchanged at `build/reviews/steward-upkeep-2026-10-03/predecessors/HANDOFF.md`.
