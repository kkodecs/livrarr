# HTTP handlers

`livrarr-handlers` owns HTTP inputs, validation, calls through service contracts and
response mapping. Its dependency boundary excludes db, metadata, tagwrite and
download implementation crates. That compiler boundary does not prove all business
logic has been removed. [Root architecture](../../ARCHITECTURE.md).

## Start here

| Concern | Navigation |
|---|---|
| Required services | `context.rs`, narrow Has* capabilities |
| Shared state access | `accessors.rs`, composition adapters in server |
| Error/status mapping | `types/api_error.rs`; [error handling](../patterns/error-handling.md) |
| Work lookup/add/refresh | `work.rs`; [creation](../architecture/work-creation-pipeline.md) |
| Manual and list import | `manual_import.rs`, `list_import.rs`; [import](../architecture/import-pipeline.md) |
| Review actions | `identity_layer.rs`; [review reference](../architecture/identity-review-census.md) |
| Covers and file delivery | `cover.rs`, `mediacover.rs`, `coverproxy.rs`, `workfile.rs` |
| Authenticated route composition | server router and middleware, plus handler admin extractors |

The former long route/method inventory is retained in history. Use Serena on the
actual route registration, handler and service trait to establish availability.
A declared function can be unregistered; a registered test can still be ignored.

## Boundaries to preserve

Bind only the capabilities a handler uses. A cohesive group can define a composite
of those narrow capabilities; it need not inherit full AppContext. Configuration
and credential-bearing access are separate contracts.

Cloneable handler state can own a background continuation. Preserve the triggering
user intent, failures, generation observations, cancellation and task lifetime.
Different branches of the Add completion chain are not interchangeable. Trace the
real HTTP path: a service-level Add test does not cover a handler using identity
settlement directly.

Authentication, tenant scope and admin checks are part of the production entry
path. Test those with the real router rather than a toy route that omits middleware.
Errors need stable codes, useful recovery text and request context without secrets
or internal paths. Pagination is part of the API contract; one page is not all Works.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/crates/handlers.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="livrarr-handlers"></a>
<a id="capability-traits-contextrs"></a>
<a id="accessor-traits-accessorsrs"></a>
<a id="middleware-middlewarers"></a>
<a id="route-handlers"></a>
<a id="workrs"></a>
<a id="authorrs"></a>
<a id="seriesrs"></a>
<a id="releasers"></a>
<a id="queuers"></a>
<a id="historyrs"></a>
<a id="workfilers"></a>
<a id="coverrs"></a>
<a id="indexerrs"></a>
<a id="download_clientrs"></a>
<a id="root_folderrs"></a>
<a id="remote_path_mappingrs"></a>
<a id="notificationrs"></a>
<a id="configrs"></a>
<a id="systemrs"></a>
<a id="authrs"></a>
<a id="userrs"></a>
<a id="profilers"></a>
<a id="setuprs"></a>
<a id="manual_importrs"></a>
<a id="list_importrs"></a>
<a id="readarr_importrs"></a>
<a id="coverproxyrs"></a>
<a id="mediacoverrs"></a>
<a id="filesystemrs"></a>
<a id="opdsrs"></a>
