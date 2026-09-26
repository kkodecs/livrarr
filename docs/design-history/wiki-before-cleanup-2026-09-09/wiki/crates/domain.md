# livrarr-domain

Foundation crate. All dependency arrows point here. Defines entities, enums, errors, and service/workflow traits consumed by all other crates.

---

## Entities (entities.rs, enrichment_types.rs, infra_config.rs)

### ID Type Aliases

> **These are type aliases, not newtypes.** Every one is `pub type X = i64`. They document
> intent and read well in signatures, but they provide **no compile-time protection** — a
> `WorkId` and an `AuthorId` are literally the same type, and passing one where the other
> belongs compiles cleanly.

- `UserId` — users
- `WorkId` — works (books)
- `AuthorId` — authors
- `LibraryItemId` — physical files in the library
- `RootFolderId` — root folder paths
- `GrabId` — grab records
- `DownloadClientId` — download clients
- `RemotePathMappingId` — remote path mappings
- `HistoryId` — history events
- `NotificationId` — notifications
- `ExternalIdRowId` — external ID rows
- `IndexerId` — indexers

### Core Entity Structs
- `User` — account record; fields: id, username, password_hash, role, api_key_hash, setup_pending, timestamps
- `Session` — auth session; fields: token_hash, user_id, persistent, created_at, expires_at
- `Work` — a book/audiobook entry; fields: id, user_id, title variants, author, series, metadata keys (ol/hc/gr/isbn/asin), `enrichment_status` **and** `identity_status` (the two-state split — see the enums below), monitor flags, ebook + audiobook cover slots, timestamps
- `Author` — an author record; fields: id, user_id, name, sort_name, provider keys, monitor settings, added_at
- `Series` — a book series; fields: id, user_id, author_id, name, gr_key, monitor flags, work_count, added_at
- `LibraryItem` — a file on disk linked to a Work; fields: id, user_id, work_id, root_folder_id, path, media_type, file_size, import_id, imported_at, `tag_status`, `tagged_at_generation`, duration_seconds, chapter_scan_status
- `PlaybackProgress` — reading/listening position for a user/item pair. Not audiobook-only: `position` is a CFI string for EPUB, a page number for PDF, or seconds for audio
- `RootFolder` — a watched library root path and its media type
- `DownloadClient` — a configured torrent/usenet client (qBit, SAB, etc.)
- `Grab` — a grab record linking a Work to a download; tracks status, download_id, content_path, retry state
- `RemotePathMapping` — maps a seedbox remote path to a local path
- `HistoryEvent` — an event log entry (grab, import, delete, etc.)
- `HistoryFilter` — filter params for history queries
- `Notification` — in-app notification for a user; tracks read/dismissed state
- `ExternalId` — a provider-specific ID (e.g. Goodreads) linked to a Work
- `Indexer` — a Torznab/Newznab indexer config; URL, api_key, category/search flags, `enable_rss`. The RSS *cursor* is a separate struct, `IndexerRssState`
- `IndexerRssState` — per-indexer RSS cursor (last_publish_date, last_guid)
- `IndexerConfig` — global indexer settings: rss_sync_interval_minutes, rss_match_threshold, rss_grab_failure_limit
- `Import` — a Readarr import job record; tracks progress counts and status
- `FieldProvenance` — which provider/setter last wrote a given Work field
- `MergeResolved<T>` — newtype wrapper around a value resolved by a merge; carries no conflict information
- `QueueProgress` — download progress snapshot (percent, eta, download_status)
- `QueueSummary` — aggregate queue counts (total, downloading, importing)

> **Not listed above.** From `entities.rs`: `AudiobookChapter`, `KashLink`, `NewKashLink`,
> `CrossFormatState`, `Bookmark` — the chapter and cross-format-resume types. From
> `enrichment_types.rs`: `CoverCandidate`, `InternalCoverCandidate`, `SelectCoverRequest`,
> `CoverResolution`, `FieldDissent`, `LogSurfaceStatus`.

