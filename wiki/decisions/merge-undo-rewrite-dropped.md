# Decision: the merge/conflict/undo rewrite is dropped (2026-09-23)

## Executive summary

The PO dropped the identity-conflict-authority rewrite on 2026-09-23 after it was stopped on 2026-09-07. Both an Opus 5.5 assessment and an independent Astra check agreed the tree could not ship and that the cheapest safe route was to fix four concrete defects on the clean containment base. The parked working set is preserved on the local branch `wip/identity-conflict-authority-stopped`; nothing was deleted. See [what replaces it](#what-replaces-it) and [where things live](#where-things-live).

## Why

- After 41 coding dispatches the rewrite's every new web endpoint was still a placeholder; the build refused all merges; 43 tests that pass on the containment build failed on the rewrite tree.
- Structural causes, from the assessment: the design was revised 20 times during coding; the first build unit was large and had no user value on its own; tests pinned mechanism (lock counts) rather than outcomes; the spec asked for exact whole-row restore across 37 tables, which forced an 8,600-line capture-and-restore engine that never ran on real data.
- The PO's stated reason for the stop: token spend out of proportion to a minor feature.

Evidence: `build/reviews/identity-conflict-authority/assessment-20260923/ASSESSMENT.md` and `astra-check/CHECK.md` (local, gitignored `build/`).

## What replaces it

Four defects on the containment base, each with a narrow fix in existing code (numbers from the assessment):

- D1: "different book" with a proposal overwrote the existing book's title, author and provider ids.
- D2: "same book / merge" forced the survivor and absorbed every other member.
- D3: only the survivor's version stamp was checked, so a concurrent edit to the absorbed book was silently erased.
- D4: absorbing a book moved its provider ids onto the survivor with no contradiction check.

**Amended 2026-09-24 (PO decision): manual merging is dropped altogether, not deferred.** Two duplicate Works are resolved by the user deleting the extra one with the existing delete-Work action. Consequences:

- D2, D3 and D4 live only inside the merge path. That path stays refused; they are not fixed.
- The undo question is moot.
- The replacement feature fixes D1, turns manual title/author edits back on, and turns the "different book" card answer back on. The "same book / merge" card answer, which triggered a merge, is removed from the card; cards offer "different book" and dismiss.
- Machine merging and the startup clean-ups stay off. No migration.

Earlier plan (superseded by the amendment above): fix D1–D4; edits and card decisions first; manual merging to return after a PO undo call; the containment gate to split into three (user edits, user merges and card decisions, machine merges).

## Where things live

- Parked tree: local branch `wip/identity-conflict-authority-stopped` (`57189130`, 403 files). It also holds unrelated files that need sorting (other features' spec files, design-history snapshots, wiki pages). Not pushed.
- Containment (what is live): [merge-containment](merge-containment.md).
- Previous feature status: `~/Projects/kk-build/build/state/STATUS-identity-conflict-authority.md`.
