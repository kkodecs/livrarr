//! Readarr Import Service — thin DB wrapper that eliminates direct `state.db.*`
//! calls from the readarr_import handler.

use livrarr_db::{
    AuthorDb, CreateAuthorDbRequest, CreateImportDbRequest, CreateLibraryItemDbRequest, ImportDb,
    LibraryItemDb, RootFolderDb, UpdateWorkEnrichmentDbRequest, UpdateWorkUserFieldsDbRequest,
    WorkDb,
};
use livrarr_domain::{
    Author, DbError, Import, LibraryItem, LibraryItemId, RootFolder, RootFolderId, UserId, Work,
    WorkId,
};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ReadarrImportError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Thin service layer wrapping all DB operations needed by the readarr_import
/// handler. Each method delegates to the underlying DB traits without adding
/// business logic.
#[trait_variant::make(Send)]
pub trait ReadarrImportService: Send + Sync {
    // -- Root folder --
    async fn get_root_folder(&self, id: RootFolderId) -> Result<RootFolder, ReadarrImportError>;

    // -- Import tracking --
    async fn create_import(&self, req: CreateImportDbRequest) -> Result<(), ReadarrImportError>;
    async fn get_import(&self, id: &str) -> Result<Option<Import>, ReadarrImportError>;
    async fn list_imports(&self, user_id: UserId) -> Result<Vec<Import>, ReadarrImportError>;
    async fn update_import_status(&self, id: &str, status: &str) -> Result<(), ReadarrImportError>;
    async fn update_import_counts(
        &self,
        id: &str,
        authors: i64,
        works: i64,
        files: i64,
        skipped: i64,
    ) -> Result<(), ReadarrImportError>;
    async fn set_import_completed(&self, id: &str) -> Result<(), ReadarrImportError>;

    // -- Library items (import / undo) --
    async fn list_library_items_by_import(
        &self,
        import_id: &str,
    ) -> Result<Vec<LibraryItem>, ReadarrImportError>;
    async fn delete_library_item_by_id(&self, id: LibraryItemId) -> Result<(), ReadarrImportError>;

    // -- Orphan cleanup (undo) --
    async fn list_orphan_work_ids_by_import(
        &self,
        import_id: &str,
    ) -> Result<Vec<i64>, ReadarrImportError>;
    async fn delete_orphan_works_by_import(
        &self,
        import_id: &str,
    ) -> Result<i64, ReadarrImportError>;
    async fn delete_orphan_authors_by_import(
        &self,
        import_id: &str,
    ) -> Result<i64, ReadarrImportError>;

    // -- Author operations (run_import) --
    async fn create_author(&self, req: CreateAuthorDbRequest)
        -> Result<Author, ReadarrImportError>;
    async fn list_authors(&self, user_id: UserId) -> Result<Vec<Author>, ReadarrImportError>;

    // -- Work operations (run_import) --
    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, ReadarrImportError>;
    async fn create_work(
        &self,
        req: livrarr_db::CreateWorkDbRequest,
    ) -> Result<Work, ReadarrImportError>;
    async fn update_work_enrichment(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkEnrichmentDbRequest,
    ) -> Result<Work, ReadarrImportError>;
    async fn update_work_user_fields(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkUserFieldsDbRequest,
    ) -> Result<Work, ReadarrImportError>;

    // -- Library item creation (run_import) --
    async fn create_library_item(
        &self,
        req: CreateLibraryItemDbRequest,
    ) -> Result<LibraryItem, ReadarrImportError>;
}

// ---------------------------------------------------------------------------
// Implementation — delegates to SqliteDb
// ---------------------------------------------------------------------------

/// Concrete implementation backed by any type satisfying the required DB traits.
pub struct LiveReadarrImportService<D> {
    db: D,
}

impl<D> LiveReadarrImportService<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }
}

