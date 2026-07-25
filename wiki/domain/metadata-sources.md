# Metadata Sources

Which providers supply metadata, their priority, fallback behavior, and the foreign language problem.

## English Pipeline

| Provider | Role | API | Auth | Rate Limit |
|----------|------|-----|------|------------|
| **Hardcover** | Primary metadata | GraphQL | Token, sent as `Authorization: Bearer <token>` | 1 req/s |
| **OpenLibrary** | Fallback metadata | REST | None | 1 req/s |
| **Audnexus** | Audiobook enrichment | REST | None | 0.5 req/s |
| **Goodreads** | Cover quality, series data, bibliography | HTML scraping (LLM repair fallback) | None | 1 per 1.5s |
| **LLM** | HTML-parse repair on foreign GR pages; cleanup tasks | OpenAI-compatible | API key | Provider-dependent |

Rate limits are the outbound queue's per-bucket pace
(`crates/livrarr-http/src/outbound_queue.rs:239-249`) — HC/OL/GB 1s, Goodreads 1.5s, Audnexus
2s. On Hardcover auth: HC's published format is a raw lowercase `authorization` header, but
what we send is `Authorization: Bearer <token>` — see `wiki/integrations/hardcover.md`, where
that gap is an open P1.

> **Goodreads does NOT require an LLM.** The GR pick is deterministic: `gr_best_match` — a
> junk-edition filter plus the shared title+author picker — and the adapter's own doc states
> "no LLM is involved in the pick"
> (`crates/livrarr-external-data/src/provider_client.rs:1967-1976`). Nothing clearing the bar
> means GR **abstains**, which is where `NotFound` comes from — not from a missing LLM. The
> only LLM on the GR path is `llm_repair`, an extraction fallback for foreign-language detail
> pages whose parse failed (`crates/livrarr-external-data/src/goodreads/mod.rs:4-5`); when no
> live-config handle is present that fallback is simply disabled, and the rest of the provider
> works normally (`provider_client.rs:1998-2002`). GR is still a hostile scraping target —
> anti-bot, HTML drift, noisy hits — which is why the bar is set to abstain rather than guess.

### Provider Priority

**This is a merge rank, not a fallback chain.** Every applicable provider is dispatched in
parallel into one `JoinSet` (`crates/livrarr-enrichment/src/provider_queue.rs:1-4`,
`:348-352`); the merge engine then resolves each field by rank. Nothing waits for Hardcover to
"fail" first.

English content/description rank (`crates/livrarr-enrichment/src/merge_engine.rs:38-54`):

1. Hardcover → deterministic match by title+author, highest `users_read_count`
2. Goodreads
3. Readarr
4. OpenLibrary → description + ISBN from editions
5. Audible

Covers do not use this order — they have their own rank table
(`cover_rank::CoverRankModel::EbookEnglish`, `merge_engine.rs:55-58`). Audnexus contributes
narrator, duration and ASIN independently of the content rank.

### Timeouts

The HTTP timeout is fixed per call site, not per enrichment mode — there is no
synchronous-vs-background switch:

- **10s:** Hardcover (`crates/livrarr-external-data/src/hardcover.rs:79`, `:394`), Google Books
  (`google_books.rs:102`, `:150`), OpenLibrary `search.json` (`openlibrary.rs:313`)
- **30s:** OpenLibrary work-detail / editions / ISBN (`openlibrary.rs:55`, `:141`, `:234`) and
  its title+author search (`provider_client.rs:1051`), Goodreads
  (`goodreads/client.rs:173`, `:231`), Audnexus (`audnexus.rs:160`), Audible
  (`audible.rs:252`)

## Foreign Language Pipeline

Foreign works go through the same enrichment pipeline as English works but with different provider priority and an additional provider (Google Books).

### Search/Discovery

Foreign lookup routes through **OpenLibrary first** (with `language=` filter using ISO 639-3 codes), falling back to **Goodreads** (HTML parsing). OL is used for discovery (finding the work, getting OLID + ISBN), not for metadata enrichment.

### Enrichment Providers

| Provider | Role | API | Auth | Rate Limit |
|----------|------|-----|------|------------|
| **Google Books** | Primary foreign metadata | REST JSON | API key (X-Goog-Api-Key header) | 1 req/s |
| **Goodreads** | Fallback metadata (LLM repairs a failed parse) | HTML scraping | None | 1 per 1.5s |
| **Audnexus** | Audiobook enrichment | REST | None | 0.5 req/s |

