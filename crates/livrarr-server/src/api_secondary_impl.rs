//! Secondary API implementations for testing — AuthorApi, NotificationApi,
//! RootFolderApi, DownloadClientApi, RemotePathMappingApi, ConfigApi,
//! SystemApi, LibraryFileApi, HistoryApi.

use chrono::Utc;

use crate::*;
use livrarr_db::{
    sqlite::SqliteDb, AuthorDb, ConfigDb, CreateAuthorDbRequest, CreateDownloadClientDbRequest,
    CreateLibraryItemDbRequest, CreateNotificationDbRequest, CreateUserDbRequest,
    CreateWorkDbRequest, DownloadClientDb, HistoryDb, HistoryFilter, LibraryItemDb, NotificationDb,
    RemotePathMappingDb, RootFolderDb, UpdateAuthorDbRequest, UpdateDownloadClientDbRequest,
    UpdateEmailConfigRequest, UpdateMediaManagementConfigRequest, UpdateMetadataConfigRequest,
    UpdateProwlarrConfigRequest, UserDb, WorkDb,
};
use livrarr_handlers::types::work::work_to_detail;

/// Map DbError to the semantically correct ApiError.
fn db_err(e: DbError) -> ApiError {
    match e {
        DbError::NotFound { .. } => ApiError::NotFound,
        DbError::Constraint { message } | DbError::Conflict { message } => {
            ApiError::Conflict { reason: message }
        }
        DbError::DataCorruption { detail, .. } => ApiError::Internal(detail),
        DbError::IncompatibleData { detail } => ApiError::Internal(detail),
        DbError::Io(e) => ApiError::Internal(e.to_string()),
    }
}

/// Combined API implementation backed by SqliteDb.
pub struct SecondaryApiImpl {
    db: SqliteDb,
    data_dir: String,
}

impl SecondaryApiImpl {
    pub fn new(db: SqliteDb) -> Self {
        Self {
            db,
            data_dir: "/tmp/livrarr-test".into(),
        }
    }
}

// AuthorApi
impl AuthorApi for SecondaryApiImpl {
    async fn lookup(&self, _uid: UserId, _term: &str) -> Result<Vec<AuthorSearchResult>, ApiError> {
        Ok(vec![])
    }

    async fn add(&self, uid: UserId, req: AddAuthorApiRequest) -> Result<AuthorResponse, ApiError> {
        // Check if author exists by name
        if let Some(existing) = self
            .db
            .find_author_by_name(uid, &req.name)
            .await
            .map_err(db_err)?
        {
            // Update existing with new ol_key
            let updated = self
                .db
                .update_author(
                    uid,
                    existing.id,
                    UpdateAuthorDbRequest {
                        name: None,
                        sort_name: req.sort_name.clone().map(Some),
                        ol_key: Some(Some(req.ol_key.clone())),
                        gr_key: None,
                        monitored: None,
                        monitor_new_items: None,
                        monitor_since: None,
                    },
                )
                .await
                .map_err(db_err)?;
            return Ok(author_to_response(&updated));
        }
        let author = self
            .db
            .create_author(CreateAuthorDbRequest {
                user_id: uid,
                name: req.name,
                sort_name: req.sort_name,
                ol_key: Some(req.ol_key),
                gr_key: None,
                hc_key: None,
                import_id: None,
            })
            .await
            .map_err(db_err)?;
        Ok(author_to_response(&author))
    }

    async fn list(&self, uid: UserId) -> Result<Vec<AuthorResponse>, ApiError> {
        let authors = self.db.list_authors(uid).await.map_err(db_err)?;
        Ok(authors.iter().map(author_to_response).collect())
    }

    async fn get(&self, uid: UserId, id: AuthorId) -> Result<AuthorDetailResponse, ApiError> {
        let author = self.db.get_author(uid, id).await.map_err(db_err)?;
        let works = self.db.list_works(uid).await.map_err(db_err)?;
        let author_works: Vec<WorkDetailResponse> = works
            .iter()
            .filter(|w| w.author_id == Some(id))
            .map(work_to_detail)
            .collect();
        Ok(AuthorDetailResponse {
            author: author_to_response(&author),
            works: author_works,
        })
    }

