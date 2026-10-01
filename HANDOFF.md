# Livrarr — current handoff

## Executive summary

**Current feature: prerelease-trust-pass, at the code stage, with two code packets written and ready to dispatch.** The next PM starts with the [feature handoff](build/state/handoff-prerelease-trust-pass.md). The PO has no open decision for this feature. Main is one commit ahead of origin (`37dd4426`, a documentation commit), and it stays unpushed until the PO says to push. Nothing beyond the feature's scope is approved; scope additions need the PO's explicit word. See [the current feature](#current-feature), [git and push](#git-and-push), [what is delivered](#what-is-delivered), [open items](#open-items) and [where to read next](#read-next).

## Current feature

**prerelease-trust-pass** — at stage code since 2026-10-01. Three parts:

- correct the wrong claims in the user docs (15 of the scan's 16, plus 7 lines the PO added), including the context file that feeds the in-app AI help;
- remove the dead `[auth]` proxy-login keys, and make config warnings reach the log; the reverse-proxy sub-path (`url_base`) is deferred to the backlog;
- add a one-time setup token, so a fresh install cannot be claimed by whoever reaches it first.

Spec v5 is approved by the PO and passed review; the red tests passed review. The tests gate was passed by the PO's bypass because the gate check times out. Packets `packet-3a-code` and `packet-3b-docs` are written and not dispatched; a fresh PM session dispatches them. **Open PO decisions for this feature: none.** Source: [feature handoff](build/state/handoff-prerelease-trust-pass.md), [state](build/state/prerelease-trust-pass.yaml) (`decisions`), [status](build/state/STATUS-prerelease-trust-pass.md), [spec](spec-prerelease-trust-pass.md#executive-summary).

## Git and push

Main is one commit ahead of origin/main (`59b7e9db`): `37dd4426`, the previous steward commit. The push waits for the PO's word. The spec and the red tests are uncommitted in the working tree; the feature handoff lists them.

## What is delivered

- **Push and issues, 2026-10-01** — livrarr `43b369c2..59b7e9db` on origin/main (small-bugs-pass, errors-and-delete-pass, retro commit `59b7e9db`); GitHub #178, #180, #182 and #183 closed.
- **errors-and-delete-pass** (2026-09-30) and its retro follow-ups (2026-10-01), **small-bugs-pass** (2026-09-30) and **identity-upgrade-inplace** (2026-09-29) — delivered and pushed. Recorded limits are in [spec v6, limits](spec-errors-and-delete-pass.md#limits). Earlier features: see the [steward continuation](build/ops/document-steward/CONTINUE.md).
- Standing PO directions: Gemini and Grok benched, Astra sole reviewer (2026-09-24); let the user edit what they want (2026-09-25); no push without the PO's word; the steward pass runs at every feature close.

## Open items

Unapproved candidates are in [TODO](build/plans/TODO.md), including the PDF reader fix, the delete-with-files folder-swap race (Security), "Make url_base work" and the items added during prerelease-trust-pass. Also open: files on the parked branch `wip/identity-conflict-authority-stopped` ([proposal](build/reviews/steward-upkeep-2026-09-26/WIP-BRANCH-PROPOSAL.md)).

## Read next

PM: `~/Projects/kk-build/templates/claude-md/pm-context.md`, then [CLAUDE.md](CLAUDE.md), then the [prerelease-trust-pass handoff](build/state/handoff-prerelease-trust-pass.md), which gives the reading order, the dispatch steps and the limits. Document upkeep: [steward continuation](build/ops/document-steward/CONTINUE.md) and this pass's [receipt](build/reviews/steward-upkeep-2026-10-01b/RECEIPT.md), which lists contradictions for the PM.

The previous root handoff is preserved unchanged at `build/reviews/steward-upkeep-2026-10-01b/predecessors/HANDOFF.md`.
