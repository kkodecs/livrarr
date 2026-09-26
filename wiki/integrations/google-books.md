# Google Books

Google Books is the documented foreign-language metadata provider. Earlier plans
for broader English fallback are not proof of implementation. It indexes editions
and offers volume search/detail; there is no equivalent author bibliography endpoint.

## Authentication and request budget

Use the administrator’s own key, never a shared shipped key. The documented client
sends `X-Goog-Api-Key` in a header. Old comparison tables saying query-string auth
were inconsistent with that client description. OAuth-gated personal-library
endpoints are outside Livrarr’s described integration.

The historical default quota is 1,000 requests per day, resetting at Pacific
midnight; keyless calls were effectively unusable. These are recorded observations,
not a fresh quota check. Canonical transport pacing does not itself track remaining
daily quota. Avoid speculative fetches and unnecessary pagination.

## Search and metadata

`/books/v1/volumes?q=isbn:...` is a filtered search, not a guaranteed single-result
lookup. Validate the returned edition evidence. Title/author lookup uses bounded
results and the shared deterministic picker; an LLM must not select a result.
Language filtering and response language are distinct—never stamp the query’s
language onto an unverified result.

Descriptions are cleaned for storage and cover URLs normalized/validated before
use. Provider data is reference-only and is not a source for upstream contribution.
Google Books has no public correction API in the documented integration.

## Efficiency and failure handling

Partial responses (`fields=`), bounded `maxResults` and compression were recorded
as desirable request practices. The old module inspection did not establish the
URL builders’ field selection, and found no gzip headers/suffix there. Do not
report those suggestions as implemented. There is no documented Books batching
endpoint.

The search error path distinguishes quota-related 403 bodies and trips the breaker
until reset; non-quota 403 is not the same error. An older second fetch path
swallowed 403 as empty. Verify both entrances at the chosen revision rather than
assuming a helper fixed every caller. Cache successful provider responses at the
shared seam; user Refresh uses its explicit bypass policy.

[Official API](https://developers.google.com/books/docs/v1/getting_started) ·
[performance guidance](https://developers.google.com/books/docs/v1/performance) ·
[transport lessons](../insights/providers-and-transport.md).
Current quotas, terms and implementation were not rechecked during cleanup.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/integrations/google-books.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="google-books-integration"></a>
<a id="current-operational-status"></a>
<a id="authentication"></a>
<a id="rate-limits-and-quotas"></a>
<a id="endpoint-surface"></a>
<a id="performance-best-practices-googles-published-guidance"></a>
<a id="partial-response--use-fields-on-every-call"></a>
<a id="gzip-compression"></a>
<a id="no-batching-endpoint"></a>
<a id="caching-strategy"></a>
<a id="anti-patterns-to-avoid"></a>
<a id="terms-of-service-highlights"></a>
<a id="how-gb-compares-to-openlibrary"></a>
<a id="scaling--what-happens-when-we-hit-the-cap"></a>
<a id="contributing-back-to-google-books"></a>
<a id="open-work-for-livrarr"></a>
<a id="related"></a>