### Core Enums
- `MediaType` — Ebook / Audiobook
- `UserRole` — Admin / User
- `GrabStatus` — grab lifecycle state. All seven variants: Sent, Confirmed, Importing, Imported, ImportFailed, Removed, Failed. There is no `Queued` and no `Downloading`
- `EnrichmentStatus` — enrichment *quality* only, four variants: Unenriched (initial / not yet attempted), Enriched, Thin (identity known, no meaningful metadata found), Failed. There is no `Pending` and no `Skipped`. Identity-track outcomes used to live here and were moved to `IdentityStatus` by migration 055
- `IdentityStatus` — the persisted identity-confidence badge, the other half of the two-state split: Pending, Confirmed (work anchor), Provisional (ISBN/ASIN bridge only), Conflict, NeedsReview, NotFound
- `TagStatus` — per-file tag sync state, tracked on LibraryItem rather than Work: Synced, Pending, Failed
- `EventType` — history event kinds
- `NotificationType` — notification category
- `NarrationType` — Human / Ai / AiAuthorizedReplica, plus Abridged and Unabridged
- `AuthType` — authentication type
- `QueueStatus` — download client queue entry state
- `DownloadClientImplementation` — concrete client type (qBittorrent, SABnzbd, etc.); provides `client_type()` and `protocol()`
- `LlmRole` — LLM message role
- `LlmProvider` — supported LLM backends
- `HealthCheckType` — health check category
- `DbError` — database-layer error variants
- `MetadataProvider` — named metadata providers: Hardcover, OpenLibrary, Goodreads, Audnexus, Llm, Readarr, GoogleBooks, Audible; provides `record_key()`
- `WorkField` — all 26 enrichable Work fields
- `ProvenanceSetter` — who/what set a provenance record. Six variants: Provider, User, System, AutoAdded, Imported (CSV list import) and Import (external system, e.g. Readarr) — `Imported` and `Import` are distinct
- `RequestPriority` — queue-ordering hint: Low, Normal, High, Interactive
- `NormalizationClass` — RichText / DisplayText / Identifier at field level, plus English / ForeignLanguage as work-level merge strategies
- `OutcomeClass` — enrichment outcome class; provides `is_phase2_terminal()`, `can_merge()`, `all_can_merge()`
- `WillRetryReason` — why a retry was scheduled
- `PermanentFailureReason` — why an enrichment permanently failed
- `ApplyMergeOutcome` — result of applying a merge
- `ExternalIdType` — provider-specific external ID type

> **Not listed above.** From `entities.rs`, all enums are covered. From `enrichment_types.rs`:
> `CoverTrust`, `CoverMediaType`, `CoverCandidateSource`, `Freshness`, `DissentReason`,
> `AnchorQuery` — the cover-trust, cache-freshness, merge-dissent and anchor-query vocabularies.

### Utility Functions (util.rs)

> **6 of the module's 12 public functions are listed here.** Not listed:
> `is_series_stub_key`, `split_series_suffix`, `decode_xml_entities`, `proxy_cover_url`,
> `unproxy_cover_url`, `strip_isbn_punctuation`.

- `sanitize_path_component(input, fallback)` — strips control characters, replaces illegal chars with `_`, trims trailing dots/spaces, and truncates to 255 bytes. Falls back to `fallback` (then to `"_"`) when the result would be empty, `.` or `..`
- `derive_sort_name(display_name)` — `"Frank Herbert"` → `"Herbert, Frank"`. Treats the last whitespace-delimited word as the surname — wrong for East Asian, Iberian and compound surnames, but matches the Readarr/Servarr convention
- `normalize_for_matching(s)` — **superseded in production** by `identity_matching::identity_key` (REQ-014) and no longer called from any production site. Retained only because existing test fixtures build `normalized_title` / `normalized_author` with it. It keeps stopwords and accents, which is exactly why it was replaced
- `normalize_language(lang)` — normalizes a language string to an ISO 639-1 code, delegating to `normalization::normalize_language` and falling back to the trimmed, lower-cased input. Strips region subtags (`"en-US"` → `"en"`)
- `normalize_language_opt(lang)` — same, returns `Option`
- `classify_file(path)` — determines MediaType from file extension: epub/mobi/azw3/pdf → Ebook, mp3/m4a/m4b/flac/ogg/wma → Audiobook, anything else → `None`

---

## Settings (settings.rs)

### Config Structs
- `NamingConfig` — file/folder naming format strings and rename flags
- `MediaManagementConfig` — CWA ingest path, preferred ebook/audiobook formats
- `ProwlarrConfig` — Prowlarr URL, api_key, enabled flag
- `MetadataConfig` — Hardcover, LLM, Audnexus and Google Books provider settings; language list
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

Not listed: `ReadarrOriginInfo` and `AddReadarrOriginRequest` — the admin-approved origin
allowlist types. The 9 bullets above cover 12 of the module's 14 public types.

