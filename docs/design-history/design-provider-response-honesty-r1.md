# Design — provider response honesty

Author: Codex. Date: 2026-07-26. Stage: design for review; no implementation yet.

## Scope and decision

This design closes the response-classification gap across the six provider buckets:
OpenLibrary, Hardcover, Google Books, Audible, Audnexus, and the Goodreads
provider-client path. It does not change C5, live-library validation, or anything
under `wiki/`.

The transport intentionally reports failures it owns, but leaves completed provider
responses to the provider client. Send, body-stream, and body-size failures already
emit one `Failure` in `HttpFetcherImpl`; a completed provider response emits no
transport `Success` (`crates/livrarr-http/src/fetcher.rs:218-237`,
`crates/livrarr-http/src/fetcher.rs:303-351`). The missing layer is therefore an
application-response classifier in each client.

The decision is:

1. A completed response is healthy only after the client can read it as the
   application response that route promises.
2. A response-derived failure emits one breaker `Failure` at the leg that discovers
   it, before returning the route's existing error value.
3. A successful breaker signal is emitted at most once, at the outermost operation
   boundary, after every provider leg that ran was healthy. A legitimate empty/null
   result is a healthy answer and is eligible for that one `Success`.
4. Hardcover additionally rejects GraphQL error envelopes and malformed
   operation-specific `data`. A response containing both `data` and non-empty
   `errors` is a failure; partial data is not consumed.
5. Goodreads autocomplete stops turning an unreadable top-level response into a
   healthy empty result. Goodreads detail pages need no new classification: they
   already distinguish a usable payload from an unreadable 200.

## The signal protocol

`Success` and `Failure` are deliberately asymmetric. `Success` clears all recent
failures and can close a half-open breaker, while `Failure` appends to the evaluation
window and reopens a half-open breaker (`crates/livrarr-http/src/breaker.rs:119-170`).
The production default is five failures in 60 seconds
(`crates/livrarr-http/src/breaker.rs:176-193`).

| Signal | Ownership and cardinality |
|---|---|
| `Success` | At most one per complete provider operation. Only the outermost boundary that knows no later provider leg will run may emit it, and only if every attempted leg was healthy. A helper may emit it only when that helper is itself the whole operation. |
| `Failure` | One per failed response leg. It is emitted where a completed response is classified as non-healthy: a non-exempt non-2xx, an unreadable application body, or an invalid application envelope. Failures are not delayed or collapsed to one per outer operation; they accumulate by design. |
| Transport failure | Owned by `HttpFetcherImpl`. A caller receiving `Err(FetchError::...)` must not emit a second `Failure`, because the transport already did so. |
| No signal | A request that was not attempted because the circuit was open or the local queue was full. Local request construction/serialization errors also have no provider-health meaning. |

This preserves Packet A's rule: a best-effort operation may still return a useful
payload after a child leg fails, but it must retain the child's `Failure` and must not
emit the operation `Success`. OpenLibrary editions already have exactly that shape
(`crates/livrarr-external-data/src/openlibrary.rs:163-188`,
`crates/livrarr-external-data/src/openlibrary.rs:240-264`), as does Hardcover's
outer health bit for editions errors
(`crates/livrarr-external-data/src/provider_client.rs:802-828`).

## Healthy miss versus fake miss

A healthy miss is an absence representation defined by that route's protocol:

- an empty search collection after the expected response type parsed;
- Hardcover `books_by_pk: null`;
- HTTP 404/410 only on the settled item/detail routes: OpenLibrary work detail,
  Audible by-ASIN, and Audnexus `/books/{asin}`;
- a parsed Goodreads autocomplete array containing no usable candidate; or
- a parsed Goodreads detail payload that is usable but does not survive identity
  matching.

A fake miss is any state where the provider did not make an absence claim:

- bytes that do not decode as the route's JSON response type;
- Goodreads autocomplete returning HTML, truncated JSON, or a non-array top level;
- a Hardcover response with non-empty top-level `errors`;
- a Hardcover response missing the operation's required `data` member, or carrying
  that member with the wrong type; or
- any non-2xx from a search/fixed-GraphQL route, including 404/410.

