# Wiki change log

## Executive summary

This log records wiki updates and their evidence. The [latest entry](#2026-10-05--log-redaction-readarr-undo-and-retry-client) adds the log-redaction page and two live-log lessons from security-before-release. The [2026-10-04 entry](#2026-10-04--saved-place-sonner-timing-and-query-error-state) adds the readers' saved-place page and two lessons from silent-failures-2. The [2026-09-26 entry](#2026-09-26--branch-documentation-brought-to-main-and-five-pages-hand-merged) records the September 2026 wiki cleanup arriving on main and the hand merge of five pages; the [2026-09-07 entry](#2026-09-07--temporary-merge-containment) is main's own record of the merge containment. Earlier entries retain their original findings and dates.

## 2026-10-05 — Log redaction, Readarr undo and Retry client

From security-before-release (`497b391e`): a new page, [log redaction](patterns/log-redaction.md),
describes the one cleanser at every log sink, how it differs from `redact_secrets`, its rules in
order, the accepted limits and the deferred type-based design.
[Lessons 108 and 109](insights/tests-and-fixtures.md#lesson-108) record two traps when reading the
live log. [LibraryItem](domain/library-item.md#removing-files-from-disk) notes that Readarr import
undo now removes files through the shared rule, and [key decisions](decisions/key-decisions.md#ssrf-trusted-infrastructure-pattern)
note that manual import Retry uses the trusted client. The tests-and-fixtures, LibraryItem and
key-decisions pages gain executive summaries. Source: `spec-security-before-release.md` v5.

## 2026-10-04 — Saved place, Sonner timing and query error state

From silent-failures-2 (`596f4f85`): a new page, [saved reading and listening place](domain/saved-place.md),
describes how the ebook reader and the audiobook player protect a saved place they could not
read. [Errors and recovery](patterns/error-handling.md#sonner-under-a-paused-test-clock) gains
the Sonner paused-clock rule and an executive summary.
[Lesson 107](insights/coding-patterns.md#lesson-107) records that TanStack Query clears an
errored query's error state while it refetches; the coding-patterns page gains an executive
summary. Source: `spec-silent-failures-2.md` v6 §7.

## 2026-09-26 — Branch documentation brought to main and five pages hand-merged

The September 2026 wiki cleanup, its preserved snapshots and the review/reference pages
written on the parked branch `wip/identity-conflict-authority-stopped` (57189130) are now on
main's working tree: 147 files copied where main was unchanged, then five pages that had
changed on both sides hand-merged with main as the base — this log, the wiki index,
[Work](domain/work.md), [enrichment priorities](domain/enrichment-priorities.md) and
[search-result metadata](domain/search-result-metadata.md). Main's card-edits-lift text,
the merge-containment and rewrite-dropped decision links and its 2026-09-07 entry below
survive; branch text that described only the dropped merge/undo rewrite was left out, and
`retained-merge-history.md` was deliberately not carried over. The two stale "not deployed"
claims about the metadata save fix were corrected against
[its status record](../build/state/STATUS-add-metadata-preservation.md). Four broken links
were repointed. Exact predecessors and hashes are in the receipts.

[Bring-to-main receipt](../build/reviews/steward-upkeep-2026-09-26/bring-to-main/RECEIPT.md) ·
[hand-merge receipt](../build/reviews/steward-upkeep-2026-09-26/hand-merge/RECEIPT.md).

## 2026-09-19 — Catalog lookup and stored identity comparison

Updated [identity lessons 95/98](insights/identity.md#lesson-95) for same-attempt healthy ISBN fallback, Work-ID preference and full-identity comparison across stored key versions. The change preserves old records and disabled merging. [Deployed result](../build/reviews/catalog-work-lookup/RESULT.md) records reviewed source, checks and remaining metadata-save work. Exact predecessors are retained in the feature upkeep evidence.

## 2026-09-18 — Remaining wiki checks resolved; Add loss reproduced

A read-only inventory found 99 present library files and zero hardlinks; no repair
was needed. Credential wording now distinguishes hashed Livrarr credentials from
reusable service secrets. A live public-books request verified Hardcover's existing
Bearer header; the raw-token header was rejected, so authentication code is unchanged.

The Add-book issue is now reproduced through the real route on isolated data:
supplied language/year remain empty and the cover is never requested. Existing-book
metadata survives dedup. No Add fix was implemented; the bounded proposal awaits
discussion. All prior decision text and exact document predecessors remain preserved.

[Results and evidence](../build/reviews/wiki-follow-ups-2026-09-18/post-deploy/SUMMARY.md).

## 2026-09-18 — Readarr correction deployed

With user approval, the tested independent-copy correction is now running from a
frozen executable. Isolated startup and live database health checks passed; all
61 UI files match, configuration and schema are unchanged, and startup logged no
warnings or errors. The prior executable and consistent database backups are saved.
No library-wide inventory or repair was run.

[Deployment, verification and rollback](../build/reviews/readarr-independent-copies/deployment-20260918T063034Z/DEPLOYMENT.md).

## 2026-09-18 — Readarr independent-copy correction implemented and tested

The separate correction worktree now uses Copy for new Readarr imports and
separates existing hardlinks on retry while keeping library contents and modes.
Eleven new regressions passed; the complete Rust workspace run passed 2,259 tests
with no failures (277 existing ignored tests). Formatting, lint and independent
OpenAI/Google code reviews passed. The correction is uncommitted and undeployed;
no library-wide inventory or repair was performed.

[Result and evidence](../build/reviews/readarr-independent-copies/RESULT.md) ·
[exact predecessors](../docs/design-history/readarr-copy-fix-2026-09-18/README.md).

## 2026-09-18 — Readarr hardlink behavior verified against the deployment baseline

A bounded Serena trace confirms Readarr requests HardlinkFirst and the shared core
attempts a filesystem hardlink before copy fallback. Existing-target adoption/skips
can retain prior links. All 736 baseline source files match, and the running binary
matches the recorded September 7 executable. No import, library inventory or repair
was run. The copy-policy mismatch is confirmed; implementation remains outstanding.

[Source evidence and correction scope](../build/reviews/wiki-follow-ups-2026-09-18/readarr-copy/REVIEW.md).

## 2026-09-18 — Cleanup follow-up decisions applied

The user retained Markdown, confirmed independent copies for Readarr imports,
updated the review resume point and settled the Final Rule, persistence,
privacy-sentence, integration-credential and provider-integration wording.
Current import and privacy lessons now reflect those decisions. Exact earlier
versions remain preserved; this entry does not establish deployed behavior.

[Decisions and remaining checks](../build/reports/wiki-decisions-2026-09-18.md) ·
[upkeep receipt](../build/ops/document-steward/provider-rule-2026-09-18/RECEIPT.md).

## 2026-09-09 — Current references consolidated

The wiki now separates current explanations from dated implementation accounts.
Creation/identity, enrichment/covers/tags and import/recovery each have a primary
page. Older overlapping pages redirect, while existing page paths and section IDs
remain available.

The lesson index links every original numbered lesson. Current rules are condensed;
full earlier records, corrections and recent detailed fixture/design conditions
remain accessible. Crate pages explain ownership and source navigation rather than
copying method signatures. Provider pages distinguish recorded constraints,
proposals and unverified present-day service status.

Every previous wiki file, including this entire earlier log, was preserved exactly
before editing. Root principles, architecture, source, tests, feature state and
runtime were outside the edit scope. Cleanup does not accept an implementation gate
or resume the parked merge/conflict/undo work.

[Morning review note](../build/reports/wiki-cleanup-2026-09-09.md) ·
[preserved revision and history catalog](../docs/design-history/wiki-before-cleanup-2026-09-09/README.md) ·
[cleanup receipt](../build/ops/document-steward/wiki-cleanup-2026-09-09/RECEIPT.md).

## 2026-09-07 — temporary merge containment

Recorded the PO-approved containment patch in `decisions/merge-containment.md`: backend and UI restrictions, safe remaining workflows, startup marker preservation, and the title/author edit limitation. Implementation documentation only; full merge work remains stopped and no deployment is claimed.

## Earlier changes

The complete April–September log is in the exact predecessor linked below,
including the original build reports, corrections and dated acceptance limits.
Future entries should state what changed and link its evidence; full process
narratives belong with their build records.

## Source and history

[Exact revision before cleanup](../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/log.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.
The untrimmed log main carried until 2026-09-26 (with its 2026-09-07 entry) is preserved under
`build/reviews/steward-upkeep-2026-09-26/hand-merge/predecessors/`.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="2026-09-04--insights-102-amended-b0-m1-r2-one-owner-and-full-b4-marker"></a>
<a id="2026-09-03--insights-102-amended-b0-m1-activation-install-vs-f2"></a>
<a id="2026-09-02--insights-102-amended-f5f4-fold-fixture-facts"></a>
<a id="2026-07-25--import-pipelinemd-only-epub-gets-tagged-and-the-failure-path-repairs-nothing"></a>
<a id="2026-07-25--architecture-cluster-a-diagnosed-defect-already-fixed-and-the-tbook-search-tier-that-never-existed"></a>
<a id="2026-07-25--wikidomain-complete-architectureoverviewmd--a-dead-crate-a-removed-edge-and-a-startup-order-backwards"></a>
<a id="2026-07-25--wikidomain-part-2-goodreads-does-not-need-an-llm-and-foreign-works-never-take-hcol-metadata"></a>
<a id="2026-07-25--wikidomain-part-1-the-scoping-model-was-inverted-and-the-state-machines-had-drifted"></a>
<a id="2026-07-25--hardcovermd--openlibrarymd-verified-wikiintegrations-complete"></a>
<a id="2026-07-25--the-dyn-pair-named-google-booksmd-verified-gb-key-is-a-header-not-a-query-param"></a>
<a id="2026-07-25--goodreadsmd-verified-we-already-stopped-hitting-search-and-the-pace-is-15s"></a>
<a id="2026-07-25--the-dyn-claim-settled-audnexusmd-verified-integrations-pass-split"></a>
<a id="2026-07-25--wikipatterns-verified-three-of-four-conventions-had-drifted-from-the-code"></a>
<a id="2026-07-25--domainmd-entities-verified-the-newtype-ids-are-type-aliases-domainmd-complete"></a>
<a id="2026-07-25--domainmd-region-1-lower-half-normalize_for_matching-is-production-dead"></a>
<a id="2026-07-25--domainmd-region-2-verified-29-configsettings-methods-do-not-take-a-user_id"></a>
<a id="2026-07-25--domainmd-service-traits-verified-against-source-partial-pass--see-boundary"></a>
<a id="2026-07-25--dbmd-verified-against-source-signatures-were-systematically-wrong-about-user-scoping"></a>
<a id="2026-07-25--handlersmd-verified-against-source-appcontext-is-not-a-union-of-everything"></a>
<a id="2026-07-24--servermd-verified-against-source-21-wrong-claims-corrected-or-cut"></a>
<a id="2026-07-14--settle-road-title-trust-cause-aware-grey--one-trust-policy-flm-colon-truncation-killed"></a>
<a id="2026-06-14--work-creation-pipeline-mapped-m9-convergence-gap-documented"></a>
<a id="2026-06-10--canonical-model-authored-1317-crate-corrections"></a>
<a id="2026-05-29--m9-amended-fully-formed-by-path-tier-work-creation-consistency"></a>
<a id="2026-05-29--crate-count--provider-stack-corrections-work-creation-consistency-prep"></a>
<a id="2026-05-14---metadata-pathway-explainer"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-sixteenth-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-fifteenth-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-fourteenth-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-thirteenth-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-twelfth-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-eleventh-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-tenth-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-ninth-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-eighth-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-seventh-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-sixth-pass--correction"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-fifth-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-fourth-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-third-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review-second-pass"></a>
<a id="2026-04-23--architecture-excellent-sprint-review"></a>
<a id="2026-04-19--compile-wall-100-second-review-pass"></a>
<a id="2026-04-19--compile-wall-100-post-session-review"></a>
<a id="2026-04-19--compile-wall-100-wiki-consult"></a>
<a id="2026-04-19--phase-5-compile-wall-documentation"></a>
<a id="2026-04-18--full-ingest-from-build-artifacts"></a>
<a id="2026-04-18--initial-wiki-scaffold"></a>
<a id="2026-07-02--phase-3-foundation-build-complete-overnight-session"></a>
<a id="2026-07-02--phase-4-data-completeness--convergence-built--dual-family-reviewed"></a>
<a id="2026-07-03--phase-5-one-matching-authority-complete-built-reviewed-merged-deployed"></a>
<a id="2026-07-03--n1-gr-series-page-parser-rewrite-react-layout"></a>
<a id="2026-07-03--n4-gr-picker-through-the-matching-authority"></a>
<a id="2026-07-03--n3-dead-url-phase1-cover-fast-fail"></a>
<a id="2026-07-04--n2-cover-pipeline-consolidation"></a>
<a id="2026-07-04--architecture-review-prep"></a>
<a id="2026-07-10--audit-the-audit--god-object-design-session"></a>
<a id="2026-07-11--work-service-split-executed-insights-64-65-handlersmd-has-table"></a>
<a id="2026-07-12--suppression-machinery-deleted-pipeline-hygiene-item-1"></a>
<a id="2026-07-13--docs-sync-quality-waves-wave-1-dead-coderename-fallout"></a>
<a id="2026-07-13--docs-sync-quality-waves-wave-2-behavior-fix-audit-no-wiki-fixes-required"></a>
<a id="2026-07-13--quality-waves-2a-shared-qbit-classifier-insights-70-65-amendment-roads-row-d1-doc"></a>
<a id="2026-07-13--quality-waves-2d--36-swallowed-writes-sweep-audit-no-wiki-fixes-required"></a>
<a id="2026-07-13--docs-sync-quality-waves-wave-3-pure-moves"></a>
<a id="2026-07-14--indexer-citizenship-unit-insight-71-servermd-grabsearchcache-refs-contract-doc-committed"></a>
<a id="2026-07-14--matching-conformance-unit-insight-59-amended-13-corrected-design-doc-committed"></a>
<a id="2026-07-19--work-history-architecture-gate-closed-spine-amended-to-17-insight-48-corrected"></a>
<a id="2026-08-10-update--work-presentation-after-author-deletion"></a>
<a id="2026-08-10-update--cutover-staging-cardinality-and-review-kind-storage"></a>
<a id="2026-08-10-update--cutover-readiness-and-nonempty-activation"></a>
<a id="2026-08-11-update--live-add-identity-seam-and-frozen-legacy-status"></a>
<a id="2026-08-11-update--delayed-groupidentity-review-cas"></a>
<a id="2026-08-11-update--dedup-review-settlement-and-card-idempotency"></a>
<a id="2026-08-14-update--convergence-terminality-work-birth-and-dto-completeness"></a>
<a id="2026-08-14-update--route-backed-dto-identifiers-and-observable-discovery-drops"></a>
<a id="2026-08-16-update--sqlite-write-admission-and-approved-readarr-transport"></a>
<a id="2026-08-18--identity-layer-rewrite-rounds-13-14-machine-search-fallback"></a>
<a id="2026-08-18--identity-layer-rewrite-rounds-15-17-decisive-links-dead-anchors-visible-review"></a>
<a id="2026-08-18--identity-layer-rewrite-round-18-route-generation-standing"></a>
<a id="2026-08-20-update--goodreads-identifier-authority-and-failure-specimens"></a>
<a id="2026-08-20-update--goodreads-book-page-authority-completed-across-consumer-doors"></a>
<a id="2026-08-20--identity-layer-rewrite-close-out-fold-rounds-7-22-complete"></a>
<a id="2026-08-22--identity-lifecycle-audit-close-out-fold"></a>
<a id="2026-08-23--identity-review-fixes-spec-rounds-r3r4-fold"></a>
<a id="2026-08-26--identity-review-fixes-fix-wave-1a-close-out-fold"></a>
<a id="2026-08-31--insightsmd-restructured-into-index--full-text-pages"></a>
<a id="2026-08-31--deletion-pass-identity-conflict-authority-buckets-ae"></a>
<a id="2026-09-01--insight-102-amended-after-the-identity-conflict-authority-ir-errata"></a>
<a id="2026-09-01--insight-102-amended-after-the-first-compiled-run-of-the-b0-contracts-suite"></a>
<a id="2026-09-05-ingest--retained-merge-history"></a>
<a id="2026-09-06-navigation--document-steward-packet-002"></a>
<a id="2026-09-06-archive--architecture-reference-separation"></a>
<a id="2026-09-06-update--accepted-f4-scope-clarification"></a>
<a id="2026-09-06-update--cancellation-test-diagnostics"></a>
<a id="2026-09-06-update--sqlite-fixture-clock-timing"></a>
<a id="2026-09-07-source-baseline--guided-architecture-review"></a>
<a id="2026-09-07-correction--matching-identity-and-enrichment-boundary"></a>
<a id="2026-09-07-review-explanation--creation-minimum-and-descriptive-completeness"></a>
<a id="2026-09-08-source-navigation--search-results-display"></a>
<a id="2026-09-09-correction--shared-principles-and-architecture-context"></a>
<a id="2026-09-09-decision--numbered-engineering-principles"></a>
<a id="2026-09-09-learning--architecture-review-simplification"></a>
<a id="2026-09-09-learning--codebase-improvement-review-method"></a>
<a id="wiki-change-log"></a>
