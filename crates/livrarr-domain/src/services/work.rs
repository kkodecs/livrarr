use serde::{Deserialize, Serialize};

use crate::{AuthorId, DbError, EnrichmentStatus, LibraryItem, MediaType, UserId, Work, WorkId};

/// Domain-owned source metadata from external systems (e.g., Readarr import).
/// Enters the enrichment pipeline as a provider input via MetadataProvider::Readarr.
/// Converted to NormalizedWorkDetail at the livrarr-metadata crate boundary.
#[derive(Debug, Clone, Default)]
pub struct SourceProviderData {
    pub description: Option<String>,
    pub isbn: Option<String>,
    pub asin: Option<String>,
    pub publisher: Option<String>,
    pub genres: Option<Vec<String>>,
    pub page_count: Option<i32>,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
    pub cover_url: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<String>,
}

#[derive(Debug)]
pub struct AddWorkResult {
    pub work: Work,
    /// true if a new work was created, false if dedup matched an existing work.
    pub created: bool,
    pub author_created: bool,
    pub author_id: Option<i64>,
    pub messages: Vec<String>,
    pub cover_mtime: Option<i64>,
    pub audiobook_cover_mtime: Option<i64>,
    /// Final enrichment status after synchronous enrichment attempt.
    pub enrichment_status: EnrichmentStatus,
}

/// Per-item result from tag sync. TagService returns these;
/// the caller updates DB tag_status per item.
#[derive(Debug)]
pub struct TagSyncItemResult {
    pub library_item_id: i64,
    pub succeeded: bool,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct UpdateWorkRequest {
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub series_name: Option<Option<String>>,
    pub series_position: Option<Option<f64>>,
    pub monitor_ebook: Option<bool>,
    pub monitor_audiobook: Option<bool>,
}

#[derive(Debug)]
pub struct WorkDetailView {
    pub work: Work,
    pub library_items: Vec<LibraryItem>,
    pub cover_mtime: Option<i64>,
    pub audiobook_cover_mtime: Option<i64>,
}

#[derive(Debug)]
pub struct PaginatedWorksView {
    pub works: Vec<WorkDetailView>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug)]
pub struct WorkFilter {
    /// Always AND'd with user_id at DB level — never bypasses tenant scoping.
    pub author_id: Option<AuthorId>,
    pub monitored: Option<bool>,
    pub enrichment_status: Option<EnrichmentStatus>,
    pub media_type: Option<MediaType>,
    /// Work language facet (REQ-015): exact match on `works.language`.
    pub language: Option<String>,
    pub sort_by: Option<WorkSortField>,
    pub sort_dir: Option<SortDirection>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkSortField {
    Title,
    DateAdded,
    Year,
    Author,
    RecentlyDownloaded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug)]
pub struct RefreshWorkResult {
    pub work: Work,
    pub messages: Vec<String>,
    pub taggable_items: Vec<LibraryItem>,
    pub merge_deferred: bool,
}

/// Outcome of a single user-triggered "retry all incomplete" sweep.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct RetrySummary {
    /// Incomplete works found and swept (Failed, Unenriched, or identity-Pending).
    pub total: usize,
    /// Works that reached a settled+enriched state after the pass.
    pub recovered: usize,
    /// Works still incomplete after the pass (left for a later retry).
    pub still_incomplete: usize,
}

// Dead: bulk refresh is implemented at the handler layer
// (`crates/livrarr-handlers/src/work.rs::refresh_all`) per insight 9g
// (handler-level spawning for long-running background work). This type
// was an earlier design that never wired up. Restore only if the spawn
// pattern is ever moved into the service layer.
// #[derive(Debug)]
// pub struct RefreshAllHandle {
//     pub total_works: usize,
// }