---

## Torznab (torznab.rs)

- `TorznabItem` — a single parsed release result from a Torznab feed
- `TorznabParseResult` — **two** variants: `Items(Vec<TorznabItem>)` and `Error { code, description }`. There is no "empty" variant — an empty feed is `Items` with an empty vec
- `parse_torznab_xml(xml)` — takes the raw response bytes and returns `Result<TorznabParseResult, String>`. The outer `Err` means the bytes could not be parsed at all (invalid UTF-8, or an XML reader error) — distinct from a well-formed feed carrying the Torznab `Error` variant

---

## Keyed Mutex (keyed_mutex.rs)

- `KeyedMutex<K>` — per-key async mutex map; prevents concurrent work on the same key. Resident keys are hard-capped at 256 by an internal semaphore, so the map cannot grow without bound however many distinct keys are ever requested (PRINCIPLES §5)
- `KeyedMutexGuard` — RAII guard returned by `lock()`; holds the per-key lock and prunes the key on drop
- `KeyedMutex::lock(key)` — acquires or creates a per-key lock. An **existing** key never waits for capacity; only a genuinely new key waits, and it does so without holding the map lock
- `KeyedMutex::sweep()` — explicit backstop only. Per-guard pruning on every release already removes unreferenced keys, so a healthy instance rarely needs this called

---

## Service Traits (services/)

### WorkService (services/work.rs)
Manages Work CRUD, refresh, merge, and the identity-edit surface.

> **12 of the trait's 24 methods are listed here.** Not listed: `resolve_identity`,
> `resolve_identity_local`, `add_fast`, `complete_add`, `is_enriching`, `converge_work`,
> `preview_merge_works`, `merge_works`, `preview_identity_edit`, `commit_identity_edit`,
> `clear_identity_slot`.

- `add(user_id, candidate)` — creates a Work from a `WorkCandidate`
- `get(user_id, work_id)` — fetches a Work by ID
- `get_detail(user_id, work_id)` — fetches a `WorkDetailView`
- `list(user_id, filter)` — lists Works with optional filtering/sorting
- `list_paginated(user_id, page, page_size, sort_by, sort_dir, media_type, language)` — paginated Work list
- `update(user_id, work_id, req)` — updates user-editable Work fields
- `delete(user_id, work_id)` — deletes a Work and its library items
- `refresh(user_id, work_id, surface)` — re-enriches a single Work; `surface` selects Interactive (a person is waiting) vs Bulk (unattended sweep, Low queue priority)
- `retry_all_incomplete(user_id)` — bulk-recovers incomplete works (Failed/Unenriched/identity-Pending) in a single pass through the one road; replaces the deleted background retry job
- `upload_cover(user_id, work_id, bytes)` — replaces the cover with a user-uploaded image
- `download_cover(user_id, work_id)` — returns cover image bytes
- `search_works(user_id, query, page, page_size)` — paginated search across library works
- `try_start_bulk_refresh(user_id)` — acquires the per-user bulk-refresh slot; returns `Option<BulkRefreshGuard>`, `None` when a run is already live. The guard frees the slot on `Drop` — completion, error, panic unwind and task abort all release it

**Bulk refresh has no service method.** `refresh_all` is implemented at the handler layer
(`livrarr-handlers/src/work.rs::refresh_all`); the trait carries only a commented-out
placeholder.

### DiscoveryService (services/discovery.rs)
Provider search — split out of `WorkService`.

- `lookup(req)` — searches metadata providers for works (takes no `user_id`)
- `lookup_filtered(user_id, req, raw)` — same, applying library-dedup and language filters
- `eager_match_by_author(user_id, queries)` — bulk best-guess discovery for manual import: groups queries by author and issues one author-scoped query per provider. Suggestion-only — no resolver call, so results carry `candidate_id: None`

### AuthorService (services/author.rs)
Manages Author CRUD, lookup, and bibliography.

- `add(user_id, req)` — creates or finds an Author
- `get(user_id, id)` — fetches an Author by ID
- `list(user_id)` — lists all Authors for a user
- `update(user_id, id, req)` — updates author metadata and monitor settings
- `delete(user_id, id)` — deletes an Author and cascades to their Works
- `lookup(query, limit)` — searches metadata providers by author name (takes no `user_id`)
- `search(user_id, query)` — search within library authors
- `bibliography(user_id, author_id, raw)` — returns bibliography entries (cached or fresh); `raw` selects the unfiltered set
- `refresh_bibliography(user_id, author_id)` — forces a fresh bibliography fetch
- `spawn_bibliography_refresh(author_id, user_id)` — spawns a background bibliography refresh (note the argument order: author first)
- `lookup_authors(term, limit)` — multi-provider author candidate search (takes no `user_id`)

