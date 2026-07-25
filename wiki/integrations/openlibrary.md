# OpenLibrary Integration

OL is one of Livrarr's primary metadata providers. This page is the canonical reference for how we should use OL — what's allowed, what's not, what we owe back. Read before changing anything that touches OL.

## Current operational status (as of 2026-05-26)

- **OL is currently 403-blocking our User-Agent.** The string `Livrarr/<version>` shipped without a contact field, violating OL's published policy. OL added `Livrarr` (and subsequently `LivrarrBot` and `KkodecsBookBot`) to a substring/identifier blocklist.
- **Apology + clarification email sent to `openlibrary@archive.org`** on 2026-05-25 (see `build/correspondence/2026-05-25-openlibrary-ua-apology.txt`).
- **Standing instruction: NO MORE UA CHANGES until OL responds.** Every test from our IPs teaches OL's automated systems to escalate. New identifiers we try just get burned.
- **Deployed UA**: `KkodecsBookBot/<version> (Livrarr; kkodecs@proton.me; https://github.com/kkodecs/livrarr)`. This UA is *also* currently blocked, but it's the right format for when OL clears the block.
- **User-facing impact:** search returns empty for English titles, some covers fail to backfill, bibliography matching misses works whose primary key wasn't anchored.
- **How a block is classified today (not a `will_retry` stall):** a 403 falls into `ProviderFetchError::Other` (`crates/livrarr-external-data/src/openlibrary.rs:20-26`) and maps straight to `PermanentFailure { Unsupported }` on the first blocked attempt (`crates/livrarr-external-data/src/provider_client.rs:850-852`) — there is no retry stage in front of it. Only 5xx/transport failures (`WillRetry { ServerError }`, `:843-846`) and 429s (`WillRetry { RateLimit }`, 6h + up to 3h jitter, `:242-250`) schedule a retry.
- **Tracking:** GH [#83](https://github.com/kkodecs/livrarr/issues/83) (UA fix), [#73](https://github.com/kkodecs/livrarr/issues/73) (search fallback when OL unavailable).

## OL's published policy

Source of truth: <https://openlibrary.org/developers/api>.

### Rate limits

| Identification | Rate limit |
|---|---|
| **Unidentified** (no UA, or UA without contact) | 1 req/sec |
| **Identified** (UA = `App/<version> (contact-email)` or `(URL)`) | 3 req/sec |

**There is no API key, partner account, or registered application program for read traffic.** UA + contact is the only identification mechanism.

### Covers API (separate, stricter limits)

`https://covers.openlibrary.org/b/$key/$value-$size.jpg` where size = S, M, L.

| Lookup key | Limit |
|---|---|
| **ISBN, OCLC, LCCN** | 100 req / IP / 5 min |
| **Cover ID or OLID** | **Unlimited** |

**Operational rule (insight 44):** resolve any ISBN/OCLC/LCCN to a Cover ID exactly once, persist that Cover ID on the work, and thereafter fetch covers by Cover ID. We have an open improvement to wire this end-to-end.

### Identification format

Required format:
```
App/<version> (contact-email)
```

Optional but useful additions:
```
App/<version> (contact-email; https://project-url)
App/<version> (project-description; contact-email; https://project-url)
```

Our current: `KkodecsBookBot/<version> (Livrarr; kkodecs@proton.me; https://github.com/kkodecs/livrarr)`

The `Livrarr` token is in the **descriptor** position (after first space/paren), which OL allows. Only `Livrarr` in the **app-name** position (before first space) triggers the block.

## Endpoint preferences (OL's expressed wishes)

### Prefer

- `search.json?q=...&fields=...` — preferred for batch retrieval. OL specifically calls out hundreds of single-record fetches as an anti-pattern.
- Lookups by **Cover ID** or **OLID** when fetching covers (unlimited).
- **Bulk data dumps** for any operation touching >1k records (see below).
- Real-time, low-volume, human-triggered queries.

### Avoid

- HTML scraping. Always use the API.
- Hundreds of single-record `/works/<key>.json` round-trips when one `search.json` would do.
- Crawling the covers endpoint. ("Please, do not crawl our cover API.")
- Rotating IPs to bypass rate limits — explicitly called out as a bannable offense.
- Routing high-traffic commercial backends through OL.
- Bulk harvesting via live API (use dumps).

### Backoff and retries

OL doesn't publish a formal backoff schedule, but:

- Honor `Retry-After` if present in the response.
- Exponential backoff with jitter is the safe default.
- Cap retries (e.g., 5) before surfacing to the caller.
- Our current circuit-breaker doesn't honor `Retry-After` yet — open improvement.

## Bulk data dumps

For any feature that scans >1k records, we should use the dumps, not the API.

- **Location:** <https://openlibrary.org/developers/dumps>
- **Frequency:** monthly
- **Size:** Editions ~9.2 GB, Works ~2.9 GB, Authors ~0.5 GB, All Types ~12.4 GB. Plus Ratings, Lists, Reading-log, Wikidata-authors.
- **Format:** TSV (type, key, revision, last_modified, JSON-blob).
- **Available via HTTP or torrent** — torrent is faster.
- **Historical archive:** IA's `ol_exports` collection.
- **Recommended workflow:** mirror dumps locally (DuckDB on the JSONL works well — no DB infra needed), query offline, use the live API only for freshness deltas via the RecentChanges API.
- **For very high volume:** email `openlibrary@archive.org` first.

## Caching strategy

OL doesn't publish per-endpoint TTL guidance. Recommended internal defaults:

| Endpoint type | Cache TTL |
|---|---|
| `/works/<key>.json`, `/authors/<key>.json` | Weeks — canonical metadata changes slowly |
| `/books/<key>.json` (editions) | Days — identifiers/cover IDs can churn |
| `/search.json?q=...` | Hours |
| Covers (by Cover ID) | Months — immutable |

Treat OL's `Cache-Control` headers as advisory; set our own conservative TTLs.

## Contributing back to OL

Long-term plan: become a contributor, not just a consumer. Reduces our dependence and is the right thing to do.

### Editor accounts

- **Anyone can edit OL via the website** after a free signup. Reference: <https://openlibrary.org/help/faq/editing>
- **Librarian role** unlocks merges and advanced edits. Apply via the Librarians Portal: <https://openlibrary.org/librarians>
- **Bot accounts** are separate identities — username must end in "Bot". Require a GitHub issue to @mekarpeles for "API usergroup" membership. Source must live in `internetarchive/openlibrary-bots` and be PR-reviewed before any bulk run. >100 changes without prior review → privileges revoked.
- Our chosen UA app-name `KkodecsBookBot` happens to satisfy the "ends in Bot" convention. Aligned by coincidence.

### Programmatic contribution endpoints

| Endpoint | Use |
|---|---|
| `POST /api/import` | Submit a new edition as JSON (title, authors, ISBNs, publisher, publish_date, source_records). Dedupes against existing ISBNs. |
| `POST /api/import/ia` | Import directly from an Internet Archive item ID. |
| `POST /import/batch/new` | JSONL bulk batch (requires bot account approval). |
| `POST /books/add` | Same endpoint the UI uses. |
| `add_cover()` (Python `openlibrary-client`) | Attach a cover to an OLID-M (edition, **not** work). |

All require authenticated session cookies. The official `openlibrary-client` Python library is the recommended wrapper.

### Contribution opportunities for Livrarr

| Trigger | Path |
|---|---|
| User finds a book in their library not in OL | Offer one-click "submit to OL" via `/api/import` (with user consent) |
| User uploads a cover override + OL has no cover for that edition | Offer one-click "contribute cover to OL" via `add_cover()` |
| We detect two OL works that look identical | Surface to user; submit librarian merge request (or wait for programmatic merge endpoint per issue #2114) |
| High-volume data exchange (publisher catalogs, library catalogs) | Email `openlibrary@archive.org` first |

### Past examples worth studying

- **ImportBot** — OL's own canonical continuous-import bot.
- **catharbot** (@hornc) — modern reference implementation OL points to.
- **BookWyrm** — consumes OL extensively; contribution is user-initiated rather than programmatic.

## Anti-patterns to never introduce

1. Don't add UA name containing `Livrarr` (the literal substring) in the app-name position. Use descriptor or URL instead.
2. Don't bypass rate limits via IP rotation. Documented bannable.
3. Don't crawl the covers endpoint. Resolve to Cover ID, cache, fetch by ID.
4. Don't HTML-scrape OL pages. Use APIs.
5. Don't bulk-harvest via the live API. Use the monthly dumps.
6. Don't ship UA changes that target OL without coordinating with OL first — at minimum, wait until OL has cleared any active block before introducing a new identifier.

## Related

- `wiki/insights.md` insights 44, 45 (OL operational rules)
- `wiki/decisions/key-decisions.md` — multi-anchor identity model (reduces OL as a SPOF)
- `feedback_kkodecs_contact_email.md` (always use `kkodecs@proton.me` for project contact)
- GH issue [#73](https://github.com/kkodecs/livrarr/issues/73), [#83](https://github.com/kkodecs/livrarr/issues/83)
- `build/correspondence/2026-05-25-openlibrary-ua-apology.txt`
