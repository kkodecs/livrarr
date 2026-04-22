use super::common::ServiceError;

#[trait_variant::make(Send)]
pub trait ReadarrImportWorkflow: Send + Sync {
    async fn connect(
        &self,
        req: crate::readarr::ReadarrConnectRequest,
    ) -> Result<crate::readarr::ReadarrConnectResponse, ServiceError>;

    async fn preview(
        &self,
        user_id: i64,
        req: crate::readarr::ReadarrImportRequest,
    ) -> Result<crate::readarr::ReadarrPreviewResponse, ServiceError>;

    async fn start(
        &self,
        user_id: i64,
        req: crate::readarr::ReadarrImportRequest,
    ) -> Result<crate::readarr::ReadarrStartResponse, ServiceError>;

    async fn progress(&self) -> crate::readarr::ReadarrImportProgress;

    async fn history(
        &self,
        user_id: i64,
    ) -> Result<crate::readarr::ReadarrHistoryResponse, ServiceError>;

    async fn undo(
        &self,
        user_id: i64,
        import_id: String,
    ) -> Result<crate::readarr::ReadarrUndoResponse, ServiceError>;
}
