//! Series list cache data access: `SeriesCacheDb` trait.

use crate::DbError;

/// Cached series list entry for an author (from GR scraping).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesCacheEntry {
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    /// ISO 639-1 code if a confident Google Books match was found; `None`
    /// means Unknown (shown by default, not treated as foreign).
    #[serde(default)]
    pub language: Option<String>,
}

/// Cached series list for an author.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorSeriesCache {
    pub author_id: i64,
    pub entries: Vec<SeriesCacheEntry>,
    pub raw_entries: Option<Vec<SeriesCacheEntry>>,
    pub fetched_at: String,
}

#[trait_variant::make(Send)]
pub trait SeriesCacheDb: Send + Sync {
    async fn get_series_cache(&self, author_id: i64) -> Result<Option<AuthorSeriesCache>, DbError>;

    async fn save_series_cache(
        &self,
        author_id: i64,
        entries: &[SeriesCacheEntry],
        raw_entries: Option<&[SeriesCacheEntry]>,
    ) -> Result<AuthorSeriesCache, DbError>;

    async fn delete_series_cache(&self, author_id: i64) -> Result<(), DbError>;
}
