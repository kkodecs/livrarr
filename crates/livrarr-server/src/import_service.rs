use std::path::PathBuf;
use std::sync::Arc;

use livrarr_domain::services::{
    AppConfigService, ImportFileResult, ImportGrabResult, ImportService, ImportSingleFileRequest,
    ServiceError,
};
use livrarr_domain::MediaType;
use livrarr_http::HttpClient;

use crate::state::{LiveImportIoService, LiveImportWorkflow, LiveSettingsService};

#[derive(Clone)]
pub struct LiveImportService {
    import_io: Arc<LiveImportIoService>,
    import_workflow: Arc<LiveImportWorkflow>,
    tag_service: Arc<crate::tag_service::LiveTagService<LiveImportIoService>>,
    settings_service: Arc<LiveSettingsService>,
    http_client_safe: HttpClient,
    data_dir: Arc<PathBuf>,
}

impl LiveImportService {
    pub fn new(
        import_io: Arc<LiveImportIoService>,
        import_workflow: Arc<LiveImportWorkflow>,
        tag_service: Arc<crate::tag_service::LiveTagService<LiveImportIoService>>,
        settings_service: Arc<LiveSettingsService>,
        http_client_safe: HttpClient,
        data_dir: Arc<PathBuf>,
    ) -> Self {
        Self {
            import_io,
            import_workflow,
            tag_service,
            settings_service,
            http_client_safe,
            data_dir,
        }
    }
}