The settled HTTP absence split does not change. OpenLibrary work detail keeps its
404/410 exemption (`crates/livrarr-external-data/src/openlibrary.rs:93-100`);
Audible by-ASIN keeps it
(`crates/livrarr-external-data/src/audible.rs:334-350`); Audnexus keeps it only for
`/books/{asin}` (`crates/livrarr-external-data/src/audnexus.rs:207-223`).
OpenLibrary search, Audible search, Google Books volumes, Goodreads autocomplete,
Hardcover's fixed GraphQL route, and OpenLibrary editions continue to treat every
non-2xx as a provider failure
(`crates/livrarr-external-data/src/openlibrary.rs:380-388`,
`crates/livrarr-external-data/src/audible.rs:289-300`,
`crates/livrarr-external-data/src/google_books.rs:115-128`,
`crates/livrarr-external-data/src/goodreads/client.rs:263-279`,
`crates/livrarr-external-data/src/hardcover.rs:95-104`,
`crates/livrarr-external-data/src/openlibrary.rs:173-188`).

### How strict the decoder becomes

This packet does not invent new required fields for the tolerant REST payloads.
"Readable" there means the existing decoder accepts the top-level type:

- Google Books intentionally accepts missing optional fields, including `{}`;
  that compatibility is explicit in both its serde model and its tests
  (`crates/livrarr-external-data/src/google_books.rs:20-35`,
  `crates/livrarr-external-data/src/google_books.rs:841-855`).
- Audible's `products` field intentionally defaults to an empty vector
  (`crates/livrarr-external-data/src/audible.rs:65-69`).
- OpenLibrary and Audnexus currently decode into `serde_json::Value`; this design
  adds the missing failure on JSON decode, but does not impose a speculative
  minimum field set
  (`crates/livrarr-external-data/src/openlibrary.rs:103-104`,
  `crates/livrarr-external-data/src/audnexus.rs:235-250`).

Hardcover is different: GraphQL defines an error envelope and operation-specific
data shapes. Rejecting those protocol-level errors is not speculative field
tightening. Goodreads autocomplete likewise has a defined top-level array; its parser
already knows when that array cannot be decoded, but currently discards the
distinction (`crates/livrarr-external-data/src/goodreads/parsers.rs:268-295`).

## Per-client disposition

