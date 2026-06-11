//! Composition-root `ChapterExtractor` (REQ-005): thin delegation to
//! livrarr-tagwrite so livrarr-library carries no tagwrite edge. Sync by
//! design — call sites wrap extraction in `spawn_blocking`.

use livrarr_domain::services::{
    ChapterExtractionError, ChapterExtractionResult, ChapterExtractor, ExtractedChapter,
};

pub struct ChapterExtractorImpl;

impl ChapterExtractor for ChapterExtractorImpl {
    fn extract_m4b_chapters(
        &self,
        path: &std::path::Path,
    ) -> Result<ChapterExtractionResult, ChapterExtractionError> {
        match livrarr_tagwrite::extract_m4b_chapters(path) {
            Ok(r) => Ok(ChapterExtractionResult {
                chapters: r
                    .chapters
                    .into_iter()
                    .map(|c| ExtractedChapter {
                        title: c.title,
                        start_time_secs: c.start_time_secs,
                    })
                    .collect(),
                duration_secs: r.duration_secs,
            }),
            Err(livrarr_tagwrite::ChapterExtractionError::IoError(e)) => {
                Err(ChapterExtractionError::IoError(e))
            }
            Err(livrarr_tagwrite::ChapterExtractionError::ParseError(e)) => {
                Err(ChapterExtractionError::ParseError(e))
            }
        }
    }
}