    async fn update(
        &self,
        uid: UserId,
        id: AuthorId,
        req: UpdateAuthorApiRequest,
    ) -> Result<AuthorResponse, ApiError> {
        let author = self.db.get_author(uid, id).await.map_err(db_err)?;

        let mut errors = Vec::new();
        if matches!(req.monitored, Some(None)) {
            errors.push(FieldError {
                field: "monitored".into(),
                message: "cannot be null".into(),
            });
        }
        if matches!(req.monitor_new_items, Some(None)) {
            errors.push(FieldError {
                field: "monitorNewItems".into(),
                message: "cannot be null".into(),
            });
        }
        if !errors.is_empty() {
            return Err(ApiError::Validation { errors });
        }

        let monitored = req.monitored.flatten();
        let monitor_new_items = req.monitor_new_items.flatten();

        if monitored == Some(true) && author.ol_key.is_none() {
            return Err(ApiError::Validation {
                errors: vec![FieldError {
                    field: "monitored".into(),
                    message: "cannot monitor author without OL linkage".into(),
                }],
            });
        }
        if monitor_new_items == Some(true) {
            let will_be_monitored = monitored.unwrap_or(author.monitored);
            if !will_be_monitored {
                return Err(ApiError::Validation {
                    errors: vec![FieldError {
                        field: "monitor_new_items".into(),
                        message: "monitor_new_items requires monitored=true".into(),
                    }],
                });
            }
        }
        let mut db_req = UpdateAuthorDbRequest {
            name: None,
            sort_name: None,
            ol_key: None,
            gr_key: req.gr_key,
            monitored,
            monitor_new_items,
            monitor_since: None,
        };
        if monitored == Some(true) && !author.monitored {
            db_req.monitor_since = Some(Utc::now());
        }
        let updated = self
            .db
            .update_author(uid, id, db_req)
            .await
            .map_err(db_err)?;
        Ok(author_to_response(&updated))
    }

    async fn delete(&self, uid: UserId, id: AuthorId) -> Result<(), ApiError> {
        self.db.delete_author(uid, id).await.map_err(db_err)
    }
}

// NotificationApi
impl NotificationApi for SecondaryApiImpl {
    async fn list(
        &self,
        uid: UserId,
        unread_only: bool,
    ) -> Result<Vec<NotificationResponse>, ApiError> {
        let notifs = self
            .db
            .list_notifications(uid, unread_only)
            .await
            .map_err(db_err)?;
        Ok(notifs
            .iter()
            .map(|n| NotificationResponse {
                id: n.id,
                notification_type: n.notification_type,
                ref_key: n.ref_key.clone(),
                message: n.message.clone(),
                data: n.data.clone(),
                read: n.read,
                created_at: n.created_at.to_rfc3339(),
            })
            .collect())
    }

    async fn mark_read(&self, uid: UserId, id: NotificationId) -> Result<(), ApiError> {
        self.db
            .mark_notification_read(uid, id)
            .await
            .map_err(db_err)
    }

    async fn dismiss(&self, uid: UserId, id: NotificationId) -> Result<(), ApiError> {
        self.db.dismiss_notification(uid, id).await.map_err(db_err)
    }

    async fn dismiss_all(&self, uid: UserId) -> Result<(), ApiError> {
        self.db.dismiss_all_notifications(uid).await.map_err(db_err)
    }
}

// RootFolderApi
impl RootFolderApi for SecondaryApiImpl {
    async fn list(&self) -> Result<Vec<RootFolderResponse>, ApiError> {
        let folders = self.db.list_root_folders().await.map_err(db_err)?;
        Ok(folders
            .iter()
            .map(|rf| {
                let (free, total) = get_disk_space(&rf.path);
                RootFolderResponse {
                    id: rf.id,
                    path: rf.path.clone(),
                    media_type: rf.media_type,
                    free_space: free,
                    total_space: total,
                }
            })
            .collect())
    }