Not listed: `merge(user_id, survivor_id, loser_id)` — the author-dedup merge. 11 of the trait's
12 methods are above.

### SeriesService (services/series.rs)
Manages series CRUD and monitoring.

- `list(user_id)` — lists all series for a user
- `get(user_id, id)` — fetches a series by ID
- `refresh(user_id, id)` — re-fetches series metadata from Goodreads
- `monitor(user_id, series_id, monitored)` — sets the series' monitored flag (one bool, no request struct)
- `update(user_id, series_id, title)` — updates the series title

### SeriesQueryService (services/series.rs)
Read-heavy series views and GR candidate resolution.

- `list_enriched(user_id)` — lists series with library-membership counts
- `get_detail(user_id, id)` — fetches series with full Work/LibraryItem list
- `update_flags(user_id, series_id, monitor_ebook, monitor_audiobook, language)` — updates monitor flags
- `resolve_gr_candidates(user_id, author_id)` — fetches Goodreads author candidates for linking
- `list_author_series(user_id, author_id, raw)` — lists all series for an author
- `refresh_author_series(user_id, author_id)` — refreshes series list from Goodreads for an author
- `monitor_series(user_id, author_id, req)` — starts monitoring a series by GR key
- `run_series_monitor_worker(params)` — background worker that adds missing series Works

Not listed: `promote_stub` and `series_books` — 8 of the trait's 10 methods are above.

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
Data access used by the download queue view and the polling loop.

- `list_grabs_paginated(user_id, page, per_page)` — paginated grab list for UI queue view
- `list_download_clients()` — lists download clients for polling (takes no `user_id`)
- `try_set_importing(user_id, grab_id)` — atomically marks a grab as importing
- `update_grab_status(user_id, grab_id, status, error)` — updates grab status after poll
- `fetch_download_progress(client, download_id)` — polls a download client for progress
- `summary(user_id)` — returns queue aggregate counts

### ImportWorkflow (services/import.rs)
Orchestrates the full import pipeline for a completed grab.

- `import_grab(user_id, grab_id)` — runs the complete import workflow for a finished download
- `import_file(user_id, req)` — brings one file into the library as a `LibraryItem`; the shared entry point for every import door (grab, manual, Readarr, scan)

### BibliographyTrigger (services/import.rs)
- `trigger(author_id, user_id)` — fires a bibliography refresh after import (note the argument order: author first)

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

- `enrich_work(user_id, work_id, mode, candidate_id, priority, freshness)` — fetches and merges metadata from providers. `priority` is the queue-ordering hint, independent of `mode`; `freshness` decides whether fetches may be served from the persistent provider-response cache
- `reset_for_manual_refresh(user_id, work_id)` — clears enrichment state for a re-run

Not listed: `inject_source_data` and `fetch_anchor_preview` — 2 of the trait's 4 methods are above.

### AuthorMonitorWorkflow (services/monitor.rs)
Checks monitored authors for new works.

- `run_monitor(user_id, cancel)` — scans that user's monitored authors and adds new Works found in bibliography; `cancel` is the cooperative-shutdown token. The trait's only method

### RssSyncWorkflow (services/rss.rs)
Polls RSS feeds and auto-grabs matching releases.

- `run_sync()` — checks all enabled RSS indexers, matches items, grabs if threshold met

### ReadarrImportWorkflow (services/readarr.rs)
Handles the full Readarr library migration flow.

- `connect(req)` — validates Readarr API connection and returns root folder list
- `preview(user_id, req)` — dry-runs an import and returns what would be created/skipped
- `start(user_id, req)` — launches a Readarr import job, returns import_id
- `progress(user_id, import_id)` — returns live progress for **this user's own** run; `import_id` is optional and, when given, additionally requires the caller's run to match it. A mismatch or a non-owned run yields `NotFound`, never confirming another user owns it
- `history(user_id)` — lists past Readarr imports
- `undo(user_id, import_id)` — rolls back a completed Readarr import

