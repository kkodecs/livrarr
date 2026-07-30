//! Author data access: `AuthorDb` trait + request types.

use crate::{Author, AuthorId, DbError, RenameAuthorDbRequest, UserId};
use livrarr_domain::AuthorMonitorTarget;

/// Author data access.
///
/// Satisfies: AUTHOR-001, SEARCH-005
#[trait_variant::make(Send)]
pub trait AuthorDb: Send + Sync {
    /// Get author by ID for a user.
    async fn get_author(&self, user_id: UserId, id: AuthorId) -> Result<Author, DbError>;

    /// List authors for a user.
    async fn list_authors(&self, user_id: UserId) -> Result<Vec<Author>, DbError>;

    /// Create author, or converge on the existing row holding the same
    /// (user, stored identity key). The bool is `true` iff a new row was
    /// inserted; a creation-race loser gets the winning row and `false`,
    /// indistinguishable from a lookup hit (issue #175). Rows whose name
    /// does not canonicalize store a NULL key and always insert (ST-010).
    async fn create_author(&self, req: CreateAuthorDbRequest) -> Result<(Author, bool), DbError>;

    /// Update author (monitoring settings). A name change recomputes the
    /// stored identity key in the same statement; a recomputed key already
    /// held by a different row fails with `DbError::IdentityCollision`
    /// naming that row, with nothing written.
    async fn update_author(
        &self,
        user_id: UserId,
        id: AuthorId,
        req: UpdateAuthorDbRequest,
    ) -> Result<Author, DbError>;

    /// Delete author.
    async fn delete_author(&self, user_id: UserId, id: AuthorId) -> Result<(), DbError>;

    /// Find author by exact normalized name for a user (dedup).
    ///
    /// Satisfies: SEARCH-005, AUTHOR-001
    /// Merge `loser_id` into `survivor_id` in ONE transaction (author-dedup
    /// design §1): repoint works (author_id + display author_name +
    /// merge_generation bump; normalized_author untouched), fold/move series
    /// with monitoring intent preserved, drop loser caches, fill survivor's
    /// missing fields monotonically, delete the loser row.
    async fn merge_authors(
        &self,
        user_id: UserId,
        survivor_id: AuthorId,
        loser_id: AuthorId,
    ) -> Result<livrarr_domain::services::AuthorMergeReport, DbError>;

    async fn find_author_by_name(
        &self,
        user_id: UserId,
        normalized_name: &str,
    ) -> Result<Option<Author>, DbError>;

    /// List monitored authors with ol_key for a specific user (for author monitoring job).
    ///
    /// Satisfies: AUTHOR-002
    async fn list_monitored_authors(&self, user_id: UserId) -> Result<Vec<Author>, DbError>;

    async fn list_author_monitor_targets(
        &self,
        user_id: UserId,
    ) -> Result<Vec<AuthorMonitorTarget>, DbError>;

    /// The display-name cascade for a name the user chose.
    ///
    /// One transaction: exactly one variant carries the user's selection,
    /// `authors.name` and `works.author_name` change, and `merge_generation`
    /// bumps so tag convergence re-syncs. `works.normalized_author` is never
    /// touched.
    async fn rename_author_and_cascade(
        &self,
        request: RenameAuthorDbRequest,
    ) -> Result<Author, DbError>;

    /// The same cascade for a name automatic convergence ranked.
    ///
    /// It moves `authors.name`, `works.author_name`, and `merge_generation`
    /// exactly as the user-chosen path does, and touches no user authority: no
    /// `User` variant is fabricated and no `user_selected_at` is stamped or
    /// cleared. Writing one would make the machine's own choice outrank every
    /// later provider name and freeze convergence on it.
    async fn converge_author_display_name(
        &self,
        request: RenameAuthorDbRequest,
    ) -> Result<Author, DbError>;
}

pub struct CreateAuthorDbRequest {
    pub user_id: UserId,
    pub name: String,
    pub sort_name: Option<String>,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
    pub import_id: Option<String>,
}

pub struct UpdateAuthorDbRequest {
    pub name: Option<String>,
    pub sort_name: Option<Option<String>>,
    pub ol_key: Option<Option<String>>,
    pub gr_key: Option<Option<String>>,
    pub monitored: Option<bool>,
    pub monitor_new_items: Option<bool>,
    pub monitor_since: Option<chrono::DateTime<chrono::Utc>>,
    /// `None` = leave unchanged; `Some(None)` = clear back to unset.
    pub monitor_language: Option<Option<String>>,
}
