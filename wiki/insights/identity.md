# Identity lessons

## Executive summary

These lessons explain when catalog results identify the same Work and when differences must remain separate. For lookup behavior, see [provider fallback](#lesson-95) and [comparison across stored key versions](#lesson-98); historical records retain their original limits.

Current guidance with links to the full dated evidence. Read implementation claims
against the named source revision; accepted design is not proof of runtime behavior.

<a id="13-no-llm-chooses-a-match-anywhere-phase-5-2026-07-03"></a>
<a id="lesson-13"></a>
## 13. LLMs never choose identity

Matching is deterministic. LLMs may repair public metadata extraction or clean already-trusted text, but never choose, confirm or auto-accept a match. Repaired payloads gain no extra trust. The legacy identity-verification function and settle road were deleted; missing LLM configuration must not disable ordinary deterministic Goodreads matching.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#13-no-llm-chooses-a-match-anywhere-phase-5-2026-07-03).

<a id="15-identity-locked-at-add-time"></a>
<a id="lesson-15"></a>
## 15. User choice survives identity completion

Keep the user-selection commitment. The old add-time settle/quorum mechanism was retired on August 31; current Add captures routes and settles through the identity road. Do not recreate the retired mechanism or infer background progress from its frozen badge.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#15-identity-locked-at-add-time).

<a id="53-sprint-d-seeds--doors"></a>
<a id="53-sprint-d-seeds-doors"></a>
<a id="lesson-53"></a>
## 53. One seed policy across creation paths

The documented six creation doors use shared seed construction. Language defaults live in one authority and must populate both descriptive seed and identity evidence. Monitor language labels additions; it does not select translated editions.

At the guarded author/series monitoring writes, enabled monitoring must not leave null language. Prefer the unique dominant language, then the shared default; a tie gives no dominant language. Explicit clears and bare UI toggles must pass the same rule.

Author-monitor screening is deterministic and rejects high-confidence bundles, multi-author anthologies, malformed/titleless entries and study-guide shapes without discarding ordinary titles. The legacy LLM scraper discovery road was removed. Source details retain the exact thresholds and fixtures.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#53-sprint-d-seeds--doors).

<a id="54-series-monitor--readarr-seed-identity-pending-by"></a>
<a id="54-series-monitor-readarr-seed-identity-pending-by"></a>
<a id="lesson-54"></a>
## 54. Background creation must converge

Background creation can start with incomplete evidence, but cannot strand a Work silently. The original missing-convergence defect was subsequently fixed; its old Pending badge and chase selector were later retired. Current recovery uses captured routes and current review state. Verify the actual scheduler and path instead of acting on the June defect description.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#54-series-monitor--readarr-seed-identity-pending-by).

<a id="57-convergeoutcomecompleted--the-work-is-not"></a>
<a id="57-convergeoutcomecompleted-the-work-is-not"></a>
<a id="lesson-57"></a>
## 57. Completion must agree with retry selection

A workflow claiming completion should no longer be selected for unchanged recovery work. The old converge_outcome helper and anchor accounting were deleted; the temporal invariant survives. Current route/search accounting is covered by lessons 93 and 97.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#57-convergeoutcomecompleted--the-work-is-not).

<a id="59-one-matching-authority-phase-5-2026-07-03"></a>
<a id="lesson-59"></a>
## 59. Shared identity comparison

Use livrarr-domain identity_matching for title, author, language and identifier comparison. Never duplicate a permissive local scorer or truncate titles at a colon. Volumes and real subtitles matter; author agreement is stronger than a shared token. Edition-ID inequality is no evidence, and work-key contradictions cannot be hidden by an edition bridge.

Comparison, stored identity normalization and provider selection must use the same relevant authorities. Grey handling is caller-specific: historical provider pickers usually required Same, with a Goodreads exception, while later group reconciliation has its own tuple semantics. Do not copy an old numeric threshold into a new caller. Lessons 72, 95, 97 and 98 preserve later qualifications.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#59-one-matching-authority-phase-5-2026-07-03).

<a id="60-grey-park-review-surface-phase-5-j2"></a>
<a id="lesson-60"></a>
## 60. Historical review surface

The July grey-park implementation used legacy identity status and work_identity_review_candidates. That is historical navigation after the identity-layer cutover. Current review uses captured identity and typed cards. Preserve the general guarantees: real evidence, atomic claims, explicit user recovery and no duplicate decision application.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#60-grey-park-review-surface-phase-5-j2).

<a id="67-author-identity-has-one-adoption-gate--a-real-merge"></a>
<a id="67-author-identity-has-one-adoption-gate-a-real-merge"></a>
<a id="lesson-67"></a>
## 67. Author adoption and merge

Author adoption uses one unambiguous-match authority over the user’s authors. Initial/glued-initial/name-prefix compatibility is deliberately constrained; multiple compatible candidates must not silently collapse. Provider-key fills are monotonic.

The documented author merge owns related Work/series/cache/monitor changes in one database transaction; operation order matters around cascades. Exact-full-name and initial compatibility are not transitive, so a guessed uniqueness constraint is unsafe. Availability and changed merge behavior require checking current source and containment scope.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#67-author-identity-has-one-adoption-gate--a-real-merge).

<a id="72-settle-road-title-trust-is-cause-aware-and"></a>
<a id="lesson-72"></a>
## 72. Subtitle trust needs its cause

A one-sided subtitle is different evidence from conflicting subtitles, volume asymmetry or merely similar main titles. The shared title-trust policy checks raw same-provider work-key contradictions before accepting text. The July correction removed the requirement for matching edition IDs in the one-sided-subtitle arm; callers still apply their author bar.

The old settle-road consumers were deleted on August 31. Keep the shared comparison policy, not the old call graph. At tests, a counter-only reset stub does not reproduce production retry-state recovery; seed coherent identity evidence rather than arbitrary title prefixes.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#72-settle-road-title-trust-is-cause-aware-and).

<a id="73-three-subtitle-variance-bugs-in-one-night-exposed-that"></a>
<a id="lesson-73"></a>
## 73. Provider editions and parsing can mislead matching

OpenLibrary edition selection must consider the edition’s own title before language preference; language-only selection can pick a young-reader or other variant. Goodreads ISBN and title lookup tiers have different evidence and can still abstain honestly. Bare/subtitled titles must pass the actual shared policy rather than a local guess.

The Goodreads Next-data parser was added with field-tolerant extraction; malformed individual fields warn/drop, and ambiguous book pointers abstain instead of choosing map order. Later wrong-cover diagnosis was ID namespace confusion, not parser drift. Full historical probes, rejected designs and real fixtures remain in the source record; their former UNFIXED labels are not current backlog.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#73-three-subtitle-variance-bugs-in-one-night-exposed-that).

<a id="78-anchor-uniqueness-is-per-user-over-all-anchor-types"></a>
<a id="lesson-78"></a>
## 78. Reconstruct the full migration history

Anchor uniqueness was already user-scoped after migrations 042/044; reading only 041 and 044 produced a false claim that persisted through several reviews. Enumerate intervening migrations and run a decisive scratch-database probe before reversing schema facts. Historical index definitions are not current schema authority.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#78-anchor-uniqueness-is-per-user-over-all-anchor-types).

<a id="80-identity-edits-are-protected-by-a-generation-stamp"></a>
<a id="lesson-80"></a>
## 80. Observe generation before making the decision

A compare-and-swap check protects the decision only if the claimed generation was observed before provider I/O or other work informing it. Reading the generation afterward can authorize a stale decision over a user edit. Audit every writer, not merely the presence of a CAS call.

Ownership queries must read the authoritative ledger/routes, not a map reconstructed from populated compatibility columns. Many unguarded legacy writers were removed on August 31; their names are historical. Apply the observation rule to the remaining real writers.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#80-identity-edits-are-protected-by-a-generation-stamp).

<a id="82-author-provider-linking-is-live-feature-176-branch"></a>
<a id="lesson-82"></a>
## 82. Author routes and contributor evidence

Author routes own provider identity; old author scalar keys are frozen. Preserve three credit classes: asserted authorship, unlabeled author-shaped slot and explicit label. Malformed shapes warn/drop; an all-dropped response is layout drift, never a valid empty list.

Asserted mismatches park. The documented unlabeled Latin mismatch drops but records a durable authorial observation; unlabeled non-Latin mismatch retains a review question. Deduplicate identical stored name spellings before compatibility checks, while genuinely distinct columns remain distinct evidence.

Tier-two name search proposes review rather than auto-linking. User re-resolution revokes standing dismissals; removing a route re-arms without revoking them. Review cards need offered name, provider and evidence Work. Test at least one real popular-book payload: many editions can carry dozens of translators/narrators. Historical inherited-route repair remains separate from ordinary linking.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#82-author-provider-linking-is-live-feature-176-branch).

<a id="85-work-presentation-degrades"></a>
<a id="lesson-85"></a>
## 85. Presentation can degrade without weakening identity

A surviving Work must still render after its related Author disappears, using stored author text and cover information. Keep captured-identity aggregate reads strict for settlement. Presentation may handle that specific NotFound after proving the Work exists; other database errors propagate. Test through the real route with related-entity deletion.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#85-work-presentation-degrades).

<a id="90-post-cutover-live-add-is-one-f2-capture-settlement"></a>
<a id="lesson-90"></a>
## 90. Captured identity replaces frozen badges

The clicked route and independently captured provider evidence must enter the identity road. Edition identifiers need Edition ownership. Remove parser-extracted title decorations without losing their subtitle/volume evidence. After activation, current identity presentation and generation-changing audit derive from the captured authority, never the frozen legacy badge. Repairs need bounded, transactional, auditable evidence.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#90-post-cutover-live-add-is-one-f2-capture-settlement).

<a id="92-review-is-a-settlement-outcome-not-permission-to"></a>
<a id="lesson-92"></a>
## 92. Review must not leave a hidden duplicate

When creation finds an established group needing review, bind the proposal to that durable Work before commit. Do not insert an unrelated route-less Work behind ReviewPending. Equivalent questions are user-scoped semantic keys over the normalized proposal/cohort/routes/choices, excluding volatile row IDs and observation timestamps. Reuse the oldest equivalent card. Repair old residue only with conservative proof and no dependent user data, never a title-only delete.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#92-review-is-a-settlement-outcome-not-permission-to).

<a id="93-an-authoritative-convergence-loop-must-make-selector"></a>
<a id="lesson-93"></a>
## 93. No-op, selection and attempt accounting

An identical machine capture is not a settlement: no generation claim or audit. Active selectors read routes/current state. Edition-only discovery attempts are durably bounded by identity generation, and reaching the limit must actually make unchanged work unselectable. Work insertion and its typed door-labeled birth event share a transaction and timestamp. Presentation overlays must preserve the complete unrelated DTO field set.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#93-an-authoritative-convergence-loop-must-make-selector).

<a id="94-compatibility-dto-identifiers-are-route-projections"></a>
<a id="lesson-94"></a>
## 94. Project identifiers from active routes

After cutover, frozen Work scalar IDs are neither fetch nor presentation fallback authority. One route projection feeds Work-bearing responses, pending anchors and tag identifiers. Edition selection uses explicit deterministic ordering, favoring user confirmation and evidence provenance before time/tiebreaks.

Interactive provider timeouts start after queue admission. Every real provider error needs a provider/cause warning; absent call records alone do not prove a fetch never occurred because some discovery paths do not emit those records.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#94-compatibility-dto-identifiers-are-route-projections).

<a id="95-req-027s-machine-search-fallback-is-route-native"></a>
<a id="lesson-95"></a>
## 95. Bounded search fallback

Use typed routes and durable provider standing. Prefer a usable provider Work route over an edition ISBN. A healthy ISBN miss with no usable Work route can trigger the existing provider-local title/Author search in the same attempt; retryable failures retain their retry rules. A decisive unambiguous text match or corroborating edition evidence can settle through the identity road; near matches/ties become idempotent, visible review questions. The [catalog lookup result](../../build/reviews/catalog-work-lookup/RESULT.md) records the verified change.

A newly minted non-inline question notifies exactly once; reuse does not. Decide whether a corroboration probe is needed before optional I/O: zero-route evidence can use search directly. Search remains cache-blind and honest about actual attempts. Normal convergence preserves standing; manual Refresh clears it; a settlement/identity edit clears standing only when its active route snapshot changes. Lesson 97 supersedes the earlier global search-eligibility wording.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#95-req-027s-machine-search-fallback-is-route-native).

<a id="96-goodreads-work-ids-and-book-ids-are-disjoint"></a>
<a id="lesson-96"></a>
## 96. Goodreads Book and Work IDs are disjoint

Only GoodreadsBookEdition addresses /book/show. GoodreadsWork is identity evidence and cannot be reinterpreted as GrKey. Enrichment, author linking, cover alternatives and selection must all enforce that distinction. Work-only graphs do not magically provide detail anchors; frozen gr_key is not a fallback.

Unreadable detail diagnostics retain exact local bytes through an optional observation path, with newest-ten retention and infallible failure handling. Concurrent prune NotFound is benign. Capture does not authorize external disclosure. Empty-200 and small Next-shell responses are documented soft-block classes; wrong covers came from namespace confusion. See [Goodreads](../integrations/goodreads.md) for limits and containment.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#96-goodreads-work-ids-and-book-ids-are-disjoint).

<a id="97-search-eligibility-is-provider-local-accounting-is"></a>
<a id="lesson-97"></a>
## 97. Search eligibility is per provider

An active Work route suppresses that provider’s search only. Other applicable providers evaluate their own routes/anchor standing. All fired search legs fold into one Work/generation ledger: card/miss-only burns once; any settlement or failed provider leg makes the pass non-burnable. Carry fired and burnable facts independently across layers.

Goodreads Work routes come only from genuine Work identifiers in autocomplete or referenced Work payloads. File/list/legacy/direct gr_key inputs are Book-edition identifiers. Repair mislabels only where typed provenance proves the Book channel; ambiguous and genuine Work rows stay untouched. Actual graph repair advances generation, invalidates standing, audits and is idempotent.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#97-search-eligibility-is-provider-local-accounting-is).

<a id="98-leading-articles-are-identity-irrelevant-po-ruling"></a>
<a id="lesson-98"></a>
<a id="98-matching-and-storage-share-normalization"></a>
## 98. Compare complete identities across stored key versions

Leading a/an/the are identity-irrelevant under the recorded user decision; interior articles remain meaningful and display titles remain unchanged. The shared comparator treats an ordinary ampersand as “and.” Stored keys come from different writer generations, so compatibility lookup reconstructs the complete identity from the stored title, separate subtitle and volume within the same user/author scope. Equivalent spellings find the existing Work; an omitted conjunction or different subtitle/volume must not become an exact match. Conflicting catalog IDs still require review. This is read-time compatibility, not a stored-key rewrite or automatic duplicate repair.

Historical repair designs fold only proof-qualified duplicates and park unsafe collisions; they do not authorize re-enabling currently disabled automatic merging. Cover refresh observability must name one gate outcome or typed no-candidate reason per invocation; absence is evidence only when every branch emits. Ordinary search-card covers and imported automatic covers must not become absolute user locks.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#98-leading-articles-are-identity-irrelevant-po-ruling).

<a id="99-a-correct-settlement-no-op-guard-starved-the-identity"></a>
<a id="lesson-99"></a>
## 99. A correctly called seam can still be unwired

Machine callers once fed the identity no-op guard only their own stored routes while the new-capture persistence seam had no production callers. Assert real route/generation/audit effects, not simply that settle was called. Account attempts only when a real fetch starts; cached payloads can settle without burning network budget.

Add background completion and delayed refresh both hand new evidence through the shared adapter with machine origin and the generation observed before capture. Tests with process-global hooks need shared locks across all callers, and asynchronous checks wait for persisted effects with bounded timeouts. The provider-ID-only machine-creation precedent and its incomplete spec fold are historical unresolved policy evidence, not blanket permission to weaken creation requirements.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md#99-a-correct-settlement-no-op-guard-starved-the-identity).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/identity.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="identity-resolution--matching-insights"></a>
