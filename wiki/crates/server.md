# livrarr-server

Composition root. Depends on all other crates. Nothing depends on it.

---

## Entry Point (main.rs)

- `main` — load config, init tracing, connect DB, run migrations, build `AppState`, start job runners, start Axum server
- `load_config` — read and validate `config.toml`, merge CLI overrides
- `init_tracing` — configure tracing subscriber with log buffer and optional JSON format
- `validate_llm_endpoint_startup` — warn at startup if the configured LLM endpoint is unreachable
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
| `config` | `Arc<AppConfig>` | Parsed TOML config |
| `data_dir` | `Arc<PathBuf>` | Data directory root |
| `startup_time` | `DateTime<Utc>` | Server start timestamp |
| `job_runner` | `Option<JobRunner>` | Background job handle (None in tests) |
| `log_buffer` | `Arc<LogBuffer>` | In-memory ring buffer of recent log lines |
| `log_level_handle` | `Arc<LogLevelHandle>` | Runtime log level control |
| `import_semaphore` | `Arc<Semaphore>` | Limits concurrent import I/O |
| `grab_search_cache` | `Arc<GrabSearchCache>` | TTL cache for release search results |
| `provider_health` | `Arc<ProviderHealthState>` | Metadata provider health snapshots |
| `cover_proxy_cache` | `Arc<CoverProxyCache>` | LRU cache for proxied cover images |
| `live_metadata_config` | `LiveMetadataConfig` | Mutable snapshot of `MetadataConfig`; updated on config save, read by enrichment |
| `goodreads_rate_limiter` | `Arc<GoodreadsRateLimiter>` | Token bucket for Goodreads requests |
| `ol_rate_limiter` | `Arc<OlRateLimiter>` | Token bucket for OpenLibrary requests |
| `manual_import_scans` | `Arc<ManualImportScanMap>` | In-progress scan state keyed by scan ID |
| `readarr_import_progress` | `Arc<Mutex<ReadarrImportProgress>>` | Polled by frontend during Readarr import |
| `refresh_in_progress` | `Arc<Mutex<HashSet<UserId>>>` | Prevents concurrent refreshes for the same user |
| `rss_last_run` | `Arc<AtomicI64>` | Unix timestamp of last RSS sync |
| `rss_sync_running` | `Arc<AtomicBool>` | Guard against concurrent RSS sync |
| `enrichment_notify` | `Arc<Notify>` | Wakes the enrichment job when new work is queued |
| `provider_queue` | `Arc<LiveProviderQueue>` | Phase 1.5 plumbing: provider queue (not yet on live enrichment path) |
| `enrichment_service` | `Arc<LiveEnrichmentService>` | Phase 1.5 plumbing: enrichment service (not yet on live enrichment path) |

### Service layer fields (Phase 4)
| Field | Service |
|---|---|
| `author_service` | `Arc<LiveAuthorService>` |
| `series_service` | `Arc<LiveSeriesService>` |
| `series_query_service` | `Arc<LiveSeriesQueryService>` |
| `work_service` | `Arc<LiveWorkService>` |
| `grab_service` | `Arc<LiveGrabService>` |
| `release_service` | `Arc<LiveReleaseService>` |
| `file_service` | `Arc<LiveFileService>` |
| `import_workflow` | `Arc<LiveImportWorkflow>` |
| `list_service` | `Arc<LiveListService>` |
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
| Field | Accessor trait implemented |
|---|---|
| `rss_sync_state` | `RssSyncAccessor` |
| `system_state` | `SystemAccessor` |
| `provider_health_accessor` | `ProviderHealthAccessor` |
| `live_metadata_config_accessor` | `LiveMetadataConfigAccessor` |
| `cover_proxy_cache_accessor` | `CoverProxyCacheAccessor` |
| `tag_service` | `Arc<LiveTagService<LiveImportIoService>>` |
| `email_svc` | `Arc<LiveEmailService<SqliteDb>>` |
| `import_svc` | `Arc<LiveImportService>` |
| `matching_svc` | `LiveMatchingService` |
| `manual_import_scan_svc` | `ManualImportScanAccessor` |
| `readarr_import_wf` | `Arc<LiveReadarrImportWorkflow>` |

### Type aliases (state.rs)
All `Live*` type aliases are defined here. Examples:
- `LiveProviderQueue` — concrete provider queue type
- `LiveEnrichmentService` — concrete enrichment service type
- `LiveEnrichmentWorkflow` — concrete enrichment workflow type
- `LiveWorkService`, `LiveAuthorService`, `LiveSeriesService`, etc. — concrete domain service types

---

## Service Implementations

### LiveSettingsService (services/settings_service.rs)
Implements eight service traits over a single `SqliteDb` generic:
- `AppConfigService` — app-level naming/media management config
- `DownloadClientSettingsService` — download client CRUD
- `DownloadClientCredentialService` — download client credential storage
- `IndexerSettingsService` — indexer CRUD
- `IndexerCredentialService` — indexer credential storage
- `RootFolderService` — root folder CRUD
- `RemotePathMappingService` — remote path mapping CRUD

### ReleaseService (services/release_service.rs)
- Handles release search (via indexers) and grab (via download clients)

### ManualImportService (services/manual_import_service.rs)
- DB-backed service for persisting manual import state (grabbed files, import records)

### ReadarrImportService (services/readarr_import_service.rs)
- DB-backed service for Readarr import session state

