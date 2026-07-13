use std::path::{Path, PathBuf};
use std::sync::Arc;

use livrarr_domain::services::{
    AdoptScannedFileRequest, AppConfigService, ImportFileOutcome, ImportFileRequest,
    ImportFileResult, ImportGrabResult, ImportService, ImportSingleFileRequest,
    ImportWorkflowError, Materialization, ServiceError,
};
use livrarr_domain::{MediaType, Work};
use livrarr_http::HttpClient;

use crate::infra::email;
use crate::infra::import_pipeline::cwa_copy;
use crate::state::{LiveImportIoService, LiveImportWorkflow, LiveSettingsService};

enum ImportFileError {
    Warning(String), // file imported but tag failed
    Failed(String),  // file not imported
}

#[derive(Clone)]
pub struct LiveImportService {
    import_io: Arc<LiveImportIoService>,
    import_workflow: Arc<LiveImportWorkflow>,
    tag_service: Arc<crate::tag_service::LiveTagService<LiveImportIoService>>,
    settings_service: Arc<LiveSettingsService>,
    http_client_safe: HttpClient,
}

impl LiveImportService {
    pub fn new(
        import_io: Arc<LiveImportIoService>,
        import_workflow: Arc<LiveImportWorkflow>,
        tag_service: Arc<crate::tag_service::LiveTagService<LiveImportIoService>>,
        settings_service: Arc<LiveSettingsService>,
        http_client_safe: HttpClient,
    ) -> Self {
        Self {
            import_io,
            import_workflow,
            tag_service,
            settings_service,
            http_client_safe,
        }
    }
}

