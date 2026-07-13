//! Series data access: `SeriesDb` trait + request types.

use crate::{AuthorId, DbError, Series, UserId, WorkId};

pub struct LinkWorkToSeriesRequest {
    pub work_id: WorkId,
    pub series_id: i64,
    pub series_work_count: i32,
    pub series_name: String,
    pub series_position: Option<f64>,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
}

pub struct CreateSeriesDbRequest {
    pub user_id: UserId,
    pub author_id: AuthorId,
    pub name: String,
    pub gr_key: String,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    /// `None` = leave an existing row's setting untouched (and unset on insert).
    pub monitor_language: Option<String>,
    pub work_count: i32,
}

#[trait_variant::make(Send)]
pub trait SeriesDb: Send + Sync {
    /// Get a series by ID, scoped to user.
    async fn get_series(&self, user_id: UserId, id: i64) -> Result<Option<Series>, DbError>;

    /// List all series for a user.
    async fn list_all_series(&self, user_id: UserId) -> Result<Vec<Series>, DbError>;

    /// List all series for an author.
    async fn list_series_for_author(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<Series>, DbError>;

    /// Create or update a series (upsert on user_id + author_id + gr_key).
    async fn upsert_series(&self, req: CreateSeriesDbRequest) -> Result<Series, DbError>;

    /// Update monitoring flags on a series and propagate to linked works.
    /// `monitor_language`: `Some` persists a new language for monitor-created
    /// works; `None` leaves the existing setting untouched. Never re-stamps
    /// linked works.
    async fn update_series_flags(
        &self,
        user_id: UserId,
        id: i64,
        monitor_ebook: bool,
        monitor_audiobook: bool,
        monitor_language: Option<String>,
    ) -> Result<Series, DbError>;

    /// Update work_count for a series.
    async fn update_series_work_count(
        &self,
        user_id: UserId,
        id: i64,
        work_count: i32,
    ) -> Result<(), DbError>;

    /// Link a work to a series (with assignment guard: only if current series_id is NULL
    /// or new series has smaller work_count). Validates work ownership.
    async fn link_work_to_series(
        &self,
        user_id: UserId,
        req: LinkWorkToSeriesRequest,
    ) -> Result<(), DbError>;

    /// List monitored series (either flag true) for a list of author IDs, scoped to user.
    async fn list_monitored_series_for_authors(
        &self,
        user_id: UserId,
        author_ids: &[AuthorId],
    ) -> Result<Vec<Series>, DbError>;

    /// Delete a series row. Linked works' series_id is NULLed by the FK
    /// (ON DELETE SET NULL).
    async fn delete_series(&self, user_id: UserId, id: i64) -> Result<(), DbError>;

    /// Count works FK-linked to a series.
    async fn count_works_in_series(&self, user_id: UserId, series_id: i64) -> Result<i64, DbError>;

    /// Relink every work pointing at `from_series_id` to `to_series_id`
    /// (stub-merge during promotion). Returns the number of works moved.
    async fn relink_series_works(
        &self,
        user_id: UserId,
        from_series_id: i64,
        to_series_id: i64,
    ) -> Result<u64, DbError>;

    /// Set a series' gr_key (stub promotion). When `work_count` is Some it is
    /// written too; None leaves the stored count untouched.
    async fn update_series_identity(
        &self,
        user_id: UserId,
        id: i64,
        gr_key: &str,
        work_count: Option<i32>,
    ) -> Result<(), DbError>;
}