| Client and response site | What counts as unreadable | Signal and returned value after the change | Healthy miss/control |
|---|---|---|---|
| OpenLibrary work detail (`query_ol_detail`) | JSON decoding fails after a 2xx (`crates/livrarr-external-data/src/openlibrary.rs:103-104`). | Emit OpenLibrary `Failure`, then preserve `Err(ProviderFetchError::Other("parse: ..."))`. Enrichment callers therefore retain their current `PermanentFailure { Unsupported }` mapping (`crates/livrarr-external-data/src/provider_client.rs:871-890`). | Work 404/410 remains a healthy item absence. A readable work response may continue to the editions leg; only the outer caller may emit `Success`. |
| OpenLibrary ISBN lookup (`isbn_lookup`) | JSON decoding fails after a 2xx (`crates/livrarr-external-data/src/openlibrary.rs:321-334`). | Emit `Failure`, then preserve the existing `Err(ProviderFetchError::Other(...))`; no fuzzy fallback is introduced. | `Ok(None)` from a readable response, or 404/410, remains a healthy miss whose final operation boundary owns `Success`. |
| OpenLibrary discovery search (`search_openlibrary`) | JSON decoding fails (`crates/livrarr-external-data/src/openlibrary.rs:391-404`). | Emit `Failure`, then preserve the existing `Err(String)`. | A readable empty `docs` result remains healthy and the single-leg operation still emits `Success` after parsing. |
| OpenLibrary provider title/author search | JSON decoding fails (`crates/livrarr-external-data/src/provider_client.rs:1168-1178`). | Emit `Failure`, then preserve `Err(ProviderFetchError::Other(...))`. | A readable search miss reports the operation's one `Success` only at a terminal outer arm (`crates/livrarr-external-data/src/provider_client.rs:1210-1235`). |
| OpenLibrary editions | Already compliant: unreadable 2xx emits `Failure`, marks the child leg failed, and keeps the useful work payload (`crates/livrarr-external-data/src/openlibrary.rs:240-264`). | No production change for this site. It is the reference for best-effort payload plus unhealthy operation. | A readable empty `entries` collection is healthy; no child-helper `Success`. |
| Hardcover common GraphQL POST (`hc_post`) | JSON decode failure; non-empty `errors`; malformed `errors`; missing/non-object top-level `data` (`crates/livrarr-external-data/src/hardcover.rs:107-115`). | Emit Hardcover `Failure`, then return `Err(HardcoverError::Http(...))`. Do not include the raw body or token in the error. | `errors` absent or an empty array may proceed to operation-specific validation. `hc_post` still emits no `Success`. |
| Hardcover search and ISBN search | Expected `/data/search/results/hits` is missing or not an array; a non-empty array remains subject to the existing matcher (`crates/livrarr-external-data/src/hardcover.rs:127-132`, `crates/livrarr-external-data/src/hardcover.rs:178-203`, `crates/livrarr-external-data/src/hardcover.rs:478-503`). | Emit `Failure`, return `Err(HardcoverError::Http(...))`. This replaces the fake `NoResults`/`Ok(None)` only for invalid envelopes. | An explicitly present empty `hits` array is a healthy miss. |
| Hardcover key lookup | `/data/books_by_pk` is absent or is neither null nor an object (`crates/livrarr-external-data/src/hardcover.rs:610-626`). | Emit `Failure`, return `Err(HardcoverError::Http(...))`; the anchor caller becomes `WillRetry { ServerError }` instead of `NotFound` (`crates/livrarr-external-data/src/provider_client.rs:654-686`). | An explicitly present `books_by_pk: null` is a healthy miss and the anchor boundary reports `Success`. |
| Hardcover editions | JSON decode failure, GraphQL error envelope, or missing/non-array `/data/editions` (`crates/livrarr-external-data/src/hardcover.rs:421-442`). | Emit `Failure` and preserve the helper's `Err(String)`. `build_success` still treats ISBN enrichment as best-effort, returns the book payload, and suppresses the outer breaker `Success` (`crates/livrarr-external-data/src/provider_client.rs:809-828`). | An explicitly present empty `editions` array is healthy and returns `Ok(None)`. |
| Google Books shared volumes search | `GbSearchResponse` deserialization fails (`crates/livrarr-external-data/src/google_books.rs:131-140`). | Emit Google Books `Failure`, then preserve `Err(String)`. | `totalItems: 0`, absent `items`, and the explicitly tolerated `{}` remain readable healthy empties. |
| Google Books provider search | `GbSearchResponse` deserialization fails (`crates/livrarr-external-data/src/google_books.rs:237-243`). | Emit `Failure`, then preserve `WillRetry { ServerError }` with the existing 300-second backoff. | A readable empty response keeps the current `Success` and `NotFound` path (`crates/livrarr-external-data/src/google_books.rs:357-375`). |
| Audible title search | `AudibleSearchResponse` deserialization fails (`crates/livrarr-external-data/src/audible.rs:303-307`). | Emit Audible `Failure`, then preserve `Err(ProviderFetchError::Other(...))`; the client retains `WillRetry { ServerError }` (`crates/livrarr-external-data/src/audible.rs:178-220`). | A readable empty `products` vector is a healthy miss. |
| Audible by-ASIN item lookup | `AudibleSearchResponse` deserialization fails (`crates/livrarr-external-data/src/audible.rs:353-357`). | Emit `Failure`, then preserve the same `Err`; both anchor and seeded callers retain `WillRetry { ServerError }` (`crates/livrarr-external-data/src/audible.rs:108-124`, `crates/livrarr-external-data/src/audible.rs:132-175`). | A readable empty `products` vector and item-route 404/410 remain healthy absence. |
| Audnexus cached fetch, shared by item and search | JSON decoding fails (`crates/livrarr-external-data/src/audnexus.rs:235-250`). | Emit Audnexus `Failure`, then preserve `Err(ProviderFetchError::Other("parse: ..."))`; enrichment callers retain `PermanentFailure { Unsupported }` (`crates/livrarr-external-data/src/provider_client.rs:471-498`). The unreadable body is not cached. | Search `[]` is healthy empty; item 404/410 is healthy absence; a 304 reuses only an already parsed cached value (`crates/livrarr-external-data/src/audnexus.rs:207-213`). |
| Goodreads established-key detail | Both the deterministic parser and optional LLM extraction fail to yield a usable book (`crates/livrarr-external-data/src/provider_client.rs:2316-2338`). | Already compliant: emit Goodreads `Failure` and return `WillRetry { ServerError }` (`crates/livrarr-external-data/src/provider_client.rs:2341-2374`). No production change. | A usable parser or LLM payload emits the operation `Success`. |
| Goodreads search-resolved detail | The fetched detail page yields neither parser nor LLM payload (`crates/livrarr-external-data/src/provider_client.rs:2506-2560`). | Already compliant: emit `Failure`; retain the PO-accepted key-only `Success` when Goodreads search data can still vouch for the key, otherwise return `WillRetry`. No breaker `Success` accompanies the degraded payload. | A usable detail payload remains healthy. |
| Goodreads autocomplete used by the provider client | Top-level JSON is invalid or not an array. Today the parser logs and returns an empty vector, and `search_goodreads` returns it as `Ok`, creating a fake miss (`crates/livrarr-external-data/src/goodreads/parsers.rs:268-295`, `crates/livrarr-external-data/src/goodreads/client.rs:281-288`). | Emit Goodreads `Failure` in `search_goodreads`, then return the existing `GoodreadsFetchError::Parse`; the terminal title tier becomes `WillRetry { ServerError }` through the existing mapper (`crates/livrarr-external-data/src/provider_client.rs:2657-2722`). | A decoded empty array is a healthy miss. Individual malformed entries remain isolated and dropped; one bad entry does not poison valid siblings. |

