# History and review lessons

Current guidance with links to the full dated evidence. Read implementation claims
against the named source revision; accepted design is not proof of runtime behavior.

<a id="77-work-history-is-complete-and-single-authority"></a>
<a id="lesson-77"></a>
## 77. History records moments of truth

Create history through typed constructors, with the Work title/author needed to render after deletion; release title remains distinct. Observe successful state transitions, not intent before a fallible write. An unsuccessful status update must not emit an event that repeats next tick.

Use one event per actual lifecycle moment. Enrichment attempted/changed facts come from real control flow and content differences, not outcome-map emptiness or a nonempty merge echo. Birth history belongs atomically with creation under the later identity correction.

User-facing history observation warns without disrupting the business operation; atomic identity/settlement audit has a different contract. Unknown history kinds are skipped with warnings on tolerant reads; the raw count discrepancy is documented. Backfills use persisted fact dates, are additive/idempotent, preserve dedup distinctions, and withhold the marker after failures. Never erase history to make a rerun pass. Detailed event/backfill rules remain in the source record.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/history-and-review.md#77-work-history-is-complete-and-single-authority).

<a id="86-when-one-workflow-reads-resource-a-and-records"></a>
<a id="lesson-86"></a>
## 86. Separate source and approval storage

A workflow can read a snapshot but write its approval ledger to the live database. Tests sharing one handle for both hide ownership bugs. Prove the boundary with separate production processes and inspect both databases; explicitly name the source handle and write owner.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/history-and-review.md#86-when-one-workflow-reads-resource-a-and-records).

<a id="87-a-migration-report-is-an-executable-approval-contract"></a>
<a id="lesson-87"></a>
## 87. Migration reports are contracts

Derive report counts and staged artifacts from the same deterministic grouping helper. Verify every approved category count inside the staging transaction. Persist discriminators through one explicit storage codec; JSON serialization adds quotes and is not a plain TEXT codec. Test exact cardinalities and the real operator-command round trip.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/history-and-review.md#87-a-migration-report-is-an-executable-approval-contract).

<a id="88-nonempty-is-not-a-cutover-blocker"></a>
<a id="lesson-88"></a>
## 88. Nonempty databases can be ready

Readiness checks the staged identity tuple for collisions, not whether the database contains any Works. A nonempty clean fixture must reach Ready; a blocked fixture needs exactly the actionable cohort artifact the operator can resolve. Never leave a permanent Blocked state with zero recovery artifacts.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/history-and-review.md#88-nonempty-is-not-a-cutover-blocker).

<a id="89-a-migration-writer-migration-report-and-startup"></a>
<a id="lesson-89"></a>
## 89. Migration compatibility is one contract

The migration writer, rehearsal report and startup version guard must agree on schema version. A binary must accept the highest schema it writes while rejecting a genuinely newer one. Verify the entire boot ordering with the same nonempty database that Apply produced.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/history-and-review.md#89-a-migration-writer-migration-report-and-startup).

<a id="91-a-review-cards-mint-generation-is-history"></a>
<a id="lesson-91"></a>
## 91. Review generation is observed at decision time

A card retains its immutable mint generation, but its read response exposes the current anchor generation for the user decision. Resolve rechecks that claimed generation and the proposal in the transaction: proposed Works must exist and snapshotted routes must still belong to the cohort. Mere generation drift is not an invalid proposal. Invalid proposals refuse without writes and remain dismissable.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/history-and-review.md#91-a-review-cards-mint-generation-is-history).

<a id="100-the-identity-lifecycle-audit-2026-08-2022"></a>
<a id="lesson-100"></a>
## 100. Identity audit findings need their baseline

The August lifecycle audit found review actions that reported success without acting, unsafe group continuations, frozen scalar consumers and cutover gaps. Later fixes and containment supersede parts of that inventory; the audit is evidence, not current feature status.

Its durable lessons: enumerate full files rather than truncating at the first cfg(test), verify independent duplicate discoveries, and trace actual paths. A LibraryItem relative path passed directly to materialize/tagwrite caused FileNotFound even though the file existed; the TagService path joined the root correctly. File presence alone did not prove the workflow.

The merge/conflict/undo rewrite remains governed by live PM state. Neither a historical test result nor an accepted design authorizes resuming it. Keep full findings, fix-wave dispositions and experiment evidence in build/reviews and the retained source record.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/history-and-review.md#100-the-identity-lifecycle-audit-2026-08-2022).

<a id="101-review-is-a-surface-with-one-mint-authority-one"></a>
<a id="lesson-101"></a>
## 101. Review questions and dismissal have shared owners

The August fix wave centralized runtime card mint/reuse/suppression, canonical dismissal keys and notification decisions. Staging-only minting was intentionally separate. A reused card emits no new notification; inline choices must not alert on an already-resolved question.

Dismissal records a durable user decision over equivalent questions. Machine cancellation is not that decision. Decide suppression before settlement mutation and reuse that disposition; unrelated evidence can still commit. Explicit intent, rather than route name alone, controls bypass. Edition writers may return the existing aggregate as normal success so file import continues.

Unsupported continuation kinds refuse by name before writes instead of fabricating success. Historical settlement-plus-continuation paths each claimed a generation by design; a committed first step surviving a refused second step was not itself an atomicity defect. Newer incomplete authority work and containment must be checked before applying old writer/caller counts. See [review ownership](../architecture/identity-review-census.md).

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/history-and-review.md#101-review-is-a-surface-with-one-mint-authority-one).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/history-and-review.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="work-history-identity-review--cutover-insights"></a>