impl LiveImportService {
    #[allow(clippy::too_many_arguments)]
    async fn do_import_single_file(
        &self,
        source: &Path,
        target_path: &str,
        root_folder_path: &str,
        root_folder_id: i64,
        media_type: MediaType,
        user_id: i64,
        work_id: i64,
        work: &Work,
        import_id: Option<String>,
        media_mgmt: &livrarr_db::MediaManagementConfig,
        author_name: &str,
        title: &str,
    ) -> Result<ImportFileOutcome, ImportFileError> {
        let target_relative = target_path
            .strip_prefix(root_folder_path)
            .unwrap_or(target_path)
            .trim_start_matches('/')
            .to_string();

        use livrarr_domain::services::ImportWorkflow;
        let outcome = self
            .import_workflow
            .import_file(
                user_id,
                ImportFileRequest {
                    work_id,
                    root_folder_id,
                    source: source.to_path_buf(),
                    target_relative,
                    media_type,
                    materialization: Materialization::Copy,
                    import_id,
                    extract_chapters: true,
                },
            )
            .await
            .map_err(|e| ImportFileError::Failed(format!("{}: {e}", source.display())))?;

        let item_id = match &outcome {
            ImportFileOutcome::Skipped { .. } => return Ok(outcome),
            ImportFileOutcome::Imported { item_id, .. } => *item_id,
            ImportFileOutcome::Adopted { item_id, .. } => *item_id,
        };

        // Retag unconditionally — mirrors this door's prior ungated
        // tag-on-import behavior (build_tag_metadata was always called
        // regardless of enrichment status), now routed through TagService
        // instead of the old inline write_tags call. Same post-step shape as
        // import_grab (:353), minus the enrichment-status gate.
        use livrarr_domain::services::{ImportIoService, TagService};
        let mut tag_warning: Option<String> = None;
        let items = self
            .import_io
            .list_library_items_by_work(user_id, work_id)
            .await
            .unwrap_or_default();
        if let Some(item) = items.iter().find(|i| i.id == item_id) {
            let tag_results = self
                .tag_service
                .retag_library_items(work, std::slice::from_ref(item))
                .await;
            if let Some(result) = tag_results.into_iter().find(|r| !r.succeeded) {
                tag_warning = Some(format!(
                    "tag write failed for {}: {}",
                    source.display(),
                    result.error.unwrap_or_else(|| "unknown error".to_string())
                ));
            }
        }

        // CWA integration (ebooks only, non-fatal).
        if media_type == MediaType::Ebook {
            if let Some(ref cwa_path) = media_mgmt.cwa_ingest_path {
                let ext = source
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("epub")
                    .to_string();
                let tp = target_path.to_string();
                let cwa = cwa_path.clone();
                let auth = author_name.to_string();
                let t = title.to_string();
                let cwa_result = tokio::task::spawn_blocking(move || {
                    cwa_copy(&tp, &cwa, user_id, &auth, &t, &ext)
                })
                .await
                .ok()
                .flatten();
                if let Some(warn) = cwa_result {
                    // CWA warning doesn't fail the import.
                    return Err(ImportFileError::Warning(warn));
                }
            }
        }

        // Auto-send to email/Kindle on import (ebooks only, non-fatal).
        if media_type == MediaType::Ebook {
            if let Ok(email_cfg) = self.settings_service.get_email_config().await {
                if email_cfg.send_on_import && email_cfg.enabled {
                    let ext = source
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let file_size = Path::new(target_path)
                        .metadata()
                        .map(|m| m.len() as i64)
                        .unwrap_or(0);
                    if email::ACCEPTED_EXTENSIONS.contains(&ext.as_str())
                        && file_size <= email::MAX_EMAIL_SIZE
                    {
                        let target_str = target_path.to_string();
                        match tokio::fs::read(&target_str).await {
                            Ok(bytes) => {
                                let filename = std::path::Path::new(&target_str)
                                    .file_name()
                                    .and_then(|f| f.to_str())
                                    .unwrap_or("book");
                                if let Err(e) =
                                    email::send_file(&email_cfg, bytes, filename, &ext).await
                                {
                                    tracing::warn!(file = %target_str, "Auto-send email failed: {e}");
                                } else {
                                    tracing::info!(file = %target_str, "Auto-sent to email on import");
                                }
                            }
                            Err(e) => {
                                tracing::warn!(file = %target_str, "Auto-send: failed to read file: {e}");
                            }
                        }
                    }
                }
            }
        }

        match tag_warning {
            Some(w) => Err(ImportFileError::Warning(w)),
            None => Ok(outcome),
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
            if work.enrichment_status != livrarr_domain::EnrichmentStatus::Unenriched {
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
                    let tag_results = self.tag_service.retag_library_items(&work, &matching).await;
                    warnings.extend(tag_results.into_iter().filter(|r| !r.succeeded).map(|r| {
                        format!(
                            "tag rewrite warning (item {}): {}",
                            r.library_item_id,
                            r.error.unwrap_or_else(|| "unknown error".to_string())
                        )
                    }));
                }
            }

            // CWA copy + email — fire-and-forget for ebooks.
            let media_mgmt = match self.settings_service.get_media_management_config().await {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    tracing::warn!(
                        "media management config read failed — CWA ingest skipped this import: {e}"
                    );
                    None
                }
            };
            let root_folders = match self.import_io.list_root_folders().await {
                Ok(rf) => rf,
                Err(e) => {
                    tracing::warn!(
                        "root folder list read failed — CWA/email skipped this import: {e}"
                    );
                    Vec::new()
                }
            };
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
                        let work = match self.import_io.get_work(user_id, grab.work_id).await {
                            Ok(w) => Some(w),
                            Err(e) => {
                                tracing::warn!(
                                    work_id = grab.work_id,
                                    "CWA copy skipped: work read failed: {e}"
                                );
                                None
                            }
                        };
                        if let Some(work) = work {
                            let tp = abs_path.clone();
                            let cwa = cwa_path.clone();
                            let auth = work.author_name.clone();
                            let t = work.title.clone();
                            let cwa_result = match tokio::task::spawn_blocking(move || {
                                import_pipeline::cwa_copy(&tp, &cwa, user_id, &auth, &t, &ext)
                            })
                            .await
                            {
                                Ok(r) => r,
                                Err(e) => {
                                    tracing::warn!("CWA copy task failed to run: {e}");
                                    None
                                }
                            };
                            if let Some(warn) = cwa_result {
                                warnings.push(warn);
                            }
                        }
                    }
                }

                // Auto-send to email/Kindle
                let email_cfg = match self.settings_service.get_email_config().await {
                    Ok(cfg) => Some(cfg),
                    Err(e) => {
                        tracing::warn!(
                            "email config read failed — auto-send skipped this import: {e}"
                        );
                        None
                    }
                };
                if let Some(email_cfg) = email_cfg {
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

        let media_mgmt = match self.settings_service.get_media_management_config().await {
            Ok(cfg) => cfg,
            Err(e) => return ImportFileResult::Failed(format!("failed to load media config: {e}")),
        };

        match self
            .do_import_single_file(
                &req.source,
                &req.target_path,
                &req.root_folder_path,
                req.root_folder_id,
                req.media_type,
                req.user_id,
                req.work_id,
                &work,
                req.import_id.clone(),
                &media_mgmt,
                &req.author_name,
                &req.title,
            )
            .await
        {
            Ok(ImportFileOutcome::Skipped { .. }) => {
                ImportFileResult::Skipped(format!("{} already imported", req.target_path))
            }
            Ok(_) => ImportFileResult::Ok,
            Err(ImportFileError::Warning(w)) => ImportFileResult::Warning(w),
            Err(ImportFileError::Failed(e)) => ImportFileResult::Failed(e),
        }
    }

    async fn adopt_scanned_file(
        &self,
        user_id: i64,
        req: AdoptScannedFileRequest,
    ) -> Result<ImportFileOutcome, ImportWorkflowError> {
        use livrarr_domain::services::ImportWorkflow;
        self.import_workflow
            .import_file(
                user_id,
                ImportFileRequest {
                    work_id: req.work_id,
                    root_folder_id: req.root_folder_id,
                    source: req.path,
                    target_relative: req.target_relative,
                    media_type: req.media_type,
                    materialization: Materialization::AdoptInPlace,
                    import_id: None,
                    extract_chapters: false,
                },
            )
            .await
    }

    async fn reorganize_work_files(
        &self,
        user_id: i64,
        work_id: i64,
    ) -> Result<Vec<String>, ServiceError> {
        use livrarr_domain::services::ImportIoService;

        let work = self.import_io.get_work(user_id, work_id).await?;
        let items = self
            .import_io
            .list_library_items_by_work(user_id, work_id)
            .await?;
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let root_folders = self.import_io.list_root_folders().await?;

        let (mut warnings, moved_items) =
            reorganize_items(&*self.import_io, user_id, &work, items, &root_folders).await;

        if !moved_items.is_empty()
            && work.enrichment_status != livrarr_domain::EnrichmentStatus::Unenriched
        {
            use livrarr_domain::services::TagService;
            let tag_results = self
                .tag_service
                .retag_library_items(&work, &moved_items)
                .await;
            warnings.extend(tag_results.into_iter().filter(|r| !r.succeeded).map(|r| {
                format!(
                    "tag rewrite warning (item {}): {}",
                    r.library_item_id,
                    r.error.unwrap_or_else(|| "unknown error".to_string())
                )
            }));
        }

        Ok(warnings)
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

/// How a file physically reached its new target during reorganization —
/// determines how a failed follow-up path-record update is reverted.
enum MoveMethod {
    /// `rename` moved it; revert = rename back.
    Renamed,
    /// Cross-device copy + source removal (the EXDEV fallback); revert =
    /// copy back (when the source was actually removed) and delete OUR
    /// copy at the target — never user data.
    Copied { original_removed: bool },
}

/// Per-item move-and-record loop behind
/// `LiveImportService::reorganize_work_files`, generic over the IO seam so
/// the record-failure/revert/heal behavior is testable with a
/// fault-injecting stub. Returns (warnings, items whose stored path changed
/// this pass — the retag set).
///
/// Consistency invariant: an item is never left unlocatable. Either the
/// file and its path record agree when an iteration ends, or the warning
/// names both locations and the mismatch is exactly the shape the heal
/// branch repairs on the next pass.
async fn reorganize_items<IO: livrarr_domain::services::ImportIoService>(
    io: &IO,
    user_id: i64,
    work: &livrarr_domain::Work,
    items: Vec<livrarr_domain::LibraryItem>,
    root_folders: &[livrarr_domain::RootFolder],
) -> (Vec<String>, Vec<livrarr_domain::LibraryItem>) {
    let mut warnings = Vec::new();
    let mut moved_items = Vec::new();

    for item in items {
        let Some(root) = root_folders.iter().find(|rf| rf.id == item.root_folder_id) else {
            warnings.push(format!(
                "{}: root folder no longer exists — left in place",
                item.path
            ));
            continue;
        };

        let current_abs = format!("{}/{}", root.path.trim_end_matches('/'), item.path);
        let current_path = Path::new(&current_abs);

        // Same naming/layout rule the import path uses — never
        // reimplemented here.
        let new_target = crate::infra::import_pipeline::build_target_path(
            &root.path,
            user_id,
            &work.author_name,
            &work.title,
            item.media_type,
            current_path,
            current_path,
        );
        if new_target == current_abs {
            continue; // already at its canonical path
        }
        let new_path = PathBuf::from(&new_target);
        let new_relative = new_target
            .strip_prefix(&root.path)
            .unwrap_or(&new_target)
            .trim_start_matches('/')
            .to_string();

        let source_exists = tokio::fs::try_exists(current_path).await.unwrap_or(false);
        let target_exists = tokio::fs::try_exists(&new_path).await.unwrap_or(false);

        // Self-healing: nothing at the recorded path but the computed target
        // is occupied — a previous pass moved the file and failed to record
        // it. Repair the record only; no file operation.
        if !source_exists && target_exists {
            match io
                .update_library_item_path(user_id, item.id, &new_relative)
                .await
            {
                Ok(()) => {
                    tracing::info!(
                        item_id = item.id,
                        "reorganize: healed stale path record — file was already at its target"
                    );
                }
                Err(e) => {
                    warnings.push(format!(
                        "{}: file is already at {new_target} but the path record could not be repaired: {e}",
                        item.path
                    ));
                }
            }
            continue;
        }

        if target_exists {
            // Never overwrite — the item is left at its current path,
            // still owned by the survivor in the DB. No file is ever
            // deleted or clobbered here.
            warnings.push(format!(
                "{}: destination already occupied — left at its current path",
                item.path
            ));
            continue;
        }

        if let Some(parent) = new_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                warnings.push(format!(
                    "{}: could not create destination directory: {e}",
                    item.path
                ));
                continue;
            }
        }

        let move_method = match tokio::fs::rename(&current_abs, &new_path).await {
            Ok(()) => MoveMethod::Renamed,
            Err(e) => {
                // Cross-device rename: fall back to copy-then-remove-source,
                // the same EXDEV posture the CWA copy path already uses.
                if e.raw_os_error() != Some(libc::EXDEV) {
                    warnings.push(format!("{}: move failed: {e}", item.path));
                    continue;
                }
                if let Err(e) = livrarr_library::atomic_copy(current_path, &new_path).await {
                    warnings.push(format!("{}: cross-device copy failed: {e}", item.path));
                    continue;
                }
                let original_removed = match tokio::fs::remove_file(&current_abs).await {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!(
                            item_id = item.id,
                            error = %e,
                            "reorganize: cross-device copy succeeded but removing the old file failed — duplicate left on disk"
                        );
                        false
                    }
                };
                MoveMethod::Copied { original_removed }
            }
        };

        match io
            .update_library_item_path(user_id, item.id, &new_relative)
            .await
        {
            Ok(()) => {
                let mut moved = item.clone();
                moved.path = new_relative;
                moved_items.push(moved);
            }
            Err(e) => {
                // The file moved but the record didn't. Put the file back
                // (undoing only our own move/copy) so record and disk agree
                // again and a plain retry works. If the revert itself
                // fails, the warning names BOTH locations and the heal
                // branch above repairs it on the next pass.
                warnings.push(
                    revert_physical_move(
                        &move_method,
                        &current_abs,
                        &new_path,
                        &item.path,
                        &new_target,
                        &e.to_string(),
                    )
                    .await,
                );
            }
        }
    }

    (warnings, moved_items)
}

