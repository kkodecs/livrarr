# Enrichment Pipeline

The metadata enrichment system resolves book identity and populates work metadata from external providers. Lives in `livrarr-metadata`.

## Provider Stack

Six **network providers** dispatched through the `ProviderClient` enum by `DefaultProviderQueue` (all registered in `livrarr-server/src/main.rs`), plus one synthetic source provider and an optional validator. (Note: only `OpenLibraryProvider` and `LlmScraperProvider` implement the legacy `MetadataProvider` trait — the queue dispatches via `ProviderClient::fetch`, not that trait.)

1. **Hardcover** — primary English metadata. GraphQL API. Deterministic + fuzzy queries. **Excluded for foreign-language works** (applicability rule below).
2. **Open Library** — secondary. REST API. English only — **excluded for foreign-language works**. Does not emit a `cover_url` in normalized output.
3. **Goodreads** — supplementary. HTML scraping (no public API). LLM-disambiguated; returns `NotFound` without an LLM. Runs for English **and** foreign.
4. **Audnexus** — audiobook enrichment. REST API. Narration/duration, keyed on ASIN.
5. **Google Books** — **foreign-language metadata provider**. REST API, **requires an API key** (keyless quota is zero). **Excluded for English works; included for foreign** — it is the primary foreign-language metadata source (insights #12).
6. **Audible** — audiobook-axis provider. Catalog search + ASIN lookup (unauthenticated).
7. **Readarr** — *synthetic* provider, not a network client: built from injected `SourceProviderData` and arbitrated by the merge engine like any other provider.
8. **LLM Validator** — identity validation (rejects mismatched payloads). OpenAI-compatible chat completions. Fully optional — merge is deterministic without it.

### Language applicability rule

The queue applies a per-work applicability rule (`main.rs` → `with_applicability_rule`) **before** dispatch:

- **English (or unresolved) language:** every registered provider runs **except Google Books**.
- **Foreign language:** **only** Goodreads, Audnexus, Google Books, and Audible run — **OpenLibrary and Hardcover are excluded** (English-language metadata leaking into a foreign record is a known corruption; insights #12/#16).

> The provider set consulted during interactive **discovery** (Add Work search, the pre-add cover picker) is wired *separately* from the enrichment queue (`LivePreaddCoverService` / `LiveCoverService` client maps in `main.rs`) and is not identical to it. See [metadata-pathway](metadata-pathway.md) for the authoritative current add → enrich → merge flow.

## Enrichment Modes

Three modes (not five — deliberate simplification):

| Mode | Trigger | Behavior |
|------|---------|----------|
| Background | Automated (RSS sync, author monitor) | Queue-based, respects rate limits |
| Manual | User clicks "Refresh" | Immediate, single work |
| HardRefresh | User forces full re-enrichment | Clears provider state, re-queries all |

## Flow (Consolidation — Single Implementation)

After consolidation, `EnrichmentWorkflow` is the single implementation. `WorkService::add` delegates to it.

1. Work added (via search, RSS, or manual import)
2. Identity locked at add-time using LLM validator (if configured)
3. Provider dispatch (scatter-gather): the applicable providers (per the language rule above) queried based on mode
4. Normalize results via `NormalizedWorkDetail`
5. MergeEngine applies provider results with provenance tracking (pure — no DB calls)
6. Merge output includes: field updates, provenance upserts/deletes, external ID updates, conflict detection
7. Atomic merge apply via CAS (`merge_generation` column on works table)
8. Cover cached to `{data_dir}/covers/{work_id}.jpg`

## Hardcover Matching Detail

- **Deterministic (tier 1):** normalize titles, exact case-insensitive match, highest `users_read_count` breaks ties
- **LLM fallback (tier 2):** if tier 1 ambiguous and LLM configured, background task resolves
- GraphQL endpoint: `https://api.hardcover.app/v1/graphql` (fixed, not configurable)
- Auth: `authorization: <token>` header (no Bearer prefix)
- Language filtering: select edition matching configured language prefs with highest `users_read_count` for primary ISBN

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