### LiveImportService (import_service.rs)
High-level import orchestrator. Fields:
- `import_io` — `Arc<LiveImportIoService>` — low-level file I/O
- `import_workflow` — `Arc<LiveImportWorkflow>` — domain import workflow
- `tag_service` — `Arc<LiveTagService<LiveImportIoService>>` — tag writing
- `settings_service` — `Arc<LiveSettingsService>` — path mapping and config
- `http_client_safe` — SSRF-safe HTTP client (for cover fetching)
- `data_dir` — `Arc<PathBuf>` — data directory

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
- `ImportIoService` impl over `SqliteDb` — file move/copy/hardlink and DB record management

### LiveTagService (tag_service.rs)
- `TagService` impl — writes EPUB metadata tags using `livrarr-tagwrite`

### LiveEmailService (email_service.rs)
- `EmailService` impl — sends files via SMTP (Kindle delivery)

### LiveMatchingService (matching_service.rs)
- `MatchingService` impl — matches grabbed files to library works

### LiveManualImportScanService (manual_import_scan_service.rs)
- `ManualImportScanAccessor` impl — wraps the shared `ManualImportScanMap`

### SecondaryApiImpl (api_secondary_impl.rs)
- Implements multiple secondary API traits (`AuthorApi`, `NotificationApi`, `RootFolderApi`, `DownloadClientApi`, `RemotePathMappingApi`, `ConfigApi`, `SystemApi`, `LibraryFileApi`, `HistoryApi`) — used as the concrete impl for the Readarr-compatible secondary API surface

---

## Jobs (jobs/)

`JobRunner` (jobs/mod.rs) — holds `JoinHandle`s for all background tasks; tracks `JobStatus` per job.

### download_poller.rs
- `download_poller_tick` — called on interval; polls all active download clients and imports completed items
- `retry_failed_imports` — retry imports that previously failed with a transient error
- `poll_qbittorrent` — fetch status from qBittorrent client
- `poll_sabnzbd` — fetch status from SABnzbd client
- `spawn_import` — spawn an import task for a completed download (respects `import_semaphore`)

### enrichment.rs
- `enrichment_retry_tick` — retry enrichment for works stuck in a failed/pending state

### rss_sync.rs
- `rss_sync_tick` — called on interval; skips if already running
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
Pure helper functions for the import pipeline (no DB or network calls):
- `build_target_path` — compute the destination path for an imported file given naming config
- `fetch_qbit_content_path` — resolve the actual content path from qBittorrent save dir + content layout
- `fetch_sabnzbd_storage_path` — resolve the final storage path from a SABnzbd job
- `apply_remote_path_mapping` — translate a remote path to a local path via configured mappings
- `cwa_copy` — CWA-style copy (hardlink-first, fall back to copy) for import
- `build_tag_metadata` — construct `TagMetadata` from a `Work` and `Author` for tag writing
- `read_cover_bytes` — read cover image bytes from disk for embedding in tags

### cache.rs
- `GrabSearchCache` — TTL-based in-memory cache for release search results (keyed by work + query)
- `ManualImportScanState` — per-scan state for in-flight manual import scans
- `cleanup_manual_import_scans` — evict completed/stale scan entries

### cover_cache.rs
- `CoverProxyCache` — LRU + TTL cache for proxied cover images (avoids re-fetching remote URLs)

### release_helpers.rs
- `search_indexer` — send a Torznab search request to a single indexer
- `build_torznab_url` — construct a Torznab search URL with query parameters
- `clean_search_term` — normalize a search string for indexer queries
- `fetch_and_parse` — fetch and parse a Torznab XML response
- `qbit_base_url` / `qbit_login` — qBittorrent connection helpers used by download client test

### rate_limiter.rs
- `OlRateLimiter` — token bucket rate limiter for OpenLibrary API (3 req/s, burst 10)
- `GoodreadsRateLimiter` — token bucket rate limiter for Goodreads (configurable)

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
- `AppConfig` — top-level config struct (contains `ServerConfig`, `AuthConfig`, `LogConfig`)
- `validate_config` — validate parsed config and surface human-readable errors

## Middleware (middleware.rs)
- `auth_middleware` — Axum middleware layer: validates session or API key on every request
- `extract_bearer` — extract a bearer token from the `Authorization` header
- `RequireAdmin` — extractor that enforces admin role on specific routes

## Rate Limiting (rate_limit.rs)
- `SmartIpKeyExtractor` — rate limit key extractor that handles `X-Forwarded-For` via trusted proxy CIDR list

## Disk (disk.rs)
- `disk_space` — return available and total bytes for a filesystem path (used by root folder and system status)

## Readarr Client (readarr_client.rs)
- `ReadarrClient` — HTTP client for the Readarr v1 API
- `RdBook`, `RdAuthor`, `RdEdition`, `RdBookFile`, etc. — deserialization structs for Readarr API responses
- `quality_to_media_type` / `media_type_from_extension` — convert Readarr quality fields to internal `MediaType`

## Readarr Import Workflow (readarr_import_workflow.rs)
- `LiveReadarrImportWorkflow` — implements `ReadarrImportWorkflow`; orchestrates multi-step Readarr-to-Livrarr import
- `ImportPlanner` — builds an import plan from fetched Readarr data
- `ImportRunner` — executes the plan (file moves, DB writes, tag writing)
- `fetch_all_readarr_data` — fetch authors, books, editions, and files from Readarr in parallel