impl ImportService for LiveImportService {
    async fn import_grab(
        &self,
        user_id: i64,
        grab_id: i64,
    ) -> Result<ImportGrabResult, ServiceError> {
        use crate::infra::email;
        use crate::infra::import_pipeline;
        use std::path::Path;

        // Pre-service: ensure content_path is populated.
        // The download poller persists content_path when confirming a download.
        // For manual retries, content_path may be missing — resolve from the
        // download client.
        use livrarr_domain::services::ImportIoService;
        let grab = self
            .import_io
            .get_grab(user_id, grab_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        if grab.content_path.is_none() {
            if let Some(ref download_id) = grab.download_id {
                let client = self
                    .import_io
                    .get_download_client(grab.download_client_id)
                    .await
                    .map_err(|e| ServiceError::Internal(e.to_string()))?;
                let content_path = if client.client_type() == "sabnzbd" {
                    import_pipeline::fetch_sabnzbd_storage_path(
                        &self.http_client_safe,
                        &client,
                        download_id,
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(e.to_string()))?
                } else {
                    import_pipeline::fetch_qbit_content_path(
                        &self.http_client_safe,
                        &client,
                        download_id,
                    )
                    .await
                    .map_err(|e| ServiceError::Internal(e.to_string()))?
                };
                self.import_io
                    .set_grab_content_path(user_id, grab_id, &content_path)
                    .await
                    .map_err(|e| ServiceError::Internal(e.to_string()))?;
            }
        }

        // Service handles: source resolution, enumeration, format filtering,
        // file copy, library item creation, status update, history event.
        use livrarr_domain::services::ImportWorkflow;
        let result = self
            .import_workflow
            .import_grab(user_id, grab_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        let mut warnings = result.warnings;

        // Post-service I/O: tag imported files + CWA copy + email.
        if !result.imported_files.is_empty() {
            let work = self
                .import_io
                .get_work(user_id, grab.work_id)
                .await
                .map_err(|e| ServiceError::Internal(e.to_string()))?;

            // Tag writing — retag the just-imported files if enrichment data available.
            if work.enrichment_status != livrarr_domain::EnrichmentStatus::Pending {
                let items = self
                    .import_io
                    .list_library_items_by_work(user_id, work.id)
                    .await
                    .unwrap_or_default();
                let imported_ids: std::collections::HashSet<i64> = result
                    .imported_files
                    .iter()
                    .map(|f| f.library_item_id)
                    .collect();
                let matching: Vec<_> = items
                    .iter()
                    .filter(|i| imported_ids.contains(&i.id))
                    .cloned()
                    .collect();
                if !matching.is_empty() {
                    use livrarr_domain::services::TagService;
                    let tag_warnings = self.tag_service.retag_library_items(&work, &matching).await;
                    warnings.extend(tag_warnings);
                }
            }

            // CWA copy + email — fire-and-forget for ebooks.
            let media_mgmt = self
                .settings_service
                .get_media_management_config()
                .await
                .ok();
            let root_folders = self.import_io.list_root_folders().await.unwrap_or_default();
            for imp in &result.imported_files {
                if imp.media_type != MediaType::Ebook {
                    continue;
                }
                let rf = match root_folders
                    .iter()
                    .find(|rf| rf.media_type == MediaType::Ebook)
                {
                    Some(rf) => rf,
                    None => continue,
                };
                let abs_path = format!("{}/{}", rf.path, imp.target_relative_path);

                // CWA
                if let Some(ref mgmt) = media_mgmt {
                    if let Some(ref cwa_path) = mgmt.cwa_ingest_path {
                        let ext = Path::new(&imp.target_relative_path)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("epub")
                            .to_string();
                        let work = self.import_io.get_work(user_id, grab.work_id).await.ok();
                        if let Some(work) = work {
                            let tp = abs_path.clone();
                            let cwa = cwa_path.clone();
                            let auth = work.author_name.clone();
                            let t = work.title.clone();
                            let cwa_result = tokio::task::spawn_blocking(move || {
                                import_pipeline::cwa_copy(&tp, &cwa, user_id, &auth, &t, &ext)
                            })
                            .await
                            .ok()
                            .flatten();
                            if let Some(warn) = cwa_result {
                                warnings.push(warn);
                            }
                        }
                    }
                }

                // Auto-send to email/Kindle
                if let Ok(email_cfg) = self.settings_service.get_email_config().await {
                    if email_cfg.send_on_import && email_cfg.enabled {
                        let ext = Path::new(&imp.target_relative_path)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if email::ACCEPTED_EXTENSIONS.contains(&ext.as_str())
                            && (imp.file_size as i64) <= email::MAX_EMAIL_SIZE
                        {
                            match tokio::fs::read(&abs_path).await {
                                Ok(bytes) => {
                                    let filename = Path::new(&abs_path)
                                        .file_name()
                                        .and_then(|f| f.to_str())
                                        .unwrap_or("book");
                                    if let Err(e) =
                                        email::send_file(&email_cfg, bytes, filename, &ext).await
                                    {
                                        tracing::warn!(file = %abs_path, "Auto-send email failed: {e}");
                                    } else {
                                        tracing::info!(file = %abs_path, "Auto-sent to email on import");
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(file = %abs_path, "Auto-send: failed to read file: {e}");
                                }
                            }
                        }
                    }
                }
            }
        }

        let error_msg = if result.failed_files.is_empty() {
            None
        } else {
            Some(
                result
                    .failed_files
                    .iter()
                    .map(|f| f.error.as_str())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        };

        Ok(ImportGrabResult {
            final_status: result.final_status,
            imported_count: result.imported_files.len(),
            failed_count: result.failed_files.len(),
            skipped_count: result.skipped_files.len(),
            warnings,
            error: error_msg,
        })
    }

    async fn import_single_file(&self, req: ImportSingleFileRequest) -> ImportFileResult {
        use livrarr_domain::services::ImportIoService;
        let work = match self.import_io.get_work(req.user_id, req.work_id).await {
            Ok(w) => w,
            Err(e) => return ImportFileResult::Failed(format!("failed to load work: {e}")),
        };

        let tag_metadata = crate::infra::import_pipeline::build_tag_metadata(&work);
        let cover_data = crate::infra::import_pipeline::read_cover_bytes(
            &self.data_dir,
            req.user_id,
            req.work_id,
        )
        .await;

        let media_mgmt = match self.settings_service.get_media_management_config().await {
            Ok(cfg) => cfg,
            Err(e) => return ImportFileResult::Failed(format!("failed to load media config: {e}")),
        };

        match crate::infra::import_pipeline::import_single_file(
            &*self.import_io,
            &*self.settings_service,
            &req.source,
            &req.target_path,
            &req.root_folder_path,
            req.root_folder_id,
            req.media_type,
            req.user_id,
            req.work_id,
            Some(&tag_metadata),
            cover_data.as_deref(),
            &media_mgmt,
            &req.author_name,
            &req.title,
        )
        .await
        {
            Ok(()) => ImportFileResult::Ok,
            Err(crate::infra::import_pipeline::ImportFileError::Warning(w)) => {
                ImportFileResult::Warning(w)
            }
            Err(crate::infra::import_pipeline::ImportFileError::Failed(e)) => {
                ImportFileResult::Failed(e)
            }
        }
    }

    fn build_target_path(
        &self,
        root_folder_path: &str,
        user_id: i64,
        author: &str,
        title: &str,
        media_type: MediaType,
        source: &std::path::Path,
        source_root: &std::path::Path,
    ) -> String {
        crate::infra::import_pipeline::build_target_path(
            root_folder_path,
            user_id,
            author,
            title,
            media_type,
            source,
            source_root,
        )
    }
}
