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

Metadata explicitly set by the user must not be overwritten by automated enrichment, refresh, or any background process. Provenance (User > Provider > System) enforces this. User-owned fields survive manual refresh, hard refresh, and re-enrichment. This is non-negotiable.

## M6: DB metadata and file metadata must be synced

The metadata stored in the database and the metadata embedded in the file (EPUB/M4B/MP3 tags) must agree. When DB metadata changes (enrichment, user edit, refresh), the corresponding file tags must be updated. When a file is imported, it should be tagged with current DB metadata. Stale tags are a bug.

## M7: Use LLM cleanup liberally

Public metadata (titles, authors, descriptions, series names) can and should be cleaned up by LLM. There is no privacy concern with sharing publicly available book metadata with an LLM provider. Apply title cleanup, bibliography filtering, series list cleaning, and identity validation wherever it improves quality. The LLM privacy boundary (never send filenames, paths, checksums, user preferences, API keys, IDs) still applies.

## M8: We are the authority — always enrich

Source data (Readarr, CSV, search result, monitor detection) seeds identity — title, author, provider keys for matching. Livrarr's enrichment pipeline is the authority on final metadata. We always run our own enrichment regardless of how rich the source data is. Source metadata is a starting point, not a substitute.

## M9: Works enter the system fully formed

Enrichment is synchronous — part of work creation, not a background afterthought. A work is not "created" until enrichment has completed (or explicitly failed). No deferred enrichment, no "eventually consistent" metadata. The user sees progress and gets a complete result.

## M10: No special cases by language

Foreign language works go through the same enrichment states and lifecycle as English works. The pipeline routes to different providers internally (SRU national libraries, LLM scrapers) but the status model, provenance, tag sync, and creation gate are identical. No separate states, no separate code paths.

---

## Relationship to existing principles

- M5 operationalizes the existing provenance system (insight 15, wiki/architecture/enrichment-pipeline.md)
- M7 relaxes the "LLM is a fallback" posture (insight 13) for cleanup tasks while maintaining it for matching
- M2 exposes the current inconsistencies documented in the metadata lifecycle report (provenance gaps in manual/Readarr import, missing tag writing in Readarr import, no post-enrichment retag)
- M6 is implied by "files are the artifact" (key invariant) but was never stated as a sync requirement
