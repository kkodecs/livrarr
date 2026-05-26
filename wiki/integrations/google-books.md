# Google Books Integration

Google Books (GB) is Livrarr's foreign-language metadata provider today, and is the planned English-search fallback for when OpenLibrary is unavailable. This page is the canonical reference for how we should use GB.

## Current operational status

- **Active use today:** foreign-language enrichment only (per `project_google_books_integration` memory; the language gate was relaxed and OL search routing changed to put GB in the foreign pipeline).
- **Planned expansion:** primary English-search fallback when OL returns 4xx/5xx/timeout (i.e., now — given the OL block).
- **Required: API key.** Keyless requests are quota-throttled to effectively zero (insight from earlier session, `feedback_gb_api_key_required` memory). Without a key, we get nothing.
- **Default daily quota: 1,000 requests / day** (per API key, resets midnight Pacific Time). For our scale, this is **very restrictive** — see "Scaling" below.

## Authentication

| Mechanism | When required |
|---|---|
| **API key** (`?key=YOUR_KEY` in query string) | Any non-OAuth request. This is what we use. |
| **OAuth 2.0** (`Authorization: Bearer ...`, scope `https://www.googleapis.com/auth/books`) | Required only for **per-user data** (`/mylibrary/*` endpoints — bookshelves, etc.). Livrarr doesn't touch user-data endpoints. |

GB does **not** require a User-Agent contact field the way OL does. Identification is via the API key. The key is tied to a GCP project, which is tied to a billing account (even though we stay in the free tier).

**Key management:** the Livrarr admin provides the key in settings. We do not ship a default project key — each install gets their own. This naturally scales the 1000/day quota across users.

## Rate limits and quotas

| Tier | Quota |
|---|---|
| Default (free) | **1,000 requests / day / API key** |
| Per-second (implicit) | ~100 req/sec for Google APIs generally — undocumented for Books specifically; treat 5 req/sec as a safe ceiling |
| Quota increase | Request via GCP console; takes days–weeks; requires usage history + project description |

**Why the 1000/day cap matters for Livrarr.** A user enriching ~50 works/day across all providers, where GB participates in each enrichment (foreign discovery + potentially English fallback), can easily hit the cap with: search candidates + per-work detail fetch + per-edition detail fetch. We must:

1. **Cache aggressively** (per provider response, by volume ID) — see "Caching" below.
2. **Use `fields=` partial-response** on every call (see "Performance" below).
3. **Document the per-user-key recommendation** so users with heavy use can split load.

## Endpoint surface

```
GET /books/v1/volumes?q=...                # search
GET /books/v1/volumes/{volumeId}           # detail by volume ID
GET /books/v1/volumes?q=isbn:9780000000000  # ISBN lookup (just a search w/ filter)
```

There is no separate "lookup by ISBN" endpoint — ISBN searches are filtered search queries. The response field for ISBN lookups still returns multiple candidates if any match. **Always pick by ISBN equality before falling back to title-similarity scoring** when consuming ISBN-search results.

Notable: there's **no /authors endpoint and no bibliography endpoint**. GB indexes by edition, not author. Series detection in GB is via the `seriesInfo` field on the volume — not always populated.

## Performance best practices (Google's published guidance)

### Partial response — use `fields=` on every call

Reference: <https://developers.google.com/books/docs/v1/performance>

```
GET /books/v1/volumes/{id}?fields=id,volumeInfo(title,authors,industryIdentifiers,publishedDate,language,imageLinks)
```

Cuts response size 5-10x. Reduces JSON parse cost. Required for our scale.

### Gzip compression

```
Accept-Encoding: gzip
User-Agent: <our-ua> (gzip)
```

Note the `(gzip)` suffix on UA is **required by Google's convention** when requesting gzip. We must include it in our GB UA even though our OL UA doesn't need it.

### No batching endpoint

Unlike some Google APIs, Books doesn't support request batching. Each call is one HTTP round-trip. Plan for it.

## Caching strategy

GB doesn't publish per-endpoint TTL guidance. Recommended internal defaults (similar to OL):

| Endpoint | Cache TTL |
|---|---|
| `/volumes/{volumeId}` (detail) | Weeks |
| `/volumes?q=isbn:...` (ISBN search) | Weeks (ISBN is stable per edition) |
| `/volumes?q=...` (title search) | Hours (query may change) |
| Cover URLs (`books.google.com/books/content?id=...`) | Months — image is immutable per volume ID |

