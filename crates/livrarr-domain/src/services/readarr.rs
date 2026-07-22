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

    /// This user's active (or, once it finishes, their most recently
    /// completed) import — never another user's (Unit B3 Part 2, audit
    /// finding #11). `import_id`, when given, additionally requires the
    /// caller's own run to match that exact id; a mismatch or a non-owned
    /// run yields `ServiceError::NotFound` (indistinguishable from "no such
    /// import" — never confirms or denies that a DIFFERENT user owns it). A
    /// caller with no owned run and no specific `import_id` gets an idle
    /// default, never another user's owner/import_id/counts/errors/paths.
    async fn progress(
        &self,
        user_id: i64,
        import_id: Option<String>,
    ) -> Result<crate::readarr::ReadarrImportProgress, ServiceError>;

    async fn history(
        &self,
        user_id: i64,
    ) -> Result<crate::readarr::ReadarrHistoryResponse, ServiceError>;

    async fn undo(
        &self,
        user_id: i64,
        import_id: String,
    ) -> Result<crate::readarr::ReadarrUndoResponse, ServiceError>;

    // --- Origin trust boundary (Unit B3 Part 1) — admin-managed allowlist ---

    /// All admin-approved private Readarr origins.
    async fn list_origins(&self) -> Result<Vec<crate::readarr::ReadarrOriginInfo>, ServiceError>;

    /// Approve a new origin from a raw URL — normalized (Part 1 point 2) and
    /// stored by its bare origin (scheme://host[:port]).
    async fn add_origin(
        &self,
        url: String,
    ) -> Result<crate::readarr::ReadarrOriginInfo, ServiceError>;

    /// Revoke a previously-approved origin.
    async fn remove_origin(&self, id: i64) -> Result<(), ServiceError>;
}