### Goodreads autocomplete composition

`search_goodreads` is an intermediate helper and must continue to emit no `Success`
(`crates/livrarr-external-data/src/goodreads/client.rs:281-288`). It may now emit a
response-derived `Failure`. The ISBN autocomplete tier intentionally falls through
after an error (`crates/livrarr-external-data/src/provider_client.rs:2884-2892`), so
the resolution result must carry an `all_legs_succeeded` bit to the final Goodreads
boundary. A later title/detail success may return a useful application payload, but
must not clear the earlier ISBN-leg failure.

The same bit must be available when resolution returns no detail candidate. A decoded
empty array or a set of readable candidates that fails matching is a healthy terminal
miss and receives one outer `Success`; a miss reached after a failed ISBN leg receives
no `Success`. This is the Goodreads equivalent of Packet A's
`OlDetailResult::all_legs_succeeded` contract
(`crates/livrarr-external-data/src/openlibrary.rs:35-48`,
`crates/livrarr-external-data/src/openlibrary.rs:255-264`).

The public lenient autocomplete parser also has a discovery caller outside the
provider-client path (`crates/livrarr-metadata/src/discovery_service.rs:752-756`).
Implementation should expose a checked parser result and have
`goodreads::search_goodreads` use it. The discovery caller may deliberately map the
checked error back to an empty union contribution, preserving its current user-facing
behavior; this packet does not redesign discovery orchestration. The provider-client
path must not discard the error.

## Hardcover GraphQL policy

### Common envelope check

The top-level `errors` check belongs in `hc_post`, immediately after JSON decoding
and before any query-specific extractor. That is the one common path used by
`query_hardcover`, `query_hardcover_by_isbn`, and `query_hardcover_by_key`
(`crates/livrarr-external-data/src/hardcover.rs:178-199`,
`crates/livrarr-external-data/src/hardcover.rs:478-487`,
`crates/livrarr-external-data/src/hardcover.rs:610-620`).

`fetch_hardcover_editions` currently duplicates the POST/status/decode sequence and
therefore bypasses `hc_post`
(`crates/livrarr-external-data/src/hardcover.rs:380-434`). It must be routed through
`hc_post` (mapping `HardcoverError` back to its existing `String` error at its public
boundary), or through the exact same private envelope decoder. Routing it through
`hc_post` is preferred because it removes the duplicate transport/status logic and
makes one check cover every GraphQL query.

