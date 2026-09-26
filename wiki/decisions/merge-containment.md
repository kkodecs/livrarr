# Temporary work merge containment

The PO stopped the merge/conflict/undo rewrite on 2026-09-07 and approved a bounded containment patch. This checkout implements it on `fix/merge-containment`, based on `af865bdb`; the original dirty checkout and continuation001–007 evidence remain preserved. This page describes the patch, not a live deployment or acceptance of the unfinished rewrite.

`WORK_MERGING_AVAILABLE` in the domain identity services is temporarily false. Manual merge preview/execution and all GroupIdentity resolution are refused. The real SQLite settlement and absorption writers also reject absorption, so a stale client or direct repository caller cannot bypass the UI restriction. Existing review cards remain visible and dismissible; PendingRoute linking remains available. No undo implementation is introduced.

Automatic reconciliation of multiple stored works produces review instead of absorption. Ordinary creation and attachment to one stored work remain available. The startup title-policy and duplicate-residue consolidation routines return without writes or advancing their completion markers; their earlier algorithms are retained for the paused effort.

**Superseded in part by card-edits-lift (2026-09-25):** title/author edits and the "different
book" card answer are back on; the merge answer, manual merge, automatic absorption and the
startup clean-ups stay refused; manual merging is dropped for good (see
[merge-undo-rewrite-dropped](merge-undo-rewrite-dropped.md) and `wiki/domain/work.md`, "Dedup
review"). The paragraph below describes the containment as deployed on 2026-09-07.

Manual title and author changes are temporarily read-only because their current update path performs GroupIdentity re-identification, including in one-member groups. The backend rejects actual changes before creating an author or review card. Forms may still submit unchanged title/author fields with ordinary metadata changes; the handler discards those unchanged fields and saves the remaining edits.

The source snapshot receipt is `/mnt/opt/livrarr/build/reviews/merge-containment/pre-containment-source/RECEIPT.json`. The bounded contract is [spec-merge-containment.md](../../spec-merge-containment.md); validation evidence is in `build/reviews/merge-containment/` in this checkout. Historical tests expecting successful merging are evidence about the superseded behavior, not grounds to reopen merge implementation. Full-suite and independent-review status must remain explicit before deployment.

The September 7 baseline comparison passed all 240 tests in the three affected historical suites. Twenty-seven success contracts for the now-disabled merge, GroupIdentity and startup consolidation paths are explicitly marked ignored with reasons; their bodies remain available for the stopped feature. Active tests instead pin refusal, unchanged ownership and safe deferral. The complete list is in `build/reviews/merge-containment/suspended-contracts.json`. These suspensions do not waive normal workflow checks or declare the merge implementation complete.

Fresh or already-active libraries retain normal startup. A legacy, not-yet-activated database with unresolved identity collisions cannot complete GroupIdentity resolution or activation during containment; the real CLI regression test pins refusal, retained review cards and continued startup refusal for that case.
