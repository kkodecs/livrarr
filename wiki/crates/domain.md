# livrarr-domain

Foundation crate. All dependency arrows point here. Defines entities, enums, errors, and service/workflow traits consumed by all other crates.

---

## Entities (lib.rs)

### Newtype IDs
- `UserId` — typed ID for users
- `WorkId` — typed ID for works (books)
- `AuthorId` — typed ID for authors
- `LibraryItemId` — typed ID for physical files in the library
- `RootFolderId` — typed ID for root folder paths
- `GrabId` — typed ID for grab records
- `DownloadClientId` — typed ID for download clients
- `RemotePathMappingId` — typed ID for remote path mappings
- `HistoryId` — typed ID for history events
- `NotificationId` — typed ID for notifications
- `ExternalIdRowId` — typed ID for external ID rows
- `IndexerId` — typed ID for indexers

### Core Entity Structs
- `User` — account record; fields: id, username, password_hash, role, api_key_hash, setup_pending, timestamps
- `Session` — auth session; fields: token_hash, user_id, persistent, created_at, expires_at
- `Work` — a book/audiobook entry; fields: id, user_id, title variants, author, series, metadata keys (ol/hc/gr/isbn/asin), enrichment state, monitor flags, cover, timestamps
- `Author` — an author record; fields: id, user_id, name, sort_name, provider keys, monitor settings, added_at
- `Series` — a book series; fields: id, user_id, author_id, name, gr_key, monitor flags, work_count, added_at
- `LibraryItem` — a file on disk linked to a Work; fields: id, user_id, work_id, root_folder_id, path, media_type, file_size, import_id, imported_at
- `PlaybackProgress` — audiobook playback position for a user/item pair
- `RootFolder` — a watched library root path and its media type
- `DownloadClient` — a configured torrent/usenet client (qBit, SAB, etc.)
- `Grab` — a grab record linking a Work to a download; tracks status, download_id, content_path, retry state
- `RemotePathMapping` — maps a seedbox remote path to a local path
- `HistoryEvent` — an event log entry (grab, import, delete, etc.)
- `HistoryFilter` — filter params for history queries
- `Notification` — in-app notification for a user; tracks read/dismissed state
- `ExternalId` — a provider-specific ID (e.g. Goodreads) linked to a Work
- `Indexer` — a Torznab/Newznab indexer config; URL, api_key, category/search flags, RSS state
- `IndexerRssState` — per-indexer RSS cursor (last_publish_date, last_guid)
- `IndexerConfig` — global indexer settings (rss_sync_interval, rss_match_threshold)
- `Import` — a Readarr import job record; tracks progress counts and status
- `FieldProvenance` — which provider/setter last wrote a given Work field
- `MergeResolved<T>` — wrapper indicating a merge conflict has been resolved
- `QueueProgress` — download progress snapshot (percent, eta, download_status)
- `QueueSummary` — aggregate queue counts (total, downloading, importing)

### Core Enums
- `MediaType` — Ebook / Audiobook
- `UserRole` — Admin / User
- `GrabStatus` — grab lifecycle state (queued, downloading, importing, imported, failed, etc.)
- `EnrichmentStatus` — metadata enrichment state (pending, enriched, skipped, failed, etc.)
- `EventType` — history event kinds
- `NotificationType` — notification category
- `NarrationType` — narration style (unabridged, abridged, etc.)
- `AuthType` — authentication type
- `QueueStatus` — download client queue entry state
- `DownloadClientImplementation` — concrete client type (qBittorrent, SABnzbd, etc.); provides `client_type()` and `protocol()`
- `LlmRole` — LLM message role
- `LlmProvider` — supported LLM backends
- `HealthCheckType` — health check category
- `DbError` — database-layer error variants
- `SourceKind` — metadata source kind; provides `is_foreign()`, `Display`, `FromStr`
- `MetadataProvider` — named metadata providers (OpenLibrary, Hardcover, etc.)
- `WorkField` — all enrichable Work fields; provides `normalization_class()`
- `ProvenanceSetter` — who/what set a provenance record (user, provider, import)
- `RequestPriority` — HTTP request priority hint
- `NormalizationClass` — text normalization category for matching
- `OutcomeClass` — enrichment outcome class; provides `is_phase2_terminal()`, `can_merge()`, `all_can_merge()`
- `WillRetryReason` — why a retry was scheduled
- `PermanentFailureReason` — why an enrichment permanently failed
- `ApplyMergeOutcome` — result of applying a merge
- `ExternalIdType` — provider-specific external ID type

