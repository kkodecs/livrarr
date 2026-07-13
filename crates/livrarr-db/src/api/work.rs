//! Work data access: `WorkDbCreate` + `WorkDb` + `EnrichmentRetryDb` traits, plus request types.

use chrono::{DateTime, Utc};

use crate::{
    ApplyMergeOutcome, AuthorId, DbError, EnrichmentStatus, MediaType, MergeResolved,
    NarrationType, SetFieldProvenanceRequest, UserId, Work, WorkField, WorkId,
};

/// Work creation — separated from `WorkDb` so only `WorkServiceImpl` can
/// create works (compile-time enforcement of M2: single creation gate).
/// All other code paths must call `WorkService::add()`.
#[trait_variant::make(Send)]
pub trait WorkDbCreate: Send + Sync {
    /// Create work. Returns `(work, actually_created)`.
    /// `actually_created == false` indicates the UNIQUE constraint matched
    /// an existing row; the returned `work` is the existing one.
    ///
    /// Precondition: `normalized_title` and `normalized_author` were computed
    ///               via `livrarr_domain::identity_matching::identity_key()`
    ///               (REQ-014; supersedes the retired `normalize_for_matching`).
    async fn create_work(&self, req: CreateWorkDbRequest) -> Result<(Work, bool), DbError>;

    async fn create_work_with_anchor(
        &self,
        req: CreateWorkDbRequest,
        ol_key: &str,
        anchor_setter: livrarr_domain::identity::AnchorSetter,
    ) -> Result<(Work, bool), DbError>;
}

/// Work data access. All queries scoped to user_id.
///
/// Satisfies: AUTH-003
#[trait_variant::make(Send)]
pub trait WorkDb: Send + Sync {
    /// Get work by ID for a specific user.
    async fn get_work(&self, user_id: UserId, id: WorkId) -> Result<Work, DbError>;

    /// List works for a user (unbounded — for internal use).
    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError>;

    /// List works for a specific author.
    async fn list_works_by_author(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<Work>, DbError>;

    /// List works for a user, paginated with server-side sort.
    #[allow(clippy::too_many_arguments)]
    async fn list_works_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
        sort_by: &str,
        sort_dir: &str,
        media_type: Option<MediaType>,
        language: Option<&str>,
    ) -> Result<(Vec<Work>, i64), DbError>;

    /// Update work (enrichment fields -- overwrites).
    async fn update_work_enrichment(
        &self,
        user_id: UserId,
        id: WorkId,
        req: UpdateWorkEnrichmentDbRequest,
    ) -> Result<Work, DbError>;

    /// Update user-editable fields only.
    ///
    /// Satisfies: SEARCH-013
    async fn update_work_user_fields(
        &self,
        user_id: UserId,
        id: WorkId,
        req: UpdateWorkUserFieldsDbRequest,
    ) -> Result<Work, DbError>;

    /// Set cover_manual flag.
    ///
    /// Satisfies: SEARCH-014
    async fn set_cover_manual(
        &self,
        user_id: UserId,
        id: WorkId,
        manual: bool,
    ) -> Result<(), DbError>;

    /// Set the persisted identity-confidence badge (REQ-014 two-state split):
    /// the flat, user-facing identity status derived from a work's anchors.
    async fn set_identity_status(
        &self,
        user_id: UserId,
        id: WorkId,
        status: livrarr_domain::IdentityStatus,
    ) -> Result<(), DbError>;

    #[allow(clippy::too_many_arguments)]
    async fn update_cover_metadata(
        &self,
        user_id: UserId,
        work_id: WorkId,
        cover_url: Option<&str>,
        cover_source: &str,
        cover_trust: livrarr_domain::CoverTrust,
        cover_width: i32,
        cover_height: i32,
    ) -> Result<(), DbError>;

    #[allow(clippy::too_many_arguments)]
    async fn update_audiobook_cover_metadata(
        &self,
        user_id: UserId,
        work_id: WorkId,
        audiobook_cover_url: Option<&str>,
        audiobook_cover_source: &str,
        audiobook_cover_trust: livrarr_domain::CoverTrust,
        audiobook_cover_width: i32,
        audiobook_cover_height: i32,
    ) -> Result<(), DbError>;

    async fn update_cover_dimensions(
        &self,
        user_id: UserId,
        work_id: WorkId,
        width: i32,
        height: i32,
    ) -> Result<(), DbError>;

    async fn update_audiobook_cover_dimensions(
        &self,
        user_id: UserId,
        work_id: WorkId,
        width: i32,
        height: i32,
    ) -> Result<(), DbError>;

    /// Delete work. Returns deleted work for file cleanup.
    async fn delete_work(&self, user_id: UserId, id: WorkId) -> Result<Work, DbError>;

