# Workflow entry-point guide

Each responsibility has one policy owner. A change to a workflow must identify
every affected handler, job and continuation and show how they reach that owner.
The intended seam/entity model is [canonical-model.yaml](../../docs/canonical-model.yaml).
Root architecture and principles take precedence over this navigation guide.

## Follow the operation

| Operation | Entry families to examine | Current explanation |
|---|---|---|
| Create or select a Work | Direct Add, manual file import, list confirmation, author/series monitoring, Readarr | [Creation and identity](work-creation-pipeline.md) |
| Enrich or refresh | Add completion, manual/bulk refresh, Retry-All, scheduled convergence, affirm/import follow-ups | [Enrichment](enrichment-pipeline.md) |
| Change a cover | Explicit select/upload, provider candidates, startup recovery | [Cover authority](../insights/covers.md#lesson-63) |
| Resolve identity/review | Captured route handoffs, user choices, typed review, mutation continuations, cutover CLI | [Review reference](identity-review-census.md) |
| Grab a release | User selection and scheduled/on-demand RSS | [Downloads](grab-system.md) |
| Import a file | Client pollers, automatic/manual retry, manual files, Readarr, scan adoption | [Import](import-pipeline.md) |
| Project file metadata | Import post-steps, authorized enrichment completion, explicit changes, recovery | [Tags and permission](enrichment-pipeline.md#tags-and-permission) |
| Monitor authors/series | Scheduled work, explicit triggers, create/promote/refresh follow-ups | [Author](../domain/author.md), [Series](../domain/series.md) |
| Sync playback | Progress writes, explicit cross-format sync/decline | [Cross-format resume](../domain/cross-format-resume.md) |

This table is a starting set for source enumeration, not a certified current caller
census. CRUD and configuration also require ownership and validation even when they
have only one known entrance. Adding another caller must not create another policy.

## Validation

Trace the real entry, evidence, state write and failure recovery. Check shared
instance identity, generation timing and shutdown behavior across continuations.
Prove persistent effects rather than only a function call. Source/tests may live
outside an index’s coverage; check generic consumers and whether tests are registered,
enabled and actually executed.

The previous 14-road map’s “all CLEAN” verdict was a July claim. Later audits found
counterexamples, and the identity rewrite removed several listed entrances. Its
original R1–R14 sections and decisions are preserved below through historical
links/bookmarks. They do not establish present convergence or permission to resume
the unfinished merge/conflict/undo feature.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/architecture/roads.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="roads--the-whole-app-one-road-map"></a>
<a id="r1--work-creation"></a>
<a id="r2--enrichment--refresh"></a>
<a id="r3--cover-change"></a>
<a id="r4--identity-state-changes"></a>
<a id="r5--release-grab"></a>
<a id="r6--import-file--libraryitem-one-road-for-all-doors"></a>
<a id="r7--manual-import-file-handling--absorbed-into-r6-2026-07-04"></a>
<a id="r8--readarr-import-file-handling--absorbed-into-r6-2026-07-04"></a>
<a id="r9--library-scan--absorbed-into-r6-2026-07-04"></a>
<a id="r10--tag-write--sync"></a>
<a id="r11--author-monitoring"></a>
<a id="r12--series-monitoring"></a>
<a id="r13--rss-sync"></a>
<a id="r14--playback-progress--cross-format-resume"></a>
<a id="not-roads-single-door-writes--no-convergence-contract-needed"></a>
<a id="dead-code-queued-for-deletion-found-during-mapping-2026-07-04"></a>
<a id="wiki-corrections-queued-stale-statements-this-mapping-falsified"></a>
