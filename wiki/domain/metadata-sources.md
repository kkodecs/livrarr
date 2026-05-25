# Metadata Sources

Which providers supply metadata, their priority, fallback behavior, and the foreign language problem.

## English Pipeline

| Provider | Role | API | Auth | Rate Limit |
|----------|------|-----|------|------------|
| **Hardcover** | Primary metadata | GraphQL | Token (no Bearer prefix) | 1 req/s |
| **OpenLibrary** | Fallback metadata | REST | None | Courtesy only |
| **Audnexus** | Audiobook enrichment | REST | None | 0.5 req/s |
| **Goodreads** | Series data, bibliography | HTML scraping | None | Courtesy only |
| **LLM** | Ambiguity resolution | OpenAI-compatible | API key | Provider-dependent |

### Provider Priority

1. Hardcover (if token configured) → deterministic match by title+author, highest `users_read_count`
2. OpenLibrary (if Hardcover fails/not configured) → description + ISBN from editions
3. Audnexus (always, independent) → narrator, duration, ASIN

### Timeouts

- Synchronous enrichment (add-time): 10s per provider (amended from 3s — Hardcover GraphQL regularly takes 2-5s)
- Background enrichment (retry queue): 30s default
- Total enrichment budget: 30s

## Foreign Language Pipeline

Foreign works go through the same enrichment pipeline as English works but with different provider priority and an additional provider (Google Books).

### Search/Discovery

Foreign lookup routes through **OpenLibrary first** (with `language=` filter using ISO 639-3 codes), falling back to **Goodreads** (HTML parsing). OL is used for discovery (finding the work, getting OLID + ISBN), not for metadata enrichment.

### Enrichment Providers

| Provider | Role | API | Auth | Rate Limit |
|----------|------|-----|------|------------|
| **Google Books** | Primary foreign metadata | REST JSON | API key (X-Goog-Api-Key header) | 1 req/s |
| **Goodreads** | Fallback metadata (LLM-dependent) | HTML scraping + LLM | None | 1 req/s |
| **Audnexus** | Audiobook enrichment | REST | None | 0.5 req/s |

### Foreign Priority Order

GoogleBooks → Goodreads → Hardcover → Readarr → OpenLibrary (for content/description/cover fields)

### Language Gate

Non-English languages are selectable when EITHER `llm_configured` OR `google_books_configured` is true. The `requires_llm` field in SUPPORTED_LANGUAGES is display metadata only — not used for gating.

### Google Books Details

- ISBN lookup preferred (direct match, no scoring needed)
- Title+author fallback with `langRestrict` and candidate scoring (Jaccard >= 0.75, author overlap >= 1)
- All data is `reference_only` — display/cache, never contributed upstream
- Cover URLs normalized: HTTPS, zoom=0, SSRF validated, embedded credentials rejected
- Descriptions HTML-stripped before storage
- CJK titles (Japanese, Korean) currently return NotFound due to Latin-centric tokenization (#54)

### Cover Resolution (foreign)

Google Books cover (from enrichment) → OL covers API by ISBN → no cover.

## Key Rules

- **Never use OpenLibrary for foreign language.** OL's foreign language coverage is unreliable.
- **Foreign works store `metadata_source`** at creation. Metadata refresh is skipped for foreign-source works.
- **LLM-dependent languages can't be enabled without LLM configured.** Backend enforces — strips LLM-dependent languages if LLM config is incomplete.
- **All SRU string fields are NFC-normalized at parse time.**
- **SRU timeouts:** 10s. LLM scrape timeouts: 60s (includes HTTP fetch + LLM round-trip).

## Provider Gotchas

- **DNB** needs SRU v1.1 (not 1.2), uses `rdau:P60327` for author (not Dublin Core `creator`), `bibo:isbn13` for ISBN
- **KB** needs bare CQL queries, not `title="{query}"`
- **NDL** returns entity-escaped DC XML in recordData
- **OPAC SBN** is client-side rendered — doesn't work for scraping, replaced by OL language filter
- **Goodreads CDN thumbnails** are often 50-75px — can upsize via URL rewrite (`_SY75_` → `_SX200_`)