/// Best-effort undo of a just-performed physical move after the path-record
/// update failed, so disk and record stay consistent. Returns the warning to
/// surface. When the revert itself fails, the warning carries BOTH paths —
/// the stale recorded location and the file's actual location — so recovery
/// (automatic, via the heal branch on the next pass, or manual) is
/// unambiguous.
async fn revert_physical_move(
    method: &MoveMethod,
    current_abs: &str,
    new_path: &Path,
    stored_relative: &str,
    new_target: &str,
    record_error: &str,
) -> String {
    match method {
        MoveMethod::Renamed => match tokio::fs::rename(new_path, current_abs).await {
            Ok(()) => format!(
                "{stored_relative}: path record update failed ({record_error}); \
                 the move was reverted — retry the reorganize later"
            ),
            Err(re) => format!(
                "{stored_relative}: path record update failed ({record_error}) and the \
                 revert also failed ({re}); the record still points at {current_abs} but \
                 the file is at {new_target} — the next reorganize heals this automatically"
            ),
        },
        MoveMethod::Copied {
            original_removed: true,
        } => match livrarr_library::atomic_copy(new_path, Path::new(current_abs)).await {
            Ok(_) => {
                let mut warning = format!(
                    "{stored_relative}: path record update failed ({record_error}); \
                     the file was copied back — retry the reorganize later"
                );
                if let Err(e) = tokio::fs::remove_file(new_path).await {
                    warning.push_str(&format!(
                        " (note: our extra copy at {new_target} could not be removed: {e})"
                    ));
                }
                warning
            }
            Err(re) => format!(
                "{stored_relative}: path record update failed ({record_error}) and the \
                 copy-back also failed ({re}); the record still points at {current_abs} but \
                 the file is at {new_target} — the next reorganize heals this automatically"
            ),
        },
        MoveMethod::Copied {
            original_removed: false,
        } => {
            // The original never left the recorded path (its removal failed
            // right after the copy), so the record is already consistent —
            // reverting means deleting only the extra copy WE created at
            // the target, never user data.
            match tokio::fs::remove_file(new_path).await {
                Ok(()) => format!(
                    "{stored_relative}: path record update failed ({record_error}); the \
                     file never left its recorded path and our extra copy was removed — \
                     retry the reorganize later"
                ),
                Err(re) => format!(
                    "{stored_relative}: path record update failed ({record_error}); the \
                     file is still at its recorded path {current_abs}, but an extra copy \
                     remains at {new_target} ({re})"
                ),
            }
        }
    }
}

