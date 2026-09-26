# Providers and transport lessons

Current guidance with links to the full dated evidence. Read implementation claims
against the named source revision; accepted design is not proof of runtime behavior.

<a id="22-enrichment-timeout-is-10s-per-provider"></a>
<a id="lesson-22"></a>
## 22. Provider deadlines differ

Ten seconds was an enrichment timing amendment, not a universal per-provider timeout. The detailed clients historically used both 10s and 30s requests. Interactive request budgets exclude queue admission; verify the actual request site before changing a deadline.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#22-enrichment-timeout-is-10s-per-provider).

<a id="28-sabnzbd-search-parameter-searches-by-name-not"></a>
<a id="lesson-28"></a>
## 28. SABnzbd history search

SABnzbd history search matches names, not nzo_id. A missing result from an ID-shaped search does not prove the download disappeared. See [Usenet recovery](../architecture/usenet-pipeline.md).

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#28-sabnzbd-search-parameter-searches-by-name-not).

<a id="29-url-encode-all-api-keys-in-query-strings"></a>
<a id="lesson-29"></a>
## 29. Encode query credentials

Use proper URL/query encoding for any credential or parameter sent in a query. Raw concatenation breaks values containing &, = or spaces. Do not log credentials while diagnosing the request.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#29-url-encode-all-api-keys-in-query-strings).

<a id="30-all-outbound-provider-http-flows-through-one"></a>
<a id="lesson-30"></a>
## 30. One outbound queue

All provider traffic uses the process-global canonical queue. It owns per-bucket pacing, bounded admission, in-flight limits, priority and breaker behavior. Cover/None buckets and provider/indexer buckets have different policies; derive them from request provenance.

Queue-full and circuit-open are pauses, not consumed retry budget or terminal absence. A completed HTTP 2xx is healthy only after the client decodes the promised response. Each operation has one success owner; decoding/envelope failure emits one failure at the discovering leg. Partial GraphQL data with errors is rejected. Healthy item absence differs from a broken search endpoint.

Historical suppression machinery had no production producer and was deleted; unrelated RSS or identity dismissal suppression remains a different concept. See [enrichment](../architecture/enrichment-pipeline.md) and the per-provider pages for current roles and dated transport measurements.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#30-all-outbound-provider-http-flows-through-one).

<a id="31-filter-grabs-by-download_client_id-before-matching"></a>
<a id="lesson-31"></a>
## 31. Download matching is client-scoped

Filter Grabs by download_client_id before matching download IDs or unlinked candidates. Otherwise one client can adopt or mutate another client’s download.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#31-filter-grabs-by-download_client_id-before-matching).

<a id="33-per-provider-error-enums-not-shared-providererror"></a>
<a id="lesson-33"></a>
## 33. Map provider errors at the adapter

Providers have their own error enums mapped to shared outcomes at the boundary. Preserve actual distinctions rather than prematurely flattening every failure into one generic provider error.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#33-per-provider-error-enums-not-shared-providererror).

<a id="43-openlibrary-author-works-endpoint-structure"></a>
<a id="lesson-43"></a>
## 43. OpenLibrary bibliography shape

The documented author works response uses entries, with keys like /works/OL12345W. Publication dates are not uniformly ISO: year extraction historically accepts the first four-digit token. Preserve prefix normalization and tolerant date handling; never infer works/results/items from another endpoint.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#43-openlibrary-author-works-endpoint-structure).

<a id="45-ol-anti-pattern"></a>
<a id="lesson-45"></a>
## 45. Avoid per-record bulk API harvest

OpenLibrary explicitly discourages hundreds of individual work fetches when search fields can return a batch. Large corpus work belongs on monthly dumps plus freshness deltas, not the live API. Keep provider identification and caching rules; see [OpenLibrary](../integrations/openlibrary.md).

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#45-ol-anti-pattern).

<a id="58-process-global-breaker-state-wedges-parallel-tests"></a>
<a id="lesson-58"></a>
## 58. Shared breaker state needs one test guard

A process-global breaker can remain open for an hour and prevent a sibling test’s server from ever receiving a connection. Resetting only after the tripping test does not prevent overlap. Every request-driving/emitting/reading test in the binary takes the same private guard, with reset on entry and Drop. Two same-named mutexes do not serialize anything.

