# livrarr-server

Composition root. Depends on all other crates. Nothing depends on it.

---

## Entry Point (main.rs)

- `main` — load config, init tracing, connect DB, run migrations, build `AppState`, start job runners, start Axum server
- `load_config` — read and validate `config.toml`
- `init_tracing` — configure tracing subscriber with log buffer and optional JSON format
- `validate_llm_endpoint_startup` — validate the *shape* of a configured LLM endpoint URL: rejects non-`http(s)` schemes, embedded credentials, and private IPs. Makes no network call — it does not check reachability
- `shutdown_signal` — future that resolves on `SIGTERM` or `Ctrl-C`
- `LogBufferLayer` — `tracing_subscriber::Layer` that captures recent log lines into the in-memory `LogBuffer`

---

## AppState (state.rs)

`AppState` is the single shared struct cloned into every Axum handler. It satisfies all `Has*` capability traits from `livrarr-handlers`.

### Core infrastructure fields
| Field | Type | Purpose |
|---|---|---|
| `db` | `SqliteDb` | Primary DB connection pool |
| `auth_service` | `Arc<ServerAuthService<RealAuthCrypto>>` | Session auth and lockout |
| `http_client` | `HttpClient` | General outbound HTTP |
| `http_client_safe` | `HttpClient` | SSRF-safe client (rejects private IPs) — use for user-supplied URLs |
| `http_fetcher` | `HttpFetcherImpl` | Shared fetcher — routes admin-triggered outbound requests through the process-global rate-limit queue |
| `config` | `Arc<AppConfig>` | Parsed TOML config |
| `data_dir` | `Arc<PathBuf>` | Data directory root |
| `startup_time` | `DateTime<Utc>` | Server start timestamp |
| `job_runner` | `Option<JobRunner>` | Background job handle (None in tests) |
| `log_buffer` | `Arc<LogBuffer>` | In-memory ring buffer of recent log lines |
| `log_level_handle` | `Arc<LogLevelHandle>` | Runtime log level control |
| `import_semaphore` | `Arc<Semaphore>` | Limits concurrent import I/O |
| `cover_proxy_cache` | `Arc<CoverProxyCache>` | TTL cache for proxied cover images (300s TTL, 200-entry cap, oldest-inserted evicted first — not LRU) |
| `live_metadata_config` | `LiveMetadataConfig` | Mutable snapshot of `MetadataConfig`; updated on config save, read by enrichment |
| `manual_import_scans` | `Arc<ManualImportScanMap>` | In-progress scan state keyed by scan ID |
| `readarr_import_progress` | `Arc<Mutex<ReadarrImportProgress>>` | Polled by frontend during Readarr import |
| `rss_last_run` | `Arc<AtomicI64>` | Unix timestamp of last RSS sync |
| `rss_sync_running` | `Arc<AtomicBool>` | Guard against concurrent RSS sync |
| `provider_queue` | `Arc<LiveProviderQueue>` | Provider dispatch layer — on the live enrichment path (work service / unified enrichment) |
| `enrichment_service` | `Arc<LiveEnrichmentService>` | Wraps `provider_queue` + the merge engine — drives live enrichment through the work service |

### Service layer fields (Phase 4)

All 25, in declaration order (`state.rs:148-173`).

| Field | Service |
|---|---|
| `author_service` | `Arc<LiveAuthorService>` |
| `series_service` | `Arc<LiveSeriesService>` |
| `series_query_service` | `Arc<LiveSeriesQueryService>` |
| `work_service` | `Arc<LiveWorkService>` |
| `discovery_service` | `Arc<LiveDiscoveryService>` |
| `grab_service` | `Arc<LiveGrabService>` |
| `release_service` | `Arc<LiveReleaseService>` |
| `file_service` | `Arc<LiveFileService>` |
| `chapter_service` | `Arc<LiveChapterService>` |
| `bookmark_service` | `Arc<LiveBookmarkService>` |
| `cross_format_service` | `Arc<LiveCrossFormatService>` |
| `import_workflow` | `Arc<LiveImportWorkflow>` |
| `list_service` | `Arc<LiveListService>` |
| `identity_conflict_service` | `Arc<services::identity_conflict_service::LiveIdentityConflictService>` (no `Live*` alias) |
| `identity_resolver` | `Arc<LiveIdentityResolver>` |
| `rss_sync_workflow` | `Arc<LiveRssSyncWorkflow>` |
| `author_monitor_workflow` | `Arc<LiveAuthorMonitorWorkflow>` |
| `enrichment_workflow` | `Arc<LiveEnrichmentWorkflow>` |
| `readarr_import_service` | `Arc<ReadarrImportServiceImpl>` |
| `settings_service` | `Arc<LiveSettingsService>` |
| `notification_service` | `Arc<LiveNotificationService>` |
| `history_service` | `Arc<LiveHistoryService>` |
| `queue_service` | `Arc<LiveQueueService>` |
| `import_io_service` | `Arc<LiveImportIoService>` |
| `manual_import_db_service` | `Arc<LiveManualImportDbService>` |

