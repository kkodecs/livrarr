//! Readarr API client for library import.
//!
//! Lightweight client that fetches authors, books, editions, and book files
//! from a Readarr instance via its REST API.

use reqwest::Client;
use serde::Deserialize;

/// Readarr API client.
pub struct ReadarrClient {
    base_url: String,
    api_key: String,
    http: Client,
}

impl ReadarrClient {
    pub fn new(url: &str, api_key: &str, http: Client) -> Self {
        let base_url = url.trim_end_matches('/').to_string();
        Self {
            base_url,
            api_key: api_key.to_string(),
            http,
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ReadarrError> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            .header("X-Api-Key", &self.api_key)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| ReadarrError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ReadarrError::Api {
                status: status.as_u16(),
                body,
            });
        }

        resp.json::<T>()
            .await
            .map_err(|e| ReadarrError::Parse(e.to_string()))
    }

    pub async fn root_folders(&self) -> Result<Vec<RdRootFolder>, ReadarrError> {
        self.get("/api/v1/rootfolder").await
    }

    pub async fn authors(&self) -> Result<Vec<RdAuthor>, ReadarrError> {
        self.get("/api/v1/author").await
    }

    pub async fn books(&self) -> Result<Vec<RdBook>, ReadarrError> {
        self.get("/api/v1/book").await
    }

    pub async fn book_files_by_author(
        &self,
        author_id: i64,
    ) -> Result<Vec<RdBookFile>, ReadarrError> {
        self.get(&format!("/api/v1/bookfile?authorId={author_id}"))
            .await
    }
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ReadarrError {
    Network(String),
    Api { status: u16, body: String },
    Parse(String),
}

impl std::fmt::Display for ReadarrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(e) => write!(f, "network error: {e}"),
            Self::Api { status, body } => write!(f, "API error {status}: {body}"),
            Self::Parse(e) => write!(f, "parse error: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Response types — lightweight deserialization structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdRootFolder {
    pub id: i64,
    pub name: Option<String>,
    pub path: String,
    pub accessible: Option<bool>,
    pub free_space: Option<i64>,
    pub total_space: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdAuthor {
    pub id: i64,
    pub author_name: Option<String>,
    pub sort_name: Option<String>,
    pub foreign_author_id: Option<String>,
    pub overview: Option<String>,
    pub genres: Option<Vec<String>>,
    pub images: Option<Vec<RdImage>>,
    pub monitored: Option<bool>,
    pub added: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdBook {
    pub id: i64,
    pub title: Option<String>,
    pub author_id: i64,
    pub foreign_book_id: Option<String>,
    pub series_title: Option<String>,
    pub overview: Option<String>,
    pub release_date: Option<String>,
    pub page_count: Option<i32>,
    pub genres: Option<Vec<String>>,
    pub ratings: Option<RdRatings>,
    pub images: Option<Vec<RdImage>>,
    pub monitored: Option<bool>,
    pub added: Option<String>,
    pub editions: Option<Vec<RdEdition>>,
}

impl RdBook {
    /// Returns the monitored edition, or the first edition if none is monitored.
    pub fn monitored_edition(&self) -> Option<&RdEdition> {
        let editions = self.editions.as_ref()?;
        editions
            .iter()
            .find(|e| e.monitored.unwrap_or(false))
            .or_else(|| editions.first())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdEdition {
    pub id: i64,
    pub book_id: Option<i64>,
    pub foreign_edition_id: Option<String>,
    pub isbn13: Option<String>,
    pub asin: Option<String>,
    pub title: Option<String>,
    pub language: Option<String>,
    pub overview: Option<String>,
    pub format: Option<String>,
    pub is_ebook: Option<bool>,
    pub publisher: Option<String>,
    pub page_count: Option<i32>,
    pub release_date: Option<String>,
    pub images: Option<Vec<RdImage>>,
    pub monitored: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdBookFile {
    pub id: i64,
    pub author_id: Option<i64>,
    pub book_id: i64,
    pub path: String,
    pub size: i64,
    pub date_added: Option<String>,
    pub quality: Option<RdQuality>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdQuality {
    pub quality: Option<RdQualityInner>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdQualityInner {
    pub id: i32,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdRatings {
    pub votes: Option<i32>,
    pub value: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdImage {
    pub url: Option<String>,
    pub cover_type: Option<String>,
    pub remote_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a Readarr quality ID to a Livrarr MediaType string.
/// Returns None for Unknown(0) — caller should infer from file extension.
pub fn quality_to_media_type(quality_id: i32) -> Option<&'static str> {
    match quality_id {
        1..=4 => Some("ebook"),       // PDF, MOBI, EPUB, AZW3
        10..=13 => Some("audiobook"), // MP3, FLAC, M4B, UnknownAudio
        _ => None,                    // Unknown(0) or unrecognized
    }
}

/// Infer media type from file extension when quality is Unknown.
pub fn media_type_from_extension(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_lowercase();
    match ext.as_str() {
        "epub" | "mobi" | "azw" | "azw3" | "pdf" | "cbz" | "cbr" => Some("ebook"),
        "mp3" | "m4b" | "m4a" | "flac" | "ogg" | "wma" | "aac" => Some("audiobook"),
        _ => None,
    }
}
