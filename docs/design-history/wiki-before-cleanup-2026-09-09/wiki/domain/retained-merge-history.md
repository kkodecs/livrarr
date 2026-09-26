# Retained merge history

When a later merge absorbs an earlier merge's surviving Work, the earlier record
must remain readable under its original identity. This is the approved retention
design; implementation acceptance is tracked separately in the build state.

The earlier archive stays byte-identical while the later merge is effective.
Its history pointer may follow the surviving Work only through authenticated,
recorded pointer changes. A live endpoint alone does not prove the intervening
merges: every capture, deletion, pointer change and reversal must agree.

Undoing a later merge validates the complete chain before writing. A capture
explains the earlier record; it is not a backup that authorizes recreating or
overwriting a missing archive. Later undo restores the approved pointers and
ordinary effects while preserving the earlier archive.

An empty file inventory does not make retained history actionable. Its display
may legitimately say Current because no file metadata is unavailable, while its
merge dependency still forbids report writes and undo. Item validation alone
cannot enforce this rule: an empty loop checks no dependency. A late report that
changes the retained archive would also invalidate the later merge's undo witness.
Every mutation door therefore needs the record-level dependency check inside its
transaction, independently of display state and file count.

Sources: [protected design, retention representation](../../docs/design-history/ir-v2-identity-conflict-authority-fold-r22.yaml)
(`anchor_relation`, `nonactionable`, `later_undo`) and
[retention review, findings 002–004](../../build/reviews/identity-conflict-authority/review-code-b0-j-m3f3-retention-openai.md).
The review's counterexamples are static traces, not executed experiments.
Related: [Work lifecycle](work.md), [history and review insights](../insights/history-and-review.md).
