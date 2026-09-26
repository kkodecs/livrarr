# Identity review: authority and census limits

## Executive summary

The rules below are what survives from the August 2026 identity-review census; the
counts and door inventories in the preserved original are dated evidence, not a current
inventory. Since 2026-09-25 the review card offers "Different book" and Dismiss only, and
manual merging is dropped for good ([Work](../domain/work.md), "Dedup review"). See
[the surviving rules](#rules-that-survive-the-census) and
[what to check before changing review behavior](#before-changing-review-behavior).

The detailed census previously at this path enumerated August source after a
specific fix wave and deletion pass. It is preserved in full in the linked source
record. It is not a current inventory of the parked rewrite or deployed containment.

## Rules that survive the census

- Runtime card minting, equivalent-question reuse and suppression share an owner.
  A staging-only writer can have a deliberately separate cutover contract.
- User dismissal is a durable decision about a keyed question. Cancelled card
  status alone does not distinguish user action from machine cleanup.
- Review reads expose the actionable generation observed for the decision;
  resolve revalidates both generation and proposal inside the transaction.
- Unsupported actions must refuse before writes and explain recovery rather
  than fabricate a resolved audit or successful badge.
- Creation review must not leave a hidden duplicate; minimum-only import must
  not commit half an Author/Work result when deferring.
- Shared matching belongs behind the service boundary. A handler cannot reproduce
  a group evaluator merely because the implementation is inaccessible.

## Before changing review behavior

Resolve the actual source, current project state and containment restrictions.
Enumerate writers, readers and real callers with Serena and check index coverage.
Do not reuse historical counts such as seven callers/six doors without a fresh
walk. The August GroupIdentity defects and September unfinished authority designs
are separate evidence; neither implies a working current merge UI.

[Creation](work-creation-pipeline.md) · [identity lessons](../insights/identity.md) ·
[history and review lessons](../insights/history-and-review.md) ·
[merge/undo rewrite dropped (2026-09-23)](../decisions/merge-undo-rewrite-dropped.md).
The retained-merge-history design page belonged to that dropped rewrite and was not carried
to the current wiki; its last text is preserved in the
[pre-cleanup snapshot](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/domain/retained-merge-history.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/architecture/identity-review-census.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="identity-review-cards--the-one-mint-authority-the-dismissal-ledger-and-every-door-into-the-one-continuation"></a>
<a id="one-authority-mints-every-runtime-card-req-007-u7"></a>
<a id="seven-of-nine-kinds-now-refuse-the-continuation-by-name-req-001-u1"></a>
<a id="every-door-into-the-one-continuation-resolve_review"></a>
<a id="dismiss-is-a-standing-decision-not-a-card-state-req-005-u5"></a>
<a id="suppression-is-decided-once-before-any-settlement-mutation"></a>
<a id="exactly-two-doors-revoke--each-inside-its-own-actions-transaction"></a>
<a id="one-time-adoption-of-historical-dismissals"></a>
<a id="cancellation-is-five-sites-only-one-is-the-user"></a>
<a id="the-groupidentity-continuation-is-still-known-broken-until-wave-b"></a>
<a id="manual-import-never-parks-and-the-coordinator-is-atomic-req-003-u2"></a>
<a id="the-import-screen-now-has-a-titleauthor-editor"></a>
<a id="the-resolve-request-notes-field-is-gone"></a>
<a id="user_confirmed1-is-not-identity-edit-only-and-observed_at-refreshes"></a>
<a id="settlement-and-continuation-each-claim-a-generation--by-design"></a>
<a id="the-road-service-trait-is-four-methods-the-door-cannot-reach-the-impl"></a>
