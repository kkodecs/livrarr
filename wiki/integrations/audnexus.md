# Audnexus Integration

Audnexus (audnex.us) is Livrarr's audiobook metadata provider — a community-run Audible mirror that scrapes per-region and exposes a clean read-only API. We use it for ASIN-keyed audiobook lookups, narrator info, chapter data, and audiobook covers.

## Current operational status

- **Active use today:** audiobook enrichment by ASIN, audiobook covers, chapter extraction.
- **Maintainer:** [@djdembeck](https://github.com/djdembeck) under [laxamentumtech org](https://github.com/laxamentumtech/audnexus). Active — v1.14.0 released May 23, 2026 (192 stars, GPL-3.0).
- **Infra:** Cloudflare-fronted; Bun + MongoDB + Redis stack.
- **Status page:** none. Only signal of issues is HTTP responses.
- **Contact:** GitHub issues only — no Discord, no email, no formal support channel.

Important context: Audnexus runs on **volunteer infrastructure** with no advertised donations program. Being a polite consumer matters more here than for VC-backed providers — we're sharing finite donated capacity with every other client (Audiobookshelf, Audnexus.bundle for Plex, Booksonic, etc).

## Authentication

| Method | Required for |
|---|---|
| **None** | All `GET` endpoints — book/author/chapter/search lookups |
| `Authorization` | `DELETE` endpoints (cache invalidation; not relevant to Livrarr) |

Audnexus does not enforce a User-Agent contact requirement, but **set one voluntarily**: per the OpenLibrary lesson, we want them to be able to contact us if our usage looks off. Our standard UA (`KkodecsBookBot/<v> (Livrarr; kkodecs@proton.me; URL)`) applies here too.

## Rate limits

Documented in README as `MAX_REQUESTS=100/min`, but the **live production rate limit is 300 req / 60 sec per source IP** (confirmed empirically — Audnexus returns these headers on every response):

```
x-ratelimit-limit: 300
x-ratelimit-remaining: <n>
x-ratelimit-reset: <seconds-until-window-resets>
```

**On exceed:** HTTP 429 with body containing `RATE_LIMIT_EXCEEDED` and `retryAfterSeconds`. Source = IP (not UA).

**Operational rule:** back off **proactively** when `x-ratelimit-remaining < 30` (don't ride it down to 0). Never set `Cache-Control: no-cache` on outbound requests — it bypasses their Cloudflare cache and forces upstream Audible scrape.

## Caching — the big win

Audnexus has **real, working cache headers**:

```
cache-control: max-age=86400      # 24 hours
last-modified: <date>             # on every response
cf-cache-status: HIT|MISS|DYNAMIC # Cloudflare layer status
```

Empirically confirmed: **`If-Modified-Since` works and returns HTTP 304** — no body, no rate-limit charge for the revalidation. No ETag support, but `Last-Modified` covers the same use case.

**Operational rule:**
- Cache Audnexus responses locally for 24h (matching their `max-age`).
- On stale-read, send `If-Modified-Since: <stored Last-Modified>` to revalidate. 304 → reuse cached body, refresh TTL.
- This is the cheapest possible polite-consumer behavior — costs us almost nothing and meaningfully reduces their upstream Audible scraping.

## Endpoint surface

```
GET /books/{ASIN}                            # book metadata
GET /books/{ASIN}/chapters                   # chapter data (requires Audnexus server to have Audible creds)
GET /authors/{ASIN}                          # author bio
GET /authors?name=X&region=Y                 # author search
GET /health                                  # status — do NOT poll aggressively
```

**Region routing.** Audnexus supports 10 Audible regions via `?region=` parameter:

```
au, ca, de, es, fr, in, it, jp, us, uk
```

Default region is `us`. Wrong region returns `REGION_UNAVAILABLE` (HTTP 404). If a user has audiobooks from multiple regions, we need to track which region each ASIN came from and query the right one.

**Special parameters:**
- `?update=1` — bypasses cache, forces upstream Audible refresh. **Reserve for explicit user-triggered "refresh metadata" actions only.** Default usage of this would be abusive.
- `?seedAuthors=1` (on `/books`) — populates author records as side-effect of book lookup.

## Coverage limits

What's NOT in Audnexus:

- **Self-published outside Audible** (only Audible-distributed audiobooks are scraped).
- **Pre-orders** (active GitHub issues; coverage gap).
- **Podcasts** (separate corpus; Audnexus doesn't cover them).
- **Deleted-from-Audible books may still be returned** if Audnexus cached them before delisting — useful but stale.

## Backoff and retries

- **429 (rate limit exceeded):** honor `retryAfterSeconds` from body. Don't retry immediately.
- **5xx (especially 522 — Cloudflare-to-origin timeout):** exponential backoff starting 5s, jitter, cap retries at 5. Then surface error.
- **404 (book/author not found OR region mismatch):** don't retry. Check if it's a region issue before giving up; try `?region=us` as fallback if user didn't specify.
- **The one documented outage** ([Audnexus.bundle #36](https://github.com/djdembeck/Audnexus.bundle/issues/36), Jan 29, 2022) was a Cloudflare 522 — resolved by waiting it out. Our retry logic should handle this transparently.

## Anti-patterns to avoid

1. **Don't bypass cache with `?update=1` in automated flows.** Reserve for user-initiated refresh actions only.
2. **Don't poll `/health`.** Only check on first request of a session or when failures begin to occur.
3. **Don't request `Cache-Control: no-cache` on outbound calls.** Defeats their Cloudflare layer; forces costly upstream Audible scrape per call.
4. **Don't ignore the `x-ratelimit-*` headers.** Proactive throttling at remaining<30 prevents 429s entirely.
5. **Don't burn requests on speculative lookups.** Only call Audnexus when we actually need the data for a specific work.
6. **Don't rotate IPs to bypass rate limits** — applies universally; documented for OL but the principle is the same everywhere.

## Self-hosting — our pressure relief valve

If we ever risk overwhelming Audnexus (or the community asks us to offload), we can stand up our own instance. Audnexus is fully self-hostable:

- **Runtime:** Bun
- **Deps:** MongoDB, Redis
- **Deploy options:** Coolify, Docker Swarm, direct Bun
- **Source:** <https://github.com/laxamentumtech/audnexus>

**This is meaningful insurance** — if Audnexus goes down or rate-limits us aggressively, we have a fully open-source path to keep operating. Worth documenting as a Livrarr deployment option for power users.

## Contributing back

**Code contributions:**
- Repo: <https://github.com/laxamentumtech/audnexus>
- [CONTRIBUTING.md](https://github.com/laxamentumtech/audnexus/blob/main/CONTRIBUTING.md)
- Conventional Commits; `bun install && bun run serve` workflow.
- 100% authorship attestation required.

**No donation/sponsorship program advertised.** No GitHub Sponsors button on the org.

**Realistic Livrarr contribution opportunities:**
- Fix bugs we encounter
- Contribute test cases for edge regions (e.g., Japan, Spain) where coverage might be thin
- Document/improve the wiki/READMEs based on our integration experience
- File good-quality bug reports with reproduction steps for any 5xx we see

## How Audnexus compares to other providers

| Dimension | OpenLibrary | Google Books | Audnexus |
|---|---|---|---|
| Identification | UA + contact | API key | None enforced (set UA voluntarily) |
| Rate limit | 1-3 req/sec | 1000/day | 300/min (per IP) |
| Cache headers | None published | None published | `Cache-Control`, `Last-Modified`, supports 304 |
| Self-hostable | No (use dumps) | No (closed) | **Yes** — pressure relief |
| Contribution path | Editor + bot accounts, well-defined | None (closed corpus) | Code contributions to the proxy |
| Failure mode | UA blocked → 403 | Quota exhausted → 403 | Cloudflare 522 / 429 backoff |
| Resilience for us | Brittle (UA dependency) | Daily-cap brittle | **Most robust** (low limits, but self-host fallback exists) |

## Open work for Livrarr

| Item | Status |
|---|---|
| Set descriptive UA on Audnexus calls | Audit current client; share UA with OL/HC stack |
| Implement `If-Modified-Since` for 304 revalidation | TBD — pure win, no code-side downsides |
| Honor `x-ratelimit-remaining` for proactive throttling | TBD |
| Region-aware ASIN lookups (track origin region per audiobook) | Already partial; verify per code path |
| `?update=1` gating — UI-triggered only | Audit; ensure no automated flow sends it |
| Self-host instructions in deployment docs (advanced) | Future — only if Audnexus pressure becomes real |

## Related

- `wiki/integrations/openlibrary.md` (contrast — much stricter polite-consumer requirements)
- `wiki/integrations/google-books.md` (contrast — daily quota model vs Audnexus's per-minute)
- `wiki/insights.md` — pending insight on Audnexus's 304 revalidation pattern
- GH: <https://github.com/laxamentumtech/audnexus>
