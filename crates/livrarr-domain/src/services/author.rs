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
    pub monitor_language: Option<Option<String>>,
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
    /// ISO 639-1 code if a real edition in some language was confirmed;
    /// `None` means Unknown (shown by default).
    pub language: Option<String>,
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

/// Outcome of merging two author rows (author-dedup): what moved where.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorMergeReport {
    pub works_moved: u64,
    pub series_moved: u64,
    pub series_folded: u64,
}

#[trait_variant::make(Send)]
pub trait AuthorService: Send + Sync {
    async fn add(
        &self,
        user_id: UserId,
        req: AddAuthorRequest,
    ) -> Result<AddAuthorResult, AuthorServiceError>;
    /// Merge `loser_id` into `survivor_id` (author-dedup): works and series
    /// repoint to the survivor, monitoring intent is preserved monotonically,
    /// external keys fill survivor-first, and the loser row is deleted — one
    /// transaction, delegated to the DB layer.
    async fn merge(
        &self,
        user_id: UserId,
        survivor_id: AuthorId,
        loser_id: AuthorId,
    ) -> Result<AuthorMergeReport, AuthorServiceError>;
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

    async fn rename(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        name: String,
    ) -> Result<Author, AuthorServiceError>;

    async fn select_name_variant(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        variant_id: i64,
    ) -> Result<Author, AuthorServiceError>;

    async fn set_monitoring(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        monitored: bool,
        monitor_new_items: Option<bool>,
        monitor_language: Option<String>,
    ) -> Result<Author, AuthorServiceError>;
}

/// The author's route panel, read-only.
///
/// It exists as its own capability because every author response needs the same
/// four derived values and none of them may be recomputed at a door: link state,
/// monitorability, and the compatibility keys all follow from the route ledger,
/// which lives behind the compile wall.
#[trait_variant::make(Send)]
pub trait AuthorViewService: Send + Sync {
    /// The author's routes, link state, whether monitoring is available, and the
    /// scalar key projection kept for API compatibility — all derived from the
    /// route ledger, never from the frozen `authors.*_key` columns.
    async fn route_view(
        &self,
        user_id: UserId,
        author: &Author,
    ) -> Result<AuthorRouteView, AuthorServiceError>;
}

/// The four derived values every author response carries.
#[derive(Debug, Clone)]
pub struct AuthorRouteView {
    pub routes: Vec<crate::AuthorRoute>,
    pub link_state: crate::AuthorLinkState,
    /// True only for an active OpenLibrary route: a Goodreads or Hardcover route
    /// makes an author linked, never monitorable.
    pub monitorable: bool,
    pub compatibility: crate::AuthorCompatibilityProjection,
}

impl AuthorRouteView {
    /// Derive the panel from an author's active route rows.
    ///
    /// The one place these three answers are computed, so a caller that already
    /// holds the route set cannot reach a different conclusion than a caller
    /// that reads it again. Pending review evidence outranks an existing route:
    /// a linked author with an open question is still a question. Every value
    /// comes from the route ledger — never from the frozen `authors.*_key`
    /// columns.
    pub fn from_active_routes(routes: Vec<crate::AuthorRoute>, under_review: bool) -> Self {
        use crate::{AuthorLinkState, AuthorProvider};

        let key_for = |provider: AuthorProvider| {
            routes
                .iter()
                .find(|route| route.key.provider() == provider)
                .map(|route| route.key.value())
        };
        let compatibility = crate::AuthorCompatibilityProjection {
            ol_key: key_for(AuthorProvider::OpenLibrary),
            gr_key: key_for(AuthorProvider::Goodreads),
            hc_key: key_for(AuthorProvider::Hardcover),
        };
        let link_state = if under_review {
            AuthorLinkState::NeedsReview
        } else if routes.is_empty() {
            AuthorLinkState::Unlinked
        } else {
            AuthorLinkState::Linked
        };
        Self {
            monitorable: compatibility.ol_key.is_some(),
            routes,
            link_state,
            compatibility,
        }
    }
}
