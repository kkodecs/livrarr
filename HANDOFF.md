# Livrarr — current handoff

## Executive summary

**No feature is in progress. Both recent features are delivered and pushed.** The metadata-save fix (add-metadata-preservation) was merged on 2026-09-23. The card and edit fix (card-edits-lift) closed on 2026-09-26 at main `185ae019`. Nothing is approved beyond what is merged; scope additions need the PO's explicit word. See [what is delivered](#what-is-delivered), [open items](#open-items) and [where to read next](#read-next).

## What is delivered

- **add-metadata-preservation** — merged and pushed 2026-09-23. Add now saves the metadata it already fetched; new Works monitor both ebook and audiobook; the empty "Other books by this author" panel is gone. Existing Works keep their current flags. Source: [status](build/state/STATUS-add-metadata-preservation.md).
- **card-edits-lift** — closed 2026-09-26, main = origin/main = `185ae019`, spec v5 (`spec-card-edits-lift.md`). Title and author edits and the "different book" answer are back on. The defect that overwrote books from a review answer is deleted. The merge answer is removed; the PO dropped manual merging for good (2026-09-24): a duplicate is deleted by hand. Source: [status](build/state/STATUS-card-edits-lift.md), [handoff](build/state/handoff-card-edits-lift.md).
- Standing PO directions: Gemini and Grok are benched, Astra is the sole reviewer (2026-09-24); let the user edit what they want, no further guardrails (2026-09-25).

## Open items

Nothing is owed. Candidates awaiting a PO word, all unapproved, are in [TODO](build/plans/TODO.md): show the subtitle on the book page; generic contributor merge reverses secondary credits; delete-Work also deletes files; re-add after "different book". Also open: sorting the files parked on local branch `wip/identity-conflict-authority-stopped` (see the [proposal](build/reviews/steward-upkeep-2026-09-26/WIP-BRANCH-PROPOSAL.md)) and the `/mnt/opt` disk cleanup.

## Read next

PM: `~/Projects/kk-build/templates/claude-md/pm-context.md`, then [CLAUDE.md](CLAUDE.md), the two status files above and [TODO](build/plans/TODO.md); then ask the PO what is next. Document upkeep: [steward continuation](build/ops/document-steward/CONTINUE.md).

The previous root handoff (the "stopped" state of 2026-09-21) is preserved unchanged at `build/reviews/steward-upkeep-2026-09-26/predecessors/HANDOFF.md.from-wip-branch-57189130`; it was not present on main when this file was written.
