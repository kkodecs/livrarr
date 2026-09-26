# Metadata Principles

Metadata is one of the most critical aspects of the system. These principles govern all metadata handling across every entry path, enrichment flow, and file operation.

Read these with [root product principles](../../ARCHITECTURE.md) and
[engineering principles](../../PRINCIPLES.md), which take precedence. Policy goals
are not evidence that every path implements them. The
[2026-09-09 context review](../../build/reviews/architecture-review-2026-09-07/context-docs-017/REVIEW.md)
records corrected stale implementation wording and unresolved policy wording.

## M1: Metadata is sacred — treat with care

Metadata quality determines the user experience for every downstream feature: search, matching, display, tag writing, series grouping, cover display, RSS sync, OPDS. Bad metadata cascades. Every code path that touches metadata should be deliberate about what it writes, overwrites, or discards.

## M2: Every work and file gets the same treatment

All entry paths (search+add, manual import, Readarr import, author monitor, series monitor, list import) should produce the same metadata state for a given work. Provenance, enrichment, title cleanup, cover download, and tag writing apply uniformly. Exceptions must be explicitly noted; the default is full treatment.

## M3: Covers are particularly important

Cover images are the primary visual identity of a work. Cover resolution, quality, and availability should be prioritized. The cover pipeline (fetch → cache → embed in tags → serve via API) should be robust and complete for every entry path.

## M4: Improve metadata sources — give back to the community

Where possible, contribute corrections and additions back to open metadata sources (OpenLibrary, etc.). Don't just consume — improve the ecosystem. Design metadata flows with upstream contribution in mind.

## M5: User metadata is sovereign

Metadata explicitly set by the user must not be overwritten by automated enrichment, refresh, or any background process. Provenance enforces this. User-owned fields survive manual refresh, hard refresh, and re-enrichment. This is non-negotiable.

The setter is one of six, not three: `Provider`, `User`, `System`, `AutoAdded`, `Imported`, `Import` (`crates/livrarr-domain/src/enrichment_types.rs:176-198`). The distinction that matters for this principle is that **`AutoAdded` is not `User`** — a work created by the author or series monitor was never per-work validated by anyone, so it is deliberately not a lock anchor (`:187-192`).

## M6: DB metadata and file metadata must be synced

The metadata stored in the database and the metadata embedded in the file (EPUB/M4B/MP3 tags) must agree. When DB metadata changes (enrichment, user edit, refresh), the corresponding file tags must be updated. When a file is imported, it should be tagged with current DB metadata. Stale tags are a bug.

This synchronization goal operates within the file-write permission in root
`ARCHITECTURE.md`: import and its unfinished background completion are authorized
by the originating action; a settled library file requires a new explicit user
action. M6 does not independently authorize an unattended rewrite of settled files.

## M7: Use LLM cleanup liberally

Public metadata (titles, authors, descriptions, series names) can and should be cleaned up by LLM. There is no privacy concern with sharing publicly available book metadata with an LLM provider. Apply title cleanup, bibliography filtering and series list cleaning wherever it improves quality. The LLM privacy boundary (never send filenames, paths, checksums, user preferences, API keys, IDs) still applies.

**Identity validation is the exception, and it is off.** No LLM selects or confirms
a match. The old `settle_identity` workflow and unused LLM identity-verification
function were removed in the 2026-08-31 deletion pass; current provider capture and
deterministic settlement use the identity-layer workflow. "Cleanup" is repair of
text we already trust — it is not selection, and a repaired payload carries no
extra trust. See the amended [identity insights](../insights/identity.md).

## M8: We are the authority — always enrich

Source data (Readarr, CSV, search result, monitor detection) seeds identity — title, author, provider keys for matching. Livrarr's enrichment pipeline is the authority on final metadata. We always run our own enrichment regardless of how rich the source data is. Source metadata is a starting point, not a substitute.

## M9: Works enter the system fully formed — by path tier

**Binding invariant:** every entry path converges on the same identity and metadata for
the same work. Consistency means the same destination, not the same completion time.
Missing provider data and unresolved identity must remain visible and recoverable.

**Current interactive Add:** the local work is created or selected before the
response. Provider capture, enrichment and a conditional delayed refresh run in
the background for a newly created work. This supersedes the older claim that Add
waits for all descriptive metadata except covers and audiobook details. A title
and linked local Author permit creation; they do not by themselves prove an
unambiguous provider match or complete metadata.

**Background creation and recovery:** use current captured identity, provider routes
and review state. The legacy `identity_status` badge is frozen after the identity
cutover, so the old `identity-pending`/`needs-review` descriptions are not current
runtime selectors. Do not promise one universal wait policy, fixed throughput or
retry behavior for every entry path without checking that path.

The [Add trace](../../build/reviews/architecture-review-2026-09-07/search-add-011/EVIDENCE.json)
and [creation-minimum check](../../build/reviews/architecture-review-2026-09-07/minimum-012/EVIDENCE.json)
are targeted static evidence. They do not prove convergence across all paths.

## M10: No special cases by language

Foreign language works go through the same enrichment states and lifecycle as English works. The pipeline routes to different providers internally (Goodreads scraping, Hardcover API) but the status model, provenance, tag sync, and creation gate are identical. No separate states, no separate code paths.

---

## Relationship to existing principles

- M5 operationalizes the existing provenance system; see `wiki/architecture/enrichment-pipeline.md`
- M7 permits LLM cleanup assistance while prohibiting LLM match selection or confirmation
- M2 exposes the current inconsistencies documented in the metadata lifecycle report (provenance gaps in manual/Readarr import, missing tag writing in Readarr import, no post-enrichment retag)
- M6 is implied by "files are the artifact" (key invariant) but was never stated as a sync requirement