### Infrastructure accessor fields (Phase 5)

All 16, in declaration order (`state.rs:176-193`). Not all of them are accessors — the block
also holds plain service fields. The "Accessor trait" column is filled only where the field's
type is a wrapper implementing a `livrarr_handlers::accessors` trait; the rest are concrete
service types covered under **Service Implementations** below.

| Field | Type | Accessor trait |
|---|---|---|
| `rss_sync_state` | `RssSyncState` | `RssSyncAccessor` |
| `system_state` | `SystemState` | `SystemAccessor` |
| `provider_stats_service` | `Arc<LiveProviderStatsService>` | — (implements the domain `ProviderStatsService`) |
| `log_surface_accessor` | `LogSurfaceAccessorImpl` | `LogSurfaceAccessor` |
| `live_metadata_config_accessor` | `LiveMetadataConfigAccessorImpl` | `LiveMetadataConfigAccessor` |
| `cover_proxy_cache_accessor` | `CoverProxyCacheAccessorImpl` | `CoverProxyCacheAccessor` |
| `tag_service` | `Arc<LiveTagService<LiveImportIoService>>` | — |
| `email_svc` | `Arc<LiveEmailService<SqliteDb>>` | — |
| `import_svc` | `Arc<LiveImportService>` | — |
| `matching_svc` | `LiveMatchingService` | — |
| `manual_import_scan_svc` | `LiveManualImportScanService` | `ManualImportScanAccessor` |
| `readarr_import_wf` | `Arc<LiveReadarrImportWorkflow>` | — |
| `cover_service` | `Arc<LiveCoverService>` | — |
| `preadd_cover_service` | `Arc<livrarr_metadata::preadd_cover_service::LivePreaddCoverService>` | — |
| `hmac_key` | `Vec<u8>` | — |
| `trusted_origins_rebuilder` | `TrustedOriginsRebuilderImpl` | `TrustedOriginsRebuilder` |

### Type aliases (state.rs)
All `Live*` type aliases are defined here. Examples:
- `LiveProviderQueue` — concrete provider queue type
- `LiveEnrichmentService` — concrete enrichment service type
- `LiveEnrichmentWorkflow` — concrete enrichment workflow type
- `LiveWorkService`, `LiveAuthorService`, `LiveSeriesService`, etc. — concrete domain service types

---

## Service Implementations

### LiveSettingsService (services/settings_service.rs)
Implements seven service traits over a single `SqliteDb` generic:
- `AppConfigService` — app-level naming/media management config
- `DownloadClientSettingsService` — download client CRUD
- `DownloadClientCredentialService` — download client credential storage
- `IndexerSettingsService` — indexer CRUD
- `IndexerCredentialService` — indexer credential storage
- `RootFolderService` — root folder CRUD
- `RemotePathMappingService` — remote path mapping CRUD

### ReleaseServiceImpl — NOT in this crate
- Lives in `livrarr-download` (`crates/livrarr-download/src/release_service.rs`), not under
  `livrarr-server/src/services/`. This crate aliases it as `LiveReleaseService` in `state.rs`.
- "ReleaseService implementation — search indexers and grab releases."

### ManualImportServiceImpl (manual_import_service.rs)
- DB-backed `ManualImportService` impl. At the crate root, not under `services/`.

### LiveReadarrImportService (readarr_import_service.rs)
- `ReadarrImportService` impl backed by the DB. At the crate root, not under `services/`.