Cover URLs returned by GB are signed/parameterized — they don't 403 on rate limit the way OL covers do, but they should still be cached locally to avoid repeat hits.

## Anti-patterns to avoid

1. **Don't ship a hardcoded shared API key.** Per-install configuration only.
2. **Don't fetch full response objects when partial-response works.** Always use `fields=`.
3. **Don't skip gzip.** It's free bandwidth, and GB's convention requires the UA suffix.
4. **Don't burn quota on speculative fetches.** Only call GB when we actually need the data (don't pre-warm caches with unprompted lookups).
5. **Don't use OAuth-gated endpoints (`/mylibrary/*`)** — Livrarr is server-side and shouldn't be managing user GB bookshelves.
6. **Don't paginate beyond what's necessary.** GB returns up to 40 items per page (`maxResults=40`); for our use we rarely need more than ~10 — set `maxResults` explicitly.

## Terms of Service highlights

Reference: <https://developers.google.com/books/terms>

- **No commercial use without Google written permission.** Livrarr is open-source and free-to-use → we're fine.
- **Content removal on request** — if Google or a rights-holder requests removal of indexed content, we comply. Practically: if a user reports a cover/metadata issue traceable to GB, defer to GB.
- **Caching is permitted** for performance reasons; aggressive caching is encouraged given the quota.

## How GB compares to OpenLibrary

| Dimension | OpenLibrary | Google Books |
|---|---|---|
| Authentication | UA with contact email | API key in query string |
| Daily quota | 1/sec unidentified, 3/sec identified — no daily cap | 1,000 / day / key (default) |
| Bulk data dumps | Yes — monthly TSV+JSON | No (proprietary index) |
| Contribution path | Anyone can edit; bot accounts available | Closed corpus; submit corrections via "Report a problem" link only |
| Coverage strengths | Pre-1990 books; obscure editions; non-English where editors are active | Recent books; broad multilingual; reliable cover thumbnails |
| Coverage weaknesses | Cover-sparse; metadata sometimes thin | Spotty pre-1990; aggressive deduplication can merge variant editions |
| Best-citizen friction | Need to follow their UA policy carefully (we learned this 2026-05-25) | Just need to manage quota; no relationship-level friction |

## Scaling — what happens when we hit the cap

If Livrarr's GB usage exceeds 1,000 req/day:

1. **Default behavior:** GB returns HTTP 403 with `quotaExceeded` error code in the body.
2. **Our handler:** must catch this as a separate-from-other-403s case — it's NOT a hard ban, it's a daily quota reset.
3. **Retry-after:** next reset is midnight Pacific Time. Surface this to the user.
4. **Mitigation paths:**
   - User requests quota increase from GCP for their own project (free, but takes time)
   - User adds a second GB API key for failover (multiple GCP projects)
   - Livrarr-side: deeper caching, smarter fields selection, longer TTLs

## Contributing back to Google Books

Limited compared to OL:

- **No public contribution API.** GB's corpus is sourced from publisher feeds + library scans (Internet Archive partnership + Google's own scanning).
- **"Report a problem" link** on each book page is the only public path for corrections (cover, metadata, content).
- **Publishers** can submit through Google Books Partner Program — not relevant to Livrarr.
- **Practical Livrarr contribution path:** none. We're consumers only. This is part of why GB shouldn't become our long-term primary — no symmetric relationship possible.

## Open work for Livrarr

Tracked separately as bugs/enhancements:

| Item | Status |
|---|---|
| GB as English search fallback when OL fails | **In progress** (this session) |
| Cache layer for GB volume IDs (avoid re-fetching same volume) | TBD |
| `fields=` partial response on all GB calls | Audit current GB client; many calls likely fetch full payload today |
| Gzip + `(gzip)` UA suffix on GB calls | Audit; not sure if enabled |
| User-visible `quotaExceeded` error surface | Pairs with #77 (error-message scrubbing + AI help) |
| Per-install API key documentation in onboarding | Pairs with #72 (onboarding GB step) |

## Related

- `feedback_gb_api_key_required.md` (memory)
- `project_google_books_integration.md` (memory)
- `wiki/integrations/openlibrary.md` (the contrast case)
- GH issue [#72](https://github.com/kkodecs/livrarr/issues/72) (onboarding GB step)
- GH issue [#73](https://github.com/kkodecs/livrarr/issues/73) (search fallback — what we're implementing now)