    async fn create(
        &self,
        path: &str,
        media_type: MediaType,
    ) -> Result<RootFolderResponse, ApiError> {
        let trimmed = path.trim_end_matches('/');
        // Validate: absolute path
        if !trimmed.starts_with('/') {
            return Err(ApiError::Validation {
                errors: vec![FieldError {
                    field: "path".into(),
                    message: "path must be absolute".into(),
                }],
            });
        }
        // Check for duplicate media type
        if self
            .db
            .get_root_folder_by_media_type(media_type)
            .await
            .map_err(db_err)?
            .is_some()
        {
            return Err(ApiError::Conflict {
                reason: format!("root folder for {:?} already exists", media_type),
            });
        }
        let rf = self
            .db
            .create_root_folder(trimmed, media_type)
            .await
            .map_err(db_err)?;
        let (free, total) = get_disk_space(&rf.path);
        Ok(RootFolderResponse {
            id: rf.id,
            path: rf.path,
            media_type: rf.media_type,
            free_space: free,
            total_space: total,
        })
    }

    async fn get(&self, id: RootFolderId) -> Result<RootFolderResponse, ApiError> {
        let rf = self.db.get_root_folder(id).await.map_err(db_err)?;
        let (free, total) = get_disk_space(&rf.path);
        Ok(RootFolderResponse {
            id: rf.id,
            path: rf.path,
            media_type: rf.media_type,
            free_space: free,
            total_space: total,
        })
    }

    async fn delete(&self, id: RootFolderId) -> Result<(), ApiError> {
        // Check if library items exist
        if self
            .db
            .library_items_exist_for_root(id)
            .await
            .map_err(db_err)?
        {
            return Err(ApiError::Conflict {
                reason: "root folder has library items".into(),
            });
        }
        self.db.delete_root_folder(id).await.map_err(db_err)
    }
}

// DownloadClientApi
impl DownloadClientApi for SecondaryApiImpl {
    async fn list(&self) -> Result<Vec<DownloadClientResponse>, ApiError> {
        let clients = self.db.list_download_clients().await.map_err(db_err)?;
        Ok(clients.iter().map(dc_to_response).collect())
    }

    async fn create(
        &self,
        req: CreateDownloadClientApiRequest,
    ) -> Result<DownloadClientResponse, ApiError> {
        validate_download_client(&req)?;
        let (host, ssl_override) = livrarr_handlers::download_client::normalize_host(&req.host);
        let use_ssl = ssl_override.unwrap_or(req.use_ssl);
        let dc = self
            .db
            .create_download_client(CreateDownloadClientDbRequest {
                name: req.name,
                implementation: req.implementation,
                host,
                port: req.port,
                use_ssl,
                skip_ssl_validation: req.skip_ssl_validation,
                url_base: req.url_base,
                username: req.username,
                password: req.password,
                category: req.category,
                enabled: req.enabled,
                api_key: req.api_key,
            })
            .await
            .map_err(db_err)?;
        Ok(dc_to_response(&dc))
    }

    async fn get(&self, id: DownloadClientId) -> Result<DownloadClientResponse, ApiError> {
        let dc = self.db.get_download_client(id).await.map_err(db_err)?;
        Ok(dc_to_response(&dc))
    }

    async fn update(
        &self,
        id: DownloadClientId,
        req: UpdateDownloadClientApiRequest,
    ) -> Result<DownloadClientResponse, ApiError> {
        let (host, ssl_override) = match &req.host {
            Some(h) => {
                let (clean, ssl) = livrarr_handlers::download_client::normalize_host(h);
                (Some(clean), ssl)
            }
            None => (None, None),
        };
        let use_ssl = ssl_override.or(req.use_ssl);
        let dc = self
            .db
            .update_download_client(
                id,
                UpdateDownloadClientDbRequest {
                    name: req.name,
                    host,
                    port: req.port,
                    use_ssl,
                    skip_ssl_validation: req.skip_ssl_validation,
                    url_base: req.url_base,
                    username: req.username,
                    password: req.password,
                    category: req.category,
                    enabled: req.enabled,
                    api_key: req.api_key,
                    is_default_for_protocol: req.is_default_for_protocol,
                },
            )
            .await
            .map_err(db_err)?;
        Ok(dc_to_response(&dc))
    }

    async fn delete(&self, id: DownloadClientId) -> Result<(), ApiError> {
        self.db.delete_download_client(id).await.map_err(db_err)
    }

    async fn test(&self, _req: CreateDownloadClientApiRequest) -> Result<(), ApiError> {
        Ok(()) // Test connection is a no-op in test mode
    }
}