### LiveImportService (import_service.rs)
High-level import orchestrator. Fields:
- `import_io` — `Arc<LiveImportIoService>` — DB reads/writes for the import path (not file I/O; see `ImportIoServiceImpl` below)
- `import_workflow` — `Arc<LiveImportWorkflow>` — domain import workflow
- `tag_service` — `Arc<LiveTagService<LiveImportIoService>>` — tag writing
- `settings_service` — `Arc<LiveSettingsService>` — path mapping and config
- `http_client_safe` — SSRF-safe HTTP client (for cover fetching)

### ServerAuthService (auth_service.rs)
- `ServerAuthService<C>` — session auth with lockout tracking (`LockoutState`)
- Implements `AuthService`

### AuthCryptoService (auth_crypto.rs)
- `RealAuthCrypto` — production argon2 password hashing
- `TestAuthCrypto` — fast dummy hasher for tests

### HistoryServiceImpl (history_service.rs)
- Thin `HistoryService` impl over `SqliteDb`

### NotificationServiceImpl (notification_service.rs)
- Thin `NotificationService` impl over `SqliteDb`

### QueueServiceImpl (queue_service.rs)
- `QueueService` impl; also contains helpers `fetch_qbit_progress`, `fetch_sab_progress`, `parse_sab_timeleft`

### ImportIoServiceImpl (import_io_service.rs)
- `ImportIoService` impl over `SqliteDb` — DB reads/writes for the import path only. Ten
  methods covering grabs, download clients, works, library items, root folders, and remote
  path mappings. **No file I/O** — no move, copy, or hardlink lives here.

### LiveTagService (tag_service.rs)
- `TagService` impl — writes file metadata tags via `livrarr-tagwrite`. Not EPUB-specific:
  MP3 items are batched through `write_tags_batch`, everything else goes one file at a time
  through `write_tags` (copy to `.tmp` → write → fsync → rename). An `Unsupported` result
  from `livrarr-tagwrite` is treated as success, not an error.

### LiveEmailService (email_service.rs)
- `EmailService` impl — sends files via SMTP (Kindle delivery)

### LiveMatchingService (matching_service.rs)
- `MatchingService` impl — extracts metadata from file paths and reconciles it into
  `MatchCluster`s (author, title, series, series_position, language, isbn, asin, year).
  It does not look up or return works. Delegates to `livrarr-matching`, which `lib.rs`
  re-exports as `crate::matching`.

### LiveManualImportScanService (manual_import_scan_service.rs)
- `ManualImportScanAccessor` impl — wraps the shared `ManualImportScanMap`

### SecondaryApiImpl (api_secondary_impl.rs) — TEST-ONLY
- **The whole module is `#[cfg(test)]`** (declared that way in `lib.rs`), so it is not
  compiled into the server binary. Its own module doc reads "Secondary API implementations
  for testing." This is not a production API surface.
- Implements nine traits: `AuthorApi`, `NotificationApi`, `RootFolderApi`,
  `DownloadClientApi`, `RemotePathMappingApi`, `ConfigApi`, `SystemApi`, `LibraryFileApi`,
  `HistoryApi`.

---

## Jobs (jobs/)

`JobRunner` (jobs/mod.rs) — holds `JoinHandle`s for all background tasks; tracks `JobStatus` per job.

### download_poller.rs
- `download_poller_tick` — called on interval; polls all active download clients and imports completed items
- `retry_failed_imports` — retry imports that previously failed with a transient error
- `poll_qbittorrent` — fetch status from qBittorrent client
- `poll_sabnzbd` — fetch status from SABnzbd client
- `spawn_import` — spawn an import task for a completed download (respects `import_semaphore`)

### rss_sync.rs
- `rss_sync_tick` — called on interval; returns early if cancelled, if
  `rss_sync_interval_minutes` is 0, or if the interval has not elapsed. It does NOT check
  whether a sync is already running — that guard is in `rss_sync_run`, which returns
  `Err("already running")` rather than skipping
- `rss_sync_run` — full RSS sync cycle: fetch feeds, evaluate against monitored works, grab matches

### author_monitor.rs
- `author_monitor_tick` — check monitored authors for new releases and trigger grabs

