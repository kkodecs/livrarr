# Enrichment Pipeline

The metadata enrichment system resolves book identity and populates work metadata from external providers. Lives in `livrarr-metadata`.

## Provider Stack

Six **network providers** dispatched through the `ProviderClient` enum by `DefaultProviderQueue` (all registered in `livrarr-server/src/main.rs`), plus one synthetic source provider.

1. **Hardcover** — primary English metadata. GraphQL API. Deterministic + fuzzy queries. **Excluded for foreign-language works** (applicability rule below).
2. **Open Library** — secondary. REST API. English only — **excluded for foreign-language works**. Emits a `cover_url` built from OL cover IDs in normalized output.
3. **Goodreads** — supplementary. HTML scraping (no public API). Fully deterministic matching: `gr_best_match` drops junk editions, then delegates to the one shared picker `identity_matching::pick_best_candidate` — **not** a "0.75 picker"; that loose jaccard scorer is gone (`crates/livrarr-external-data/src/provider_client.rs:2623-2651`). GR passes `accept_grey = true`, the only provider that does (`:2649`); everything else passes `false`. The only LLM use anywhere on its path is HTML-parse repair. Runs for English **and** foreign.
4. **Audnexus** — audiobook enrichment. REST API. Narration/duration, keyed on ASIN.
5. **Google Books** — **foreign-language metadata provider**. REST API, **requires an API key** (keyless quota is zero). **Excluded for English works; included for foreign** — it is the primary foreign-language metadata source (insights #12).
6. **Audible** — audiobook-axis provider. Catalog search + ASIN lookup (unauthenticated).
7. **Readarr** — *synthetic* provider, not a network client: built from injected `SourceProviderData` and arbitrated by the merge engine like any other provider.

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
2. Identity settled at add-time by the deterministic `settle_identity` authority — **no LLM validator** (`crates/livrarr-identity/src/async_resolver.rs:125-284`; the FLM gate is title+author, `:318-353`, and the one LLM identity-verify function has no caller, `:46-97`)
3. Provider dispatch (scatter-gather): the applicable providers (per the language rule above) queried based on mode
4. Normalize results via `NormalizedWorkDetail`
5. MergeEngine applies provider results with provenance tracking (pure — no DB calls)
6. Merge output includes: field updates, provenance upserts/deletes, external ID updates, conflict detection
7. Atomic merge apply via CAS (`merge_generation` column on works table)
8. Cover cached to `{covers_dir}/{work_id}{suffix}.jpg` — suffix is `""` for the ebook slot and `"_audio"` for the audiobook slot (`crates/livrarr-materialize/src/lib.rs:17-23`)

## Hardcover Matching Detail

- **Deterministic (tier 1):** normalize titles, exact case-insensitive match, highest `users_read_count` breaks ties
- **Tier 2 is deterministic too — there is no LLM tier.** A tier-1 miss falls through to the same shared `pick_best_candidate`, and nothing clearing the bar means Hardcover abstains rather than guessing (`crates/livrarr-external-data/src/hardcover.rs:231-266`). The old `llm_disambiguate` pick is gone.
- GraphQL endpoint: `https://api.hardcover.app/v1/graphql` (fixed, not configurable)
- Auth: we send `Authorization: Bearer <token>` (`crates/livrarr-external-data/src/hardcover.rs:72`, `:388`). HC's *published* format is a raw lowercase `authorization` header with no prefix — the gap is a known open P1, see [hardcover](../integrations/hardcover.md).
- Language filtering: select edition matching configured language prefs with highest `users_read_count` for primary ISBN

## Provenance System

Every enrichable field has provenance metadata:
- **Who set it:** one of six setters — `Provider`, `User`, `System`, `AutoAdded`, `Imported`, `Import` (`crates/livrarr-domain/src/enrichment_types.rs:176-198`). `AutoAdded` is deliberately not a user lock anchor (`:187-192`)
- **Which provider:** one of eight — Hardcover, OpenLibrary, Goodreads, Audnexus, Llm, Readarr, GoogleBooks, Audible (`enrichment_types.rs:13-24`)
- User-owned fields survive manual refresh (reset_for_manual_refresh does NOT touch provenance)

## Error Handling

- **Provider timeout / 5xx:** `WillRetry { ServerError }` with a **fixed** next attempt, not exponential backoff — 5 minutes for Hardcover (`crates/livrarr-external-data/src/provider_client.rs:578`, `:758-761`). A live 429 is the one that backs off hard: `WillRetry { RateLimit }` at 6h + up to 3h jitter (`:242-250`).
- All providers fail: work created with available data (Principle 6)
- **Identity conflict is not an `EnrichmentStatus` and no LLM is involved.** `EnrichmentStatus` has four values — `Unenriched`, `Enriched`, `Thin`, `Failed` (`crates/livrarr-domain/src/entities.rs:83-102`); the identity outcomes left it in migration 055 (`:97-101`). `IdentityStatus::Conflict` means a differing confirmed anchor, terminal until the user resolves it (`:122-124`).
- **Retry budget is 5 attempts, per provider, and there is no `EnrichmentStatus::Exhausted`.** Reaching `max_attempts` converts that one provider to `PermanentFailure { RetryBudgetExhausted }` (`crates/livrarr-enrichment/src/provider_queue.rs:586-590`); production sets `max_attempts = 5` (`crates/livrarr-server/src/main.rs:833-836`). The work's own status is unchanged by it.

## Privacy Boundary

Public metadata (titles, authors, ISBNs) sent to providers. Never send: filenames, paths, checksums, user preferences, API keys, user IDs.
