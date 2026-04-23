# livrarr-db

SQLite database layer. All SQL lives here. Implemented by `SqliteDb` (in `sqlite.rs`). Tests use `create_test_db()` which returns an in-memory SQLite instance.

All traits are implemented by `SqliteDb` unless noted otherwise.

---

## DB Traits

### UserDb
CRUD for user accounts.

- `get_user(id)` — fetch user by ID
- `get_user_by_username(username)` — fetch user by login name
- `get_user_by_api_key_hash(hash)` — fetch user by hashed API key
- `list_users()` — list all users
- `create_user(req)` — insert a new user
- `update_user(id, req)` — update username, password_hash, role
- `delete_user(id)` — delete a user
- `count_admins()` — returns number of admin accounts (used for setup guard)
- `complete_setup(req)` — finishes initial setup by setting credentials
- `update_api_key_hash(id, hash)` — rotate a user's API key hash

### SessionDb
Auth session management.

- `get_session(token_hash)` — fetch session by token hash
- `create_session(req)` — insert a new session
- `delete_session(token_hash)` — delete a specific session (logout)
- `extend_session(token_hash, new_expiry)` — push expiry forward for active sessions
- `delete_user_sessions(user_id)` — delete all sessions for a user
- `delete_expired_sessions()` — purge all expired sessions

### WorkDb
Work CRUD, enrichment state, and search.

- `get_work(user_id, id)` — fetch a Work by ID
- `list_works(user_id, filter)` — list Works with optional filter/sort
- `list_works_by_author(user_id, author_id)` — list all Works for an Author
- `list_works_paginated(user_id, filter, page, per_page)` — paginated Work list
- `create_work(req)` — insert a new Work
- `update_work_enrichment(id, req)` — apply enrichment data to a Work
- `update_work_user_fields(user_id, id, req)` — update user-editable fields
- `set_cover_manual(user_id, id, flag)` — mark cover as manually overridden
- `delete_work(user_id, id)` — delete a Work
- `work_exists_by_ol_key(user_id, ol_key)` — check dedup by OpenLibrary key
- `list_works_for_enrichment(user_id)` — list Works pending enrichment
- `list_works_by_author_ol_keys(user_id, keys)` — lookup Works by author OL keys (bibliography match)
- `list_work_provider_keys_by_author(user_id, author_id)` — fetch all provider keys for an author's Works
- `find_by_normalized_match(user_id, normalized_title, normalized_author)` — fuzzy dedup lookup
- `reset_pending_enrichments(user_id)` — reset all works stuck in pending state
- `list_monitored_works_all_users()` — list monitored Works across all users (for RSS/monitor workers)
- `set_enrichment_status_skipped(user_id, id)` — mark a Work's enrichment as skipped
- `apply_enrichment_merge(req)` — atomically apply a merged enrichment result with provenance
- `reset_for_manual_refresh(user_id, id)` — clear enrichment state for a re-run
- `list_conflict_works(user_id)` — list Works with unresolved merge conflicts
- `get_merge_generation(user_id, id)` — fetch the current merge generation counter for optimistic concurrency
- `search_works(user_id, term)` — full-text search across Work titles/authors

### AuthorDb
Author CRUD and monitor queries.

- `get_author(user_id, id)` — fetch Author by ID
- `list_authors(user_id)` — list all Authors for a user
- `create_author(req)` — insert a new Author
- `update_author(user_id, id, req)` — update author name, keys, monitor settings
- `delete_author(user_id, id)` — delete an Author
- `find_author_by_name(user_id, name)` — dedup lookup by normalized name
- `list_monitored_authors(user_id)` — list Authors with monitoring enabled

### LibraryItemDb
Physical file record management.

- `get_library_item(user_id, id)` — fetch a LibraryItem by ID
- `list_library_items(user_id)` — list all files for a user
- `list_library_items_paginated(user_id, page, per_page)` — paginated file list
- `list_library_items_by_work_ids(user_id, ids)` — bulk fetch items by multiple Work IDs
- `list_library_items_by_work(user_id, work_id)` — fetch all files linked to a Work
- `create_library_item(req)` — insert a new file record
- `delete_library_item(user_id, id)` — delete a file record
- `library_items_exist_for_root(root_folder_id)` — check if any files reference a root folder (delete guard)
- `list_taggable_items_by_work(user_id, work_id)` — fetch items eligible for tag writing
- `update_library_item_size(id, size)` — update stored file size
- `work_has_library_item(user_id, work_id, media_type)` — check if a Work has a file of given type