    /// Merge two works in one transaction (REQ-015): reassigns `loser_id`'s
    /// library items and grabs to `survivor_id`, writes the caller-resolved
    /// user-sovereign field values onto the survivor, then deletes the
    /// loser row — in that order, so FK `ON DELETE CASCADE` from `works`
    /// never fires on a row that still has children (library_items/grabs
    /// are reassigned first, so nothing is left to cascade-delete; the
    /// loser's identity/enrichment metadata, which is NOT reassigned, is
    /// expected to cascade away with the row). Ownership of both ids by
    /// `req.user_id` is re-verified inside the transaction — returns
    /// `NotFound` if either id doesn't belong to the caller, without
    /// revealing which (AC-024).
    async fn merge_works(&self, req: MergeWorksDbRequest) -> Result<Work, DbError>;

    /// Set (or NULL) a work's series_id directly — the series-reconcile link
    /// path. Unlike `SeriesDb::link_work_to_series` this performs NO
    /// assignment-guard arbitration and touches no other column; the caller
    /// (series_link service) owns the arbitration rules.
    async fn set_work_series_id(
        &self,
        user_id: UserId,
        work_id: WorkId,
        series_id: Option<i64>,
    ) -> Result<(), DbError>;

    /// Rewrite a work's series_name to its normalized form and fill
    /// series_position from an extracted positional suffix — position is
    /// written only when the work has none (COALESCE; an existing position is
    /// never clobbered).
    async fn normalize_work_series_fields(
        &self,
        user_id: UserId,
        work_id: WorkId,
        series_name: &str,
        series_position: Option<f64>,
    ) -> Result<(), DbError>;

    /// All-users listing of works carrying an orphan series string: non-empty
    /// series_name, NULL series_id, non-NULL author_id. System back-fill job
    /// only (cross-user by design, like the chapter back-fill listing).
    async fn list_orphan_series_works_all_users(&self) -> Result<Vec<Work>, DbError>;

    /// Check if user already has a work with given ol_key.
    ///
    /// Satisfies: SEARCH-004 (duplicate detection)
    async fn work_exists_by_ol_key(&self, user_id: UserId, ol_key: &str) -> Result<bool, DbError>;

    /// List all works for bulk re-enrichment.
    ///
    /// Satisfies: SEARCH-011
    async fn list_works_for_enrichment(&self, user_id: UserId) -> Result<Vec<Work>, DbError>;

    /// Get all works for a user by a specific author (for monitoring dedup).
    ///
    /// Satisfies: AUTHOR-002
    async fn list_works_by_author_ol_keys(
        &self,
        user_id: UserId,
        author_ol_key: &str,
    ) -> Result<Vec<String>, DbError>;

