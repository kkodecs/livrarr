# Metadata provider roles

## Executive summary

Text search keeps up to three results from each of four providers, for a maximum
of 12 before duplicate removal and filtering. There is no additional-results link.
See [search limits](#search-limits). Discovery, enrichment and cover selection have
separate provider rules; see [provider roles](#provider-roles) and [shared rules](#shared-rules).
The metadata branch adds [database-backed enrichment priorities](enrichment-priorities.md),
including Google Books for English enrichment. This change is deployed.

## Provider roles

Providers supply different evidence. Discovery, identity capture, descriptive
enrichment and cover selection have separate policies; a provider’s place in one
does not establish its place in another.

| Provider | Documented role | Key constraint |
|---|---|---|
| [Hardcover](../integrations/hardcover.md) | English descriptive metadata; book/author/edition discovery | Token required; GraphQL request limits |
| [OpenLibrary](../integrations/openlibrary.md) | Discovery, bibliography and English enrichment | Foreign discovery is allowed; foreign descriptive contribution is excluded |
| [Google Books](../integrations/google-books.md) | English and foreign descriptive metadata | Administrator-supplied key and restrictive daily quota |
| [Goodreads](../integrations/goodreads.md) | Deterministic discovery/capture, series and supplemental metadata | Book/Work namespace split; hostile detail-page responses; cover containment |
| [Audnexus](../integrations/audnexus.md) | ASIN-based audiobook metadata | Community API, revalidation and region limitations |
| Audible | Audiobook catalog/ASIN evidence | Distinct adapter and transport bucket; do not treat its outcomes as Audnexus outcomes |
| Readarr source payload | Imported metadata offered to the merge engine | Synthetic source, not another network enrichment client |

## Search limits

Ordinary text search retains at most three matches per provider: Hardcover,
OpenLibrary, Google Books and Goodreads. Duplicate removal, AI cleanup and hiding
existing library entries can reduce the visible list below twelve. Identifier
searches follow a separate resolution path.

The cap limits retained results, not upstream request sizes. Google Books still
requests 20, OpenLibrary 50 and Hardcover 15; Goodreads controls its autocomplete
batch size. The page has no additional-results link. The existing Raw/Filtered
switch changes views of the same retained batch.

[September 18 change and verification](../../build/reviews/search-result-cap-2026-09-18/RESULT.md).

## Selected-result field mapping

[Provider field mapping](search-result-metadata.md) records what each text-search
provider supplies, where Add currently loses details, and the proposed save scope.
It distinguishes source-reported values from guessed language and separates a
cover address from a downloaded image. The preservation fix is not implemented.

## Shared rules

Applicable providers are dispatched concurrently within the shared HTTP limits.
Priority ranks field offers after capture; it is not an instruction to wait for
one provider to fail before querying the next. Covers have their own ranking and
write gate. Historical tables listing Goodreads first do not override its later
candidate exclusion.

The [deployed priority change](enrichment-priorities.md) includes Google Books for
English and unknown language, while keeping OpenLibrary/Hardcover excluded for
foreign language.
The merge boundary also rejects their foreign descriptive payloads. Discovery
wiring is separate; an OpenLibrary foreign search does not violate that rule.
Language selection was unlocked by Google Books or an LLM configuration, not by an
LLM requirement on deterministic identity.

No LLM chooses or confirms a match. It can repair/clean permitted public metadata
without increasing its trust. Missing LLM configuration must leave ordinary
deterministic workflows useful. ISBN is edition evidence; conflicting editions
are not automatically conflicting Works.

Provider success requires a readable response of the promised shape. Unreadable,
empty, unavailable, unconfigured and rate-limited are different outcomes. Preserve
good stored metadata and expose failures. User-owned fields remain protected.

## Dates and external policy

The integration pages retain measured constraints and known gaps from the original
May–August records. The cleanup did not call providers, verify their current terms
or change traffic. Recheck the relevant external contract before implementing a
provider change. Proposals for a primary provider, contribution or a new proxy
are product decisions, not implemented capabilities.

[Enrichment and covers](../architecture/enrichment-pipeline.md) ·
[transport lessons](../insights/providers-and-transport.md) ·
[metadata principles](metadata-principles.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/domain/metadata-sources.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="metadata-sources"></a>
<a id="english-pipeline"></a>
<a id="provider-priority"></a>
<a id="timeouts"></a>
<a id="foreign-language-pipeline"></a>
<a id="searchdiscovery"></a>
<a id="enrichment-providers"></a>
<a id="foreign-priority-order"></a>
<a id="language-gate"></a>
<a id="google-books-details"></a>
<a id="cover-resolution-foreign"></a>
<a id="key-rules"></a>
<a id="provider-gotchas"></a>