#[cfg(test)]
mod reorganize_tests {
    use super::*;
    use livrarr_domain::services::{ImportIoService, ImportIoServiceError};
    use livrarr_domain::{
        DbError, DownloadClient, DownloadClientId, Grab, GrabId, LibraryItem, LibraryItemId,
        RemotePathMapping, RootFolder, RootFolderId, TagStatus, UserId, Work, WorkId,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// Fault-injecting stand-in for the IO seam: `update_library_item_path`
    /// fails a configured number of times, then succeeds and records the
    /// write. Every other method is unreachable from `reorganize_items`.
    struct StubIo {
        path_updates: Mutex<Vec<(LibraryItemId, String)>>,
        failures_left: AtomicUsize,
    }

    impl StubIo {
        fn failing(times: usize) -> Self {
            Self {
                path_updates: Mutex::new(Vec::new()),
                failures_left: AtomicUsize::new(times),
            }
        }
    }

    impl ImportIoService for StubIo {
        async fn update_library_item_path(
            &self,
            _user_id: UserId,
            item_id: LibraryItemId,
            new_path: &str,
        ) -> Result<(), ImportIoServiceError> {
            let should_fail = self
                .failures_left
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok();
            if should_fail {
                return Err(ImportIoServiceError::Db(DbError::Constraint {
                    message: "injected path-update failure".into(),
                }));
            }
            self.path_updates
                .lock()
                .unwrap()
                .push((item_id, new_path.to_string()));
            Ok(())
        }

        async fn get_grab(&self, _: UserId, _: GrabId) -> Result<Grab, ImportIoServiceError> {
            unreachable!("not used by reorganize_items")
        }

        async fn get_download_client(
            &self,
            _: DownloadClientId,
        ) -> Result<DownloadClient, ImportIoServiceError> {
            unreachable!("not used by reorganize_items")
        }

        async fn set_grab_content_path(
            &self,
            _: UserId,
            _: GrabId,
            _: &str,
        ) -> Result<(), ImportIoServiceError> {
            unreachable!("not used by reorganize_items")
        }

        async fn get_work(&self, _: UserId, _: WorkId) -> Result<Work, ImportIoServiceError> {
            unreachable!("not used by reorganize_items")
        }

        async fn list_library_items_by_work(
            &self,
            _: UserId,
            _: WorkId,
        ) -> Result<Vec<LibraryItem>, ImportIoServiceError> {
            unreachable!("not used by reorganize_items")
        }

        async fn get_root_folder(
            &self,
            _: RootFolderId,
        ) -> Result<RootFolder, ImportIoServiceError> {
            unreachable!("not used by reorganize_items")
        }

        async fn list_root_folders(&self) -> Result<Vec<RootFolder>, ImportIoServiceError> {
            unreachable!("not used by reorganize_items")
        }

        async fn list_remote_path_mappings(
            &self,
        ) -> Result<Vec<RemotePathMapping>, ImportIoServiceError> {
            unreachable!("not used by reorganize_items")
        }

        async fn update_library_item_size(
            &self,
            _: UserId,
            _: LibraryItemId,
            _: i64,
        ) -> Result<(), ImportIoServiceError> {
            unreachable!("not used by reorganize_items")
        }
    }

    fn test_item(id: i64, root_id: i64, path: &str) -> LibraryItem {
        LibraryItem {
            id,
            user_id: 1,
            work_id: 1,
            root_folder_id: root_id,
            path: path.into(),
            media_type: MediaType::Ebook,
            file_size: 4,
            import_id: None,
            imported_at: chrono::Utc::now(),
            tag_status: TagStatus::Pending,
            tagged_at_generation: 0,
            duration_seconds: None,
            chapter_scan_status: None,
        }
    }

    fn test_work() -> Work {
        Work {
            user_id: 1,
            title: "New Title".into(),
            author_name: "New Author".into(),
            ..Work::default()
        }
    }

    #[tokio::test]
    async fn record_failure_after_move_reverts_file_and_plain_retry_completes() {
        let dir = tempfile::tempdir().unwrap();
        let root_path = dir.path().to_string_lossy().into_owned();
        let root = RootFolder {
            id: 7,
            path: root_path.clone(),
            media_type: MediaType::Ebook,
        };

        let old_abs = format!("{root_path}/stale/old.epub");
        tokio::fs::create_dir_all(format!("{root_path}/stale"))
            .await
            .unwrap();
        tokio::fs::write(&old_abs, b"book").await.unwrap();
        let new_abs = format!("{root_path}/1/New Author/New Title.epub");

        let io = StubIo::failing(1);
        let work = test_work();

        let (warnings, moved) = reorganize_items(
            &io,
            1,
            &work,
            vec![test_item(11, 7, "stale/old.epub")],
            std::slice::from_ref(&root),
        )
        .await;

        // The failed record update reverted the physical move — disk and
        // record agree again (both at the old path); nothing entered the
        // retag set, and the item stayed locatable throughout.
        assert!(moved.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("reverted"),
            "warning was: {}",
            warnings[0]
        );
        assert!(tokio::fs::try_exists(&old_abs).await.unwrap());
        assert!(!tokio::fs::try_exists(&new_abs).await.unwrap());
        assert!(io.path_updates.lock().unwrap().is_empty());

        // With the fault cleared, a plain retry completes the move end to end.
        let (warnings2, moved2) = reorganize_items(
            &io,
            1,
            &work,
            vec![test_item(11, 7, "stale/old.epub")],
            std::slice::from_ref(&root),
        )
        .await;
        assert!(warnings2.is_empty(), "{warnings2:?}");
        assert_eq!(moved2.len(), 1);
        assert!(!tokio::fs::try_exists(&old_abs).await.unwrap());
        assert!(tokio::fs::try_exists(&new_abs).await.unwrap());
        assert_eq!(
            io.path_updates.lock().unwrap().as_slice(),
            &[(11, "1/New Author/New Title.epub".to_string())]
        );
    }

    #[tokio::test]
    async fn orphaned_file_at_target_is_healed_record_only() {
        let dir = tempfile::tempdir().unwrap();
        let root_path = dir.path().to_string_lossy().into_owned();
        let root = RootFolder {
            id: 7,
            path: root_path.clone(),
            media_type: MediaType::Ebook,
        };

        // The orphan window's aftermath (a historical failure, or a failed
        // revert): the file already lives at the computed target while the
        // record still points at a path with nothing there.
        let new_abs = format!("{root_path}/1/New Author/New Title.epub");
        tokio::fs::create_dir_all(format!("{root_path}/1/New Author"))
            .await
            .unwrap();
        tokio::fs::write(&new_abs, b"book").await.unwrap();
        let old_abs = format!("{root_path}/stale/old.epub");

        let io = StubIo::failing(0);
        let work = test_work();

        let (warnings, moved) = reorganize_items(
            &io,
            1,
            &work,
            vec![test_item(11, 7, "stale/old.epub")],
            std::slice::from_ref(&root),
        )
        .await;

        // Recognized as already-moved: the record is repaired with zero
        // file operations and the item is locatable again.
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(moved.is_empty(), "heal is record-only — no retag set");
        assert_eq!(
            io.path_updates.lock().unwrap().as_slice(),
            &[(11, "1/New Author/New Title.epub".to_string())]
        );
        assert!(tokio::fs::try_exists(&new_abs).await.unwrap());
        assert!(!tokio::fs::try_exists(&old_abs).await.unwrap());
    }
}

// =============================================================================
// Manual-import door (R7) tests — no tests/behavioral harness can reach this
// door (livrarr-behavioral does not depend on livrarr-server), so this
// mirrors the `reorganize_tests` precedent above: a real SqliteDb-backed
// `LiveImportService`, constructed the same way `main.rs` wires it, minus a
// real HTTP client / production ChapterExtractor (neither is exercised by an
// .epub fixture — see NoopChapterExtractor below).
// =============================================================================

#[cfg(test)]
mod manual_import_door_tests {
    use super::*;
    use livrarr_db::{LibraryItemDb, RootFolderDb, UserDb, WorkDbCreate};

