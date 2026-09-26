# Direct Add: seed persistence and test coverage

Static source review on 9 September 2026 found that the direct Add route discards
descriptive seed values before work creation. This is an open defect finding,
not an implemented fix or a runtime reproduction. The source was the containment
worktree at `af865bdb76d3cd1e42302f9791830479a26220f5` plus its recorded patch;
the parked main checkout is a different implementation baseline.

The search page sends language, year and cover information. The handler prepares
`seed_add_box`, then passes only title/author identity evidence and provider routes
into identity settlement. The settlement transaction creates the work without
those descriptive fields. Background completion receives the work ID and optional
candidate ID, not the discarded seed. A later successful provider response may
fill descriptive data, but cannot guarantee preservation of the original selection.

The important testing distinction is the entry path. The selected-cover test in
`test_wcc_add.rs` calls `WorkService::add`; the real HTTP handler calls the identity
road instead. The similarly named async-handler suite has seven ignored placeholder
tests, including a local boolean assertion for manual-cover intent. Existing real
router identity tests remain useful; their presence alone does not establish seed
persistence coverage. Test the actual router, authentication, database and cover
writer, with controlled external seams and assertions on the selected fields.

Preserve the single identity creator and atomic birth history when fixing the
wiring. A metadata seed must not overwrite a dedup winner or mint a second work.
Selected-cover intent also differs from a successful manual lock: the existing
cover helper locks only after bytes land, leaving failed downloads repairable.
The fix design must handle that distinction and interrupted completion explicitly.

Completion branches are not interchangeable. Capture/handoff failures stop the
direct-add chain; an enrichment failure may still proceed to delayed refresh.
Author bibliography and candidate jobs are independently scheduled for a newly
created author. Each asynchronous capture uses its own identity generation.
This review did not establish permanent recovery failure across all schedulers.

The [focused review](../../build/reviews/codebase-improvement-review-2026-09-09/focus-002/REVIEW.md)
contains exact source links, saved evidence, replacement proposals and future
validation requirements. It also confirms the previously recorded six unregistered
implementation suites and distinguishes them from the shared `common.rs` helper.