Envelope rules:

- missing `errors` or `errors: []`: continue;
- non-empty `errors`: emit one `Failure` and return `HardcoverError::Http`;
- `errors` present with any non-array shape, including null: malformed envelope,
  same failure;
- missing or non-object `data`: malformed envelope, same failure;
- never log or persist the full response body; a bounded generic message and error
  count are sufficient.

### Partial responses

`data` plus non-empty `errors` is a failure, even when the requested field is
present. The client must not merge the partial data. GraphQL's error tells us the
operation was not fully satisfied; accepting whichever fields survived could both
clear accumulated failures and persist metadata from an authorization/schema failure.

This is intentionally all-or-nothing at the GraphQL operation boundary. Hardcover
editions remains best-effort at the *parent payload* boundary: its GraphQL operation
fails and emits `Failure`, but the already obtained book payload may still be
returned without a breaker `Success`.

## Application behavior, retries, and provider health

Most malformed-JSON branches already return an error. Adding the missing breaker
signal does not change their immediate application outcome:

- OpenLibrary and Audnexus enrichment parse errors remain terminal
  `PermanentFailure { Unsupported }`.
- Audible and Google Books provider parse errors remain
  `WillRetry { ServerError }`.
- OpenLibrary/Google Books discovery helpers retain their existing `Err` return.
- Hardcover malformed JSON already maps through `HardcoverError::Http` to
  `WillRetry { ServerError }`.

Two fake-miss classes do change:

1. Hardcover GraphQL `errors` or malformed required `data` changes from
   `NotFound`/fallback to `WillRetry { ServerError }`. In particular, the two anchor
   `Ok(None)` arms currently report `Success` and return `NotFound`
   (`crates/livrarr-external-data/src/provider_client.rs:632-651`,
   `crates/livrarr-external-data/src/provider_client.rs:667-686`). After this
   change, an auth/schema refusal cannot clear failures or persist absence.
2. A terminal Goodreads autocomplete decode failure changes from `NotFound` to
   `WillRetry { ServerError }`. A failed ISBN autocomplete followed by a usable
   title/detail result may still return a payload, but keeps the leg's breaker
   `Failure` and emits no operation `Success`.

`WillRetry { ServerError }` consumes one retry-budget attempt and becomes
`PermanentFailure { RetryBudgetExhausted }` at the configured limit. Once the
breaker opens, later `CircuitOpen` outcomes are pauses and consume no attempts
(`crates/livrarr-enrichment/src/provider_queue.rs:554-599`,
`crates/livrarr-enrichment/src/provider_queue.rs:669-689`).

The call-record health surface follows the final `ProviderOutcome`, not breaker
signals. `WillRetry { ServerError }` and `PermanentFailure` map to
`CallOutcomeClass::Error`, while `NotFound` and `Success` do not
(`crates/livrarr-external-data/src/provider_client.rs:252-293`);
`is_error` counts error/rate-limit/timeout records
(`crates/livrarr-db/src/sqlite_provider_calls.rs:33-44`).
Consequently:

- the changed Hardcover anchor and terminal Goodreads fake misses become
  `is_error = true`;
- parse branches whose final outcome was already an error keep their existing
  `is_error` value;
- best-effort Hardcover/OpenLibrary editions and Goodreads key-only degradation
  may still produce a successful application record (`is_error = false`) while
  filing a breaker `Failure`. This is intentional: payload usefulness and route
  health are different facts.

## Implementation shape

1. Add the response-derived `Failure` adjacent to each existing JSON decode error.
   The closure must wrap only decoding of an already completed `FetchResponse`; it
   must never wrap `fetch(...).await`, which would double-count transport failures.
2. Keep the current error variants and retry mapping for all existing decode-error
   branches.
3. Make `hc_post` the common GraphQL envelope authority and route editions through
   it. Change the search hit extractor to distinguish an explicitly empty array from
   a missing/wrong-typed path. Apply equivalent operation-field checks to key and
   editions responses.
4. Give Goodreads autocomplete a checked top-level parser. Preserve per-entry
   isolation after a valid array has decoded.