#[derive(Debug)]
pub struct LookupRequest {
    pub term: String,
    pub lang_override: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LookupResult {
    pub ol_key: Option<String>,
    pub title: String,
    pub author_name: String,
    pub author_ol_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_position: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isbn_13: Option<String>,
    /// Handle to the cached per-provider payloads fetched during discovery, so
    /// the add path can reuse them without re-querying (R-002/R-009; REQ-014/015).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<crate::identity::CandidateId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hc_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gr_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asin: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LookupResponse {
    pub results: Vec<LookupResult>,
    pub filtered_count: usize,
    pub raw_count: usize,
    pub raw_available: bool,
}

/// One parsed manual-import file's best-guess query for the eager auto-match
/// pass (#97). `id` ties the match back to the originating file (results are
/// returned as `(id, LookupResult)` pairs); the eager matcher groups these by
/// `author` so one author-scoped provider query serves all of that author's
/// files, then matches each `title` — or the embedded `isbn`, which pins the
/// exact edition — against the author's returned corpus.
#[derive(Debug, Clone)]
pub struct EagerQuery {
    pub id: usize,
    pub title: String,
    pub author: String,
    pub language: Option<String>,
    pub isbn: Option<String>,
}

/// A user-sovereign field that both works in a merge can independently
/// carry a value for (REQ-015 d). Title and author are deliberately
/// excluded — the survivor's identity fields are not up for negotiation in
/// a merge; only the survivor's own value is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeableField {
    SeriesName,
    SeriesPosition,
}

/// Which side's value to keep for one [`MergeableField`] conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeFieldChoice {
    KeepSurvivor,
    TakeLoser,
}

/// One explicit choice supplied to [`WorkService::merge_works`].
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct MergeFieldChoiceEntry {
    pub field: MergeableField,
    pub choice: MergeFieldChoice,
}

/// A field where both works carry a differing user value. Surfaced by
/// [`WorkService::preview_merge_works`]; the loser's value is shown, never
/// silently discarded (REQ-015 d).
#[derive(Debug, Clone)]
pub struct MergeFieldConflict {
    pub field: MergeableField,
    pub survivor_value: String,
    pub loser_value: String,
}

/// The plan for combining two works, computed without applying anything
/// (REQ-015 b).
#[derive(Debug, Clone)]
pub struct MergePreview {
    pub survivor_id: WorkId,
    pub loser_id: WorkId,
    /// Library items that will reassign to the survivor.
    pub library_items_moving: usize,
    /// Grabs that will reassign to the survivor.
    pub grabs_moving: usize,
    /// Monitoring flags are additive (OR'd) — never a conflict.
    pub monitor_ebook_result: bool,
    pub monitor_audiobook_result: bool,
    /// Fields needing an explicit choice at execute time (AC-025).
    pub conflicts: Vec<MergeFieldConflict>,
}

/// Outcome of [`WorkService::merge_works`].
#[derive(Debug, Clone)]
pub struct MergeWorksResult {
    pub survivor: Work,
    pub library_items_moved: usize,
    pub grabs_moved: usize,
    /// Non-fatal issues from the best-effort physical file reorganization
    /// step (REQ-015 c) — e.g. a destination path collision left a file at
    /// its prior location. The DB reassignment itself always completes in
    /// full; these are reorg-only warnings, never a sign of lost data.
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkServiceError {
    #[error("work not found")]
    NotFound,
    #[error("work already exists")]
    AlreadyExists,
    #[error("enrichment conflict")]
    EnrichmentConflict,
    #[error("cover too large")]
    CoverTooLarge,
    #[error("validation error: {0}")]
    Validation(String),
    #[error("enrichment failed: {0}")]
    Enrichment(String),
    #[error("cover download failed: {0}")]
    Cover(String),
    #[error("database error: {0}")]
    Db(#[from] DbError),
    /// A merge left one or more conflicting fields without an explicit
    /// choice (REQ-015 d, AC-025). The caller must re-request with a
    /// [`MergeFieldChoiceEntry`] for every field listed.
    #[error("merge requires an explicit choice for: {0:?}")]
    MergeChoiceRequired(Vec<MergeableField>),
}

/// Outcome of one [`WorkService::converge_work`] pass, driving the background
/// convergence job's next-attempt pacing. `Completed` — identity and enrichment
/// are both satisfied; stop selecting the work. `Terminal` — a dead-end was
/// reached (needs-review / conflict / not-found); stop. `StillIncomplete` —
/// progress made or mid-flight; re-select after the cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvergeOutcome {
    Completed,
    StillIncomplete,
    Terminal,
}

/// Surface that triggered a [`WorkService::refresh`] call. `Interactive` — a
/// person is watching (existing behavior). `Bulk` — an unattended sweep;
/// provider work rides the outbound queue at Low priority with background
/// identity semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshSurface {
    Interactive,
    Bulk,
}

