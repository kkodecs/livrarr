# Goodreads Integration

Goodreads (goodreads.com) is Livrarr's series-matching and supplementary-ratings source. **We do not use any official API** — Goodreads shut their API down in December 2020. Everything we get from GR is via HTML scraping of public pages.

This page is also a deprecation-tracking page. We should be actively reducing GR dependence over time.

## Current operational status

- **No official API since 2020-12-08.** Goodreads deprecated the public API; existing keys broke ~2022. Amazon (which acquired GR in 2013) cut the external data feed. **No public signal of reopening.**
- **What we do:** scrape JSON-LD blobs embedded in `/book/show/<id>` and `/series/<id>` pages.
- **Anti-bot defense GR runs:** **Cloudflare + DataDome** with TLS/HTTP2 fingerprinting, IP reputation scoring, behavioral analysis, Turnstile challenges.
- **Our current rate limit:** GR bucket paced at 1.5s through the process-global outbound queue — confirmed in `interval_for` (`livrarr-http/src/outbound_queue.rs`) — still faster than the 5-7s polite floor observed by surviving scrapers; see "Open work" below.
- **Matching is fully deterministic since Phase 5 (2026-07-03, insight #13):** junk-edition filter + the shared 0.75 picker with explicit abstain. No LLM chooses a match anywhere; the only LLM on a GR path is `llm_extract_payload` (HTML-parse repair — repair, not selection). An earlier version of this page said "GR requires LLM disambiguation" — wrong since Phase 5.

## Authentication

**None.** We're scraping public pages. **Logged-in cookie-based sessions would violate GR's TOS** and risk account suspension — keep cookie-free.

## What we get from GR

| Endpoint | Use | Stability |
|---|---|---|
| `/book/show/<id>` | Book detail; series position; rating; description; cover URL | High — JSON-LD blob is server-rendered, stable |
| `/series/<id>` | Series member books | **Redesigned ~2026-07** — React layout, books as `data-react-props` JSON (see "Series pages" below); the old `<h3>Book N</h3>` + JSON-LD layout is GONE |
| `/author/show/<id>` | Author bio, bibliography links | Medium |
| `/search?q=...` | Search results | **Volatile + disallowed by robots.txt** — risky to hit |

## Series pages (2026-07 React layout — N1, measured on 108562 + 43318)

- Books ship as HTML-attribute-encoded JSON inside `data-react-props` mounts
  (`ReactComponents.SeriesHeader` / `SeriesList` / `FullPagePaginationControls`);
  multiple `SeriesList` blobs per page (split around ad slots).
- **The list carries ALL editions, primaries FIRST**: series 43318 (Night's
  Dawn, "3 primary works • 27 total works") lists the 3 primaries, then the
  omnibus, split editions, and Romanian/Italian/Polish translations. The
  header subtitle's "N primary works" is the ONLY primary-set signal — the
  roster keeps the first N entries (`fetch_series_roster_pages`); a shortfall
  vs N (unreadable later page) means drift and yields an EMPTY roster, never
  a partial one.
- **No per-book position labels exist anywhere on the page.** The only
  position signal is the title decoration "(Series Name, #N)", trusted only
  when it names the page's own series — umbrella pages (108562 Confederation
  Universe decorates with "Night's Dawn, #N") never borrow sub-series
  numbers; those books ride unnumbered, in page order.
- Pagination is the counter blob (`numWorks`/`currentPageNumber`/`perPage`);
  `numWorks` counts TOTAL works, not primaries. The old `next_page` link is
  gone (the author series LIST page `/series/list?id=` still uses the old
  layout + `RE_NEXT_PAGE` — unverified since the redesign; candidate for the
  N5 drift probes).