    /// `do_import_single_file` requests `extract_chapters: true` unconditionally,
    /// but the core only ever calls the extractor for a `.m4b` extension (see
    /// `try_extract_chapters`) — every fixture here is a fake `.epub`, so this
    /// is never actually invoked.
    struct NoopChapterExtractor;
    impl livrarr_domain::services::ChapterExtractor for NoopChapterExtractor {
        fn extract_m4b_chapters(
            &self,
            _path: &std::path::Path,
        ) -> Result<
            livrarr_domain::services::ChapterExtractionResult,
            livrarr_domain::services::ChapterExtractionError,
        > {
            unreachable!("test fixtures are .epub, never .m4b")
        }
    }

    fn make_service(db: livrarr_db::sqlite::SqliteDb, data_dir: Arc<PathBuf>) -> LiveImportService {
        let import_io = Arc::new(LiveImportIoService::new(db.clone()));
        let semaphore = Arc::new(tokio::sync::Semaphore::new(2));
        let import_workflow = Arc::new(LiveImportWorkflow::new(
            db.clone(),
            semaphore,
            data_dir.clone(),
            Arc::new(NoopChapterExtractor),
        ));
        let tag_service = Arc::new(crate::tag_service::LiveTagService::new(
            import_io.clone(),
            data_dir.clone(),
        ));
        let settings_service = Arc::new(LiveSettingsService::new(db.clone()));
        let http_client_safe = livrarr_http::HttpClientBuilder::default().build().unwrap();
        LiveImportService::new(
            import_io,
            import_workflow,
            tag_service,
            settings_service,
            http_client_safe,
        )
    }