impl<D> ReadarrImportService for LiveReadarrImportService<D>
where
    D: ImportDb + RootFolderDb + AuthorDb + WorkDb + LibraryItemDb + Send + Sync,
{
    async fn get_root_folder(&self, id: RootFolderId) -> Result<RootFolder, ReadarrImportError> {
        Ok(self.db.get_root_folder(id).await?)
    }

    async fn create_import(&self, req: CreateImportDbRequest) -> Result<(), ReadarrImportError> {
        Ok(self.db.create_import(req).await?)
    }

    async fn get_import(&self, id: &str) -> Result<Option<Import>, ReadarrImportError> {
        Ok(self.db.get_import(id).await?)
    }

    async fn list_imports(&self, user_id: UserId) -> Result<Vec<Import>, ReadarrImportError> {
        Ok(self.db.list_imports(user_id).await?)
    }

    async fn update_import_status(&self, id: &str, status: &str) -> Result<(), ReadarrImportError> {
        Ok(self.db.update_import_status(id, status).await?)
    }

    async fn update_import_counts(
        &self,
        id: &str,
        authors: i64,
        works: i64,
        files: i64,
        skipped: i64,
    ) -> Result<(), ReadarrImportError> {
        Ok(self
            .db
            .update_import_counts(id, authors, works, files, skipped)
            .await?)
    }

    async fn set_import_completed(&self, id: &str) -> Result<(), ReadarrImportError> {
        Ok(self.db.set_import_completed(id).await?)
    }

    async fn list_library_items_by_import(
        &self,
        import_id: &str,
    ) -> Result<Vec<LibraryItem>, ReadarrImportError> {
        Ok(self.db.list_library_items_by_import(import_id).await?)
    }

    async fn delete_library_item_by_id(&self, id: LibraryItemId) -> Result<(), ReadarrImportError> {
        Ok(self.db.delete_library_item_by_id(id).await?)
    }

    async fn list_orphan_work_ids_by_import(
        &self,
        import_id: &str,
    ) -> Result<Vec<i64>, ReadarrImportError> {
        Ok(self.db.list_orphan_work_ids_by_import(import_id).await?)
    }

    async fn delete_orphan_works_by_import(
        &self,
        import_id: &str,
    ) -> Result<i64, ReadarrImportError> {
        Ok(self.db.delete_orphan_works_by_import(import_id).await?)
    }

    async fn delete_orphan_authors_by_import(
        &self,
        import_id: &str,
    ) -> Result<i64, ReadarrImportError> {
        Ok(self.db.delete_orphan_authors_by_import(import_id).await?)
    }

    async fn create_author(
        &self,
        req: CreateAuthorDbRequest,
    ) -> Result<Author, ReadarrImportError> {
        Ok(self.db.create_author(req).await?)
    }

    async fn list_authors(&self, user_id: UserId) -> Result<Vec<Author>, ReadarrImportError> {
        Ok(self.db.list_authors(user_id).await?)
    }

    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, ReadarrImportError> {
        Ok(self.db.list_works(user_id).await?)
    }

    async fn create_work(
        &self,
        req: livrarr_db::CreateWorkDbRequest,
    ) -> Result<Work, ReadarrImportError> {
        Ok(self.db.create_work(req).await?)
    }

    async fn update_work_enrichment(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkEnrichmentDbRequest,
    ) -> Result<Work, ReadarrImportError> {
        Ok(self
            .db
            .update_work_enrichment(user_id, work_id, req)
            .await?)
    }

    async fn update_work_user_fields(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkUserFieldsDbRequest,
    ) -> Result<Work, ReadarrImportError> {
        Ok(self
            .db
            .update_work_user_fields(user_id, work_id, req)
            .await?)
    }

    async fn create_library_item(
        &self,
        req: CreateLibraryItemDbRequest,
    ) -> Result<LibraryItem, ReadarrImportError> {
        Ok(self.db.create_library_item(req).await?)
    }
}

pub use livrarr_domain::readarr::ReadarrImportProgress;