### RootFolderDb
Root folder CRUD.

- `get_root_folder(id)` — fetch a root folder by ID
- `list_root_folders(user_id)` — list all root folders
- `create_root_folder(req)` — insert a new root folder
- `delete_root_folder(id)` — delete a root folder
- `get_root_folder_by_media_type(user_id, media_type)` — fetch the default root folder for a media type

### GrabDb
Download grab lifecycle management.

- `get_grab(id)` — fetch a grab by ID
- `list_active_grabs(user_id)` — list grabs not in terminal state
- `upsert_grab(req)` — insert or update a grab record (used on re-grab)
- `update_grab_status(id, status)` — set grab status
- `update_grab_download_id(id, download_id)` — store the download client's internal ID
- `get_grab_by_download_id(download_id)` — find a grab by download client ID (queue poll)
- `reset_importing_grabs()` — reset stuck "importing" grabs on startup
- `set_grab_content_path(id, path)` — record the extracted content path for import
- `list_grabs_paginated(user_id, filter)` — paginated grab list for UI
- `try_set_importing(id)` — atomically transition grab to importing (returns false if already claimed)
- `active_grab_exists(user_id, work_id, media_type)` — check if an active grab exists to prevent duplicates
- `list_retriable_grabs(user_id)` — list failed grabs eligible for retry
- `increment_import_retry(id)` — bump the retry counter on import failure
- `queue_summary(user_id)` — returns aggregate queue counts

### DownloadClientDb
Download client configuration CRUD.

- `get_download_client(id)` — fetch a client record (no credentials)
- `get_download_client_with_credentials(id)` — fetch with decrypted credentials
- `list_download_clients(user_id)` — list all clients
- `create_download_client(req)` — insert a new client
- `update_download_client(id, req)` — update client settings
- `delete_download_client(id)` — delete a client
- `get_default_download_client(protocol)` — fetch the default client for a given protocol

### RemotePathMappingDb
Remote-to-local path mapping CRUD.

- `get_remote_path_mapping(id)` — fetch a mapping
- `list_remote_path_mappings(user_id)` — list all mappings
- `create_remote_path_mapping(req)` — insert a new mapping
- `update_remote_path_mapping(id, req)` — update a mapping
- `delete_remote_path_mapping(id)` — delete a mapping

### HistoryDb
Event history.

- `list_history(user_id, filter)` — list history events with optional filter
- `list_history_paginated(user_id, filter, page, per_page)` — paginated filtered history
- `create_history_event(req)` — insert a history event

### NotificationDb
In-app notification CRUD.

- `list_notifications(user_id)` — list all notifications
- `list_notifications_paginated(user_id, page, per_page)` — paginated notification list
- `create_notification(req)` — insert a new notification
- `mark_notification_read(user_id, id)` — mark a notification read
- `dismiss_notification(user_id, id)` — dismiss a single notification
- `dismiss_all_notifications(user_id)` — dismiss all notifications for a user

### ConfigDb
Application configuration reads and writes.

- `get_naming_config()` — fetch naming format config
- `get_media_management_config()` — fetch media management settings
- `update_media_management_config(req)` — update media management settings
- `get_prowlarr_config()` — fetch Prowlarr integration config
- `update_prowlarr_config(req)` — update Prowlarr config
- `get_metadata_config()` — fetch metadata provider config
- `update_metadata_config(req)` — update metadata provider config
- `get_email_config()` — fetch email delivery config
- `update_email_config(req)` — update email config
- `get_indexer_config()` — fetch global indexer/RSS config
- `update_indexer_config(req)` — update global indexer config

### EnrichmentRetryDb
Enrichment retry scheduling.

- `list_works_for_retry()` — list Works due for an enrichment retry
- `reset_enrichment_for_refresh(user_id, id)` — clear retry state for a manual re-run
- `increment_retry_count(user_id, id)` — bump the retry counter after a failed enrichment

### IndexerDb
Indexer configuration and RSS state.

