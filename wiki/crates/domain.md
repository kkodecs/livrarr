# Domain contracts and rules

`livrarr-domain` is the shared foundation: entities, typed vocabularies, service and
repository contracts, deterministic comparison and normalization. It has no
dependency on another workspace crate. External libraries are listed in its Cargo
manifest. [Root boundaries](../../ARCHITECTURE.md).

## Start here

| Concern | Navigation |
|---|---|
| Product records | [Entity guide](../domain/big7.md); `entities.rs` |
| Work identity and generation contracts | `identity_layer/`, `identity_matching.rs`; [identity workflow](../architecture/work-creation-pipeline.md) |
| Provider outcomes, field provenance and merge values | `enrichment_types.rs`; [enrichment](../architecture/enrichment-pipeline.md) |
| Service use cases | `services/`; [service pattern](../patterns/async-service.md) |
| File/path normalization and metadata extraction vocabulary | `util.rs`, `normalization/`; [import](../architecture/import-pipeline.md) |
| Playback and cross-format alignment | `kash.rs`; [resume](../domain/cross-format-resume.md) |
| Locks and bounded shared state | `keyed_mutex.rs`; [coding lessons](../insights/coding-patterns.md) |

Use Serena to locate the current symbol and references before editing. Earlier
inventories confused type aliases with newtypes, signatures with user-scoping
guarantees and enum variants with produced outcomes. Verify the actual contract.
Some generic repository interfaces historically reside in db's api modules; the
root dependency rules remain the intended boundary, not proof every contract moved.

## Decisions to preserve

Work is independent of media type; LibraryItem and Grab carry concrete format
information. Enrichment quality, current identity, download state and tag state are
different vocabularies. User, automated and imported provenance are not equivalent.
Edition identifiers do not become Work identity simply because both are strings.

The identity comparator is shared by provider selection, reconciliation and stored
normalization where their semantics require it. Do not introduce an independent
same-book scorer in a caller. Side-effect orchestration, HTTP implementations and
SQL stay outside this crate.

Async service traits use trait_variant and cannot use dyn dispatch; intentionally
synchronous contracts can. Broad AppContext is not a reason to expose every
service to every handler. Keyed locks need explicit ownership, capacity and release
behavior; an ordinary raw guard is not the unfinished checked-invocation design.

Detailed signatures and exhaustive type/method lists have been retired from this
page. Source is the exact API reference; the linked workflow pages explain why.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/crates/domain.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="livrarr-domain"></a>
<a id="entities-entitiesrs-enrichment_typesrs-infra_configrs"></a>
<a id="id-type-aliases"></a>
<a id="core-entity-structs"></a>
<a id="core-enums"></a>
<a id="utility-functions-utilrs"></a>
<a id="settings-settingsrs"></a>
<a id="config-structs"></a>
<a id="param-structs-service-input-types"></a>
<a id="readarr-import-types-readarrrs"></a>
<a id="torznab-torznabrs"></a>
<a id="keyed-mutex-keyed_mutexrs"></a>
<a id="service-traits-services"></a>
<a id="workservice-servicesworkrs"></a>
<a id="discoveryservice-servicesdiscoveryrs"></a>
<a id="authorservice-servicesauthorrs"></a>
<a id="seriesservice-servicesseriesrs"></a>
<a id="seriesqueryservice-servicesseriesrs"></a>
<a id="grabservice-servicesgrabrs"></a>
<a id="releaseservice-servicesreleasers"></a>
<a id="queueservice-servicesqueuers"></a>
<a id="importworkflow-servicesimportrs"></a>
<a id="bibliographytrigger-servicesimportrs"></a>
<a id="importservice-servicesimport_servicers"></a>
<a id="tagservice-servicesimport_servicers"></a>
<a id="coverioservice-servicesimport_servicers"></a>
<a id="enrichmentworkflow-servicesenrichmentrs"></a>
<a id="authormonitorworkflow-servicesmonitorrs"></a>
<a id="rsssyncworkflow-servicesrssrs"></a>
<a id="readarrimportworkflow-servicesreadarrrs"></a>
<a id="listservice-serviceslistrs"></a>
<a id="fileservice-servicesfilers"></a>
<a id="notificationservice-servicesnotificationrs"></a>
<a id="historyservice-serviceshistoryrs"></a>
<a id="rootfolderservice-servicesroot_folderrs"></a>
<a id="downloadclientsettingsservice-servicesdownload_client_settingsrs"></a>
<a id="downloadclientcredentialservice-servicesdownload_client_credentialsrs"></a>
<a id="indexersettingsservice-servicesindexer_settingsrs"></a>
<a id="indexercredentialservice-servicesindexer_credentialsrs"></a>
<a id="appconfigservice-servicesapp_configrs"></a>
<a id="emailservice-servicesemailrs"></a>
<a id="remotepathmappingservice-servicesremote_path_mappingrs"></a>
<a id="manualimportservice-servicesmanual_importrs"></a>
<a id="matchingservice-servicesmatchingrs"></a>
<a id="importioservice-servicesimport_iors"></a>
<a id="httpfetcher-serviceshttprs"></a>
<a id="llmcaller-servicesllmrs"></a>
<a id="common-error-servicescommonrs"></a>
