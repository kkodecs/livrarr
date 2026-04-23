# livrarr-handlers

HTTP route handlers. Generic over `AppContext`. Behind the compile wall — cannot depend on `livrarr-db`, `livrarr-metadata`, `livrarr-tagwrite`, or `livrarr-download` directly.

---

## Capability Traits (context.rs)

Each `Has*` trait exposes one service to handlers via an accessor method. `AppContext` is the composite supertrait that requires all of them.

| Trait | Exposes |
|---|---|
| `HasWorkService` | Work CRUD and search |
| `HasFileService` | Library file management |
| `HasAuthorService` | Author CRUD |
| `HasSeriesService` | Series write operations |
| `HasSeriesQueryService` | Series read operations |
| `HasGrabService` | Release grab orchestration |
| `HasReleaseService` | Release search and grab |
| `HasListService` | Import lists |
| `HasAppConfigService` | App-level config read/write |
| `HasDownloadClientSettingsService` | Download client settings |
| `HasDownloadClientCredentialService` | Download client credentials |
| `HasIndexerSettingsService` | Indexer settings |
| `HasIndexerCredentialService` | Indexer credentials |
| `HasRootFolderService` | Root folder management |
| `HasRemotePathMappingService` | Remote path mappings |
| `HasNotificationService` | Notification read/dismiss |
| `HasQueueService` | Download queue |
| `HasImportIoService` | Import I/O operations |
| `HasManualImportService` | Manual import DB service |
| `HasHistoryService` | History records |
| `HasAuthService` | Authentication (login, session) |
| `HasImportWorkflow` | Import orchestration workflow |
| `HasEnrichmentWorkflow` | Metadata enrichment workflow |
| `HasRssSyncWorkflow` | RSS sync workflow |
| `HasTagService` | EPUB/file tag writing |
| `HasEmailService` | Email sending (Kindle etc.) |
| `HasAuthorMonitorWorkflow` | Author monitoring workflow |
| `HasImportService` | High-level import service |
| `HasMatchingService` | Work/file matching |
| `HasManualImportScan` | Manual import scan state |
| `HasReadarrImportWorkflow` | Readarr import workflow |
| `HasHttpClient` | Outbound HTTP client |
| `HasDataDir` | Data directory path |
| `HasStartupTime` | Server startup timestamp |
| `HasProviderHealth` | Metadata provider health state |
| `HasLiveConfig` | Live metadata config snapshot |
| `HasRssSync` | RSS sync running/last-run state |
| `HasSystem` | System info accessor |
| `HasCoverCache` | Cover proxy cache |
| `HasEnrichmentNotify` | Enrichment wake notification |
| `AppContext` | Composite supertrait — requires all `Has*` above |

`impl AppContext for T` is a blanket impl: any type satisfying all `Has*` traits automatically implements `AppContext`.

---

## Accessor Traits (accessors.rs)

Thin accessor interfaces consumed by handlers for shared mutable state (not services).

- `ProviderHealthAccessor` — read provider health snapshots
- `LiveMetadataConfigAccessor` — read the live metadata config snapshot
- `RssSyncAccessor` — check/set RSS sync running state and last-run timestamp
- `SystemAccessor` — expose system info (uptime, hostname, etc.)
- `ManualImportScanAccessor` — access in-progress manual import scan state map
- `CoverProxyCacheAccessor` — access the cover proxy LRU cache

---

## Middleware (middleware.rs)

- `RequireAdmin` — Axum extractor that rejects non-admin requests with 403

---

## Route Handlers

### work.rs
- `lookup` — search metadata providers for a work by query string
- `add` — add a work to the library
- `list` — list all monitored works with optional filters
- `get` — get a single work by ID
- `update` — update work metadata or monitoring state
- `upload_cover` — upload a custom cover image for a work
- `delete` — delete a work and optionally its files
- `refresh` — trigger metadata refresh for a single work
- `refresh_all` — trigger metadata refresh for all works
- `send_email` — send a library file to an email address (Kindle)
- `download` — serve a library file for direct browser download
- `stream` — stream a library file for in-browser reading
- `author_search` — search for authors related to a work (internal helper used by add flow)

### author.rs
- `lookup` — search metadata providers for an author
- `add` — add an author to the library
- `list` — list all authors
- `get` — get a single author by ID
- `update` — update author metadata or monitoring state
- `delete` — delete an author
- `bibliography` — get an author's full bibliography (works from metadata provider)
- `refresh_bibliography` — refresh an author's bibliography from metadata

### series.rs
- `list_all` — list all series
- `get_detail` — get a single series with works
- `resolve_gr` — resolve a Goodreads series ID and return or create the series
- `list_series` — list series for a specific work
- `refresh_series` — refresh series metadata from provider
- `monitor_series` — update monitoring state for a series
- `update_series` — update series metadata fields

### release.rs
- `search` — search indexers for releases matching a work
- `grab` — grab a release (send to download client)

### queue.rs
- `list` — list active download queue items
- `remove` — remove an item from the queue
- `retry_import` — retry a failed import for a queued item
- `summary` — return queue summary counts by status

### history.rs
- `list` — list history records with optional filters and pagination