### Utility Functions (lib.rs)
- `sanitize_path_component(s)` — strips illegal characters for use in file paths
- `derive_sort_name(name)` — derives a sort-friendly name from an author/title string
- `normalize_for_matching(s)` — normalizes text for fuzzy matching comparison
- `normalize_language(s)` — normalizes a language string to a canonical code
- `normalize_language_opt(s)` — same as above, returns Option
- `classify_file(path)` — determines MediaType from file extension
- `is_foreign_source(source)` — returns true if SourceKind is foreign language

---

## Settings (settings.rs)

### Config Structs
- `NamingConfig` — file/folder naming format strings and rename flags
- `MediaManagementConfig` — CWA ingest path, preferred ebook/audiobook formats
- `ProwlarrConfig` — Prowlarr URL, api_key, enabled flag
- `MetadataConfig` — Hardcover, LLM, and Audnexus provider settings; language list
- `EmailConfig` — SMTP connection and delivery settings

### Param Structs (service input types)
- `UpdateMediaManagementParams` — input for updating media management settings
- `UpdateMetadataParams` — input for updating metadata provider settings
- `UpdateProwlarrParams` — input for updating Prowlarr config
- `UpdateEmailParams` — input for updating email config
- `UpdateIndexerConfigParams` — input for updating RSS/indexer global config
- `CreateDownloadClientParams` — input for creating a download client
- `UpdateDownloadClientParams` — input for updating a download client
- `CreateIndexerParams` — input for adding a new indexer
- `UpdateIndexerParams` — input for editing an indexer

---

## Readarr Import Types (readarr.rs)

- `ReadarrConnectRequest` — URL + api_key for connecting to a Readarr instance
- `ReadarrImportRequest` — full import job parameters (root folders, path mappings)
- `ReadarrConnectResponse` / `ReadarrRootFolderInfo` — connect response with root folder list
- `ReadarrPreviewResponse` — dry-run import preview (counts + file list)
- `ReadarrPreviewFileItem` / `ReadarrSkippedItem` — individual file entries in preview
- `ReadarrStartResponse` — import job ID returned on start
- `ReadarrImportProgress` — live progress for a running import job
- `ReadarrHistoryResponse` / `ReadarrImportRecord` — history of past imports
- `ReadarrUndoResponse` — counts after undoing an import

---

## Torznab (torznab.rs)

- `TorznabItem` — a single parsed release result from a Torznab feed
- `TorznabParseResult` — enum for parse outcomes (items, error response, empty)
- `parse_torznab_xml(xml)` — parses a Torznab XML response into `TorznabParseResult`

---

## Keyed Mutex (keyed_mutex.rs)

- `KeyedMutex<K>` — per-key async mutex map; prevents concurrent work on the same key
- `KeyedMutexGuard` — RAII guard returned by `lock()`; holds the per-key lock
- `KeyedMutex::lock(key)` — acquires or creates a per-key lock
- `KeyedMutex::sweep()` — removes entries for keys that are no longer locked

---

## Service Traits (services/)

### WorkService (services/work.rs)
Manages Work CRUD, metadata lookup, cover images, and bulk refresh.

