# OpenLibrary

OpenLibrary supplies discovery, bibliography and English metadata. Foreign-language
discovery may use its language filter; foreign descriptive enrichment excludes its
payloads. [Provider roles](../domain/metadata-sources.md).

## Identification and traffic

The preserved May 2026 operational record reports User-Agent blocking and a sent
clarification email. That is not a current connectivity check. Keep the standing
instruction against experimenting with replacement identities after a block;
do not rotate identifiers or IPs to evade rate limits.

The recorded public policy requires an application/version with contact information:
unidentified requests get one per second and identified requests three. Livrarr’s
canonical transport has its own pacing, which may be more conservative. The
configured identity, provider contact history and exact observations are retained
in the source record; no provider was contacted during cleanup.

Use API endpoints rather than HTML scraping. Prefer search with selected fields
over hundreds of single-Work detail calls. Large corpus operations (the recorded
guidance uses over 1,000 records) belong on monthly dumps plus RecentChanges deltas.
Dumps contain TSV rows with JSON payloads and are available over HTTP/torrent;
their old sizes are not current estimates.

## Covers, cache and response shape

The documented covers policy distinguishes ISBN/OCLC/LCCN lookups (100 per IP per
five minutes) from Cover ID/OLID fetches. Resolve stable Cover IDs once and cache
them; do not repeatedly crawl the identifier endpoint. The public cover URL pattern
is `/b/{key}/{value}-{S|M|L}.jpg` on the covers host.

Author bibliography responses use `entries`; Work keys include `/works/` and need
normalization. Publication dates have varied formats, so date parsing cannot assume
an ISO value. Edition selection must compare the edition’s own title before using
language as a preference; language alone once selected the wrong product variant.

Reuse stable metadata and cover bytes. The central provider cache owns actual TTL
policy; the old suggested weeks/hours/months table was guidance, not proof that
every endpoint had that cache. Preserve good data when parsing fails.

## Contributions and known limits

OpenLibrary has human editing, librarian and approved-bot contribution paths. Any
future contribution must have user consent and follow the provider’s then-current
rules; existing read access is not write authorization. Do not copy proposed
submission endpoints into an automatic upload workflow.

Historical 403/error mappings and the block status need fresh source/runtime checks
before troubleshooting an active failure. Keep HTTP retry/backoff decisions at the
canonical transport rather than introducing a provider-local limiter.

[Official API policy](https://openlibrary.org/developers/api) ·
[data dumps](https://openlibrary.org/developers/dumps) ·
[cover lessons](../insights/covers.md#lesson-44).
External policies and service status were not revalidated in this cleanup.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/integrations/openlibrary.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="openlibrary-integration"></a>
<a id="current-operational-status-as-of-2026-05-26"></a>
<a id="ols-published-policy"></a>
<a id="rate-limits"></a>
<a id="covers-api-separate-stricter-limits"></a>
<a id="identification-format"></a>
<a id="endpoint-preferences-ols-expressed-wishes"></a>
<a id="prefer"></a>
<a id="avoid"></a>
<a id="backoff-and-retries"></a>
<a id="bulk-data-dumps"></a>
<a id="caching-strategy"></a>
<a id="contributing-back-to-ol"></a>
<a id="editor-accounts"></a>
<a id="programmatic-contribution-endpoints"></a>
<a id="contribution-opportunities-for-livrarr"></a>
<a id="past-examples-worth-studying"></a>
<a id="anti-patterns-to-never-introduce"></a>
<a id="related"></a>
