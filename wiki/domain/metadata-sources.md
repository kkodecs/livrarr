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

Foreign language works use completely different providers. The English enrichment pipeline is **skipped** for foreign works — it would overwrite native-language metadata with English data.

### SRU National Library Providers (structured API)

| Language | Provider | Protocol | Format |
|----------|----------|----------|--------|
| Spanish | BNE (Biblioteca Nacional de España) | SRU 1.1 | MARC21 |
| French | BnF (Bibliothèque nationale de France) | SRU | UNIMARC |
| German | DNB (Deutsche Nationalbibliothek) | SRU 1.1 | RDF |
| Dutch | KB (Koninklijke Bibliotheek) | SRU | Dublin Core |
| Japanese | NDL (National Diet Library) | SRU | Dublin Core |

### LLM Scrape Providers (HTML → LLM extraction)

| Language | Provider | Notes |
|----------|----------|-------|
| Polish | lubimyczytac.pl | Works well |
| Korean | Kyobo | Works well |
| Italian | OPAC SBN → replaced by OL language filter | SBN was client-rendered |

### Cover Resolution (foreign)

Fallback chain: OL covers API by ISBN → Google Books thumbnail by ISBN → no cover.

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