    #[tokio::test]
    async fn manual_import_lands_pending_row_and_retag_does_not_fail_the_import() {
        // The door's own copy/tag/rename pipeline is gone; import_single_file
        // must now go through the core (`ImportWorkflow::import_file`, Copy
        // mode) and land a Pending row, then run TagService::retag_library_items
        // as an unconditional post-step (never a hard failure for this door).
        let db = livrarr_db::create_test_db().await;
        let user = db
            .create_user(livrarr_db::CreateUserDbRequest {
                username: "manualdoor".into(),
                password_hash: "hash".into(),
                role: livrarr_domain::UserRole::Admin,
                api_key_hash: "keyhash".into(),
            })
            .await
            .unwrap();
        let (work, _) = db
            .create_work(livrarr_db::CreateWorkDbRequest {
                user_id: user.id,
                title: "Door Book".into(),
                author_name: "Door Author".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let library_dir = tempfile::tempdir().unwrap();
        let rf = db
            .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
            .await
            .unwrap();

        let source_dir = tempfile::tempdir().unwrap();
        let source_path = source_dir.path().join("book.epub");
        std::fs::write(&source_path, b"test content for import").unwrap();

        let data_dir = Arc::new(library_dir.path().to_path_buf());
        let svc = make_service(db.clone(), data_dir);

        let target_path = format!("{}/Door Author/Door Book.epub", rf.path);
        let req = ImportSingleFileRequest {
            source: source_path,
            target_path,
            root_folder_path: rf.path.clone(),
            root_folder_id: rf.id,
            media_type: MediaType::Ebook,
            user_id: user.id,
            work_id: work.id,
            author_name: "Door Author".into(),
            title: "Door Book".into(),
            import_id: None,
        };

        let result = svc.import_single_file(req).await;
        assert!(
            !matches!(result, ImportFileResult::Failed(_)),
            "expected import to succeed (Ok or Warning), got {result:?}"
        );

        let items = db
            .list_library_items_by_work(user.id, work.id)
            .await
            .unwrap();
        assert_eq!(items.len(), 1, "the core must create exactly one row");
        assert_eq!(
            items[0].tag_status,
            livrarr_db::TagStatus::Pending,
            "tag_status is set at creation by the core and is not flipped by \
             the synchronous retag step — only the tag_convergence job does that"
        );
        assert!(
            std::path::Path::new(&format!("{}/{}", rf.path, items[0].path)).exists(),
            "the file must actually land at the target path"
        );
    }

    #[tokio::test]
    async fn manual_import_second_call_for_same_target_is_skipped() {
        // The old pipeline had no dedup surface for this door (a re-import
        // hit a raw DB constraint error). The core's outcome matrix now
        // reports Skipped(AlreadyImported), which import_single_file must
        // surface as ImportFileResult::Skipped rather than a generic Failed.
        let db = livrarr_db::create_test_db().await;
        let user = db
            .create_user(livrarr_db::CreateUserDbRequest {
                username: "manualdoor2".into(),
                password_hash: "hash".into(),
                role: livrarr_domain::UserRole::Admin,
                api_key_hash: "keyhash2".into(),
            })
            .await
            .unwrap();
        let (work, _) = db
            .create_work(livrarr_db::CreateWorkDbRequest {
                user_id: user.id,
                title: "Door Book Two".into(),
                author_name: "Door Author Two".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let library_dir = tempfile::tempdir().unwrap();
        let rf = db
            .create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
            .await
            .unwrap();

        let source_dir = tempfile::tempdir().unwrap();
        let source_path = source_dir.path().join("book.epub");
        std::fs::write(&source_path, b"test content for import").unwrap();

        let data_dir = Arc::new(library_dir.path().to_path_buf());
        let svc = make_service(db.clone(), data_dir);

        let target_path = format!("{}/Door Author Two/Door Book Two.epub", rf.path);
        let build_req = || ImportSingleFileRequest {
            source: source_path.clone(),
            target_path: target_path.clone(),
            root_folder_path: rf.path.clone(),
            root_folder_id: rf.id,
            media_type: MediaType::Ebook,
            user_id: user.id,
            work_id: work.id,
            author_name: "Door Author Two".into(),
            title: "Door Book Two".into(),
            import_id: None,
        };

        let first = svc.import_single_file(build_req()).await;
        assert!(
            !matches!(first, ImportFileResult::Failed(_)),
            "first import should succeed, got {first:?}"
        );

        let second = svc.import_single_file(build_req()).await;
        assert!(
            matches!(second, ImportFileResult::Skipped(_)),
            "re-importing the same target should be skipped, got {second:?}"
        );

        let items = db
            .list_library_items_by_work(user.id, work.id)
            .await
            .unwrap();
        assert_eq!(items.len(), 1, "dedup must not create a second row");
    }
}
