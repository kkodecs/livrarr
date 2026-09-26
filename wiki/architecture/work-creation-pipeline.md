# Book creation and identity

## Executive summary

Creating a Work, identifying it across catalogs and filling its details are separate
steps. Follow the [entry paths](#entry-paths) and [identity rules](#identity-authority).
A catalog failing to find one edition’s ISBN does not mean it lacks the Work. The
[Jekyll investigation](#observed-catalog-matching-failure) records a title comparison
and retry problem in the deployed source; its correction is still proposed.

A Work is the book independent of edition or format. Creation minimum, identity
evidence and descriptive completeness are separate questions. A parsed title and
linked local Author permit a local record; that alone proves neither a provider
match nor complete metadata. [Product requirements](../../ARCHITECTURE.md) and
[metadata principles](../domain/metadata-principles.md) govern all entry paths.

## Entry paths

The documented creation paths are direct Add, manual file import, list import,
author monitoring, series monitoring and Readarr import. They carry different
evidence and user intent, but must converge on consistent identity and metadata.
Do not infer identical timing or failure handling from a shared helper.

Direct Add creates or selects the local Work before returning. For a newly created
Work, provider capture, enrichment and a conditional delayed refresh continue in
the background. Other source/author jobs may run independently. An existing-work
selection is not permission to overwrite its metadata with the incoming seed.

**Known wiring finding:** the reviewed Add handler drops selected language, year
and cover data before creation. Later enrichment cannot guarantee recovery of that
selection. The [focused review](direct-add-seed-review.md) records the exact static
trace, relevant test gaps and constraints; it is not a runtime reproduction or fix.

## Identity authority

The confidence order is user selection, owned-file evidence, known provider route,
title plus author, then unidentified. User choices are final within data safety;
uncertainty must remain visible and recoverable. ISBN is edition-scoped: agreement
may confirm sameness, disagreement proves nothing and cannot veto a Work match.

Provider capture produces evidence. Deterministic comparison and the identity
engine decide how it relates to the Work; the identity road coordinates the
repository transaction. Enrichment hands newly captured routes back to that
authority, bound to the identity generation observed before provider work.
It must not write identity independently or merely replay already-stored routes
and call that discovery. A graph-identical machine observation is a no-op.

The old `settle_identity`/quorum/anchor-writing path was removed. After the identity
cutover, legacy Work ID columns and `identity_status` are frozen compatibility
data. Read active captured identity/routes and the current review presentation.
Do not use a legacy Pending badge as evidence of active work or a retry selector.

Provider identifiers have typed namespaces. In particular, Goodreads Work IDs
cannot fetch Book pages; only a BookEdition route can do that. See
[Goodreads](../integrations/goodreads.md).

## Recovery and review

Background convergence uses current routes, provider standing and bounded attempt
accounting. Search eligibility is per provider; attempt accounting is per Work and
identity generation. Normal convergence preserves standing, manual Refresh resets
it, and actual route changes invalidate it. Pauses and provider failures must not
consume a successful-search/miss budget. [Detailed lessons](../insights/identity.md).

Review cards carry durable questions and decisions. Minting, equivalent-question
reuse and dismissal suppression have shared authorities. A card's original
generation is history; action must revalidate the generation and proposal the user
actually reviewed. Machine cancellation does not establish user dismissal.

Merge/conflict/undo work has a separate unfinished design. Do not read old success
descriptions or accepted designs as evidence that a merge action is available.
Consult [current project state](../../build/state/) and the
[review-reference page](identity-review-census.md) before changing these paths.

## Observed catalog matching failure

The 19 September 2026 Jekyll trace, on deployed commit `9f366b77`, found that
post-Add title matching rejects `&` versus `and` even with the same author. Later
Hardcover/OpenLibrary metadata calls use the saved ISBN and do not fall back within
that call. A later pass can search by title/author after a recorded ISBN miss, but
Add's automatic Refresh clears that recorded miss and repeats the ISBN attempt.
Search results shown before Add therefore do not guarantee a saved catalog Work ID.
See the [runtime evidence and matcher reproduction](../../build/reviews/provider-priority-cache/live-add-trace-20260919T000839Z/CATALOG-LOOKUP.md).

Google Books supplies editions and is intentionally excluded from catalog Work
identity. Its missing Work-ID row is not a defect. Do not mistake that design choice
for the failed matching against Work catalogs.

## What to trace for a change

Follow the real handler/job, seed/evidence construction, settlement, persistence,
background continuation and failure recovery. Preserve selected metadata, user
provenance, atomic birth history, dedup winners and fresh generation observations.
Tests must drive the actual entry path with real SQLite and controlled external
dependencies. Calling `WorkService::add` does not prove an HTTP handler that takes
a different route is wired correctly.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/architecture/work-creation-pipeline.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="work-creation-pipeline--the-five-phases-and-the-m9-convergence-gap"></a>
<a id="the-five-phases"></a>
<a id="where-each-door-identifies-phase-1"></a>
<a id="what-is--and-isnt--a-bug-here-read-m9-first"></a>
<a id="secondary-gap-quality-not-correctness"></a>
<a id="fix-pointer--closed"></a>
<a id="confidence"></a>
<a id="stale-sibling-reconciled"></a>
