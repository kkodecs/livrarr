# Goodreads

Livrarr obtains public Goodreads metadata through autocomplete and page parsing.
The documented integration has no official API dependency and no LLM match
selection. An LLM may repair permitted metadata extraction without increasing
its trust. Existing anti-bot incidents make conservative failure handling essential.

## Book IDs and Work IDs

These are disjoint numeric namespaces:

| Route | Meaning | Permitted use |
|---|---|---|
| GoodreadsBookEdition | Book-page ID | Can become GrKey and fetch `/book/show/{id}` |
| GoodreadsWork | Work identity | Identity evidence; cannot fetch a Book page |

Only autocomplete workId or the referenced Work entity in a Book payload supplies
Work identity. File/list/direct and legacy gr_key inputs are Book-edition IDs.
Frozen `works.gr_key` is not current fetch authority or an active-route fallback.

Apply the distinction at every consumer: enrichment, author linking, cover choices
and selection. Work-only routes provide no detail anchor. Do not compensate with
a `/work/{id}` fetch or reinterpret the number. A mislabel repair requires typed
provenance proving the Book channel; ambiguous rows stay untouched, with generation
and audit rules preserved. [Identity lessons](../insights/identity.md#lesson-96).

## Parsing and series

Search uses autocomplete; the author is evaluated by the picker rather than blindly
appended to its prefix query. The documented integration avoids automated `/search`
HTML. Book parsing uses Next-data and older structured payload fallbacks, with
field-level tolerance, warnings and abstention on ambiguous referenced books.

Series pages changed to React data attributes. Lists include primary Works followed
by editions, bundles and translations. The header’s primary count is the roster
boundary; missing counts, unreadable pagination or a shortfall must not overwrite
the stored roster. Positions from title decorations are trusted only when they
name the current series. Save roster and count together. [Series](../domain/series.md).

## Soft blocks and covers

The August record captured two unreadable detail classes: empty HTTP-200 bodies,
and roughly 59KB Next shells without the real book payload. A bare 200 is not
healthy metadata. Detect challenge/shape failure at the application boundary and
use shared breaker/backoff policy; never turn it into empty authoritative data.

The wrong-cover incident was Work/Book namespace confusion, reversing an earlier
parser-drift diagnosis. **Goodreads cover candidates remain excluded under the
documented containment policy.** Historical rank tables do not re-enable them.
The separate provider work owns soft-block handling and any re-enable decision.

## Diagnostic capture and traffic

Unreadable Book details can be retained as exact local bytes under
`captures/goodreads/{timestamp}_{book-id}.html`, newest ten. Capture is optional at
the client and explicitly wired by composition; old descriptions of “disabled by
default” do not prove the deployed composition disables it. No external upload is
part of this mechanism. Every filesystem/task failure warns without changing the
provider outcome. Concurrent prune NotFound is benign; a later write restores the
bound after a burst.

The historical client used a fixed browser User-Agent and 1.5-second pacing;
slower/adaptive pacing was proposed, not installed by cleanup. Do not rotate
identities after a block or add authenticated scraping. The previous legal claims
and community “safe rate” estimates are archived observations, not current legal
advice or a service guarantee. Verify external rules before changing traffic.

Long-lived response caching, alternate sources and a proxy are possible product
choices; none is implied by documenting them. There is no implemented upstream
contribution workflow. [Shared provider policy](../domain/metadata-sources.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/integrations/goodreads.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="goodreads-integration"></a>
<a id="current-operational-status"></a>
<a id="authentication"></a>
<a id="what-we-get-from-gr"></a>
<a id="book-and-work-identifier-namespaces"></a>
<a id="unreadable-detail-page-captures"></a>
<a id="series-pages-2026-07-react-layout--n1-measured-on-108562--43318"></a>
<a id="rate-limits-no-published-spec"></a>
<a id="user-agent--opposite-of-openlibrary"></a>
<a id="robotstxt--what-gr-allows"></a>
<a id="backoff-and-failure-modes"></a>
<a id="soft-block-response-classes-on-bookshow-live-measured-2026-08-1920"></a>
<a id="caching"></a>
<a id="legal-posture"></a>
<a id="alternatives-we-should-be-developing"></a>
<a id="1-rreading-glasses-as-a-fronting-proxy"></a>
<a id="2-hardcover-as-a-co-equal-seriesratings-source"></a>
<a id="3-wikidata-for-series-graph-augmentation"></a>
<a id="4-user-csv-imports"></a>
<a id="anti-patterns-to-avoid"></a>
<a id="contributing-back-to-goodreads"></a>
<a id="how-gr-compares-to-other-providers"></a>
<a id="open-work-for-livrarr-priority-order"></a>
<a id="related"></a>