- `get_indexer(id)` — fetch an indexer (no credentials)
- `get_indexer_with_credentials(id)` — fetch with decrypted API key
- `list_indexers(user_id)` — list all indexers
- `list_enabled_interactive_indexers(user_id)` — list indexers enabled for manual search
- `create_indexer(req)` — insert a new indexer
- `update_indexer(id, req)` — update indexer settings
- `delete_indexer(id)` — delete an indexer
- `set_supports_book_search(id, flag)` — update book search capability flag
- `list_enabled_rss_indexers()` — list indexers with RSS enabled (for sync worker)
- `get_rss_state(indexer_id)` — fetch the RSS cursor for an indexer
- `upsert_rss_state(state)` — save updated RSS cursor after a sync pass

### AuthorBibliographyDb
Cached author bibliography storage.

- `get_bibliography(author_id)` — fetch cached bibliography for an author
- `save_bibliography(req)` — persist a freshly fetched bibliography
- `delete_bibliography(author_id)` — clear cached bibliography

### SeriesDb
Series records and work-series links.

- `get_series(user_id, id)` — fetch a series by ID
- `list_all_series(user_id)` — list all series for a user
- `list_series_for_author(user_id, author_id)` — list series belonging to an author
- `upsert_series(req)` — insert or update a series record
- `update_series_flags(id, monitor_ebook, monitor_audiobook)` — update monitor flags
- `update_series_work_count(id, count)` — update the known work count for a series
- `link_work_to_series(req)` — associate a Work with a Series
- `list_monitored_series_for_authors(user_id, author_ids)` — list monitored series for a set of authors

### SeriesCacheDb
Goodreads series list cache per author.

- `get_series_cache(author_id)` — fetch cached series list for an author
- `save_series_cache(req)` — persist a freshly fetched series list
- `delete_series_cache(author_id)` — clear cached series list

### ImportDb
Readarr import job tracking.

- `create_import(req)` — create a new import job record
- `get_import(id)` — fetch an import job by ID
- `list_imports(user_id)` — list all import jobs for a user
- `update_import_status(id, status)` — update job status
- `update_import_counts(id, counts)` — update running progress counters
- `set_import_completed(id, stats)` — mark an import job complete with final counts
- `list_library_items_by_import(import_id)` — fetch all files created by an import job
- `delete_library_item_by_id(id)` — delete a specific library item (used by undo)
- `delete_orphan_works_by_import(import_id)` — delete Works left orphaned after an undo
- `delete_orphan_authors_by_import(import_id)` — delete Authors left orphaned after an undo

### PlaybackProgressDb
Audiobook playback progress persistence.

- `get_progress(user_id, library_item_id)` — fetch playback position for a user/item pair
- `upsert_progress(user_id, library_item_id, position)` — save or update playback position

### ListImportDb
Book list import (CSV/ISBN) workflow storage.

- `insert_list_import_preview_row(req)` — insert a single preview row during parse
- `count_list_import_previews(preview_id)` — count rows in a preview batch
- `get_list_import_source(preview_id)` — fetch the source identifier for a preview
- `create_list_import_record(req)` — create the import record when confirm is called
- `get_list_import_record(id)` — fetch an import record
- `get_list_import_preview_row(preview_id, row_index)` — fetch one preview row for processing
- `tag_last_work_with_import(import_id, work_id)` — tag the most recently created Work with the import ID
- `increment_list_import_works_created(import_id)` — bump the works_created counter
- `complete_list_import(import_id)` — mark a list import complete
- `get_list_import_status_for_user(user_id)` — fetch import status for UI polling
- `delete_works_by_list_import(import_id)` — delete all Works created by a list import (undo)
- `mark_list_import_undone(import_id)` — mark a list import as undone
- `list_list_imports(user_id)` — list all list imports for a user
- `work_exists_by_isbn_13(user_id, isbn)` — check dedup by ISBN-13
- `work_exists_by_isbn_10(user_id, isbn)` — check dedup by ISBN-10
- `delete_stale_list_import_previews(cutoff)` — purge unclaimed preview batches older than cutoff
- `tag_work_with_import(import_id, work_id)` — associate a Work with a list import
- `list_works_by_import(import_id)` — list all Works created by a specific list import

### ProvenanceDb
Per-field metadata provenance tracking.

- `set_field_provenance(req)` — upsert provenance for a single Work field
- `set_field_provenance_batch(reqs)` — bulk upsert provenance for multiple fields
- `get_field_provenance(user_id, work_id, field)` — fetch provenance for one field
- `list_work_provenance(user_id, work_id)` — list all provenance entries for a Work
- `delete_field_provenance_batch(user_id, work_id, fields)` — delete provenance for specific fields
- `clear_work_provenance(user_id, work_id)` — delete all provenance for a Work

