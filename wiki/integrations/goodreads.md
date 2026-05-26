# Goodreads Integration

Goodreads (goodreads.com) is Livrarr's series-matching and supplementary-ratings source. **We do not use any official API** — Goodreads shut their API down in December 2020. Everything we get from GR is via HTML scraping of public pages.

This page is also a deprecation-tracking page. We should be actively reducing GR dependence over time.

## Current operational status

- **No official API since 2020-12-08.** Goodreads deprecated the public API; existing keys broke ~2022. Amazon (which acquired GR in 2013) cut the external data feed. **No public signal of reopening.**
- **What we do:** scrape JSON-LD blobs embedded in `/book/show/<id>` and `/series/<id>` pages.
- **Anti-bot defense GR runs:** **Cloudflare + DataDome** with TLS/HTTP2 fingerprinting, IP reputation scoring, behavioral analysis, Turnstile challenges.
- **Our current rate limit:** 1 req/sec (60/min) — **5-7x more aggressive than the polite floor** observed by surviving scrapers. **This is real risk; see "Open work" below.**
- **Insight #13:** Goodreads requires LLM disambiguation to be useful — the search-result hit list is noisy with study guides, alternate editions, and foreign-language alternates. Without LLM, our GR client returns NotFound. This isn't a bug; it's intentional design.

## Authentication

**None.** We're scraping public pages. **Logged-in cookie-based sessions would violate GR's TOS** and risk account suspension — keep cookie-free.

## What we get from GR

| Endpoint | Use | Stability |
|---|---|---|
| `/book/show/<id>` | Book detail; series position; rating; description; cover URL | High — JSON-LD blob is server-rendered, stable |
| `/series/<id>` | Series ordering, member books | High — JSON-LD blob is server-rendered |
| `/author/show/<id>` | Author bio, bibliography links | Medium |
| `/search?q=...` | Search results | **Volatile + disallowed by robots.txt** — risky to hit |

## Rate limits (no published spec)

**There is no documented GR rate limit.** We're inferring from community evidence:

| Source | Observed safe rate |
|---|---|
| [Automatio guide](https://automatio.ai/how-to-scrape/goodreads) | 8-12 req/min (5-7s interval) |
| [Scraperly guide 2026](https://scraperly.com/scrape/goodreads) | 8-12 req/min |
| [rreading-glasses](https://github.com/blampe/rreading-glasses) | Default `--rpm=60` (1/sec), but actively triggers backoffs in production |

**Our current implementation:** 1 req/sec (60/min) — sits at the rreading-glasses ceiling but well above the polite floor. We are **5-7x over the empirical safe rate.**

**Operational rule:** lower the GR rate-bucket to **5-7 second interval** (8-12 req/min). Single biggest risk reduction available for this provider. Code: `crates/livrarr-http/src/fetcher.rs:73`.

## User-Agent — OPPOSITE of OpenLibrary

This is a critical distinction that runs counter to the OL lesson we just learned:

| Provider | Right UA strategy |
|---|---|
| OpenLibrary | **Identify clearly** — `App/<v> (contact)` — get the 3x rate quota |
| Goodreads | **Mimic a browser** — `Mozilla/5.0 ... Chrome/...` — DataDome blocks bot-identified UAs faster |

**Do not "do the right thing" on GR by self-identifying.** DataDome's intent-based detection treats anything bot-like as adversarial. Browser UA is the polite-scraper convention.

**Specifically: never identify as `Livrarr/`, `LivrarrBot/`, or `KkodecsBookBot/` on GR.** Both are burned at OL and the substring-blocklist pattern could propagate. Keep the browser UA on GR.

**Production already does the right thing.** Our production GR client uses `GOODREADS_USER_AGENT` (defined at `crates/livrarr-metadata/src/goodreads.rs:398`):

```
Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36
```

Pure browser UA. No "Livrarr" substring. No self-identification. Exactly what the polite-scraper convention recommends for GR.

The string `Mozilla/5.0 (compatible; Livrarr/0.1 test)` exists at `goodreads.rs:977` but is **inside a `#[cfg(test)] mod tests` block** — a test fixture that never reaches production. Cosmetically ugly but operationally harmless. Earlier wiki drafts incorrectly flagged this as a production issue.

## robots.txt — what GR allows

Source: <https://www.goodreads.com/robots.txt>

Blocks outright:
- `GPTBot` (OpenAI training)
- `CCBot` (Common Crawl)
- `EtaoSpider`

Allows with restrictions:
- `bingbot` — 5s crawl-delay

**For all bots, disallowed paths:**
- `/search`
- `/api`
- RSS feeds
- `/review/*`

**Implication:** we should not be hitting `/search` from automated flows. If we do (and we may — need to audit `goodreads.rs`), that's a policy violation that could explain past 403s.

## Backoff and failure modes

- **403 → terminal-for-session.** DataDome blocks can last hours to days. Once we get 403, stop hitting GR for at least 1 hour; never aggressively switch UAs to evade (that trains DataDome faster — exactly the OL lesson).
- **429 → exponential backoff with jitter**, honor `Retry-After` if present.
- **Cloudflare 5xx → exponential backoff**, cap retries, surface.
- **Anti-bot challenge page** (HTML body contains specific markers): detected by `is_anti_bot_page` in our GR client — good. Should treat as 403-equivalent.

## Caching

GR data is not time-sensitive — book/series/author canonical data changes slowly.

| Resource | TTL |
|---|---|
| `/book/show/<id>` JSON-LD | Weeks |
| `/series/<id>` JSON-LD | Weeks |
| `/author/show/<id>` | Weeks |
| Ratings | Days (drift) |

Aggressive caching is the second-biggest risk reduction available (after lowering the rate limit). Every cached hit is one less scrape against GR's DataDome budget.

## Legal posture

- **No GR-specific lawsuit on record** against scrapers.
- **Relevant precedent:** [hiQ v. LinkedIn (9th Cir. 2022)](https://en.wikipedia.org/wiki/HiQ_Labs_v._LinkedIn) — scraping public pages doesn't violate CFAA, but hiQ paid $500K + injunction under California trespass-to-chattels.
- **Recent Amazon hostility:** [Amazon v. Perplexity (Nov 2025)](https://terms.law/2025/11/03/amazon-vs-perplexity-when-a-cease-and-desist-letter-calls-your-ai-a-computer-fraud/) — Amazon is actively pursuing scrapers under CFAA framing. GR is Amazon-owned. If we hit material volume, Amazon could act.
- **No public "we're OK with X" posture from GR.** Operate as if we're skating the line.

## Alternatives we should be developing

The structural answer to GR risk is reducing dependence, not just throttling harder.

### 1. rreading-glasses as a fronting proxy
- [github.com/blampe/rreading-glasses](https://github.com/blampe/rreading-glasses)
- Public hosted instance: `api.bookinfo.pro` (GR backend), `hardcover.bookinfo.pro` (HC backend)
- Caveat: dependency on a single maintainer; recent issues #555/#556 (May 2026) show GR backend flaking
- **Path:** offer this as a user-configurable source — admin can point Livrarr at a rreading-glasses instance instead of going direct

### 2. Hardcover as a co-equal series/ratings source
- Already discussed in [`wiki/integrations/hardcover.md`](hardcover.md)
- HC was founded specifically to replace GR API
- Has series + ratings + similar coverage on mainstream titles
- **Path:** promote HC to primary for series matching where coverage exists; demote GR to "supplemental, when HC misses"

### 3. Wikidata for series-graph augmentation
- Has series + ISBN cross-refs but no ratings
- Coverage spotty for non-canonical titles
- **Path:** secondary signal for series detection, never primary

### 4. User CSV imports
- Users can manually export from GR (account → settings → export library)
- Manual one-time import path, not ongoing
- **Path:** import wizard for users to bring their existing GR shelf data in once

## Anti-patterns to avoid

1. **Don't hit `/search`** — disallowed by robots.txt. If we do, audit and remove.
2. **Don't run faster than 8-12 req/min sustained.** Polite floor.
3. **Don't identify as `Livrarr` / `LivrarrBot` / any project name on GR.** Browser UA only.
4. **Don't rotate UAs mid-run after a 403** — trains DataDome faster, makes blocks worse.
5. **Don't use logged-in sessions.** TOS violation; account suspension risk.
6. **Don't poll the same page repeatedly** to detect change — use long TTLs and accept staleness.
7. **Don't expand to new GR endpoints** without considering whether the data is available from HC or Wikidata first.

## Contributing back to Goodreads

**There is no contribution path.** GR's data model is closed; submissions go through the website UI by individual users. No partner program, no API for corrections, no community editorial system. Amazon's posture is consumer-only.

This is part of why GR shouldn't remain our primary source — there's no symmetric relationship possible.

## How GR compares to other providers

| Dimension | OpenLibrary | Google Books | Hardcover | Audnexus | **Goodreads** |
|---|---|---|---|---|---|
| Auth | UA + contact | API key | API key (per-user) | None | **None (scraping public pages)** |
| Rate limit | 1-3 req/sec | 1000/day | 60/min | 300/min | **None published; polite floor 8-12/min** |
| Identification posture | Identify clearly | API key only | API key + optional UA | UA optional | **Mimic browser; never self-identify** |
| Anti-bot infrastructure | Substring blocklist on UA | Standard Google quota | Standard throttle | Cloudflare basic | **Cloudflare + DataDome (aggressive)** |
| Cache headers | None | None | None | Yes, supports 304 | None |
| Bulk dumps | Yes | No | No | No | No |
| Contribution path | Open + bot accounts | None | Librarian | Code | **None** |
| Resilience | Brittle (UA dependency) | Brittle (daily cap) | Brittle (beta + 60/min) | Robust (self-host fallback) | **Most brittle — actively hostile** |

## Open work for Livrarr (priority order)

| Item | Priority | Why |
|---|---|---|
| **Build adaptive rate-limiter** (stay at 1/sec normally; back off to 5-7s for ~1hr on first 4xx/5xx) | **P1** | rreading-glasses proves 1/sec is sustainable IF combined with proper backoff; adaptive captures both speed and politeness |
| ~~Switch GR UA to pure browser string~~ | n/a | Production already uses pure browser UA (`GOODREADS_USER_AGENT` constant). Earlier P0 flag was based on misreading a test fixture. |
| **Aggressive caching of `/book/show/<id>` and `/series/<id>` JSON-LD** (weeks TTL) | **P1** | Reduces request volume to the floor without losing freshness |
| **Audit code for any `/search` calls** and remove or route elsewhere | **P1** | robots.txt violation |
| **GR-specific circuit breaker** — N consecutive 403s → 1+ hour cooldown for whole provider | **P1** | Avoid OL-style escalation; once burned by DataDome, blocks last hours |
| **Honor `Retry-After` on 429** (defensive — not always sent) | **P2** | Standard polite-consumer |
| **Promote Hardcover to co-primary** for series matching where coverage exists | **P2** | Structural derisking |
| **Evaluate rreading-glasses as user-configurable proxy option** | **P3** | Externalize scraping risk for users who want it |
| **Track GR as a deprecation candidate** — formal "GR optional" mode in roadmap | **P3** | Long-term derisking |
| **Bulk import path for user's existing GR CSV export** | **P3** | One-time migration for users who already have data |

## Related

- `wiki/insights.md` insight #13 (Goodreads requires LLM)
- `wiki/integrations/openlibrary.md` (contrast — opposite UA strategy)
- `wiki/integrations/hardcover.md` (the replacement target)
- `project_gr_llm_required.md` (memory)
- `project_gr_202_antibot.md` (memory — historical GR anti-bot incident)
- Existing client: `crates/livrarr-metadata/src/goodreads.rs`
- rreading-glasses (proxy): <https://github.com/blampe/rreading-glasses>