Not listed: the origin trust boundary — `list_origins()`, `add_origin(url)`, `remove_origin(id)`,
the admin-managed allowlist of approved Readarr origins. 6 of the trait's 9 methods are above.

### ListService (services/list.rs)
Imports book lists (CSV/ISBN) into the library.

- `preview(user_id, bytes)` — parses and previews a list import without committing (takes the raw file bytes, not a request struct)
- `confirm(user_id, preview_id, import_id, row_indices, language)` — commits selected rows of a previewed list import
- `complete(user_id, import_id)` — finalizes a list import job
- `undo(user_id, import_id)` — removes works added by a list import
- `list_imports(user_id)` — lists past list import summaries

### FileService (services/file.rs)
Library file read/management operations.

- `list(user_id)` — lists all LibraryItems
- `list_paginated(user_id, page, page_size)` — paginated LibraryItem list
- `get(user_id, item_id)` — fetches a single LibraryItem
- `delete(user_id, item_id)` — deletes a LibraryItem
- `resolve_path(user_id, item_id)` — resolves the absolute path for a LibraryItem (takes ids, not an item)
- `prepare_email(user_id, item_id)` — builds an `EmailPayload` (file bytes, filename, extension) for the caller to send
- `get_progress(user_id, item_id)` — fetches playback progress
- `update_progress(user_id, item_id, position, progress_pct, kind, cross_format_ts)` — updates playback progress; only `ProgressKind::Progress` with a finite `cross_format_ts` may advance the cross-format furthest mark

Not listed: `get_progress_for_items(user_id, library_item_ids)` — the batch read. 8 of the
trait's 9 methods are above.

### NotificationService (services/notification.rs)
In-app notification management.

- `list_paginated(user_id, unread_only, page, page_size)` — paginated notification list
- `mark_read(user_id, id)` — marks a notification read
- `dismiss(user_id, id)` — dismisses a single notification
- `dismiss_all(user_id)` — dismisses all notifications for a user
- `create(req)` — creates a new notification

### HistoryService (services/history.rs)
Event history read + the observer write.

- `list_paginated(user_id, filter, page, page_size)` — paginated filtered history
- `record(user_id, draft)` — records one history event. **Infallible by signature** — history is an observer, never an actor, so the impl absorbs write failures with a logged warning and callers cannot propagate one

### RootFolderService (services/root_folder.rs)
Root folder CRUD. **No method takes a `user_id`.**

- `get_root_folder(id)` — fetches a root folder
- `list_root_folders()` — lists all root folders
- `create_root_folder(path, media_type)` — adds a new root folder
- `delete_root_folder(id)` — removes a root folder

### DownloadClientSettingsService (services/download_client_settings.rs)
Download client configuration CRUD. **No method takes a `user_id`.**

- `get_download_client(id)` — fetches a client record
- `list_download_clients()` — lists all clients
- `create_download_client(params)` — adds a new client
- `update_download_client(id, params)` — edits a client
- `delete_download_client(id)` — removes a client

### DownloadClientCredentialService (services/download_client_credentials.rs)
Credential-bearing client access. The trait's only method.

- `get_download_client_with_credentials(id)` — fetches a client including credentials

### IndexerSettingsService (services/indexer_settings.rs)
Indexer configuration CRUD and Prowlarr/RSS config. **No method takes a `user_id`.**

- `get_indexer(id)` — fetches an indexer
- `list_indexers()` — lists all indexers
- `create_indexer(params)` — adds a new indexer
- `update_indexer(id, params)` — edits an indexer
- `delete_indexer(id)` — removes an indexer
- `set_supports_book_search(id, supports)` — updates the book-search capability flag
- `get_prowlarr_config()` — fetches Prowlarr integration config
- `update_prowlarr_config(params)` — updates Prowlarr config
- `get_indexer_config()` — fetches global indexer (RSS) config
- `update_indexer_config(params)` — updates global indexer config

### IndexerCredentialService (services/indexer_credentials.rs)
Credential-bearing indexer access. The trait's only method.

- `get_indexer_with_credentials(id)` — fetches an indexer including its API key

### AppConfigService (services/app_config.rs)
Application-wide configuration reads and updates. **No method takes a `user_id`.**