### ProviderRetryStateDb
Per-provider enrichment retry state machine.

- `get_retry_state(user_id, work_id, provider)` — fetch retry state for a specific provider
- `list_retry_states(user_id, work_id)` — list all provider retry states for a Work
- `record_will_retry(user_id, work_id, provider, reason, next_attempt_at)` — record a scheduled retry
- `record_suppressed(user_id, work_id, provider)` — record a suppressed (rate-limited) attempt
- `record_terminal_outcome(user_id, work_id, provider, reason)` — record a permanent failure
- `reset_all_retry_states(user_id, work_id)` — clear all retry states for a Work
- `list_works_due_for_retry()` — list Works with at least one provider due for retry
- `list_works_with_terminal_provider_rows()` — list Works with permanently failed providers
- `reset_not_configured_outcomes()` — reset terminal "not configured" outcomes when config changes

### ExternalIdDb
External provider ID storage (e.g. Goodreads IDs).

- `upsert_external_id(req)` — insert or update a single external ID
- `upsert_external_ids_batch(reqs)` — bulk upsert external IDs
- `list_external_ids(work_id)` — list all external IDs for a Work

---

## DB Request/Response Structs

### User/Session
- `CreateUserDbRequest` — fields: username, password_hash, role, api_key_hash
- `UpdateUserDbRequest` — fields: username, password_hash, role
- `CompleteSetupDbRequest` — fields: username, password_hash, api_key_hash

### Work
- `CreateWorkDbRequest` — all required fields for inserting a new Work
- `UpdateWorkEnrichmentDbRequest` — all enrichment fields to apply after a provider fetch
- `UpdateWorkUserFieldsDbRequest` — user-editable fields (title, author_name, series, monitor flags)

### Author
- `CreateAuthorDbRequest` — fields: user_id, name, sort_name, provider keys, import_id
- `UpdateAuthorDbRequest` — fields: name, sort_name, provider keys, monitor settings

### LibraryItem
- `CreateLibraryItemDbRequest` — fields: user_id, work_id, root_folder_id, path, media_type, file_size, import_id

### Grab
- `CreateGrabDbRequest` — all fields for inserting a new grab record

### Download Client
- `CreateDownloadClientDbRequest` — all fields for inserting a new client
- `UpdateDownloadClientDbRequest` — all updatable client fields

### History/Notification
- `CreateHistoryEventDbRequest` — fields: user_id, work_id, event_type, data
- `CreateNotificationDbRequest` — fields: user_id, notification_type, ref_key, message, data

### Config
- `UpdateMediaManagementConfigRequest` — CWA path and format preferences
- `UpdateProwlarrConfigRequest` — Prowlarr URL/key/enabled
- `UpdateEmailConfigRequest` — full SMTP config
- `UpdateIndexerConfigRequest` — RSS sync interval and match threshold
- `UpdateMetadataConfigRequest` — all metadata provider settings

### Indexer
- `CreateIndexerDbRequest` — all fields for inserting a new indexer
- `UpdateIndexerDbRequest` — all updatable indexer fields

### Bibliography/Series Cache
- `BibliographyEntry` / `AuthorBibliography` — cached bibliography data structure
- `SeriesCacheEntry` / `AuthorSeriesCache` — cached series list data structure

### Series
- `CreateSeriesDbRequest` — fields for inserting a new series
- `LinkWorkToSeriesRequest` — fields for associating a Work with a Series

### Import
- `CreateImportDbRequest` — fields: id, user_id, source, source_url, target_root_folder_id

### List Import
- `ListImportPreviewRow` — a single row in a preview batch (title, author, ISBNs, year)
- `ListImportSummaryRow` — summary row for the list import history view
- `ListImportRecord` — active import status record (user_id, status)

### External IDs
- `ExternalId` — a stored external ID (id, user_id, work_id, id_type, id_value)
- `UpsertExternalIdRequest` — fields: work_id, id_type, id_value

### Provenance
- `SetFieldProvenanceRequest` — fields: user_id, work_id, field, source, setter, cleared

### Provider Retry State
- `ProviderRetryState` — full retry state row for one work/provider pair

### Enrichment Merge
- `ApplyEnrichmentMergeRequest` — atomic merge payload: work update, new status, provenance upserts/deletes, external ID updates, expected generation

---

## Test Helpers

- `create_test_db()` — creates an in-memory SQLite database with all migrations applied; used in unit and integration tests
