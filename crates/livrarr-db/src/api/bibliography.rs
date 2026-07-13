//! Author bibliography cache data access: `AuthorBibliographyDb` trait.

use serde::Deserialize;

use crate::DbError;

/// Cached author bibliography entry (from OL/LLM cleanup).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibliographyEntry {
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub ol_key: Option<String>,
    pub title: String,
    pub year: Option<i32>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    /// ISO 639-1 code if a real edition in some language was confirmed; `None`
    /// means no language signal was found anywhere (Unknown, not "English").
    /// `#[serde(default)]` so cached JSON blobs written before this field
    /// existed deserialize as `None` rather than failing.
    #[serde(default)]
    pub language: Option<String>,
}

fn empty_string_as_none<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let s: Option<String> = Option::deserialize(deserializer)?;
    Ok(s.filter(|v| !v.is_empty()))
}

/// Cached bibliography for an author.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorBibliography {
    pub author_id: i64,
    pub entries: Vec<BibliographyEntry>,
    pub raw_entries: Option<Vec<BibliographyEntry>>,
    pub fetched_at: String,
}

#[trait_variant::make(Send)]
pub trait AuthorBibliographyDb: Send + Sync {
    async fn get_bibliography(&self, author_id: i64)
        -> Result<Option<AuthorBibliography>, DbError>;
    async fn save_bibliography(
        &self,
        author_id: i64,
        entries: &[BibliographyEntry],
        raw_entries: Option<&[BibliographyEntry]>,
    ) -> Result<AuthorBibliography, DbError>;

    async fn delete_bibliography(&self, author_id: i64) -> Result<(), DbError>;
}