// RemotePathMappingApi
impl RemotePathMappingApi for SecondaryApiImpl {
    async fn list(&self) -> Result<Vec<RemotePathMappingResponse>, ApiError> {
        let mappings = self.db.list_remote_path_mappings().await.map_err(db_err)?;
        Ok(mappings.iter().map(rpm_to_response).collect())
    }

    async fn create(
        &self,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMappingResponse, ApiError> {
        if !remote_path.ends_with('/') || !local_path.ends_with('/') {
            return Err(ApiError::Validation {
                errors: vec![FieldError {
                    field: "path".into(),
                    message: "both paths must end with /".into(),
                }],
            });
        }
        let m = self
            .db
            .create_remote_path_mapping(host, remote_path, local_path)
            .await
            .map_err(db_err)?;
        Ok(rpm_to_response(&m))
    }

    async fn get(&self, id: RemotePathMappingId) -> Result<RemotePathMappingResponse, ApiError> {
        let m = self.db.get_remote_path_mapping(id).await.map_err(db_err)?;
        Ok(rpm_to_response(&m))
    }

    async fn update(
        &self,
        id: RemotePathMappingId,
        req: UpdateRemotePathMappingRequest,
    ) -> Result<RemotePathMappingResponse, ApiError> {
        let existing = self.db.get_remote_path_mapping(id).await.map_err(db_err)?;
        let remote = req.remote_path.as_deref().unwrap_or(&existing.remote_path);
        let local = req.local_path.as_deref().unwrap_or(&existing.local_path);
        if !remote.ends_with('/') || !local.ends_with('/') {
            return Err(ApiError::Validation {
                errors: vec![FieldError {
                    field: "path".into(),
                    message: "both paths must end with /".into(),
                }],
            });
        }
        let host = req.host.as_deref().unwrap_or(&existing.host);
        let m = self
            .db
            .update_remote_path_mapping(id, host, remote, local)
            .await
            .map_err(db_err)?;
        Ok(rpm_to_response(&m))
    }

    async fn delete(&self, id: RemotePathMappingId) -> Result<(), ApiError> {
        self.db.delete_remote_path_mapping(id).await.map_err(db_err)
    }
}

// ConfigApi
impl ConfigApi for SecondaryApiImpl {
    async fn get_naming(&self) -> Result<NamingConfigResponse, ApiError> {
        let c = self.db.get_naming_config().await.map_err(db_err)?;
        Ok(NamingConfigResponse {
            author_folder_format: c.author_folder_format,
            book_folder_format: c.book_folder_format,
            rename_files: c.rename_files,
            replace_illegal_chars: c.replace_illegal_chars,
        })
    }

    async fn get_media_management(&self) -> Result<MediaManagementConfigResponse, ApiError> {
        let c = self
            .db
            .get_media_management_config()
            .await
            .map_err(db_err)?;
        Ok(MediaManagementConfigResponse {
            cwa_ingest_path: c.cwa_ingest_path,
            preferred_ebook_formats: c.preferred_ebook_formats,
            preferred_audiobook_formats: c.preferred_audiobook_formats,
        })
    }

    async fn update_media_management(
        &self,
        req: UpdateMediaManagementApiRequest,
    ) -> Result<MediaManagementConfigResponse, ApiError> {
        let c = self
            .db
            .update_media_management_config(UpdateMediaManagementConfigRequest {
                cwa_ingest_path: req.cwa_ingest_path,
                preferred_ebook_formats: req.preferred_ebook_formats,
                preferred_audiobook_formats: req.preferred_audiobook_formats,
            })
            .await
            .map_err(db_err)?;
        Ok(MediaManagementConfigResponse {
            cwa_ingest_path: c.cwa_ingest_path,
            preferred_ebook_formats: c.preferred_ebook_formats,
            preferred_audiobook_formats: c.preferred_audiobook_formats,
        })
    }

    async fn get_prowlarr(&self) -> Result<ProwlarrConfigResponse, ApiError> {
        let c = self.db.get_prowlarr_config().await.map_err(db_err)?;
        Ok(ProwlarrConfigResponse {
            url: c.url,
            api_key_set: c.api_key.is_some(),
            enabled: c.enabled,
        })
    }