### Foreign Priority Order

Content and description: GoogleBooks → Goodreads → Hardcover → Readarr → OpenLibrary →
**Audible** (`crates/livrarr-enrichment/src/merge_engine.rs:65-83`). Covers are ranked
separately (`cover_rank::CoverRankModel::EbookForeign`, `:84-87`), so this is not a
cover order.

**Hardcover and OpenLibrary never actually contribute to a foreign work**, despite sitting in
that list. Their payloads are removed from the merge inputs at the chokepoint
(`drop_language_incompatible_providers`, `merge_engine.rs:263-282`) — the function's own note
explains why reordering was insufficient. Their *anchors* are still captured upstream at the
identity resolver, which is language-agnostic (`:267-269`); only metadata contribution is
dropped.

### Language Gate

Non-English languages are selectable when EITHER `llm_configured` OR `google_books_configured` is true — a non-`en` code is stripped only when both are false (`crates/livrarr-external-data/src/language.rs:115-117`). The `requires_llm` field in SUPPORTED_LANGUAGES is display metadata only — not used for gating; every one of the nine supported languages currently carries `requires_llm: false` (`language.rs:11-75`).

### Google Books Details

- ISBN lookup preferred (direct match, no scoring needed)
- Title+author fallback: `intitle:`/`inauthor:` with `langRestrict` and `maxResults=5`
  (`crates/livrarr-external-data/src/google_books.rs:398`), then the **shared deterministic
  picker** `identity_matching::pick_best_candidate` with `accept_grey = false` (`:455-460`) —
  the same authority every other provider uses. The old Jaccard ≥ 0.75 / author-overlap ≥ 1
  scoring is gone; nothing clearing the bar means GB abstains
- All data is `reference_only` — display/cache, never contributed upstream
- Cover URLs normalized: HTTPS, zoom=0, SSRF validated, embedded credentials rejected
- Descriptions HTML-stripped before storage
- CJK titles (Japanese, Korean) currently return NotFound due to Latin-centric tokenization (#54)

### Cover Resolution (foreign)

The foreign-ebook cover rank is seven providers, not two: GB → Goodreads → Hardcover → Readarr
→ OpenLibrary → Audnexus → Audible (`crates/livrarr-enrichment/src/cover_rank.rs:55-64`). That
one table is the single authority for all three cover call sites (`:1-8`). Two caveats worth
holding together: Hardcover and OpenLibrary *provider payloads* are dropped for foreign works
(see the drop rule above), and a cover can also arrive from a non-provider source — EPUB,
ISBN→OL, ISBN→Amazon (`crates/livrarr-domain/src/enrichment_types.rs:84-90`), which is the
"OL covers API by ISBN" step.

## Key Rules

- **Never use OpenLibrary for foreign language.** OL's foreign language coverage is unreliable.
  Enforced in code, not just by convention — see the drop rule above.
- **There is no `metadata_source` column.** It was dropped by migration 061 as a dead column —
  "zero readers and zero writers anywhere in the workspace" — superseded by `works.language`
  plus `works.enrichment_source` (`crates/livrarr-db/migrations/061_drop_metadata_source.sql:1-5`).
  Nothing is stored at creation on it and no refresh is skipped because of it.
- **A Google Books key alone unlocks non-English languages** — an LLM is not required. The
  strip runs only when neither is configured (`crates/livrarr-external-data/src/language.rs:115-117`),
  and no supported language is marked LLM-dependent today (`:11-75`).

## Provider Gotchas

> **The four library-catalogue entries below are historical.** No SRU client, and no DNB, KB,
> NDL or OPAC SBN provider, exists anywhere in `crates/*/src/` or the frontend — the foreign
> path is Google Books plus Goodreads, and OL's language filter replaced the catalogue
> approach. Keep these as a record of why that route was abandoned; do not read them as live
> behaviour.

- **DNB** needs SRU v1.1 (not 1.2), uses `rdau:P60327` for author (not Dublin Core `creator`), `bibo:isbn13` for ISBN
- **KB** needs bare CQL queries, not `title="{query}"`
- **NDL** returns entity-escaped DC XML in recordData
- **OPAC SBN** is client-side rendered — doesn't work for scraping, replaced by OL language filter
- **Goodreads CDN thumbnails** are often 50-75px — can upsize via URL rewrite (`_SY75_` → `_SX200_`)
