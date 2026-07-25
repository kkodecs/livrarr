# Metadata Principles

Metadata is one of the most critical aspects of the system. These principles govern all metadata handling across every entry path, enrichment flow, and file operation.

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

## M7: Use LLM cleanup liberally

Public metadata (titles, authors, descriptions, series names) can and should be cleaned up by LLM. There is no privacy concern with sharing publicly available book metadata with an LLM provider. Apply title cleanup, bibliography filtering and series list cleaning wherever it improves quality. The LLM privacy boundary (never send filenames, paths, checksums, user preferences, API keys, IDs) still applies.

**Identity validation is the exception, and it is off.** No LLM selects or confirms a match anywhere on the identity path: the add door runs the deterministic `settle_identity` authority, and the one LLM identity-verify function in the tree has no caller at all (`crates/livrarr-identity/src/async_resolver.rs:46-97`; see `wiki/domain/work.md`). "Cleanup" is repair of text we already trust — it is not selection, and a repaired payload carries no extra trust.

## M8: We are the authority — always enrich

Source data (Readarr, CSV, search result, monitor detection) seeds identity — title, author, provider keys for matching. Livrarr's enrichment pipeline is the authority on final metadata. We always run our own enrichment regardless of how rich the source data is. Source metadata is a starting point, not a substitute.

## M9: Works enter the system fully formed — by path tier

A work's creation completes **synchronously where the user is watching, and converges in the background where they are not.** "Consistency" here means *same destination, not same clock.*

- **Interactive paths (Add Work, manual-import per-file review):** synchronous and fully-formed. The work is returned already populated with every field derivable from data in hand (REQ-015); only genuinely async work — cover bytes, Audnexus narrator/duration — completes afterward.
- **Batch / background paths (list import, Readarr import, series/author monitors):** MAY create a work in an `identity-pending` state that converges to full identity via the shared async resolver (REQ-022, REQ-026). An item the resolver cannot deterministically resolve transitions to a terminal, **surfaced** `needs-review` state — never silent limbo, never an indefinite retry loop.

**Binding invariant:** every path converges on the same identity and metadata for the same work (REQ-022). This is *stricter* than the prior state, where single-anchor works created by different paths could diverge permanently.

**Rationale:** synchronous-complete creation was an ideal for simplicity, not an architectural necessity — the CAS/retry machinery already supports incremental updates, and the providers already preclude it (cover download is async; Goodreads is anti-bot + LLM-gated; a timed-out provider must abstain and converge later, REQ-025). The split keeps interactive simplicity where it is user-visible and confines eventual-consistency complexity to background paths the user does not watch.

For bulk operations, multiple `add()` calls run with bounded concurrency (`buffer_unordered(5)`); rate limiters throttle provider calls naturally (~30 works/minute sustained).

## M10: No special cases by language

Foreign language works go through the same enrichment states and lifecycle as English works. The pipeline routes to different providers internally (Goodreads scraping, Hardcover API) but the status model, provenance, tag sync, and creation gate are identical. No separate states, no separate code paths.

---

## Relationship to existing principles

- M5 operationalizes the existing provenance system (insight 15, wiki/architecture/enrichment-pipeline.md)
- M7 relaxes the "LLM is a fallback" posture (insight 13) for cleanup tasks while maintaining it for matching
- M2 exposes the current inconsistencies documented in the metadata lifecycle report (provenance gaps in manual/Readarr import, missing tag writing in Readarr import, no post-enrichment retag)
- M6 is implied by "files are the artifact" (key invariant) but was never stated as a sync requirement