- Parser: `parse_series_detail_html` (livrarr-external-data `goodreads/parsers.rs`) —
  per-entry tolerant JSON parsing, WARN on every unreadable shape (insight
  #62). Real captured pages live at
  `crates/livrarr-external-data/fixtures/gr-series-{108562,43318}.html`.

## Rate limits (no published spec)

**There is no documented GR rate limit.** We're inferring from community evidence:

| Source | Observed safe rate |
|---|---|
| [Automatio guide](https://automatio.ai/how-to-scrape/goodreads) | 8-12 req/min (5-7s interval) |
| [Scraperly guide 2026](https://scraperly.com/scrape/goodreads) | 8-12 req/min |
| [rreading-glasses](https://github.com/blampe/rreading-glasses) | Default `--rpm=60` (1/sec), but actively triggers backoffs in production |

**Our current implementation: 1.5 s between dispatches (~40/min)**, set in `interval_for` in
`crates/livrarr-http/src/outbound_queue.rs` — the single source of per-provider pacing. Goodreads
is paced slower than the API-backed providers there precisely because it is an anti-bot-hostile
scrape target. Against the 5-7 s floor the sources above report, that is roughly **3-5x over**,
not the 5-7x this page previously claimed (that figure was computed from a 1 req/sec baseline the
code does not use).

**Operational rule (stated intent):** lower the GR bucket to a **5-7 second interval** (8-12
req/min). Single biggest risk reduction available for this provider. The value to change is the
`RateBucket::Goodreads` arm of `interval_for`.

## User-Agent — OPPOSITE of OpenLibrary

This is a critical distinction that runs counter to the OL lesson we just learned:

| Provider | Right UA strategy |
|---|---|
| OpenLibrary | **Identify clearly** — `App/<v> (contact)` — get the 3x rate quota |
| Goodreads | **Mimic a browser** — `Mozilla/5.0 ... Chrome/...` — DataDome blocks bot-identified UAs faster |

**Do not "do the right thing" on GR by self-identifying.** DataDome's intent-based detection treats anything bot-like as adversarial. Browser UA is the polite-scraper convention.

**Specifically: never identify as `Livrarr/`, `LivrarrBot/`, or `KkodecsBookBot/` on GR.** Both are burned at OL and the substring-blocklist pattern could propagate. Keep the browser UA on GR.

**Production already does the right thing.** Our production GR client uses `GOODREADS_USER_AGENT` (defined at `crates/livrarr-external-data/src/goodreads/client.rs:26`):

```
Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36
```

Pure browser UA. No "Livrarr" substring. No self-identification. Exactly what the polite-scraper convention recommends for GR.

And it is genuinely what goes on the wire: both GR fetch paths set
`user_agent: UserAgentProfile::Custom(GOODREADS_USER_AGENT.to_string())` — the page fetch
(`fetch_goodreads_html_via`) and the autocomplete search (`search_goodreads`). Neither falls back
to the fetcher's generic `UserAgentProfile::Browser`, which is a different string.

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

**Implication:** we should not be hitting `/search` from automated flows — **and we do not.** The
GR client's only search path is `search_goodreads`, which builds
`{base}/book/auto_complete?format=json&q=<title>`; its own doc calls this "the WAF-free
autocomplete JSON endpoint". No function in `goodreads/client.rs` constructs a `/search` URL.
(Bounded claim: that is the client module. I did not sweep the whole workspace for a GR `/search`
string — `grep` is unavailable in this environment.)

Note also what the autocomplete doc records: the **author is deliberately kept out of the query
string**, because prefix-matching on "Title Author" ranks study guides above the real record.
Author agreement is enforced by the picker instead.

## Backoff and failure modes

- **403 → terminal-for-session.** DataDome blocks can last hours to days. Once we get 403, stop hitting GR for at least 1 hour; never aggressively switch UAs to evade (that trains DataDome faster — exactly the OL lesson).
- **429 → exponential backoff with jitter**, honor `Retry-After` if present.
- **Cloudflare 5xx → exponential backoff**, cap retries, surface.
- **Anti-bot challenge page** (HTML body contains specific markers): detected by
  `is_anti_bot_page` — defined in `livrarr-external-data/src/provider_util.rs` (shared, not
  GR-specific) and called by the GR client on every 2xx HTML body. A hit is **already** treated as
  worse than a 403: the client reports `BreakerSignal::TripImmediately` on the Goodreads bucket, so
  a soft-blocked 200 opens the breaker at once rather than counting toward a threshold (R-8).

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

1. **Don't hit `/search`** — disallowed by robots.txt. (We do not; see the status table below.)
2. **Don't run faster than 8-12 req/min sustained.** Polite floor.
3. **Don't identify as `Livrarr` / `LivrarrBot` / any project name on GR.** Browser UA only.
4. **Don't rotate UAs mid-run after a 403** — trains DataDome faster, makes blocks worse.
5. **Don't use logged-in sessions.** TOS violation; account suspension risk.
6. **Don't poll the same page repeatedly** to detect change — use long TTLs and accept staleness.
7. **Don't expand to new GR endpoints** without considering whether the data is available from HC or Wikidata first.

> **Which of these rules the code actually implements.** Verified against
> `livrarr-external-data/src/goodreads/client.rs` and `livrarr-http/src/outbound_queue.rs`:
>
> | Rule | Status |
> |---|---|
> | Don't hit `/search` | **Honored** — the only search path is `/book/auto_complete` |
> | Browser UA, never a project name | **Honored** — `GOODREADS_USER_AGENT`, set explicitly on both fetch paths |
> | Don't run faster than 8-12 req/min | **Not honored** — the bucket paces at 1.5 s (~40/min) |
> | Don't use logged-in sessions | **Honored** — no cookie is ever set on a GR request |
> | Don't rotate UAs after a 403 | **Structurally impossible** — the UA is a single constant, not a rotating set |
> | Don't poll the same page repeatedly / long TTLs | **Stated intent** — the GR client has no response cache of its own (contrast Audnexus, which does) |

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
| ~~Audit code for any `/search` calls~~ | n/a | **Already clean** — the GR client's only search path is `/book/auto_complete`; no `/search` URL is built in `goodreads/client.rs` |
| **GR-specific circuit breaker** — N consecutive 403s → 1+ hour cooldown for whole provider | **P2** | Partly exists: an anti-bot body on a 2xx already fires `BreakerSignal::TripImmediately` on the Goodreads bucket. What is missing is the 403-counting variant and the explicit 1-hour floor |
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
- Existing client: `crates/livrarr-external-data/src/goodreads/`
- rreading-glasses (proxy): <https://github.com/blampe/rreading-glasses>
