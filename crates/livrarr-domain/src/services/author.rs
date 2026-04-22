use serde::Serialize;

use crate::{Author, AuthorId, DbError, UserId};

#[derive(Debug)]
pub struct AddAuthorRequest {
    pub name: String,
    pub sort_name: Option<String>,
    pub ol_key: Option<String>,
    pub monitored: bool,
}

#[derive(Debug)]
pub enum AddAuthorResult {
    Created(Author),
    Updated(Author),
}

impl AddAuthorResult {
    pub fn author(&self) -> &Author {
        match self {
            Self::Created(a) | Self::Updated(a) => a,
        }
    }

    pub fn is_created(&self) -> bool {
        matches!(self, Self::Created(_))
    }

    pub fn into_author(self) -> Author {
        match self {
            Self::Created(a) | Self::Updated(a) => a,
        }
    }
}

#[derive(Debug)]
pub struct UpdateAuthorRequest {
    pub name: Option<String>,
    pub sort_name: Option<Option<String>>,
    pub ol_key: Option<Option<String>>,
    pub gr_key: Option<Option<String>>,
    pub monitored: Option<bool>,
    pub monitor_new_items: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibliographyEntry {
    pub title: String,
    pub year: Option<i32>,
    pub ol_key: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub already_in_library: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibliographyResult {
    pub entries: Vec<BibliographyEntry>,
    pub filtered_count: usize,
    pub raw_count: usize,
    pub raw_available: bool,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorLookupResult {
    pub ol_key: String,
    pub name: String,
    pub sort_name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthorServiceError {
    #[error("author not found")]
    NotFound,
    #[error("author already exists")]
    AlreadyExists,
    #[error("validation: {field}: {message}")]
    Validation { field: String, message: String },
    #[error("OpenLibrary rate limited")]
    OlRateLimited,
    #[error("provider error: {0}")]
    Provider(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait AuthorService: Send + Sync {
    async fn add(
        &self,
        user_id: UserId,
        req: AddAuthorRequest,
    ) -> Result<AddAuthorResult, AuthorServiceError>;
    async fn get(&self, user_id: UserId, author_id: AuthorId)
        -> Result<Author, AuthorServiceError>;
    async fn list(&self, user_id: UserId) -> Result<Vec<Author>, AuthorServiceError>;
    async fn update(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        req: UpdateAuthorRequest,
    ) -> Result<Author, AuthorServiceError>;
    async fn delete(&self, user_id: UserId, author_id: AuthorId) -> Result<(), AuthorServiceError>;
    async fn lookup(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<AuthorLookupResult>, AuthorServiceError>;
    async fn search(&self, user_id: UserId, query: &str)
        -> Result<Vec<Author>, AuthorServiceError>;
    async fn bibliography(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        raw: bool,
    ) -> Result<BibliographyResult, AuthorServiceError>;
    async fn refresh_bibliography(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<BibliographyResult, AuthorServiceError>;
    fn spawn_bibliography_refresh(&self, author_id: i64, user_id: i64);
    async fn lookup_authors(
        &self,
        term: &str,
        limit: u32,
    ) -> Result<Vec<AuthorLookupResult>, AuthorServiceError>;
}