#[trait_variant::make(Send)]
pub trait WorkService: Send + Sync {
    async fn add(
        &self,
        user_id: UserId,
        candidate: crate::identity::WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError>;
    /// Resolve a raw identity harvest into a [`crate::identity::ResolvedIdentity`]
    /// through the shared multi-provider resolver — the single place identity is
    /// decided, so every creation path agrees on what a book is (P1). `tier`
    /// selects interactive (synchronous; a person is waiting) vs background (bulk).
    /// With no resolver configured or no usable anchor in the harvest, returns a
    /// `Pending` identity (anchors preserved) and no conflict — never a fabricated
    /// `Confirmed`.
    async fn resolve_identity(
        &self,
        user_id: UserId,
        harvest: crate::identity::RawHarvest,
        tier: crate::identity::LatencyTier,
    ) -> Result<crate::identity::ResolvedIdentity, WorkServiceError>;

    /// REQ-004 (responsiveness): zero-network, zero-DB identity derivation for
    /// the interactive add door. Sanitizes the harvest and derives the badge
    /// from what the seed already carries — work anchor (ol/gr/hc) present →
    /// `Confirmed` (method: seed anchors); bridge-only or anchorless → `Pending`
    /// with the captured seed. Never resolves against providers, never returns
    /// a conflict; background completion (`complete_add`) owns those.
    fn resolve_identity_local(
        &self,
        harvest: crate::identity::RawHarvest,
    ) -> Result<crate::identity::ResolvedIdentity, WorkServiceError>;
    /// REQ-004 (responsiveness): the response-path half of [`Self::add`] —
    /// dedup (work-anchor, verdict-gated bridge, normalized), create, badge
    /// persist, and the phase-1 cover. Nothing provider-bound: returns before
    /// any identity fan-out, enrichment scatter, or cover-gate work.
    async fn add_fast(
        &self,
        user_id: UserId,
        candidate: crate::identity::WorkCandidate,
    ) -> Result<AddWorkResult, WorkServiceError>;
    /// REQ-004/005 (responsiveness): the background half of [`Self::add`] —
    /// identity completion + enrichment + cover gates + materialize, wrapped in
    /// the enriching-registry guard so [`Self::is_enriching`] reads true for
    /// the duration. Absorbs its own failures (logged, never panics the
    /// spawner); callers spawn-and-forget.
    async fn complete_add(
        &self,
        user_id: UserId,
        work_id: WorkId,
        source_provider_data: Option<SourceProviderData>,
        candidate_id: Option<crate::identity::CandidateId>,
        mode: crate::identity::IdentityMode,
        source: crate::identity::ConflictSource,
    );
    /// REQ-005 (responsiveness): true exactly while an enrichment run is
    /// executing for this work. In-memory signal — reads false after a server
    /// restart BY DESIGN (never stale-true; the convergence lane owns durable
    /// completion of interrupted runs).
    fn is_enriching(&self, user_id: UserId, work_id: WorkId) -> bool;
    async fn get(&self, user_id: UserId, work_id: WorkId) -> Result<Work, WorkServiceError>;
    async fn get_detail(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<WorkDetailView, WorkServiceError>;
    async fn list(
        &self,
        user_id: UserId,
        filter: WorkFilter,
    ) -> Result<Vec<Work>, WorkServiceError>;
    #[allow(clippy::too_many_arguments)]
    async fn list_paginated(
        &self,
        user_id: UserId,
        page: u32,
        page_size: u32,
        sort_by: WorkSortField,
        sort_dir: SortDirection,
        media_type: Option<MediaType>,
        language: Option<&str>,
    ) -> Result<PaginatedWorksView, WorkServiceError>;
    async fn update(
        &self,
        user_id: UserId,
        work_id: WorkId,
        req: UpdateWorkRequest,
    ) -> Result<Work, WorkServiceError>;
    async fn delete(&self, user_id: UserId, work_id: WorkId) -> Result<(), WorkServiceError>;
    async fn refresh(
        &self,
        user_id: UserId,
        work_id: WorkId,
        surface: RefreshSurface,
    ) -> Result<RefreshWorkResult, WorkServiceError>;
    /// User-triggered bulk recovery (REQ-011 / PO §7): sweep every "incomplete"
    /// work for the user — Failed, Unenriched, or identity-Pending — and re-run
    /// each through the one road in a **single pass, no recurring loop**. A
    /// Pending work re-resolves identity first (the convergence the deleted
    /// background job used to do); the rest re-enrich via [`Self::refresh`].
    /// Replaces the removed `enrichment_retry_tick`.
    async fn retry_all_incomplete(&self, user_id: UserId)
        -> Result<RetrySummary, WorkServiceError>;
    // Dead: bulk refresh is implemented at the handler layer
    // (`crates/livrarr-handlers/src/work.rs::refresh_all`) per insight 9g.
    // Restore here only if the spawn pattern is ever moved into services.
    // async fn refresh_all(&self, user_id: UserId) -> Result<RefreshAllHandle, WorkServiceError>;
    async fn upload_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
        bytes: &[u8],
    ) -> Result<(), WorkServiceError>;
    async fn download_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<u8>, WorkServiceError>;
    async fn search_works(
        &self,
        user_id: UserId,
        query: &str,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<Work>, i64), WorkServiceError>;
    /// Acquire the per-user bulk-refresh slot. `None` = a run is already live
    /// (the handler keeps its 409 semantics). The returned guard releases the
    /// slot on `Drop` — completion, error return, panic unwind, and task abort
    /// all free it; a leaked permanent 409 is structurally inexpressible
    /// (REQ-016).
    fn try_start_bulk_refresh(&self, user_id: i64) -> Option<BulkRefreshGuard>;

    /// Run one background convergence pass over a single work: settle a chaseable
    /// identity anchor (or terminalize an exhausted Pending work), run background
    /// enrichment when identity permits, and account dead-end retry counters.
    /// Called by the convergence job tick — a job cannot reach the private
    /// orchestration helpers, so this is the public entry point. `threshold` is
    /// the per-anchor dead-end attempt limit (REQ-009).
    async fn converge_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
        threshold: u32,
    ) -> Result<ConvergeOutcome, WorkServiceError>;