- `add(user_id, req)` — creates a Work (and Author if needed), triggers enrichment
- `get(user_id, id)` — fetches a Work by ID
- `get_detail(user_id, id)` — fetches Work with its LibraryItems
- `list(user_id, filter)` — lists Works with optional filtering/sorting
- `list_paginated(user_id, filter, page, per_page)` — paginated Work list
- `update(user_id, id, req)` — updates user-editable Work fields
- `delete(user_id, id)` — deletes a Work and its library items
- `refresh(user_id, id)` — re-enriches a single Work from metadata providers
- `refresh_all(user_id)` — kicks off a background bulk re-enrichment pass
- `upload_cover(user_id, id, bytes)` — replaces the cover with a user-uploaded image
- `download_cover(user_id, id)` — returns cover image bytes
- `download_cover_from_url(user_id, id, url)` — fetches and stores a cover from a URL
- `lookup(user_id, req)` — searches metadata providers for works by title/author
- `lookup_filtered(user_id, req)` — same but applies library-dedup and language filters
- `search_works(user_id, term)` — full-text search across library works
- `try_start_bulk_refresh(user_id)` — starts a bulk refresh if none is running; returns handle
- `finish_bulk_refresh(user_id, handle)` — processes one batch of the bulk refresh pass

### AuthorService (services/author.rs)
Manages Author CRUD, lookup, and bibliography.

- `add(user_id, req)` — creates or finds an Author
- `get(user_id, id)` — fetches an Author by ID
- `list(user_id)` — lists all Authors for a user
- `update(user_id, id, req)` — updates author metadata and monitor settings
- `delete(user_id, id)` — deletes an Author and cascades to their Works
- `lookup(user_id, name)` — searches metadata providers by author name
- `search(user_id, term)` — full-text search within library authors
- `bibliography(user_id, author_id, filter)` — returns bibliography entries (cached or fresh)
- `refresh_bibliography(user_id, author_id)` — forces a fresh bibliography fetch
- `spawn_bibliography_refresh(user_id, author_id)` — spawns a background bibliography refresh
- `lookup_authors(user_id, req)` — multi-provider author candidate search

### SeriesService (services/series.rs)
Manages series CRUD and monitoring.

- `list(user_id)` — lists all series for a user
- `get(user_id, id)` — fetches a series by ID
- `refresh(user_id, id)` — re-fetches series metadata from Goodreads
- `monitor(user_id, req)` — sets monitor flags and triggers work creation for new entries
- `update(user_id, id, req)` — updates series metadata fields

### SeriesQueryService (services/series.rs)
Read-heavy series views and GR candidate resolution.

- `list_enriched(user_id)` — lists series with library-membership counts
- `get_detail(user_id, id)` — fetches series with full Work/LibraryItem list
- `update_flags(user_id, id, req)` — updates monitor flags only
- `resolve_gr_candidates(user_id, author_id)` — fetches Goodreads author candidates for linking
- `list_author_series(user_id, author_id)` — lists all series for an author
- `refresh_author_series(user_id, author_id)` — refreshes series list from Goodreads for an author
- `monitor_series(user_id, req)` — starts monitoring a series by GR key
- `run_series_monitor_worker(params)` — background worker that adds missing series Works

### GrabService (services/grab.rs)
Read/remove operations over active download grabs.

- `list(user_id, filter)` — lists grabs with optional status filter and pagination
- `get(user_id, id)` — fetches a single grab with live download progress
- `remove(user_id, id)` — cancels and removes a grab from the download client

### ReleaseService (services/release.rs)
Searches indexers for releases and sends grabs to download clients.

- `search(user_id, req)` — searches all enabled indexers for releases matching a Work
- `grab(user_id, req)` — sends a release to the configured download client

### QueueService (services/queue.rs)
Manages the download queue polling loop.

- `list_grabs_paginated(user_id, filter)` — paginated grab list for UI queue view
- `list_download_clients(user_id)` — lists active download clients for polling
- `try_set_importing(grab_id)` — atomically marks a grab as importing
- `update_grab_status(grab_id, status)` — updates grab status after poll
- `fetch_download_progress(client, download_id)` — polls a download client for progress
- `summary(user_id)` — returns queue aggregate counts

### ImportWorkflow (services/import.rs)
Orchestrates the full import pipeline for a completed grab.

- `import_grab(grab_id)` — runs the complete import workflow for a finished download
- `retry_import(grab_id)` — retries a previously failed import
- `confirm_scan(user_id, confirmations)` — finalizes a manual scan-based import

### BibliographyTrigger (services/import.rs)
- `trigger(user_id, author_id)` — fires a bibliography refresh after import

