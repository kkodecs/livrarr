# Author

## Executive summary

Authors belong to a user and connect their Works. Normal setup now creates the database index needed to save a new author; the repair is deployed on Oasis. Existing author identities are preserved. See [database setup](#new-database-setup) and [source and history](#source-and-history).

An Author is a user-scoped person associated with Works. Provider identity is held
in author route rows; legacy provider-key columns are frozen compatibility data.
The shared author-matching/adoption authority handles spelling and initial
compatibility. Ambiguity must not collapse two people.

Provider contributor slots do not always assert authorship. Preserve asserted,
unlabeled and explicitly labeled credits through the shared guard. Narrator,
translator and editor entries must not silently become author routes.
[Author identity lessons](../insights/identity.md#lesson-82).

Monitoring has three controls: whether the Author participates, whether new Works
are automatically added, and an optional publication cutoff. Monitor language
labels newly created Works; it is not a provider edition selector. Enabling
monitoring must leave a usable language through the shared database rule.
Work-level ebook/audiobook monitoring remains separate.

Bibliography/series caches are rebuildable data. Unreadable responses are not
empty bibliographies. A provider failure should preserve existing knowledge and
expose degraded behavior. Deleting an Author need not delete all Works; retain
the Work presentation fallback described in [Work](work.md).

## New-database setup

Development commit `dbb821b0` adds migration 089, which creates `idx_authors_identity` during normal database setup. Creating a new Work with a new author now succeeds on a fresh installation, including when metadata providers are unavailable. The test helper no longer creates the index separately; real-server tests cover setup and Add.

The migration preserves existing author rows and keys. NULL keys remain allowed, and uniqueness is scoped to the user. Conflicting non-null keys for one user cause startup to stop with a migration error, without merging/deleting records or recording the migration as successful. Compatibility metadata remains schema 83 / data 1. The old author-merging startup routine stays disabled.

Oasis already had the correct index, so its upgrade preserved the existing schema and all 24 author identities. See the [verified repair and deployment](../../build/reviews/fresh-author-index/RESULT.md), including the note about embedding newly added migration files in the executable.

At the earlier development revision `98d35a37`, normal fresh startup omitted this index and new-author Add returned HTTP 500. Migration 077 added the column only, and the August 19 change removed the startup routine that created the index. The alpha6 release tag predates that dependency; do not infer affected code from the executable's unchanged version string. The [original confirmation](../../build/reviews/catalog-work-lookup/fresh-author-confirmation-20260919/REVIEW.md) and [pre-repair guide](../../build/reviews/fresh-author-index/document-upkeep/predecessors/wiki/domain/author.md) are preserved.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/domain/author.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="lifecycle"></a>
<a id="monitoring"></a>
<a id="relationship-to-works"></a>
