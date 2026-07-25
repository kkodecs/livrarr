# Hardcover Integration

Hardcover (hardcover.app) is Livrarr's primary English metadata provider — a small startup with a GraphQL API (Hasura-backed) for book/author/edition data. We use it heavily for works, editions, series, and covers.

## Current operational status

- **Active use today:** primary English metadata enrichment, cover sourcing, series mapping.
- **API maturity:** explicitly **beta**. HC actively iterates and may break things. Quote from docs: *"we may reset tokens without notice while in beta."*
- **Endpoint:** `https://api.hardcover.app/v1/graphql`
- **Contact:** Hardcover Discord (linked from docs). No email path published.
- **Their stance:** welcoming. *"We are actively looking for feedback on this API. Build something awesome and share it with us on Discord."*
- **Funding model: bootstrapped / self-funded** (per Crunchbase profile of founder Adam Fortuna). Not venture-backed. This is structurally meaningful for resilience — no investor pressure to monetize aggressively or exit via acquisition. Closer to MusicBrainz / OpenStreetMap in posture than to a VC-backed startup-on-the-acquisition-block. Hostility-risk profile is **lower** than the Gemini Deep Research scorecard suggested (it had this wrong).

## Authentication

API key required. **Per-user, not per-app** — every Livrarr install needs its admin to bring their own HC token.

```
authorization: <token>
```

