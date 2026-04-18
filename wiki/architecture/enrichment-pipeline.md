# Enrichment Pipeline

The metadata enrichment system resolves book identity and populates work metadata from external providers. Lives in `livrarr-metadata`.

## Provider Stack

1. **Hardcover** — primary metadata provider. GraphQL API. Deterministic + fuzzy queries.
2. **Open Library** — secondary provider. REST API. English-language works only (never for foreign language).
3. **Audnexus** — audiobook-specific enrichment. REST API. Narration metadata, ASIN mapping.
4. **GoodReads** — supplementary. HTML scraping (no public API).
5. **LLM Validator** — resolves ambiguous Hardcover matches. OpenAI-compatible chat completions. Fully optional.

## Enrichment Modes

Three modes (not five — deliberate simplification):

| Mode | Trigger | Behavior |
|------|---------|----------|
| Background | Automated (RSS sync, author monitor) | Queue-based, respects rate limits |
| Manual | User clicks "Refresh" | Immediate, single work |
| HardRefresh | User forces full re-enrichment | Clears provider state, re-queries all |

## Flow

1. Work added (via search, RSS, or manual import)
2. Identity locked at add-time using LLM validator (if configured)
3. Deterministic provider query (Hardcover by ISBN/ASIN, then OL)
4. If ambiguous: LLM validator selects best match
5. MergeEngine applies provider results with provenance tracking
6. Per-field provenance: tracks which provider sourced each field
7. User edits take precedence (ProvenanceSetter::User)

## Provenance System

Every enrichable field has provenance metadata:
- **Who set it:** User / Provider / System
- **Which provider:** Hardcover / OpenLibrary / Audnexus / LLM
- User-owned fields survive manual refresh (reset_for_manual_refresh does NOT touch provenance)

## Error Handling

- Provider timeout: WillRetry state, exponential backoff
- All providers fail: work created with available data (Principle 6)
- Identity conflict (LLM disagrees with prior lock): EnrichmentStatus::Conflict terminal state
- Retry budget: 3 attempts, then EnrichmentStatus::Exhausted

## Privacy Boundary

Public metadata (titles, authors, ISBNs) sent to providers. Never send: filenames, paths, checksums, user preferences, API keys, user IDs.
