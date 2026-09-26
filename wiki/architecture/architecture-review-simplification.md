# Architecture review: simplification and evidence limits

The September 2026 automated review covered 17 crate reports plus independent source
survey and cross-crate synthesis. A genuine 50% code reduction was not demonstrated;
bounded inspection also did not prove it impossible. Conditional sketches were not
implemented or tested. The [final review](../../build/reviews/architecture-review-2026-09-07/cross-crate-022/FINAL-REVIEW.md)
and [qualified findings](../../build/reviews/architecture-review-2026-09-07/NOTES.md#automated-review-results-ready--2026-09-09)
record the candidates and compatibility limits. Enrichment-interface consolidation
is an unsized proposed design spike, not an implementation assignment.

## Verify what a reference search covers

Use indexed references to find candidates, then check index scope, generic trait
consumers, enabled configurations and ignored/unindexed root tests before calling a
definition unused. In this reviewed baseline, concrete implementation lookup missed
a generic repository consumer. Supplemental checks also found active
`livrarr_jobs::JobError` uses in the server and `StubAuthorLinkWorkflow` uses in
behavioral tests. These are specific corrections, not a claim that the index is
never useful. A textual test caller alone does not establish that its test target
runs. See [PM corrections](../../build/reviews/architecture-review-2026-09-07/scan-021/PM-CORRECTIONS.md).

## Measure deletion and replacement consistently

[Census revision 3](../../build/reviews/architecture-review-2026-09-07/scan-021/CENSUS-r3-CORRECTION.md)
excludes Rust doc comments while retaining attributes as code. Across 736 tracked
first-party files it records 277,203 physical lines, 234,449 estimated lexical code,
113,681 explicit test/support and 120,768 residual upper bound. Unmarked test helpers
may remain; the residual is not certified production. Earlier census values are
superseded, and no after-change measurement exists.

Count deletion minus replacement helpers, imports, adapters and caller changes;
relocation starts at zero net saving. Do not count removed comments, minification,
hidden generated code or reduced test coverage as simplification. A shared
abstraction needs a concrete maintenance benefit and replacement cost. Preserve
behavior, tests, clarity and safeguards. Partial inspection cannot certify global
invariants, behavior preservation or irreducible size. Integrity receipts establish
artifact/source agreement, not claim correctness or exhaustive coverage.

Source was the merge-containment worktree at
`af865bdb76d3cd1e42302f9791830479a26220f5` plus its recorded patch; the parked checkout
was not the review baseline. [Final integrity evidence](../../build/reviews/architecture-review-2026-09-07/close-023/FINAL-SOURCE-CHECK.json)
records unchanged source and canonical documents. This review authorizes no product
change or restart of the parked rewrite; unresolved product/principle questions remain open.