Bound one-shot-server waits. Never restore premature HTTP success to conceal the race. The recorded external-data sweep fixed its callers; the old enrichment flake must be rechecked at the chosen source rather than assumed fixed or permanently accepted.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#58-process-global-breaker-state-wedges-parallel-tests).

<a id="62-provider-parsers-must-distinguish-unreadable-from"></a>
<a id="lesson-62"></a>
## 62. Unreadable is not empty

Parse list entries independently where safe, warn on shape drift and never persist emptiness over good stored data. Goodreads series blobs include editions, bundles and translations: membership is not the primary roster. Use the declared primary count, reject short/unreadable pagination, and trust a title’s position only when it names that series. Empty stored rosters read as absent and can heal; every roster save updates its count.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#62-provider-parsers-must-distinguish-unreadable-from).

<a id="68-persistent-provider-response-cache"></a>
<a id="lesson-68"></a>
## 68. Provider cache policy belongs at its seam

The persistent provider_response_cache is global public-provider data keyed by provider/anchor type/value. One queue seam decides TTL, success-only writes and PreferCache versus Bypass after applicability and terminal checks. The DB owns storage, not that policy.

Historical defaults are seven days and 100,000 rows with oldest-first eviction; user refresh bypasses cache. Hits produce no HTTP call record or spent network attempt. A repeated fixture must reset retry standing too: resetting Work enrichment status alone can leave Success/NotFound terminal-skipping before the cache.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#68-persistent-provider-response-cache).

<a id="69-http-clients-are-consolidated-shared-instances"></a>
<a id="lesson-69"></a>
## 69. Share clients; measure concurrency

Clone the composition’s shared trusted client, safe client, fetcher and LLM client rather than constructing fresh pools. Queue sharing and connection-pool sharing are different facts. The LLM client uses the trusted class for configured infrastructure with an explicit timeout.

The measured three-Work bulk concurrency change gave no wall-clock gain because shared provider buckets were the bottleneck. The user cancelled the related Hardcover batching proposal. More concurrency or batching needs new evidence, not reuse of an old performance hypothesis.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#69-http-clients-are-consolidated-shared-instances).

<a id="70-one-qbittorrent-state-classifier-quality-waves-2a"></a>
<a id="lesson-70"></a>
## 70. One qBittorrent state classifier

One classifier returns both UI state and import safety. Resume checking, checking completed data and moving are not import-safe; downloaded bytes must also be at their stable path. Extend the classifier and its production-path pins together. SAB’s status vocabulary is separate and has no authority merely because qBit has one.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#70-one-qbittorrent-state-classifier-quality-waves-2a).

<a id="71-indexer-citizenship"></a>
<a id="lesson-71"></a>
## 71. Indexer pacing and breaker scopes

Normalize configured origins in one HTTP authority. Shared-host indexers share pacing and a transport breaker, but each configured indexer has its own rate-limit breaker. A 429 proves the host answered: trip that indexer’s rate-limit breaker without marking transport dead. The recorded Retry-After policy clamps delta seconds to 10s–6h, else 30m; an unidentifiable indexer has no per-indexer breaker.

Grant admission checks both breaker scopes, including after pacing waits. Every request site carries its configured indexer identity; client RPC is a different provenance class. Rate-limit/circuit-open during torrent download must not delegate the raw URL to the client.

The release cache keys title/author/indexer. Cache-only misses are successful empty results, not AllIndexersFailed; live refresh bypasses cache. Test doubles must override fetch_no_redirect explicitly when required because its default follows redirects.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#71-indexer-citizenship).

<a id="79-the-process-global-outbound-queue-can-wedge"></a>
<a id="lesson-79"></a>
## 79. Queue dispatcher lifetime

A dispatcher tied to a test runtime can die while every remaining caller is already queued. A new-arrival restart check cannot rescue those existing waiters. The recorded fix periodically rechecks dispatcher ownership while waiting and restarts if work remains. Preserve the exact mechanism/evidence before changing it; the historical question of retaining the watchdog was a separate scope decision.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md#79-the-process-global-outbound-queue-can-wedge).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/providers-and-transport.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="providers--transport-insights"></a>