5. Carry Goodreads resolution health to the final boundary so a fallback cannot
   erase an earlier leg failure.
6. Do not add `Success` to `HttpFetcherImpl`, to a request helper that can be
   followed by another provider leg, or to any operation with a failed leg.

### Test seams

The lower-level request functions are generic over `HttpFetcher`, and the existing
recording double can replay ordered responses
(`crates/livrarr-external-data/src/test_support.rs:9-50`,
`crates/livrarr-external-data/src/test_support.rs:78-107`). That makes every raw
decode branch hermetically reachable.

The Hardcover application-outcome change is not hermetically reachable through its
real client today: `HardcoverClient` owns a concrete `HttpFetcherImpl`
(`crates/livrarr-external-data/src/provider_client.rs:550-574`). The hardcoded URL
does not matter to a trait double, so a base-URL setting is unnecessary. The smallest
seam is to make the struct generic with a default:

```text
HardcoverClient<F: HttpFetcher = HttpFetcherImpl> { fetcher: F, ... }
```

`ProviderClient::Hardcover` continues to hold the default concrete specialization.
Tests in `provider_client.rs` can instantiate
`HardcoverClient<RecordingHttpFetcher>` and drive the real anchor, seeded, fallback,
and editions composition. This mirrors the existing generic OpenLibrary client
(`crates/livrarr-external-data/src/provider_client.rs:899-912`) and should require
only the type parameter/impl change plus `Clone` on the recording double. Injecting
a configurable production base URL was considered and rejected as a wider,
unnecessary API change.

The Goodreads terminal autocomplete composition has the same testability problem:
`GoodreadsClient` owns a concrete fetcher
(`crates/livrarr-external-data/src/provider_client.rs:2254-2283`), and its
search-resolved detail uses `fetch_ssrf_safe`
(`crates/livrarr-external-data/src/provider_client.rs:2453-2467`), which prevents a
loopback server from driving that door. The recording double already implements
`fetch_ssrf_safe` (`crates/livrarr-external-data/src/test_support.rs:148-162`).
Genericizing only the fetcher field with the same default-type pattern is therefore
the smallest honest seam; the separate `HttpClient` used for optional LLM repair
stays unchanged.

## Red-first test plan

Every breaker test must hold the shared provider-breaker lock; the breaker is a
process-global singleton (`crates/livrarr-external-data/src/test_support.rs:169-184`).
For response-derived failures, set a threshold of one, invoke the production response
site with a canned 200, assert the existing return value, and then assert that a new
admission is `CircuitOpen`. Those tests are red before adding each signal.

### Hermetically reachable now

| Area | Red test and positive control |
|---|---|
| OpenLibrary | Independently drive invalid JSON through `query_ol_detail`, `isbn_lookup`, `search_openlibrary`, and `OpenLibraryClient::title_author_search`; each must open a threshold-one breaker without a synthetic injected failure. Retain controls for a readable empty search and item 404/410. |
| Google Books | Drive invalid JSON through both `fetch_gb_volumes` and private `fetch_gb_search`; assert the former's `Err(String)` and the latter's existing `WillRetry`, plus breaker open. Keep `{}` and `totalItems: 0` controls healthy. |
| Audible | Drive invalid JSON through both search and by-ASIN functions; assert `Err(Other)` and breaker open. Keep `{"products":[]}` and item 404/410 controls healthy. |
| Audnexus | Drive invalid JSON through both the item and search entry points sharing `cached_fetch`; assert `Err(Other)`, breaker open, and no cache reuse of the invalid body. Keep search `[]`, item 404/410, and valid-response-then-304 controls. |
| Hardcover request layer | Drive malformed JSON, `{"errors":[...]}`, `data + errors`, malformed `errors`, and missing `data` through `hc_post`; each must return `Err(Http)` and open the breaker. Drive valid empty `hits`, `books_by_pk: null`, and empty `editions` as controls. Call all four query paths to prove the common check covers title search, ISBN search, key lookup, and editions. |
| Goodreads request layer | Drive HTML/truncated/non-array bodies through `search_goodreads`; assert `Err(Parse)` and breaker open. Drive `[]` as a healthy empty and a mixed valid/malformed array as a successful partial batch. Existing established-key unreadable-page tests remain the detail-path controls (`crates/livrarr-external-data/src/provider_client.rs:3033-3155`). |