### workfile.rs
- `list` — list library files for a work
- `get` — get a single library file by ID
- `delete` — delete a library file from disk and DB
- `get_progress` — get read progress for a file
- `update_progress` — update read progress for a file

### indexer.rs
- `list` — list configured indexers
- `get` — get a single indexer by ID
- `create` — create a new indexer
- `update` — update an existing indexer
- `delete` — delete an indexer
- `test` — test an indexer config (by payload)
- `test_saved` — test a saved indexer by ID
- `import_from_prowlarr` — bulk-import indexers from a Prowlarr instance

### download_client.rs
- `list` — list configured download clients
- `get` — get a single download client by ID
- `create` — create a new download client
- `update` — update an existing download client
- `delete` — delete a download client
- `test` — test a download client config (by payload)
- `test_saved` — test a saved download client by ID
- `import_from_prowlarr` — bulk-import download clients from a Prowlarr instance

### root_folder.rs
- `list` — list root folders
- `create` — create a root folder
- `delete` — delete a root folder
- `scan` — trigger a root folder scan
- `scan_path` — scan a specific path within a root folder

### remote_path_mapping.rs
- `list` — list remote path mappings
- `get` — get a single remote path mapping
- `create` — create a remote path mapping
- `update` — update a remote path mapping
- `delete` — delete a remote path mapping

### notification.rs
- `list` — list notifications (with read/unread filter)
- `mark_read` — mark a notification as read
- `dismiss` — dismiss a single notification
- `dismiss_all` — dismiss all notifications

### config.rs
- `get_naming` — get naming convention config
- `get_media_management` — get media management config
- `update_media_management` — update media management config
- `get_metadata` — get metadata provider config
- `validate_llm_endpoint` — validate an LLM endpoint URL/key without saving
- `update_metadata` — update metadata provider config (and refreshes live config)
- `test_hardcover` — test the Hardcover API connection
- `test_audnexus` — test the Audnexus API connection
- `test_llm` — test the configured LLM endpoint
- `get_prowlarr` — get Prowlarr integration config
- `update_prowlarr` — update Prowlarr integration config
- `get_email` — get email (Kindle) config
- `update_email` — update email config
- `get_indexer_config` — get indexer-level config
- `update_indexer_config` — update indexer-level config
- `trigger_rss_sync` — manually trigger an RSS sync run
- `test_email` — send a test email with the current config

### system.rs
- `health` — liveness probe endpoint (returns 200)
- `status` — application status (version, startup time, DB status, etc.)
- `log_tail` — return the most recent N log lines from the in-memory buffer
- `set_log_level` — change the active log level at runtime
- `routes` — list all registered routes (debug)

### auth.rs
- `login` — authenticate a user and create a session
- `logout` — destroy the current session
- `me` — return the currently authenticated user

### user.rs
- `list` — list all users
- `get` — get a single user by ID
- `create` — create a new user
- `update` — update a user
- `delete` — delete a user
- `regenerate_user_api_key` — regenerate a user's API key

### profile.rs
- `update_profile` — update the current user's own profile
- `regenerate_api_key` — regenerate the current user's own API key

### setup.rs
- `setup_status` — return whether initial setup has been completed
- `setup` — complete initial setup (create admin user, root folder, etc.)

### manual_import.rs

Composite trait `ManualImportHandlerContext` (requires `AppContext` + manual import accessors).

- `scan` — initiate a manual import scan of a filesystem path (streams OL matches)
- `scan_progress` — poll scan progress for an in-flight scan
- `search` — search metadata providers to match a scanned file to a work
- `import` — import a batch of matched files into the library
- `import_single_item` — import a single file (internal helper)
- `find_existing_work` — find a work already in the library by metadata ID
- `find_or_create_work` — find or create a work during import
- `enumerate_with_limits` — enumerate files in a directory with depth/entry limits
- `enumerate_recursive` — recursive enumeration helper

### list_import.rs
- `preview` — preview what a list import would add/change
- `confirm` — confirm and begin a list import
- `complete` — mark a list import as complete
- `undo` — undo a completed list import
- `list` — list previous list imports

### readarr_import.rs
- `connect` — verify a Readarr instance is reachable
- `preview` — preview what a Readarr import would bring in
- `start` — start a Readarr import job
- `progress` — poll progress of an in-flight Readarr import
- `history` — list past Readarr import sessions
- `undo` — undo a Readarr import

### coverproxy.rs
- `proxy_cover` — proxy a remote cover image through the server (caches result)
- `is_allowed_cover_source` — check if a URL's domain is on the cover proxy allowlist

### mediacover.rs
- `get_cover` — serve the full-size cover image for a work or author
- `get_thumb` — serve a thumbnail cover image (generated on demand)

### filesystem.rs
- `browse` — browse the server's local filesystem (used by root folder picker)

### opds.rs

Composite trait `OpdsHandlerContext` (requires `AppContext` + OPDS-specific accessors).

OPDS 1.2 catalog endpoints:
- `root` — OPDS root navigation feed
- `recent` — recently-added works feed
- `author_list` — paginated author listing feed
- `author_works` — works by a specific author feed
- `search` — OPDS search results feed
- `opensearch` — OpenSearch description document
- `cover` — serve a cover image for OPDS clients
- `download` — serve a book file for OPDS clients (with basic auth)
