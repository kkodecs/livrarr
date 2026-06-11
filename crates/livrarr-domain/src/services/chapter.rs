use crate::services::file::FileServiceError;
use crate::{AudiobookChapter, LibraryItemId, UserId};

#[trait_variant::make(Send)]
pub trait ChapterService: Send + Sync {
    async fn get_chapters(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
    ) -> Result<Vec<AudiobookChapter>, FileServiceError>;
}

/// Import-time M4B chapter extraction (REQ-005): mirrors the tagwrite
/// extraction signature so the server impl is a thin delegation;
/// livrarr-library consumes this trait and drops its direct tagwrite edge.
/// Sync by design — extraction is blocking I/O; call sites wrap it in
/// `spawn_blocking`.
pub trait ChapterExtractor: Send + Sync {
    fn extract_m4b_chapters(
        &self,
        path: &std::path::Path,
    ) -> Result<ChapterExtractionResult, ChapterExtractionError>;
}

/// A chapter as read from the container (no ids yet — assignment happens at
/// persistence).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedChapter {
    pub title: String,
    pub start_time_secs: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChapterExtractionResult {
    pub chapters: Vec<ExtractedChapter>,
    pub duration_secs: Option<f64>,
}

#[derive(Debug, thiserror::Error)]
pub enum ChapterExtractionError {
    #[error("I/O error reading M4B: {0}")]
    IoError(String),
    #[error("corrupt or unparseable M4B container: {0}")]
    ParseError(String),
}