### ImportService (services/import_service.rs)
Low-level file import operations.

- `import_grab(req)` — copies/links files into the library and creates LibraryItem records
- `import_single_file(req)` — imports one specific file into the library
- `build_target_path(req)` — computes the target path for a file under a root folder

### TagService (services/import_service.rs)
- `retag_library_items(items)` — writes metadata tags to library files

### CoverIoService (services/import_service.rs)
- `read_cover_bytes(path)` — reads cover image bytes from a file path

### EnrichmentWorkflow (services/enrichment.rs)
Runs the metadata enrichment pipeline for a Work.

- `enrich_work(user_id, work_id, mode)` — fetches and merges metadata from providers
- `reset_for_manual_refresh(user_id, work_id)` — clears enrichment state for a re-run

### AuthorMonitorWorkflow (services/monitor.rs)
Checks monitored authors for new works.

- `run_monitor()` — scans all monitored authors and adds new Works found in bibliography
- `trigger_monitor()` — enqueues a background monitor pass

### RssSyncWorkflow (services/rss.rs)
Polls RSS feeds and auto-grabs matching releases.

- `run_sync()` — checks all enabled RSS indexers, matches items, grabs if threshold met

### ReadarrImportWorkflow (services/readarr.rs)
Handles the full Readarr library migration flow.

- `connect(req)` — validates Readarr API connection and returns root folder list
- `preview(req)` — dry-runs an import and returns what would be created/skipped
- `start(req)` — launches a Readarr import job, returns import_id
- `progress(import_id)` — returns live progress for a running import
- `history(user_id)` — lists past Readarr imports
- `undo(import_id)` — rolls back a completed Readarr import

### ListService (services/list.rs)
Imports book lists (CSV/ISBN) into the library.

- `preview(req)` — parses and previews a list import without committing
- `confirm(user_id, preview_id)` — commits a previewed list import
- `complete(user_id, import_id)` — finalizes a list import job
- `undo(user_id, import_id)` — removes works added by a list import
- `list_imports(user_id)` — lists past list import summaries

### FileService (services/file.rs)
Library file read/management operations.

- `list(user_id)` — lists all LibraryItems
- `list_paginated(user_id, page, per_page)` — paginated LibraryItem list
- `get(user_id, id)` — fetches a single LibraryItem
- `delete(user_id, id)` — deletes a LibraryItem from disk and DB
- `resolve_path(item)` — resolves the absolute path for a LibraryItem
- `prepare_email(user_id, id)` — reads file bytes for email attachment delivery
- `get_progress(user_id, item_id)` — fetches playback progress for an audiobook
- `update_progress(user_id, item_id, position)` — updates playback progress

### NotificationService (services/notification.rs)
In-app notification management.

- `list_paginated(user_id, page, per_page)` — paginated notification list
- `mark_read(user_id, id)` — marks a notification read
- `dismiss(user_id, id)` — dismisses a single notification
- `dismiss_all(user_id)` — dismisses all notifications for a user
- `create(req)` — creates a new notification

### HistoryService (services/history.rs)
Event history read operations.

- `list_paginated(user_id, filter, page, per_page)` — paginated filtered history

### RootFolderService (services/root_folder.rs)
Root folder CRUD.

- `get_root_folder(user_id, id)` — fetches a root folder
- `list_root_folders(user_id)` — lists all root folders
- `create_root_folder(user_id, path, media_type)` — adds a new root folder
- `delete_root_folder(user_id, id)` — removes a root folder

### DownloadClientSettingsService (services/download_client_settings.rs)
Download client configuration CRUD.

- `get_download_client(user_id, id)` — fetches a client record
- `list_download_clients(user_id)` — lists all clients
- `create_download_client(user_id, req)` — adds a new client
- `update_download_client(user_id, id, req)` — edits a client
- `delete_download_client(user_id, id)` — removes a client

### DownloadClientCredentialService (services/download_client_credentials.rs)
Credential-bearing client access.

- `get_download_client_with_credentials(user_id, id)` — fetches client including decrypted credentials

### IndexerSettingsService (services/indexer_settings.rs)
Indexer configuration CRUD and Prowlarr/RSS config.

