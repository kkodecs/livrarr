# Work

The primary entity in Livrarr (Principle 1: work-first, not author-first). A Work represents a title — independent of format, edition, or packaging.

## Lifecycle

1. **Created** — via search+add, RSS sync auto-add, or author monitor detection
2. **Identity locked** — LLM validator confirms provider match at add-time
3. **Enriching** — metadata enrichment runs (Background/Manual/HardRefresh)
4. **Enriched** — all available metadata populated
5. **Monitored** (optional) — RSS sync watches for matching releases

### Terminal States

- **Exhausted** — 3 retry failures, no more automatic enrichment
- **Conflict** — identity drift detected (LLM disagrees with prior lock)

## Key Fields

- Title, original title, description, publication date
- ISBN, ASIN, Hardcover key, OpenLibrary key, GoodReads key
- Media type (ebook, audiobook, or both)
- Enrichment status, enriched_at, enrichment_retry_count
- merge_generation (CAS guard for atomic enrichment merge)

## Provenance

Every enrichable field has per-field provenance tracking:
- Which provider set it (Hardcover, OL, Audnexus, LLM)
- Whether the user overrode it (User > Provider > System)
- User-owned provenance survives manual refresh

## Semantics

- "Missing" (no file on disk) is NOT the same as "wanted" (monitored for download). Don't conflate these.
- A Work can have both ebook and audiobook releases simultaneously.
- Works belong to Authors (many-to-one primary, with additional authors possible).