- `get_naming_config()` — fetches file/folder naming config
- `get_media_management_config()` — fetches media management settings
- `update_media_management_config(params)` — updates media management settings
- `get_metadata_config()` — fetches metadata provider config
- `update_metadata_config(params)` — updates metadata provider config
- `get_email_config()` — fetches email delivery config
- `update_email_config(params)` — updates email config
- `validate_metadata_languages(languages, llm_enabled, llm_endpoint, llm_api_key, llm_model, google_books_api_key)` — validates a language list against the configured providers

Not listed: `get_default_language()`, `update_default_language(language)`, and
`validate_default_language(language)` — the default language applied wherever a creation door
has no explicit one. 8 of the trait's 11 methods are above.

### EmailService (services/email.rs)
Email delivery operations. Takes no user or item ids — the caller supplies the bytes.

- `send_test()` — sends a test email to the configured recipient
- `send_file(file_bytes, filename, extension)` — sends a file as an attachment. It performs no lookup; `FileService::prepare_email` produces the payload

### RemotePathMappingService (services/remote_path_mapping.rs)
Remote path mapping CRUD. **No method takes a `user_id`.**

- `get_remote_path_mapping(id)` — fetches a mapping
- `list_remote_path_mappings()` — lists all mappings
- `create_remote_path_mapping(host, remote_path, local_path)` — adds a new mapping (no request struct)
- `update_remote_path_mapping(id, host, remote_path, local_path)` — edits a mapping (no request struct)
- `delete_remote_path_mapping(id)` — removes a mapping

### ManualImportService (services/manual_import.rs)
Data access facade used by the manual import UI workflow.

- `list_works(user_id)` — lists all Works for work-file linking
- `list_root_folders()` — lists root folders for target selection (takes no `user_id`)
- `list_library_items_by_work(user_id, work_id)` — lists files linked to a Work
- `list_library_items_by_work_ids(user_id, work_ids)` — bulk fetch files by multiple Work IDs
- `get_work(user_id, work_id)` — fetches a single Work
- `delete_library_item(user_id, item_id)` — removes a file from the library
- `create_library_item(user_id, work_id, root_folder_id, path, media_type, file_size)` — links a file to a Work as a LibraryItem (six explicit params, no request struct)

### MatchingService (services/matching.rs)
Filename and embedded-metadata extraction. The trait's only method.

- `extract_and_reconcile(input)` — parses a file path (or grouped paths) into `MatchCluster`s carrying author, title, series, language, isbn, asin and year. It does **not** look up or return a Work

### ImportIoService (services/import_io.rs)
Data access used during the import pipeline — every method here reads or writes a record.

- `get_grab(user_id, grab_id)` — fetches a grab record
- `get_download_client(client_id)` — fetches a download client
- `set_grab_content_path(user_id, grab_id, content_path)` — records the content path for a completed download
- `get_work(user_id, work_id)` — fetches a Work
- `list_library_items_by_work(user_id, work_id)` — fetches existing files for a Work
- `get_root_folder(root_folder_id)` — fetches a root folder
- `list_root_folders()` — lists root folders for import targeting (takes no `user_id`)
- `list_remote_path_mappings()` — lists path mappings for seedbox resolution (takes no `user_id`)
- `update_library_item_size(user_id, item_id, new_size)` — updates file size after import
- `update_library_item_path(user_id, item_id, new_path)` — persists a new relative path after the merge reorganize step physically relocates the file

This trait has no `create_library_item`; `ManualImportService::create_library_item` does.

### HttpFetcher (services/http.rs)
Outbound HTTP client abstraction.

- `fetch(req)` — makes an HTTP request with rate limiting, timeout, and user-agent control
- `fetch_ssrf_safe(req)` — same but validates the URL is not an internal/private address
- `fetch_ssrf_safe_fast_connect(req)` — as `fetch_ssrf_safe`, but the TCP-connect phase is bounded far tighter than `req.timeout`, to fail fast on an unreachable host. **Defaulted** to plain `fetch_ssrf_safe`; `HttpFetcherImpl` is the only override
- `fetch_no_redirect(req)` — returns the raw 3xx (status + `Location`) instead of chasing it. **Defaulted to `fetch`, which FOLLOWS redirects** — a test double that needs no-redirect behavior must override this method itself; overriding `fetch` alone is not enough

### LlmCaller (services/llm.rs)
LLM invocation abstraction. The trait's only method.

- `call(req)` — calls an LLM provider with a system + user template and a typed field context; returns the response content plus the model used and elapsed time

### Common Error (services/common.rs)
- `ServiceError` — top-level service error enum; converts from `DbError`