- `get_indexer(user_id, id)` — fetches an indexer
- `list_indexers(user_id)` — lists all indexers
- `create_indexer(user_id, req)` — adds a new indexer
- `update_indexer(user_id, id, req)` — edits an indexer
- `delete_indexer(user_id, id)` — removes an indexer
- `set_supports_book_search(id, flag)` — updates the book-search capability flag
- `get_prowlarr_config(user_id)` — fetches Prowlarr integration config
- `update_prowlarr_config(user_id, req)` — updates Prowlarr config
- `get_indexer_config(user_id)` — fetches global indexer (RSS) config
- `update_indexer_config(user_id, req)` — updates global indexer config

### IndexerCredentialService (services/indexer_credentials.rs)
Credential-bearing indexer access.

- `get_indexer_with_credentials(user_id, id)` — fetches indexer including decrypted API key

### AppConfigService (services/app_config.rs)
Application-wide configuration reads and updates.

- `get_naming_config(user_id)` — fetches file/folder naming config
- `get_media_management_config(user_id)` — fetches media management settings
- `update_media_management_config(user_id, req)` — updates media management settings
- `get_metadata_config(user_id)` — fetches metadata provider config
- `update_metadata_config(user_id, req)` — updates metadata provider config
- `get_email_config(user_id)` — fetches email delivery config
- `update_email_config(user_id, req)` — updates email config
- `validate_metadata_languages(langs)` — validates a list of language codes

### EmailService (services/email.rs)
Email delivery operations.

- `send_test(user_id)` — sends a test email to configured recipient
- `send_file(user_id, item_id)` — emails a library file as attachment

### RemotePathMappingService (services/remote_path_mapping.rs)
Remote path mapping CRUD.

- `get_remote_path_mapping(user_id, id)` — fetches a mapping
- `list_remote_path_mappings(user_id)` — lists all mappings
- `create_remote_path_mapping(user_id, req)` — adds a new mapping
- `update_remote_path_mapping(user_id, id, req)` — edits a mapping
- `delete_remote_path_mapping(user_id, id)` — removes a mapping

### ManualImportService (services/manual_import.rs)
Data access facade used by the manual import UI workflow.

- `list_works(user_id)` — lists all Works for work-file linking
- `list_root_folders(user_id)` — lists root folders for target selection
- `list_library_items_by_work(user_id, work_id)` — lists files linked to a Work
- `list_library_items_by_work_ids(user_id, ids)` — bulk fetch files by multiple Work IDs
- `get_work(user_id, id)` — fetches a single Work
- `delete_library_item(user_id, id)` — removes a file from the library
- `create_library_item(user_id, req)` — links a file to a Work as a LibraryItem
- `create_history_event(req)` — records a history event

### MatchingService (services/matching.rs)
Filename parsing and Work matching.

- `extract_and_reconcile(input)` — parses a file path and matches it to a library Work

### ImportIoService (services/import_io.rs)
I/O operations used during the import pipeline.

- `get_grab(id)` — fetches a grab record
- `get_download_client(id)` — fetches a download client
- `set_grab_content_path(id, path)` — records the content path for a completed download
- `get_work(user_id, id)` — fetches a Work
- `list_library_items_by_work(user_id, work_id)` — fetches existing files for a Work
- `get_root_folder(id)` — fetches a root folder
- `list_root_folders(user_id)` — lists root folders for import targeting
- `list_remote_path_mappings(user_id)` — lists path mappings for seedbox resolution
- `update_library_item_size(id, size)` — updates file size after import
- `create_library_item(req)` — creates a LibraryItem record after successful import

### HttpFetcher (services/http.rs)
Outbound HTTP client abstraction.

- `fetch(req)` — makes an HTTP request with rate limiting, timeout, and user-agent control
- `fetch_ssrf_safe(req)` — same but validates the URL is not an internal/private address

### LlmCaller (services/llm.rs)
LLM invocation abstraction.

- `call(req)` — calls an LLM provider with a templated prompt, returns structured response

### Common Error (services/common.rs)
- `ServiceError` — top-level service error enum; converts from `DbError`