### maintenance.rs
- `recover_interrupted_state` — on startup, recover any imports that were in-flight when the server stopped
- `sweep_stale_temp_files` — clean up orphaned temp files in the data directory
- `session_cleanup_tick` — expire old auth sessions
- `state_map_cleanup_tick` — evict stale entries from in-memory state maps

---

## Infrastructure (infra/)

### import_pipeline.rs
Free helper functions for the import pipeline — no DB or service-layer access; some DO make network calls (`fetch_qbit_content_path`, `fetch_sabnzbd_storage_path`) via an explicitly-passed `HttpClient`:
- `build_target_path` — compute the destination path for an imported file given naming config
- `fetch_qbit_content_path` — call qBittorrent's `/api/v2/torrents/info` for a torrent hash and return the `content_path` field it reports. No path is computed locally
- `fetch_sabnzbd_storage_path` — resolve the final storage path from a SABnzbd job
- `apply_remote_path_mapping` — translate a remote path to a local path via configured mappings
- `cwa_copy` — CWA-style copy (hardlink-first, fall back to copy) for import

### cache.rs
- `ManualImportScanState` — per-scan state for in-flight manual import scans
- `cleanup_manual_import_scans` — evict completed/stale scan entries
- (Release search caching lives in `livrarr-download`'s `ReleaseSearchCache` now — see the indexer-citizenship unit; the old server-side `GrabSearchCache` was dead code and was deleted.)

### cover_cache.rs
- `CoverProxyCache` — TTL cache for proxied cover images (avoids re-fetching remote URLs). 300s TTL, 200-entry cap. **Not LRU:** reads never update the entry's timestamp, so eviction removes the oldest *inserted* entry, not the least recently used

### release_helpers.rs
Two functions, despite the name — nothing here touches indexers or Torznab.
- `qbit_base_url` — build a download client's base URL from its host / port / SSL flag / url_base
- `qbit_login` — POST to `{base}/api/v2/auth/login` and return the session cookie (`SID` / `QBT_SID*`)

Callers: `fetch_qbit_content_path` (`infra/import_pipeline.rs`), and `poll_qbittorrent` +
`poll_transmission` (base URL only) in `jobs/download_poller.rs`.

### log_buffer.rs
- `LogBuffer` — fixed-size ring buffer of recent log lines, fed by `LogBufferLayer`
- `LogLevelHandle` — handle for changing the active log level at runtime

### email.rs (infra)
Low-level SMTP helpers:
- `build_transport` — construct an SMTP transport from email config
- `validate_config` — validate email config before saving
- `send_test` — send a test email
- `send_file` — send a book file as an email attachment

---

## Router (router.rs)
- `build_router` — construct the full Axum router with all route groups, middleware, and static file serving

## Config (config.rs)
- `AppConfig` — top-level config struct. Five sections: `server` (`ServerConfig`), `auth` (`AuthConfig`), `log` (`LogConfig`), `convergence` (`ConvergenceConfig`), `metadata_cache` (`MetadataCacheConfig`)
- `validate_config` — validate parsed config and surface human-readable errors

## Middleware (middleware.rs)
- `auth_middleware` — Axum middleware layer: validates session or API key on every request
- `extract_bearer` — extract a bearer token from the `Authorization` header
- `RequireAdmin` — extractor that enforces admin role on specific routes

## Rate Limiting (rate_limit.rs)
- `SmartIpKeyExtractor` — rate limit key extractor that handles `X-Forwarded-For` via trusted proxy CIDR list

## Readarr Client (readarr_client.rs)
- `ReadarrClient` — HTTP client for the Readarr v1 API
- `RdBook`, `RdAuthor`, `RdEdition`, `RdBookFile`, etc. — deserialization structs for Readarr API responses
- `quality_to_media_type` / `media_type_from_extension` — convert Readarr quality fields to internal `MediaType`

## Readarr Import Workflow (readarr_import_workflow.rs)
- `LiveReadarrImportWorkflow` — implements `ReadarrImportWorkflow`; orchestrates multi-step Readarr-to-Livrarr import
- `ImportPlanner` — builds an import plan from fetched Readarr data
- `ImportRunner` — executes the plan (file moves, DB writes, tag writing)
- `fetch_all_readarr_data` — fetch root folders, then authors, then books (sequentially), then per-author book files concurrently (`buffer_unordered(10)`). No editions are fetched — `ReadarrData` carries authors, books, book_files, root_folders