    async fn update_prowlarr(
        &self,
        req: UpdateProwlarrApiRequest,
    ) -> Result<ProwlarrConfigResponse, ApiError> {
        let c = self
            .db
            .update_prowlarr_config(UpdateProwlarrConfigRequest {
                url: req.url,
                api_key: req.api_key,
                enabled: req.enabled,
            })
            .await
            .map_err(db_err)?;
        Ok(ProwlarrConfigResponse {
            url: c.url,
            api_key_set: c.api_key.is_some(),
            enabled: c.enabled,
        })
    }

    async fn test_prowlarr(&self, _req: &TestProwlarrRequest) -> Result<(), ApiError> {
        Ok(())
    }

    async fn get_metadata(&self) -> Result<MetadataConfigResponse, ApiError> {
        let c = self.db.get_metadata_config().await.map_err(db_err)?;
        Ok(MetadataConfigResponse {
            hardcover_enabled: c.hardcover_enabled,
            hardcover_api_token_set: c.hardcover_api_token.is_some(),
            llm_enabled: c.llm_enabled,
            llm_provider: c.llm_provider,
            llm_endpoint: c.llm_endpoint,
            llm_api_key_set: c.llm_api_key.is_some(),
            llm_model: c.llm_model,
            audnexus_url: c.audnexus_url,
            languages: c.languages,
            provider_status: std::collections::HashMap::new(),
        })
    }

    async fn update_metadata(
        &self,
        req: UpdateMetadataApiRequest,
    ) -> Result<MetadataConfigResponse, ApiError> {
        let c = self
            .db
            .update_metadata_config(UpdateMetadataConfigRequest {
                hardcover_enabled: req.hardcover_enabled,
                hardcover_api_token: req.hardcover_api_token,
                llm_enabled: req.llm_enabled,
                llm_provider: req.llm_provider,
                llm_endpoint: req.llm_endpoint,
                llm_api_key: req.llm_api_key,
                llm_model: req.llm_model,
                audnexus_url: req.audnexus_url,
                languages: req.languages,
            })
            .await
            .map_err(db_err)?;
        Ok(MetadataConfigResponse {
            hardcover_enabled: c.hardcover_enabled,
            hardcover_api_token_set: c.hardcover_api_token.is_some(),
            llm_enabled: c.llm_enabled,
            llm_provider: c.llm_provider,
            llm_endpoint: c.llm_endpoint,
            llm_api_key_set: c.llm_api_key.is_some(),
            llm_model: c.llm_model,
            audnexus_url: c.audnexus_url,
            languages: c.languages,
            provider_status: std::collections::HashMap::new(),
        })
    }

    async fn get_email(&self) -> Result<EmailConfigResponse, ApiError> {
        let c = self.db.get_email_config().await.map_err(db_err)?;
        Ok(EmailConfigResponse {
            enabled: c.enabled,
            smtp_host: c.smtp_host,
            smtp_port: c.smtp_port,
            encryption: c.encryption,
            username: c.username,
            password_set: c.password.is_some(),
            from_address: c.from_address,
            recipient_email: c.recipient_email,
            send_on_import: c.send_on_import,
        })
    }

    async fn update_email(
        &self,
        req: UpdateEmailApiRequest,
    ) -> Result<EmailConfigResponse, ApiError> {
        let c = self
            .db
            .update_email_config(UpdateEmailConfigRequest {
                enabled: req.enabled,
                smtp_host: req.smtp_host,
                smtp_port: req.smtp_port,
                encryption: req.encryption,
                username: req.username,
                password: req.password,
                from_address: req.from_address,
                recipient_email: req.recipient_email,
                send_on_import: req.send_on_import,
            })
            .await
            .map_err(db_err)?;
        Ok(EmailConfigResponse {
            enabled: c.enabled,
            smtp_host: c.smtp_host,
            smtp_port: c.smtp_port,
            encryption: c.encryption,
            username: c.username,
            password_set: c.password.is_some(),
            from_address: c.from_address,
            recipient_email: c.recipient_email,
            send_on_import: c.send_on_import,
        })
    }
}

// SystemApi
impl SystemApi for SecondaryApiImpl {
    async fn health(&self) -> Result<Vec<HealthCheckResult>, ApiError> {
        Ok(vec![HealthCheckResult {
            source: "database".into(),
            check_type: HealthCheckType::Ok,
            message: "in-memory DB operational".into(),
        }])
    }

