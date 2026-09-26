# Audnexus

Audnexus is a community-run audiobook metadata API. The documented Livrarr client
uses ASIN detail and title/author search. Chapters come from local media extraction
through ChapterExtractor, not an Audnexus chapters request.

## Requests and caching

Public reads require no authentication in the recorded API. Use the canonical
descriptive User-Agent and shared transport bucket. The historical live rate limit
was 300 requests per minute per IP, differing from an older README default; neither
is a new service guarantee. Livrarr’s documented bucket paced more conservatively.

Responses provide Last-Modified and supported If-Modified-Since/304 revalidation.
The client caches by URL and revalidates, rather than implementing the old proposed
24-hour local TTL literally. That in-process cache and the shared persistent
provider-response cache have different owners.

Do not send `Cache-Control: no-cache` or routinely force upstream refresh with
`update=1`. The recorded client sends neither. Proactive x-ratelimit-remaining
handling was proposed, not implemented in the inspected client; ordinary shared
pacing is not the same mechanism.

## Absence, regions and reliability

An item 404/410 can mean a healthy absence; a failed search endpoint cannot use
that exemption. Let the client boundary classify application success and the
canonical transport own pauses/retries. A region mismatch may resemble not-found.
The service supports regional catalogs, but the inspected client did not pass a
region parameter; do not advertise automatic regional fallback.

Coverage depends on Audible’s catalog and may omit preorders, removed items or
books distributed elsewhere. Cached records can survive delisting. Treat temporary
origin/Cloudflare failures as degraded service, not an empty authoritative result.

Self-hosting is a possible future operational fallback, with additional database
and cache infrastructure. It is not a configured automatic failover. Source-code
contribution is available through the upstream project; no external contact or
deployment is authorized by this page.

[Upstream project](https://github.com/laxamentumtech/audnexus) ·
[response-honesty lessons](../insights/providers-and-transport.md#lesson-30).
External limits/status were not rechecked during cleanup.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/integrations/audnexus.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="audnexus-integration"></a>
<a id="current-operational-status"></a>
<a id="authentication"></a>
<a id="rate-limits"></a>
<a id="caching--the-big-win"></a>
<a id="endpoint-surface"></a>
<a id="coverage-limits"></a>
<a id="backoff-and-retries"></a>
<a id="anti-patterns-to-avoid"></a>
<a id="self-hosting--our-pressure-relief-valve"></a>
<a id="contributing-back"></a>
<a id="how-audnexus-compares-to-other-providers"></a>
<a id="open-work-for-livrarr"></a>
<a id="related"></a>