    /// Compute the merge plan for combining `loser_id` into `survivor_id`
    /// without applying anything (REQ-015 b). Both works must belong to
    /// `user_id` — a cross-user pair or an unknown id returns `NotFound`,
    /// never leaking whether the other id exists (AC-024).
    async fn preview_merge_works(
        &self,
        user_id: UserId,
        survivor_id: WorkId,
        loser_id: WorkId,
    ) -> Result<MergePreview, WorkServiceError>;

    /// Combine two works (REQ-015): `loser_id`'s library items and grabs
    /// reassign to `survivor_id`, monitoring flags OR together, and
    /// `choices` resolves every field [`preview_merge_works`] listed as
    /// conflicting — a conflicting field with no matching entry refuses the
    /// whole call (`MergeChoiceRequired`, AC-025) rather than guessing.
    /// The DB reassignment and loser deletion happen in one transaction
    /// (REQ-015 e); physical file reorganization under the survivor's
    /// canonical path is a separate, best-effort follow-up performed by the
    /// caller via `ImportService::reorganize_work_files` — this method
    /// never touches the filesystem and never deletes a file.
    async fn merge_works(
        &self,
        user_id: UserId,
        survivor_id: WorkId,
        loser_id: WorkId,
        choices: Vec<MergeFieldChoiceEntry>,
    ) -> Result<MergeWorksResult, WorkServiceError>;
}

/// RAII slot for the per-user bulk-refresh guard (REQ-016). Acquired via
/// [`WorkService::try_start_bulk_refresh`]; the slot is freed exclusively by
/// `Drop` — no method exists to leak it.
#[derive(Debug)]
pub struct BulkRefreshGuard {
    slots: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<i64>>>,
    user_id: i64,
}

impl BulkRefreshGuard {
    /// Wrap an already-acquired slot: callers insert `user_id` into `slots`
    /// (atomically deciding the race) and construct the guard only on success.
    pub fn new(
        slots: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<i64>>>,
        user_id: i64,
    ) -> Self {
        Self { slots, user_id }
    }
}

impl Drop for BulkRefreshGuard {
    fn drop(&mut self) {
        // A panicked peer must not wedge release: take the lock through poison.
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slots.remove(&self.user_id);
    }
}