    async fn status(&self) -> Result<SystemStatus, ApiError> {
        Ok(SystemStatus {
            version: "0.1.0-test".into(),
            os_info: "Linux test".into(),
            data_directory: self.data_dir.clone(),
            log_file: format!("{}/logs/livrarr.txt", self.data_dir),
            startup_time: Utc::now(),
            log_level: "info".into(),
        })
    }
}

// LibraryFileApi
impl LibraryFileApi for SecondaryApiImpl {
    async fn list(&self, uid: UserId) -> Result<Vec<LibraryItemResponse>, ApiError> {
        let items = self.db.list_library_items(uid).await.map_err(db_err)?;
        Ok(items.iter().map(li_to_response).collect())
    }

    async fn get(&self, uid: UserId, id: LibraryItemId) -> Result<LibraryItemResponse, ApiError> {
        let item = self.db.get_library_item(uid, id).await.map_err(db_err)?;
        Ok(li_to_response(&item))
    }

    async fn delete(&self, uid: UserId, id: LibraryItemId) -> Result<(), ApiError> {
        let item = self.db.delete_library_item(uid, id).await.map_err(db_err)?;
        // Best-effort file delete
        let _ = tokio::fs::remove_file(&item.path).await;
        Ok(())
    }
}

// HistoryApi
impl HistoryApi for SecondaryApiImpl {
    async fn list(
        &self,
        uid: UserId,
        _target_uid: Option<UserId>,
        filter: HistoryFilter,
    ) -> Result<Vec<HistoryResponse>, ApiError> {
        let mut events = self.db.list_history(uid, filter).await.map_err(db_err)?;
        events.sort_by(|a, b| b.date.cmp(&a.date)); // descending
        Ok(events
            .iter()
            .map(|e| HistoryResponse {
                id: e.id,
                work_id: e.work_id,
                event_type: e.event_type,
                data: e.data.clone(),
                date: e.date.to_rfc3339(),
            })
            .collect())
    }
}

// Helper functions
fn author_to_response(a: &Author) -> AuthorResponse {
    AuthorResponse {
        id: a.id,
        name: a.name.clone(),
        sort_name: a.sort_name.clone(),
        ol_key: a.ol_key.clone(),
        gr_key: a.gr_key.clone(),
        monitored: a.monitored,
        monitor_new_items: a.monitor_new_items,
        added_at: a.added_at.to_rfc3339(),
    }
}

fn dc_to_response(dc: &DownloadClient) -> DownloadClientResponse {
    DownloadClientResponse {
        id: dc.id,
        name: dc.name.clone(),
        implementation: dc.implementation,
        host: dc.host.clone(),
        port: dc.port,
        use_ssl: dc.use_ssl,
        skip_ssl_validation: dc.skip_ssl_validation,
        url_base: dc.url_base.clone(),
        username: dc.username.clone(),
        category: dc.category.clone(),
        enabled: dc.enabled,
        client_type: dc.client_type().to_string(),
        api_key_set: dc.api_key.is_some(),
        is_default_for_protocol: dc.is_default_for_protocol,
    }
}

fn rpm_to_response(m: &RemotePathMapping) -> RemotePathMappingResponse {
    RemotePathMappingResponse {
        id: m.id,
        host: m.host.clone(),
        remote_path: m.remote_path.clone(),
        local_path: m.local_path.clone(),
    }
}

fn li_to_response(li: &LibraryItem) -> LibraryItemResponse {
    LibraryItemResponse {
        id: li.id,
        path: li.path.clone(),
        media_type: li.media_type,
        file_size: li.file_size,
        imported_at: li.imported_at.to_rfc3339(),
    }
}

fn validate_download_client(req: &CreateDownloadClientApiRequest) -> Result<(), ApiError> {
    let mut errors = Vec::new();
    if req.name.is_empty() {
        errors.push(FieldError {
            field: "name".into(),
            message: "required".into(),
        });
    }
    if req.host.is_empty() {
        errors.push(FieldError {
            field: "host".into(),
            message: "required".into(),
        });
    }
    if req.category.contains('\\')
        || req.category.contains("//")
        || req.category.starts_with('/')
        || req.category.ends_with('/')
    {
        errors.push(FieldError {
            field: "category".into(),
            message: "invalid category format".into(),
        });
    }
    if !errors.is_empty() {
        return Err(ApiError::Validation { errors });
    }
    Ok(())
}

fn get_disk_space(path: &str) -> (Option<i64>, Option<i64>) {
    // Use statvfs on Linux
    #[cfg(unix)]
    {
        use std::ffi::CString;
        if let Ok(c_path) = CString::new(path) {
            unsafe {
                let mut stat: libc::statvfs = std::mem::zeroed();
                if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
                    let free = (stat.f_bavail as i64) * (stat.f_frsize as i64);
                    let total = (stat.f_blocks as i64) * (stat.f_frsize as i64);
                    return (Some(free), Some(total));
                }
            }
        }
    }
    (None, None)
}

