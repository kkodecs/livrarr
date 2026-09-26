# Hardcover

Hardcover supplies English metadata and book/author/edition discovery through
`https://api.hardcover.app/v1/graphql`. Descriptive enrichment excludes it for foreign
Works even though it may appear in historical rank tables.

## Authentication and limits

Each installation supplies its own user token. Treat it as a credential and keep
requests in the backend. The historical record described a beta API with revocable
or expiring tokens, a 60-request/minute cap, maximum query depth three and a
30-second server deadline. Livrarr’s recorded requests used a shorter local timeout
and shared one-second pacing with two in-flight requests.

The old documentation says the published header takes a raw token while the client
sends `Bearer`. That discrepancy needs current provider documentation and an actual
request check before changing authentication. Header casing alone does not establish
an HTTP defect. Cleanup preserves the finding without prescribing a blind rewrite.

## Queries and evidence

Use explicit field sets and bounded results. `search()` returns a JSON-valued result
surface rather than a fully typed nested GraphQL object; validate it before use.
Recorded disabled Hasura operators include like/regex/similar families; exact/in
filters and search are the documented alternatives.

For bibliography, start from books filtered by contribution author to avoid a deep
authors→contributions→books→editions chain. Contributor lists include narrators,
translators and editors across editions. Carry the provider’s actual credit class
through author linking; an unlabeled slot is not asserted authorship.

ISBNs belong to editions. The documented schema did not expose OpenLibrary keys,
so do not assume a direct author-bibliography crosswalk. Shared deterministic match
rules decide adoption; a highest-popularity result is not authority to ignore title
or contributor contradictions.

## Failure and cache ownership

The GraphQL envelope parser owns errors and rejects partial data with errors.
Transport success alone is not application success. Preserve queue-full, open
breaker, throttling, timeout and authentication distinctions through the adapter.
Historical gaps in 429/401 mapping were partially superseded by later provider
fixes; verify the actual request path before acting on the old backlog.

POST caching lives at the provider-response seam, not ordinary upstream HTTP cache.
Old recommended 30-day metadata TTLs are not the central cache’s implemented defaults.
Work-level concurrency and the cancelled batching proposal gave no established
speed benefit under shared provider pacing.

Librarian website edits are a possible contribution path, not an implemented
automatic mutation API. Choosing Hardcover as a stronger primary provider remains
separate from this documentation cleanup.

[Official API documentation](https://docs.hardcover.app/api/getting-started/) ·
[author-route lessons](../insights/identity.md#lesson-82).
External limits, token rules and schema were not revalidated during cleanup.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/integrations/hardcover.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="hardcover-integration"></a>
<a id="current-operational-status"></a>
<a id="authentication"></a>
<a id="rate-limits"></a>
<a id="user-agent"></a>
<a id="query-patterns--good-vs-bad-citizen"></a>
<a id="good"></a>
<a id="bad"></a>
<a id="disabled-operators"></a>
<a id="caching"></a>
<a id="backoff-and-retries"></a>
<a id="anti-patterns-explicit-from-hc-docs"></a>
<a id="bulk-dumps"></a>
<a id="author-bibliography-access-verified-2026-05-26-via-gemini--codex-fact-check"></a>
<a id="contributing-back"></a>
<a id="how-hc-compares-to-other-providers"></a>
<a id="open-work-for-livrarr"></a>
<a id="related"></a>