### Requires the client seams above

| Area | Red test |
|---|---|
| Hardcover anchor ISBN and key | A 200 GraphQL `errors` response must produce `WillRetry { ServerError }`, map to call-record `Error`, and leave the breaker open. Before the fix it is `NotFound`, reports boundary `Success`, and leaves/returns the breaker healthy. |
| Hardcover seeded fetch | An ISBN query or title query returning GraphQL `errors` must stop as `WillRetry`, not fall through or terminalize as `NotFound`. A `data + errors` body that contains an otherwise usable hit must still make only one request and return `WillRetry`. |
| Hardcover editions composition | A valid book hit followed by an editions GraphQL error must still return the useful payload, emit the editions `Failure`, and emit no outer `Success`. A valid empty editions array must return the payload and one outer `Success`. |
| Goodreads terminal title tier | Invalid autocomplete must return `WillRetry { ServerError }`, map to call-record `Error`, and leave the breaker open instead of returning `NotFound`. |
| Goodreads ISBN fallback | Invalid ISBN autocomplete followed by valid title autocomplete/detail may return the useful payload, but must retain the first leg's `Failure` and emit no operation `Success`. A fully healthy fallback operation must still emit exactly one final `Success`. |
| Goodreads healthy miss | A valid empty title autocomplete must return `NotFound` and emit one outer `Success`; the same terminal miss after a failed ISBN leg must not emit `Success`. |

There should also be one production-threshold regression per multi-leg provider
(Hardcover and Goodreads), because a threshold-one test cannot detect a
`Failure, Success, Failure, Success` sequence. Packet A's Hardcover composition test
documents this exact reason (`crates/livrarr-external-data/src/hardcover.rs:731-801`).

After red/green targeted runs, the implementation packet should run the normal gates:
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`, and
`cargo test --workspace --no-fail-fast`, with output and exit codes captured from
their own logs.

## Considered and rejected

- **Report `Success` on every 2xx in the transport.** Rejected: a 2xx can carry HTML,
  truncated JSON, a Goodreads shell, or a GraphQL error. It would recreate the defect
  this design closes.
- **Report one breaker outcome total per outer operation.** Rejected for failures.
  `Success` must be singular because it clears history; failures represent failed
  legs and must accumulate. Transport-owned failures remain exactly once.
- **Treat unreadable responses as `NotFound`.** Rejected: absence was never asserted,
  and terminal persistence would turn provider breakage into book absence.
- **Change every parse error to a new retry taxonomy.** Rejected for this packet.
  Except where an existing fake miss must become an error (Hardcover GraphQL and
  Goodreads autocomplete), the current return variants and retry behavior are
  preserved. This change is about the missing health signal.
- **Consume Hardcover partial `data` when `errors` is present.** Rejected: it makes
  authorization/schema failures look healthy and permits incomplete metadata to be
  persisted.
- **Check GraphQL errors only in the search callers.** Rejected: it misses key lookup
  and editions. The check belongs in the common POST/decoder path.
- **Require new minimum-field schemas for every tolerant REST response.** Rejected
  without provider-specific evidence. Existing optional-field compatibility remains;
  this packet fixes actual decoder failures and protocol-defined GraphQL/autocomplete
  envelopes.
- **Inject a Hardcover base URL solely for tests.** Rejected: a generic fetcher seam
  reaches the real client control flow with the existing recording double and avoids
  a new production setting.

## Open question for review

The provider-call `is_error` surface records final application outcomes, while the
breaker records leg health. This design preserves best-effort successes whose child
leg failed, so those calls remain `is_error = false` even though the breaker receives
`Failure`. That is consistent with existing OpenLibrary editions and the accepted
Goodreads key-only degradation, but it means the 24-hour call summary alone cannot
explain why a breaker opened. If reviewers require that surface to expose degraded
successes, it should be a separate observability design (for example, a distinct
`degraded_success` detail/class), not a reason to misclassify the useful payload or
suppress the breaker failure here.