#[cfg(test)]
impl SecondaryApiImpl {
    pub async fn create_author_without_ol_key(&self, user_id: UserId) -> AuthorId {
        let author = self
            .db
            .create_author(CreateAuthorDbRequest {
                user_id,
                name: format!(
                    "NoOL-Author-{}",
                    Utc::now().timestamp_nanos_opt().unwrap_or(0)
                ),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: None,
            })
            .await
            .unwrap();
        author.id
    }

    pub async fn create_test_notification(&self, user_id: UserId, ref_key: &str) -> NotificationId {
        let n = self
            .db
            .create_notification(CreateNotificationDbRequest {
                user_id,
                notification_type: NotificationType::NewWorkDetected,
                ref_key: Some(ref_key.to_string()),
                message: format!("Test notification {ref_key}"),
                data: serde_json::json!({}),
            })
            .await
            .unwrap();
        n.id
    }

    pub async fn create_test_library_item(
        &self,
        user_id: UserId,
        root_folder_id: RootFolderId,
    ) -> LibraryItemId {
        // Ensure a work exists
        let work = self
            .db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: format!("Work-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)),
                author_name: "Test Author".into(),
                author_id: None,
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                metadata_source: None,
                detail_url: None,
                language: None,
                import_id: None,
                series_id: None,
                series_name: None,
                series_position: None,
                monitor_ebook: false,
                monitor_audiobook: false,
            })
            .await
            .unwrap();
        let item = self
            .db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id,
                work_id: work.id,
                root_folder_id,
                path: format!(
                    "test-{}.epub",
                    Utc::now().timestamp_nanos_opt().unwrap_or(0)
                ),
                media_type: MediaType::Ebook,
                file_size: 1234,
                import_id: None,
            })
            .await
            .unwrap();
        item.id
    }

    pub async fn create_test_library_file(&self, user_id: UserId) -> (LibraryItemId, String) {
        // Create a temp file
        let tmp = std::env::temp_dir().join(format!(
            "livrarr-test-{}.epub",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::write(&tmp, b"test content").unwrap();
        let path_str = tmp.to_str().unwrap().to_string();
        // Create root folder if needed
        let rf = match self
            .db
            .get_root_folder_by_media_type(MediaType::Ebook)
            .await
            .unwrap()
        {
            Some(rf) => rf,
            None => self
                .db
                .create_root_folder("/tmp", MediaType::Ebook)
                .await
                .unwrap(),
        };
        let work = self
            .db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: "File Test Work".into(),
                author_name: "File Author".into(),
                author_id: None,
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                metadata_source: None,
                detail_url: None,
                language: None,
                import_id: None,
                series_id: None,
                series_name: None,
                series_position: None,
                monitor_ebook: false,
                monitor_audiobook: false,
            })
            .await
            .unwrap();
        let item = self
            .db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id,
                work_id: work.id,
                root_folder_id: rf.id,
                path: path_str.clone(),
                media_type: MediaType::Ebook,
                file_size: 12,
                import_id: None,
            })
            .await
            .unwrap();
        (item.id, path_str)
    }
}

#[cfg(test)]
/// Create a secondary API backed by a SQLite :memory: DB with a test user.
pub async fn new_test_secondary_api() -> (SecondaryApiImpl, UserId) {
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user = db
        .create_user(CreateUserDbRequest {
            username: "testuser".into(),
            password_hash: "hash".into(),
            role: UserRole::Admin,
            api_key_hash: "apikey".into(),
        })
        .await
        .unwrap();
    (SecondaryApiImpl::new(db), user.id)
}
