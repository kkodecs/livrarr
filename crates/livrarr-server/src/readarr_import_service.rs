//! Readarr Import Service — thin DB wrapper that eliminates direct `state.db.*`
//! calls from the readarr_import handler.

use livrarr_db::{
    AuthorDb, AuthorLinkDb, AuthorNameVariantDb, CreateAuthorGateRequest, CreateImportDbRequest,
    ImportDb, LibraryItemDb, RootFolderDb, UpdateWorkUserFieldsDbRequest, WorkDb,
};
use livrarr_domain::{
    AgreedAuthorRouteEvidence, Author, AuthorId, AuthorLinkCandidate, AuthorLinkTrigger, DbError,
    Import, LibraryItem, LibraryItemId, RejectedAuthorRouteEvidence, RootFolder, RootFolderId,
    RouteWriteOutcome, UserId, Work, WorkId,
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
    async fn list_authors(&self, user_id: UserId) -> Result<Vec<Author>, ReadarrImportError>;

    // -- Author-link operations (run_import) --
    //
    // The import needs the same author-link doors the road uses. It cannot compose
    // the road itself — that needs a provider gateway this composition never
    // builds — so the four seams it does need come through here, each delegating
    // to exactly one repository or shared-road entry point.
    /// The shared create/adopt gate: the author, its first name variant, and its
    /// due author-link task commit together.
    async fn create_or_adopt_author(
        &self,
        req: CreateAuthorGateRequest,
    ) -> Result<(Author, bool), ReadarrImportError>;

    /// Every name already associated with the author, for the guard to compare
    /// the Readarr spelling against. Read *before* the Readarr observation lands,
    /// or Readarr would end up proving itself.
    async fn author_associated_names(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<String>, ReadarrImportError>;

    /// Route evidence the name guard agreed with.
    async fn submit_author_route_evidence(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        evidence: AgreedAuthorRouteEvidence,
    ) -> Result<RouteWriteOutcome, ReadarrImportError>;

    /// Persist a non-Agree Readarr verdict as reviewable evidence. It writes no
    /// route and clears no tombstone.
    async fn record_author_route_rejection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        rejected: RejectedAuthorRouteEvidence,
    ) -> Result<AuthorLinkCandidate, ReadarrImportError>;

    /// Retain the spelling Readarr used for an author it did not create.
    ///
    /// The batch adopts on a *compatible* name, so Readarr's spelling can differ
    /// from the adopted author's — and a name that is never recorded is a name
    /// the display picker can never offer (FP-035).
    async fn record_readarr_author_name(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        name: &str,
    ) -> Result<(), ReadarrImportError>;

    /// Make sure the author has a due author-link task, whatever the guard said.
    async fn enqueue_author_link(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        trigger: AuthorLinkTrigger,
    ) -> Result<(), ReadarrImportError>;

    // -- Work operations (run_import) --
    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, ReadarrImportError>;
    async fn update_work_user_fields(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkUserFieldsDbRequest,
    ) -> Result<Work, ReadarrImportError>;
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
    D: ImportDb
        + RootFolderDb
        + AuthorDb
        + WorkDb
        + LibraryItemDb
        + AuthorLinkDb
        + AuthorNameVariantDb
        + Send
        + Sync,
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

    async fn list_authors(&self, user_id: UserId) -> Result<Vec<Author>, ReadarrImportError> {
        Ok(self.db.list_authors(user_id).await?)
    }

    async fn create_or_adopt_author(
        &self,
        req: CreateAuthorGateRequest,
    ) -> Result<(Author, bool), ReadarrImportError> {
        Ok(self.db.create_or_adopt_author(req).await?)
    }

    async fn author_associated_names(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<String>, ReadarrImportError> {
        let author = self.db.get_author(user_id, author_id).await?;
        let variants = self.db.list_name_variants(user_id, author_id).await?;
        let mut names = Vec::with_capacity(variants.len() + 1);
        names.push(author.name);
        for variant in variants {
            if !names.contains(&variant.name) {
                names.push(variant.name);
            }
        }
        Ok(names)
    }

    async fn submit_author_route_evidence(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        evidence: AgreedAuthorRouteEvidence,
    ) -> Result<RouteWriteOutcome, ReadarrImportError> {
        livrarr_metadata::author_linking::submit_agreed_evidence(
            &self.db, user_id, author_id, evidence,
        )
        .await
        .map_err(|e| ReadarrImportError::Conflict(format!("author route write failed: {e:?}")))
    }

    async fn record_author_route_rejection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        rejected: RejectedAuthorRouteEvidence,
    ) -> Result<AuthorLinkCandidate, ReadarrImportError> {
        Ok(self
            .db
            .record_readarr_rejection(user_id, author_id, rejected)
            .await?)
    }

    async fn record_readarr_author_name(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        name: &str,
    ) -> Result<(), ReadarrImportError> {
        self.db
            .record_author_observed_names(
                user_id,
                author_id,
                &[livrarr_domain::ProviderAuthorNameObservation {
                    source: livrarr_domain::AuthorNameSource::Readarr,
                    name: name.to_string(),
                }],
            )
            .await?;
        Ok(())
    }

    async fn enqueue_author_link(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        trigger: AuthorLinkTrigger,
    ) -> Result<(), ReadarrImportError> {
        Ok(self.db.ensure_enqueued(user_id, author_id, trigger).await?)
    }

    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, ReadarrImportError> {
        Ok(self.db.list_works(user_id).await?)
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
}

pub use livrarr_domain::readarr::ReadarrImportProgress;