    async fn list_work_provider_keys_by_author(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<(Option<String>, Option<String>)>, DbError>;

    /// Find works by normalized title + author match (for manual scan matching).
    ///
    /// Satisfies: IMPORT-017
    async fn find_by_normalized_match(
        &self,
        user_id: UserId,
        title: &str,
        author: &str,
    ) -> Result<Vec<Work>, DbError>;

    async fn find_normalized_match_no_anchor_for_user(
        &self,
        user_id: UserId,
        raw_title: &str,
        raw_author: &str,
    ) -> Result<Option<Work>, DbError>;

    /// Find the user's works sharing an edition-bridge identifier (isbn_13 or
    /// asin) with the given candidate (responsiveness U-A, design §2.4). A
    /// local, zero-network lookup hint for `add_fast`'s verdict-gated bridge
    /// dedup — the caller gates matches through the identity-matching
    /// authority, never treats a bridge hit as merge evidence on its own.
    async fn find_works_by_bridge(
        &self,
        user_id: UserId,
        isbn_13: Option<&str>,
        asin: Option<&str>,
    ) -> Result<Vec<Work>, DbError>;

    /// List all works where monitor_ebook=1 OR monitor_audiobook=1, across all users.
    ///
    /// Satisfies: RSS-MATCH-001, RSS-FILTER-002
    async fn list_monitored_works_all_users(&self) -> Result<Vec<Work>, DbError>;

    /// Every (work id, owning user id) pair, across all users. System startup
    /// migration only — the S4 cover-layout migration maps a legacy
    /// root-level cover file's embedded work id to the user directory that
    /// should own it.
    async fn list_work_owners_all_users(&self) -> Result<Vec<(WorkId, UserId)>, DbError>;

    /// List works stuck in Unenriched state older than threshold (crash recovery).
    async fn list_identity_pending_works(&self) -> Result<Vec<Work>, DbError>;

    async fn list_stale_unenriched_works(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Work>, DbError>;

    /// List Failed works with no provider_retry_state rows
    /// (failed before any provider was queried).
    async fn list_failed_works_without_retry_state(&self) -> Result<Vec<Work>, DbError>;

    /// TEMP(pk-tdd): compile-only scaffold — apply a merge result to the work record.
    async fn apply_enrichment_merge(
        &self,
        req: ApplyEnrichmentMergeRequest,
    ) -> Result<ApplyMergeOutcome, DbError>;

    /// TEMP(pk-tdd): compile-only scaffold — reset enrichment state for manual refresh.
    async fn reset_for_manual_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError>;

    /// Select works due for a background convergence pass (REQ-006/009): identity
    /// still pending, OR confirmed/provisional with incomplete enrichment, OR
    /// confirmed/provisional with a chaseable missing anchor (NULL, not already a
    /// pending guess, not at the dead-end `threshold`). Only the missing-anchor
    /// branch is chaseable-gated. Oldest-first, capped at `limit`.
    async fn list_convergence_due(
        &self,
        user_id: UserId,
        now: DateTime<Utc>,
        threshold: u32,
        limit: i64,
    ) -> Result<Vec<WorkId>, DbError>;

    /// Set (or clear, when `at` is `None`) a work's next-convergence-due time.
    async fn set_next_convergence_at(
        &self,
        user_id: UserId,
        work_id: WorkId,
        at: Option<DateTime<Utc>>,
    ) -> Result<(), DbError>;

    /// TEMP(pk-tdd): compile-only scaffold — list works in Conflict status.
    async fn list_conflict_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError>;

    /// TEMP(pk-tdd): compile-only scaffold — get current merge generation counter.
    async fn get_merge_generation(&self, user_id: UserId, work_id: WorkId) -> Result<i64, DbError>;

    /// Search works by title or author_name LIKE match, paginated.
    async fn search_works(
        &self,
        user_id: UserId,
        query: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Work>, i64), DbError>;
}

#[derive(Debug, Clone, Default)]
pub struct CreateWorkDbRequest {
    pub user_id: i64,
    pub title: String,
    pub author_name: String,
    pub normalized_title: String,
    pub normalized_author: String,
    pub author_id: Option<i64>,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub language: Option<String>,
    pub import_id: Option<String>,
    pub series_id: Option<i64>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
    pub description: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub source_provider_json: Option<String>,
    pub cover_manual: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateWorkEnrichmentDbRequest {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub original_title: Option<String>,
    pub author_name: Option<String>,
    pub description: Option<String>,
    pub year: Option<i32>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub genres: Option<Vec<String>>,
    pub language: Option<String>,
    pub page_count: Option<i32>,
    pub duration_seconds: Option<i32>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub narrator: Option<Vec<String>>,
    pub narration_type: Option<NarrationType>,
    pub abridged: Option<bool>,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
    pub enrichment_status: EnrichmentStatus,
    pub enrichment_source: Option<String>,
    pub cover_url: Option<String>,
}

#[derive(Default)]
pub struct UpdateWorkUserFieldsDbRequest {
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub normalized_title: Option<String>,
    pub normalized_author: Option<String>,
    pub series_name: Option<Option<String>>,
    pub series_position: Option<Option<f64>>,
    pub monitor_ebook: Option<bool>,
    pub monitor_audiobook: Option<bool>,
}

/// Request for `WorkDb::merge_works` (REQ-015). The monitoring/series
/// fields carry the FINAL, already-resolved values the survivor should end
/// up with — the service layer, not the DB layer, decides the OR/conflict
/// outcome; this request just writes it.
pub struct MergeWorksDbRequest {
    pub user_id: UserId,
    pub survivor_id: WorkId,
    pub loser_id: WorkId,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

/// TEMP(pk-tdd): Request to apply an enrichment merge result to a work.
pub struct ApplyEnrichmentMergeRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub expected_merge_generation: i64,
    pub work_update: Option<MergeResolved<UpdateWorkEnrichmentDbRequest>>,
    pub new_enrichment_status: EnrichmentStatus,
    pub provenance_upserts: Vec<SetFieldProvenanceRequest>,
    pub provenance_deletes: Vec<WorkField>,
}

// ---------------------------------------------------------------------------
// v2.1 — Enrichment Retry DB
// ---------------------------------------------------------------------------

/// Enrichment retry operations. Extends v2 WorkDb contract.
///
/// Satisfies: IMPL-JOBS-005
#[trait_variant::make(Send)]
pub trait EnrichmentRetryDb: Send + Sync {
    /// Reset enrichment for manual refresh: status=pending.
    async fn reset_enrichment_for_refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError>;
}