> **⚠ Header format gotcha.** Docs show the raw token in a lowercase `authorization` header — **no `Bearer` prefix**. Our current code sends `Authorization: Bearer {token}` at every HC request-builder found: the client's two (`crates/livrarr-external-data/src/hardcover.rs:72`, `:388`), the discovery search (`crates/livrarr-metadata/src/discovery_service.rs:960`), and the admin "Test Connection" probe (`crates/livrarr-handlers/src/config.rs:310`). Works today, but doesn't match the published format and is a parsing-fragility risk if HC tightens validation. We should fix to match docs exactly. Reference incident: [Calibre-Web-Automated #770](https://github.com/crocodilestick/Calibre-Web-Automated/issues/770) — same surface area, broke them.

**Token lifecycle:**
- Tokens auto-expire after **1 year** (reset January 1).
- During beta, HC reserves the right to reset tokens without notice.
- We must handle 401 responses gracefully and prompt the user to re-paste their key.

**Where the user gets a token:** account settings → "Hardcover API" link → token at top of page.

## Rate limits

- **60 requests / minute, hard cap.** No tiers.
- **30-second max query timeout.**
- **Query depth max = 3** (added 2025) — server rejects deeper.
- **429 response body:** `{ "error": "Throttled" }`. No `Retry-After` header documented.

**What the code does today.** A request routed through the shared `HttpFetcher` waits in the process-global outbound queue before any HTTP is sent (`crates/livrarr-http/src/fetcher.rs:182-184`). That queue paces the `Hardcover` bucket at one dispatch per second — 60/min (`crates/livrarr-http/src/outbound_queue.rs:239-241`) — and caps concurrent in-flight sends per bucket at 2 (`crates/livrarr-http/src/outbound_queue.rs:38`). All four HC request-builders found carry that bucket (`crates/livrarr-external-data/src/hardcover.rs:80`, `:395`; `crates/livrarr-metadata/src/discovery_service.rs:965`; `crates/livrarr-handlers/src/config.rs:315`).

Cover *images* are outside this budget: only `covers.openlibrary.org` is paced, and every other cover host stays unpaced (`crates/livrarr-domain/src/services/http.rs:64-69`).

## User-Agent

Per HC docs: *"When authoring scripts that use the API, it is recommended to include a user-agent header with a description of the script."* — **recommendation, not requirement.** Same OL-style trap if we send a generic UA.

Our standard UA applies here too: `KkodecsBookBot/<version> (Livrarr; kkodecs@proton.me; https://github.com/kkodecs/livrarr)`.

## Query patterns — good vs bad citizen

### Good
- Small `per_page` — start ≤ 10; increase only when proven necessary.
- Specific field sets via GraphQL selection — request only what we render.
- Use the **`search()` resolver** (Typesense-backed) for any fuzzy lookup; it's a single round-trip vs walking `books → editions → contributions`.
- Query depth ≤ 3 (server cap).
- Cache aggressively client-side (GraphQL POST responses are not HTTP-cacheable upstream).

> **What the client actually sends.** `per_page` is **25** on the title search
> (`livrarr-external-data/src/hardcover.rs:178`), 10 on the ISBN search (`:465`), and 15 on
> the discovery/lookup search (`livrarr-metadata/src/discovery_service.rs:950`) — the
> "start ≤ 10" guidance above is honored only on the ISBN path. All three do go through the
> `search()` resolver (`hardcover.rs:39-47`).

### Bad
- Deep nesting (`book { editions { contributions { author { books { editions {...} } } } } }`) — server rejects past depth 3 anyway.
- Large `limit`/`offset` — pagination tax adds up fast against 60 RPM.
- Unbounded list fields — always specify `per_page`.
- Batching 100 cover image loads in parallel — known way to trip the limit ([emgoto's writeup](https://www.emgoto.com/hardcover-book-api/)).

### Disabled operators

Server rejects these — don't generate them in any query:

```
_like, _ilike, _nlike, _regex, _iregex, _nregex, _similar, _nsimilar, _niregex
```

Use `_eq` / `_in` / `_neq` instead. Use `search()` resolver for fuzzy matching.

## Caching

HC publishes no caching guidance (GraphQL POST endpoints aren't HTTP-cacheable). All caching must be client-side.

Recommended internal TTLs:

| Resource | TTL |
|---|---|
| Book / edition metadata | 30 days |
| Series | 7 days |
| Search results | 24h |
| Author bio | 7 days |
| Cover image URLs | Months (cover image itself should also be cached locally) |

Cache key: HC `id` + query selection set. Refresh on user-triggered "refresh metadata" only.

## Backoff and retries

- **429 (Throttled):** no `Retry-After` header documented. Start with 2s backoff, exponential to 60s, jitter, cap retries at 5, then surface as `RateLimited` error.
- **5xx:** exponential backoff with jitter, cap retries.
- **30-second timeout:** queries hitting it return 5xx; back off and consider simplifying the query.
- **401:** token expired or reset. Don't retry — surface to user, prompt for new token.

> **Which of these the code actually implements.** None as written — what exists is a
> circuit breaker plus one fixed retry delay:
>
> | Rule | Status |
> |---|---|
> | 429 → 2s backoff, exponential to 60s, jitter, cap at 5, then `RateLimited` | **Not implemented.** A 429 is intercepted as `FetchError::RateLimited` (`livrarr-http/src/fetcher.rs:280`) and reported as one `Failure` to the Hardcover breaker (`:276-278`) — 5 failures inside 60s open the bucket for 60s (`livrarr-http/src/breaker.rs:182-194`). The HC client folds every non-`CircuitOpen` fetch error into `HardcoverError::Http` (`livrarr-external-data/src/hardcover.rs:92`), so a 429 gets the same fixed 5-minute retry as a 5xx. No exponential schedule, no jitter, no per-call retry counter, no distinct `RateLimited` outcome. |
> | 5xx → exponential backoff with jitter, capped retries | **Not implemented.** A 5xx becomes `WillRetry { ServerError }` with a fixed next attempt 5 minutes out (`livrarr-external-data/src/provider_client.rs:578`, `:758-761`). |
> | 30-second timeout → back off, simplify the query | **Unreachable as written.** Our own request timeout is 10s (`livrarr-external-data/src/hardcover.rs:79`, `:394`), so we abort before HC's 30s server timeout; a timeout is reported as one breaker `Failure` (`livrarr-http/src/fetcher.rs:226-237`). |
> | 401 → don't retry, surface to user | **Not implemented.** Every non-2xx status becomes `HardcoverError::Http` (`livrarr-external-data/src/hardcover.rs:95-100`), and the caller schedules a retry 5 minutes out (`:758-761` in `provider_client.rs`) — a 401 is retried like any other HTTP error. |

## Anti-patterns (explicit from HC docs)

1. **"Should only be used from a code backend — never from a browser."** We're a backend; fine.
2. **"Only for offline use at this time"** — localhost or APIs only. No allowlisted web hosting yet. (OAuth support roadmap'd for external apps.)
3. **Don't share tokens.** *"Someone could delete your account with it."*
4. **Don't use disabled operators** (see above).
5. **Don't burst 100+ parallel requests.** Even within rate limit, simultaneous spikes trip throttling.

## Bulk dumps

**None.** No public dump, no S3 bucket, no torrent. Every byte comes through the rate-limited API. This is the architectural opposite of OpenLibrary.

Implication: bulk operations (re-enrichment, language pivots, audits) must be paced strictly. Anything touching >500 books gets noticeable.

## Author bibliography access (verified 2026-05-26 via Gemini + Codex fact-check)

HC supports retrieving an author's full bibliography via GraphQL. Two important caveats: **HC does NOT expose OpenLibrary keys** anywhere in its schema (so HC bibliography entries cannot be cross-walked to OL keys directly — must use HC key, ISBN, or normalized title for matching), and **the query MUST start from `books` (not `authors`)** to stay within the depth-≤-3 limit.

Correct query shape (verified by both Gemini and Codex):

```graphql
query GetBooksByAuthor($authorId: Int!, $limit: Int!, $offset: Int!) {
  books(
    where: { contributions: { author_id: { _eq: $authorId } } }
    limit: $limit
    offset: $offset
    order_by: { release_year: asc }
  ) {
    id
    title
    release_year
    slug
    image { url }
    editions(limit: 1) { isbn_13 isbn_10 }
  }
}
```

**Why not query from `authors`?** A query like `authors(where: {id: {_eq: X}}) { contributions { book { editions { isbn_13 }}}}` would be depth 5 (authors → contributions → book → editions → isbn_13), violating the depth-≤-3 cap. Starting from `books` keeps depth at 3.

**Resolving author name → HC author ID** (when we don't have `hc_key` stored yet):

```graphql
query SearchAuthor($q: String!) {
  search(query: $q, query_type: "Author", per_page: 5) {
    ids       # array of HC author IDs to follow up with
    results   # jsonb (NOT typed GraphQL) — use ids for stable follow-up
  }
}
```

`search.results` is `jsonb`, not typed GraphQL objects. Always pull `ids` and follow up with a typed `authors_by_pk(id: X)` query for the actual author detail.

**Gotchas for bibliography use:**

- **`contributions` includes non-writing roles** (narrator, illustrator, editor, translator). Filter on the contribution type field if we only want primary-author works.
- **ISBN is per-edition, not per-book.** The example query above uses `editions(limit: 1)` to grab one ISBN — sufficient for matching/display but if you need all ISBNs for a work, paginate `editions`.
- **OL key absence is the architectural takeaway.** When using HC as a bibliography source (e.g., when OL is unreachable), matching HC entries back to library works can only use: HC key (if work has `hc_key`), ISBN-13 (if both have), or normalized title scoped to the same author. The OL-key cross-walk via HC is not available.
- **Hasura-style queries.** HC uses `_eq` / `_in` / `_neq` / `_is_null` / `_lt` / `_gt`. The disabled operators (`_like`, `_ilike`, `_regex`, `_iregex`, `_similar`) will throw a GraphQL error.

## Contributing back

HC has a **Librarian system** — any user can apply, gets edit access on the website. This is the contribution path.

- **Anyone can apply** to be a librarian. See [Getting Started as a librarian](https://docs.hardcover.app/librarians/getting-started/).
- **High-impact edits gated**: anything with `impact_score >= 5` (i.e., touches 5+ reads) requires Senior Librarian approval. ([Librarian FAQ](https://docs.hardcover.app/librarians/faq/))
- **Published standards** for every entity type: `AuthorStandards`, `BookStandards`, `EditionStandards`, `SeriesStandards`, `MergingStandards`, `ComicStandards`, `MangaLightNovelStandards`. All in the docs repo under `librarians/Standards/`.
- **No public mutation API for edits.** Submissions go through the website UI.
- **HC actively wants third-party data improvements** ([Automating Book Data Q2 2024](https://hardcover.app/blog/automating-book-data)) but currently via librarian users, not API.

**Realistic Livrarr contribution opportunities:**
- Encourage power users to apply as HC librarians and fix data they correct in Livrarr
- File good bug reports in HC Discord when we encounter data issues
- If/when HC ships a mutation API, wire one-click "submit correction to HC" for user-corrected metadata
- Encourage adoption by being a high-quality, polite client (good ambassador for HC)

## How HC compares to other providers

| Dimension | OpenLibrary | Google Books | Hardcover | Audnexus |
|---|---|---|---|---|
| Auth | UA + contact | API key (query string) | API key (per-user header) | None |
| Rate limit | 1-3 req/sec | 1000/day | 60/min | 300/min |
| Query depth limit | n/a (REST) | n/a (REST) | **3** (GraphQL) | n/a (REST) |
| Cache headers | None | None | None | **Yes — 304 supported** |
| Bulk dumps | Yes (monthly) | No | No | No |
| Contribution path | Open editor + bot accounts | None (closed) | Librarian (open application) | Code contributions to proxy |
| Per-user token model | n/a | Recommended | Required | n/a |
| Maturity signal | Stable; well-documented | Stable; closed | **Beta — may break** | Active; small project |

## Open work for Livrarr

| Item | Priority |
|---|---|
| **Fix auth header to match docs:** `authorization: <token>` (no `Bearer`, lowercase) | **P1** — drift from docs is fragility risk |
| ~~**Set proper UA** on all HC calls~~ — **done.** All four HC request-builders found use `UserAgentProfile::Server` (`livrarr-external-data/src/hardcover.rs:83`, `:398`; `livrarr-metadata/src/discovery_service.rs:968`; `livrarr-handlers/src/config.rs:318`), which resolves to `KkodecsBookBot/<version> (Livrarr; kkodecs@proton.me; https://github.com/kkodecs/livrarr)` (`livrarr-http/src/lib.rs:168-173`) and is set on every request (`livrarr-http/src/fetcher.rs:203-208`) | Done |
| ~~**Global 60-RPM token bucket** for HC~~ — **done differently:** the process-global outbound queue paces the `Hardcover` bucket at 1 dispatch/sec (`livrarr-http/src/outbound_queue.rs:239-241`), which is a minimum-interval pacer rather than a token bucket | Done |
| **Exponential backoff** on 429 with jitter, cap retries at 5 | Open — not implemented as written; see *Backoff and retries* above for what exists instead |
| **Client-side cache layer** with sane TTLs (book/edition: 30d, series: 7d, search: 24h) | P2 |
| **Audit query depth and `per_page`** — add CI lint catching depth >3 or `per_page` >25 | P3 |
| **401 → re-token UX flow** — clean message + settings link | P2 |
| **Discord channel for HC contact** documented in CONTRIBUTING | P3 |
| **Per-user-token model** in DB (when HC OAuth ships, we'll need it) | Future |

## Related

- `wiki/integrations/openlibrary.md` (contrast — UA-centric identification)
- `wiki/integrations/google-books.md` (contrast — daily quota model)
- `wiki/integrations/audnexus.md` (contrast — anonymous + great cache headers)
- `feedback_kkodecs_contact_email.md` (UA contact field policy)
- Existing client: `crates/livrarr-external-data/src/hardcover.rs`
- HC docs: <https://docs.hardcover.app/api/getting-started/>
- HC docs repo: <https://github.com/hardcoverapp/hardcover-docs>
- Real-world auth-header issue: [Calibre-Web-Automated #770](https://github.com/crocodilestick/Calibre-Web-Automated/issues/770)
