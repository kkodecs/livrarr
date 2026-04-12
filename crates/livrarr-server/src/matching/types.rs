//! Shared types for the book matching engine.

use serde::Serialize;

/// Source method that produced an extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtractionSource {
    Embedded,
    Path,
    String,
    /// Synthetic pair created by combinatorial fallback.
    Synthetic,
}

/// Confidence level of an extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    Low,
    MediumLow,
    Medium,
    MediumHigh,
    High,
}

/// A single extraction hypothesis from M1, M2, or M3.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Extraction {
    pub title: Option<String>,
    pub author: Option<String>,
    pub year: Option<i32>,
    pub isbn: Option<String>,
    pub language: Option<String>,
    pub series: Option<String>,
    pub series_position: Option<f64>,
    pub narrator: Option<String>,
    pub asin: Option<String>,
    pub confidence: Confidence,
    pub source: ExtractionSource,
}

impl Extraction {
    pub fn has_title(&self) -> bool {
        self.title.as_ref().is_some_and(|t| !t.is_empty())
    }

    pub fn has_author(&self) -> bool {
        self.author.as_ref().is_some_and(|a| !a.is_empty())
    }

    pub fn has_title_and_author(&self) -> bool {
        self.has_title() && self.has_author()
    }

    /// Field completeness score for ranking during reconciliation.
    pub fn completeness(&self) -> u8 {
        let mut score = 0u8;
        if self.has_title() { score += 3; }
        if self.has_author() { score += 2; }
        if self.year.is_some() { score += 1; }
        if self.isbn.is_some() { score += 1; }
        if self.series.is_some() { score += 1; }
        score
    }
}

/// A candidate match from OpenLibrary or Goodreads.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchCandidate {
    pub title: String,
    pub author: String,
    pub year: Option<i32>,
    pub work_key: String,
    pub author_key: Option<String>,
    pub cover_url: Option<String>,
    pub series: Option<String>,
    pub series_position: Option<f64>,
    pub provider: MatchProvider,
    pub score: f64,
}

/// Which metadata provider a candidate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchProvider {
    OpenLibrary,
    Goodreads,
}

/// Final result of the matching pipeline for one input item.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchResult {
    /// The reconciled extraction used for matching.
    pub extraction: Extraction,
    /// All extractions from individual methods (for UI display if methods disagreed).
    pub all_extractions: Vec<Extraction>,
    /// Ranked candidates from OL/Goodreads.
    pub candidates: Vec<MatchCandidate>,
    /// Whether the top candidate was auto-confirmed.
    pub auto_confirmed: bool,
    /// Existing work ID if duplicate detected.
    pub existing_work_id: Option<i64>,
    /// Duplicate class if detected.
    pub duplicate_class: Option<DuplicateClass>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DuplicateClass {
    ExactFile,
    SameWorkSameFormat,
    SameWorkDifferentFormat,
    PossibleDuplicate,
}

/// Input to the matching engine.
#[derive(Debug, Clone)]
pub struct MatchInput {
    /// File path on disk (if available).
    pub file_path: Option<std::path::PathBuf>,
    /// Additional file paths for multi-file audiobooks.
    pub grouped_paths: Option<Vec<std::path::PathBuf>>,
    /// A string to parse (release title, filename, etc). If not set, derived from file_path.
    pub parse_string: Option<String>,
    /// Media type if known.
    pub media_type: Option<livrarr_domain::MediaType>,
    /// The root directory of the scan (for M2 path parsing). Required for correct author extraction.
    pub scan_root: Option<std::path::PathBuf>,
}
