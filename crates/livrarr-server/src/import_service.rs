use std::sync::OnceLock;

use livrarr_domain::services::{
    ImportFileResult, ImportGrabResult, ImportService, ImportSingleFileRequest, ServiceError,
    SettingsService,
};
use livrarr_domain::MediaType;

use crate::state::AppState;

pub struct LiveImportService {
    state: OnceLock<Box<AppState>>,
}

impl Clone for LiveImportService {
    fn clone(&self) -> Self {
        Self {
            state: OnceLock::new(),
        }
    }
}

impl Default for LiveImportService {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveImportService {
    pub fn new() -> Self {
        Self {
            state: OnceLock::new(),
        }
    }

    pub fn init(&self, state: AppState) {
        let _ = self.state.set(Box::new(state));
    }

    fn state(&self) -> &AppState {
        self.state.get().expect("LiveImportService not initialized")
    }
}

impl ImportService for LiveImportService {
    async fn import_grab(
        &self,
        user_id: i64,
        grab_id: i64,
    ) -> Result<ImportGrabResult, ServiceError> {
        let state = self.state();
        crate::infra::import_pipeline::import_grab(state, user_id, grab_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))
    }

    async fn import_single_file(&self, req: ImportSingleFileRequest) -> ImportFileResult {
        let state = self.state();

        use livrarr_domain::services::ImportIoService;
        let work = match state
            .import_io_service
            .get_work(req.user_id, req.work_id)
            .await
        {
            Ok(w) => w,
            Err(e) => return ImportFileResult::Failed(format!("failed to load work: {e}")),
        };

        let tag_metadata = crate::infra::import_pipeline::build_tag_metadata(&work);
        let cover_data =
            crate::infra::import_pipeline::read_cover_bytes(state, req.user_id, req.work_id).await;

        let media_mgmt = match state.settings_service.get_media_management_config().await {
            Ok(cfg) => cfg,
            Err(e) => return ImportFileResult::Failed(format!("failed to load media config: {e}")),
        };

        match crate::infra::import_pipeline::import_single_file(
            state,
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
