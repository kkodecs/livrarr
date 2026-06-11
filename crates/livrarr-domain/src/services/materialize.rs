//! The single save port (REQ-012): cover download + tag write, change-gated.
//! Impl in livrarr-materialize; DB-FREE (the request carries every caller-resolved
//! input). Tag fields are a domain mirror — materialize converts to the tagwrite
//! type at the boundary (insight 9e: no tagwrite types in domain signatures).

use std::path::PathBuf;

use crate::WorkId;

/// Tag fields written into a book file (REQ-012). Domain mirror of the tagwrite
/// tag model; livrarr-materialize converts this to `livrarr_tagwrite::TagMetadata`
/// at the boundary.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterializeTags {
    pub title: String,
    pub subtitle: Option<String>,
    pub author: String,
    pub narrator: Option<Vec<String>>,
    pub year: Option<i32>,
    pub genre: Option<Vec<String>>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub isbn: Option<String>,
    pub language: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

/// Current + chosen cover state for one slot, resolved by the caller from the
/// Work record so materialize reads no DB (REQ-006/008, R-001). Cover SELECTION
/// (priority) happens in the merge; materialize only downloads the chosen URL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverSlotState {
    /// The cover URL the merge chose for this slot (`None` = no new cover).
    pub chosen_new_url: Option<String>,
    /// The cover URL currently on the work (drives the non-destructive/replace decision).
    pub current_url: Option<String>,
    /// The cover file already on disk for this slot, if any.
    pub current_path: Option<String>,
    /// True if the user locked this cover (provenance Setter=User) — never overwrite (REQ-008).
    pub user_locked: bool,
}

/// Everything materialize needs for PURE I/O (REQ-012, R-001). The caller
/// (enrichment/server) resolves ALL of this from the DB BEFORE the call so
/// materialize never reads the DB.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterializeRequest {
    pub work_id: WorkId,
    /// Did the merge change anything? Materialize no-ops when false (REQ-012/AC-010).
    pub changed: bool,
    /// Did any tag-relevant field change (gates the tag write specifically).
    pub tag_fields_changed: bool,
    pub ebook_cover: CoverSlotState,
    pub audiobook_cover: CoverSlotState,
    /// Resolved absolute file targets for tag writing (LibraryItem paths).
    pub file_paths: Vec<PathBuf>,
    pub tags: MaterializeTags,
    /// Covers storage directory.
    pub covers_dir: PathBuf,
}

/// What materialize wrote — the caller persists cover_path + the change
/// generation (REQ-012).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterializeOutcome {
    pub ebook_cover_path: Option<String>,
    pub audiobook_cover_path: Option<String>,
    pub tags_written: bool,
    pub skipped_unchanged: bool,
    /// Stored ebook cover + its decoded dimensions (REQ-017). Materialize has
    /// no db edge; the orchestrator persists via `update_cover_dimensions`.
    pub saved_cover: Option<SavedCover>,
    /// Stored audiobook cover + its decoded dimensions (REQ-017).
    pub saved_audiobook_cover: Option<SavedCover>,
}

/// A cover written to disk, with dimensions decoded from the stored bytes
/// (REQ-017). Decode failure yields `None` upstream — a saved cover is better
/// than a cover rejected over a dims read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedCover {
    pub path: PathBuf,
    pub width: i32,
    pub height: i32,
}

/// Error from the save step.
#[derive(Debug, thiserror::Error)]
pub enum MaterializeError {
    #[error("cover download failed: {0}")]
    CoverDownload(String),
    #[error("tag write failed: {0}")]
    TagWrite(String),
}

/// The single save port (REQ-012): cover download + tag write, change-gated.
/// Impl in livrarr-materialize. DB-FREE — the request carries everything.
#[trait_variant::make(Send)]
pub trait MaterializeService: Send + Sync {
    async fn materialize(
        &self,
        request: MaterializeRequest,
    ) -> Result<MaterializeOutcome, MaterializeError>;
}
