//! Grab data access: `GrabDb` trait + request type.

use chrono::{DateTime, Utc};

use crate::{DbError, DownloadClientId, Grab, GrabId, GrabStatus, MediaType, UserId, WorkId};

/// Grab data access.
///
/// Satisfies: DLC-006, DLC-009, DLC-012, DLC-015
#[trait_variant::make(Send)]
pub trait GrabDb: Send + Sync {
    async fn get_grab(&self, user_id: UserId, id: GrabId) -> Result<Grab, DbError>;

    /// List every grab for a work (merge preview/reassignment, REQ-015 c).
    async fn list_grabs_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<Grab>, DbError>;

    /// List active grabs (sent/confirmed) for import polling.
    ///
    /// Satisfies: IMPORT-005
    async fn list_active_grabs(&self) -> Result<Vec<Grab>, DbError>;

    /// Create or replace grab. Enforces UNIQUE(user_id, guid, indexer).
    /// If existing grab is failed/removed, replaces it. If active, returns Constraint error.
    ///
    /// Satisfies: DLC-009
    async fn upsert_grab(&self, req: CreateGrabDbRequest) -> Result<Grab, DbError>;

    /// Update grab status.
    async fn update_grab_status(
        &self,
        user_id: UserId,
        id: GrabId,
        status: GrabStatus,
        import_error: Option<&str>,
    ) -> Result<(), DbError>;

    /// Update grab download_id (torrent hash set after confirmation).
    async fn update_grab_download_id(
        &self,
        user_id: UserId,
        id: GrabId,
        download_id: &str,
    ) -> Result<(), DbError>;

    /// Get grab by download_id (torrent hash) for poller matching.
    /// Note: cross-user lookup by design -- poller matches torrent hashes across all users.
    /// Scoping enforced by subsequent operations, not this query.
    async fn get_grab_by_download_id(&self, download_id: &str) -> Result<Option<Grab>, DbError>;

    /// Reset all importing grabs to confirmed (startup recovery — JOBS-003).
    async fn reset_importing_grabs(&self) -> Result<u64, DbError>;

    /// Persist the raw remote content path from the download client.
    async fn set_grab_content_path(
        &self,
        user_id: UserId,
        id: GrabId,
        content_path: &str,
    ) -> Result<(), DbError>;

    /// List grabs for a user, paginated, newest first.
    async fn list_grabs_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Grab>, i64), DbError>;

    /// Atomically transition grab to `importing` status.
    /// Only succeeds if current status is sent/confirmed/importing/importFailed.
    /// Returns true if transition happened, false if grab was in a non-importable state.
    ///
    /// Satisfies: IMPORT-V21-001 (atomic transition prevents concurrent imports)
    async fn try_set_importing(&self, user_id: UserId, id: GrabId) -> Result<bool, DbError>;

    /// Check if user has an active grab (sent/confirmed/importing) for this work+media type.
    ///
    /// Satisfies: RSS-FILTER-002
    async fn active_grab_exists(
        &self,
        user_id: UserId,
        work_id: WorkId,
        media_type: MediaType,
    ) -> Result<bool, DbError>;

    /// Check if a grab already exists for this exact release (guid) in a
    /// terminal-failed state (importFailed/failed) for this user+work+media_type.
    /// A fresh grab row must never re-attempt a release that already failed.
    ///
    /// Satisfies: 114a (Part 1)
    async fn release_already_failed(
        &self,
        user_id: UserId,
        work_id: WorkId,
        media_type: MediaType,
        guid: &str,
    ) -> Result<bool, DbError>;

    /// Count terminal-failed grabs (importFailed/failed) for this
    /// user+work+media_type since the given timestamp. Never counts `removed`
    /// grabs. Feeds the rss_grab_failure_limit 30-day cap.
    ///
    /// Satisfies: 114a (Part 2)
    async fn recent_failed_grab_count(
        &self,
        user_id: UserId,
        work_id: WorkId,
        media_type: MediaType,
        since: DateTime<Utc>,
    ) -> Result<i64, DbError>;

    /// List importFailed grabs eligible for retry (backoff expired, under max retries).
    async fn list_retriable_grabs(&self, max_retries: i32) -> Result<Vec<Grab>, DbError>;

    /// Increment retry count and set import_failed_at timestamp on a grab.
    async fn increment_import_retry(&self, user_id: UserId, id: GrabId) -> Result<(), DbError>;

    async fn queue_summary(&self, user_id: UserId)
        -> Result<livrarr_domain::QueueSummary, DbError>;
}

pub struct CreateGrabDbRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub download_client_id: DownloadClientId,
    pub title: String,
    pub indexer: String,
    pub guid: String,
    pub size: Option<i64>,
    pub download_url: String,
    pub download_id: Option<String>,
    pub status: GrabStatus,
    pub media_type: Option<MediaType>,
}
