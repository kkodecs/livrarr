# Server composition and crate navigation

`livrarr-server` constructs shared implementations, wires service capabilities,
owns startup/jobs/auth/router composition and serves the frontend. No production
crate depends on it. The reviewed September manifest has 13 direct production
workspace dependencies; “composition root” does not mean a dependency on every
workspace member. [Authoritative structure](../../ARCHITECTURE.md).

## Start here

| Concern | Navigation |
|---|---|
| Configuration, database startup and concrete wiring | `main.rs`, `config.rs` |
| Shared services and capability implementations | `state.rs` |
| Registered endpoints | `router.rs` |
| Authentication and proxy/rate policy | `auth_service.rs`, `middleware.rs`, `rate_limit.rs` |
| Periodic work, cancellation and startup recovery | `jobs/` |
| Import post-steps, tag service and downstream delivery | `import_service.rs`, `tag_service.rs`, related service adapters |
| Readarr connection/preview/import | `readarr_client.rs`, `readarr_import_workflow.rs` |

Use Serena to resolve symbols in the intended checkout. The parked source and
reviewed containment source differ. Old AppState tables, job lists and constructor
signatures are archived, not a live wiring inventory.

## Shared ownership

Clone the already-composed services, HTTP clients and fetcher. A new instance can
accidentally split a lock, cache or connection pool even when it implements the
right trait. Trusted administrator infrastructure and runtime-derived public URLs
use distinct transport trust classes while sharing canonical HTTP ownership.
AppState uses concrete service types/aliases and Arc sharing; avoid late-init cycles.

Startup order is load-bearing: preserve the pre-migration backup, migration/version
checks and identity readiness/installation contract before serving. A temporary
post-migration handle is not the lasting installation owner. Exact step numbering
and partial boot tests do not establish a complete boot proof.

Background jobs need cancellation-aware waits, bounded work and an explicit owner.
Per-user workflows and process-wide execution guards have different scopes. Import
utilities take explicit dependencies; coordinating services belongs in the owning
workflow, not an expanding utility module.

Deployment is separate from documentation or a successful build. Do not build or
restart from the parked dirty checkout based on historical instructions. Consult
[project state](../../build/state/) through [the handoff](../../HANDOFF.md).

## Other crate references

[Domain contracts](domain.md) · [database](db.md) · [HTTP handlers](handlers.md) ·
[architecture responsibility map](../architecture/overview.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/crates/server.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="livrarr-server"></a>
<a id="entry-point-mainrs"></a>
<a id="appstate-staters"></a>
<a id="core-infrastructure-fields"></a>
<a id="service-layer-fields-phase-4"></a>
<a id="infrastructure-accessor-fields-phase-5"></a>
<a id="type-aliases-staters"></a>
<a id="service-implementations"></a>
<a id="livesettingsservice-servicessettings_servicers"></a>
<a id="releaseserviceimpl--not-in-this-crate"></a>
<a id="manualimportserviceimpl-manual_import_servicers"></a>
<a id="livereadarrimportservice-readarr_import_servicers"></a>
<a id="liveimportservice-import_servicers"></a>
<a id="serverauthservice-auth_servicers"></a>
<a id="authcryptoservice-auth_cryptors"></a>
<a id="historyserviceimpl-history_servicers"></a>
<a id="notificationserviceimpl-notification_servicers"></a>
<a id="queueserviceimpl-queue_servicers"></a>
<a id="importioserviceimpl-import_io_servicers"></a>
<a id="livetagservice-tag_servicers"></a>
<a id="liveemailservice-email_servicers"></a>
<a id="livematchingservice-matching_servicers"></a>
<a id="livemanualimportscanservice-manual_import_scan_servicers"></a>
<a id="secondaryapiimpl-api_secondary_implrs--test-only"></a>
<a id="jobs-jobs"></a>
<a id="download_pollerrs"></a>
<a id="rss_syncrs"></a>
<a id="author_monitorrs"></a>
<a id="maintenancers"></a>
<a id="infrastructure-infra"></a>
<a id="import_pipeliners"></a>
<a id="cachers"></a>
<a id="cover_cachers"></a>
<a id="release_helpersrs"></a>
<a id="log_bufferrs"></a>
<a id="emailrs-infra"></a>
<a id="router-routerrs"></a>
<a id="config-configrs"></a>
<a id="middleware-middlewarers"></a>
<a id="rate-limiting-rate_limitrs"></a>
<a id="readarr-client-readarr_clientrs"></a>
<a id="readarr-import-workflow-readarr_import_workflowrs"></a>
