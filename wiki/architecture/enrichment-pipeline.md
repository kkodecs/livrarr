# Enrichment, covers and tags

Enrichment fills descriptive metadata for an already identified local Work.
Identity and field completeness remain separate. The rules are
[metadata principles M1–M10](../domain/metadata-principles.md); creation and route
settlement are explained in [book creation](work-creation-pipeline.md).

## Responsibilities

The work workflow coordinates refresh and completion. The enrichment service
coordinates provider dispatch, stored outcomes and field merge. The provider queue
owns applicability, anchor selection, freshness and retry accounting. Provider
adapters normalize responses. The merge engine is the single field-policy owner;
the database applies its result atomically with generation checks. Cover and file
projection follow through their own protected write authorities.

There are Background, Manual and HardRefresh enrichment modes. Request priority
and cache freshness are independent controls. Do not infer a provider deadline or
cache policy solely from the mode.

## Provider dispatch

Applicable providers run concurrently within canonical transport limits.
Provider priority is a field-ranking policy, not a serial fallback chain.
English/unresolved-language enrichment excludes Google Books; foreign-language
enrichment excludes OpenLibrary and Hardcover. Goodreads, Google Books, Audnexus
and Audible can participate for foreign works according to capabilities and keys.
Discovery has separate wiring. [Provider roles](../domain/metadata-sources.md).

Descriptive fetches use typed anchors. Route-native search may discover missing
identity through the shared workflow, but its evidence must settle through the
identity authority. Never introduce an unguarded text-search-and-merge shortcut.

All HTTP uses the shared transport queue. Queue admission precedes interactive
request budgets. Circuit-open and queue-full are pauses, not terminal absences.
A readable absence differs from unreadable response data. Log provider and cause;
never replace good stored data with a parser's empty failure.

The persistent provider-response cache is consulted at one queue seam after
applicability, anchor derivation and terminal-state checks. Background/add paths
prefer cache; explicit interactive/bulk refresh bypasses it. Only successful
payloads are cached. Cache hits are processed normally but are not HTTP calls or
spent network attempts. [Transport lessons](../insights/providers-and-transport.md).

## Merge and provenance

All merge entrances, including cached reuse, apply the same language and payload
policies. User-owned fields survive refresh. `AutoAdded` is not `User`: automated
creation does not lock a field as a personal decision. Empty strings/lists are no
offer, and missing values must preserve last-known-good data.

Identity routes and cover columns are not generic descriptive merge outputs.
Rejected provider fields can produce dissents without blocking unrelated good
fields. A stored merge output may echo existing values; its presence does not
prove a change. Compare actual content to decide whether downstream retagging is
needed. [Merge lessons](../insights/metadata.md).

Enrichment quality has four statuses: Unenriched, Enriched, Thin and Failed.
Enriched does not mean every field is present; the reviewed quality check requires
at least one meaningful descriptive field. Retry exhaustion belongs to the
individual provider, not an invented Work-level Exhausted status.

## Covers

One rank model chooses candidates per language/media slot, and one write gate owns
download, measured dimensions, comparison, locking and recoverable commit. The
documented layout is `covers/{user_id}/{work_id}.jpg` and `_audio.jpg` for audio.
No cross-user root fallback is allowed.

User selection/upload uses the same commit mechanics but bypasses automatic ranking.
A manual lock protects real bytes or a pending recoverable commit, not a missing
file. A failed selected-cover download must leave the slot repairable. Ordinary
search-card art is not an explicit manual selection. Goodreads candidates remain
excluded by the documented containment policy; restoring them is separate work.
[Cover rules and recovery details](../insights/covers.md).

## Tags and permission

Database/file agreement is a product requirement within the originating action's
permission. Import and its unfinished background completion may finish tag work;
settled files require a new explicit action. Cleanup does not grant unattended
rewrites. [Import and file recovery](import-pipeline.md).

There are historical discrepancies around materialize-versus-TagService paths,
relative paths and disabled audio writers. Keep the single-owner requirement,
but verify the named source and actual entry path before claiming tags were
written or that all callers converge. See [history lessons](../insights/history-and-review.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/architecture/enrichment-pipeline.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="enrichment-pipeline"></a>
<a id="provider-stack"></a>
<a id="language-applicability-rule"></a>
<a id="enrichment-modes"></a>
<a id="flow-consolidation--single-implementation"></a>
<a id="hardcover-matching-detail"></a>
<a id="provenance-system"></a>
<a id="error-handling"></a>
<a id="privacy-boundary"></a>
