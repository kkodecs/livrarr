# Livrarr — current handoff

## Executive summary

**No feature is in progress once the small-bugs-pass close gate finishes.** Two features were delivered since 2026-09-26. The upgrade fix (identity-upgrade-inplace) is pushed. The five-bug fix (small-bugs-pass) is committed on local main (`c48f9b02`, `132bf314`) and **not pushed**: the PO said "hold" on 2026-09-30, so main is two commits ahead of origin (`43b369c2`). GitHub issues #178, #180, #182 and #183 wait on that push. Nothing new is approved; scope additions need the PO's explicit word. See [what is delivered](#what-is-delivered), [open items](#open-items) and [where to read next](#read-next).

## What is delivered

- **identity-upgrade-inplace** — closed 2026-09-29, pushed (`43b369c2`). An alpha6 library converts itself on first start, keeping confirmed matches, book status and audiobook covers, with one protected backup per upgrade. Source: [status](build/state/STATUS-identity-upgrade-inplace.md), [handoff](build/state/handoff-identity-upgrade-inplace.md), [upgrade page](wiki/deployment/upgrading.md).
- **small-bugs-pass** — reviewed, live on port 8789, committed locally, not pushed. Fixes: duplicate pop-ups (#183), the 1,000-book cap on Search, Queue and History (#180), small usenet ebooks not importing (#178), list-undo leaving authors (#182), two unfinished code paths. Accepted limit (PO, 2026-09-29): if the author clean-up fails after books are removed, undo still finishes and some empty authors may remain; delete them by hand ([record](build/reviews/small-bugs-pass/packet-2-code/DECISION.md)). Source: [status](build/state/STATUS-small-bugs-pass.md), [retro](build/state/retro-small-bugs-pass.md), [handoff](build/state/handoff-small-bugs-pass.md), [spec](spec-small-bugs-pass.md).
- Earlier: add-metadata-preservation (2026-09-23) and card-edits-lift (2026-09-26, `185ae019`) remain delivered; see the [previous continuation entries](build/ops/document-steward/CONTINUE.md#previous-update-2026-09-26-superseded-by-the-delivered-state-above).
- Standing PO directions: Gemini and Grok benched, Astra sole reviewer (2026-09-24); let the user edit what they want (2026-09-25); no push without the PO's word.
- Wiki pages updated for this pass: `wiki/domain/list.md` (undo author rule), `wiki/architecture/import-pipeline.md` (usenet size-check rule), `wiki/insights/data-and-state.md` lesson 81 (shared library loader).

## Open items

PO decisions: push local main (held); which GitHub issues to close and when. The PM recommended the silent-failures pass as the next batch; the PO has not chosen. Other candidates, all unapproved, are in [TODO](build/plans/TODO.md). Also open: files on the parked branch `wip/identity-conflict-authority-stopped` ([proposal](build/reviews/steward-upkeep-2026-09-26/WIP-BRANCH-PROPOSAL.md)) and the housekeeping list in the identity-upgrade [status](build/state/STATUS-identity-upgrade-inplace.md#next). Small-bugs-pass live check: [LIVE-CHECK](build/reviews/small-bugs-pass/LIVE-CHECK.md).

## Read next

PM: `~/Projects/kk-build/templates/claude-md/pm-context.md`, then [CLAUDE.md](CLAUDE.md), the two status files above and [TODO](build/plans/TODO.md); confirm the close gate result for small-bugs-pass (`build/state/small-bugs-pass.yaml`, `closed_at`); then ask the PO what is next. Document upkeep: [steward continuation](build/ops/document-steward/CONTINUE.md), [receipt](build/reviews/steward-upkeep-2026-09-30/RECEIPT.md).

The previous root handoff is preserved unchanged at `build/reviews/steward-upkeep-2026-09-30/predecessors/HANDOFF.md`.
