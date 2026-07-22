//! Admin-approved Readarr origin allowlist: `ReadarrOriginDb` trait (Unit B3
//! Part 1 — origin trust boundary). Storage only — the admission POLICY
//! (approved-list OR SSRF-safe-public) lives at the `ReadarrImportWorkflow`
//! call site, never here (mirrors the `ProviderResponseCacheDb` split).

use crate::DbError;

/// One admin-approved Readarr origin. `origin` is the NORMALIZED form
/// (`livrarr_http::normalized_origin` — lowercased `scheme://host[:port]`,
/// default port omitted, no path) so lookup is a plain equality check.
/// `created_at` is an RFC3339 string — display-only, no caller does date
/// arithmetic on it, so no `chrono` round-trip is needed here.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadarrOrigin {
    pub id: i64,
    pub origin: String,
    pub created_at: String,
}

/// Shared infrastructure: admin-managed, visible to all users (mirrors
/// `RootFolderDb`). Only an admin can add/remove entries; any authenticated
/// user's Readarr import may connect against an approved origin.
#[trait_variant::make(Send)]
pub trait ReadarrOriginDb: Send + Sync {
    async fn list_readarr_origins(&self) -> Result<Vec<ReadarrOrigin>, DbError>;

    /// Insert a new approved origin. `origin` must already be normalized by
    /// the caller. A duplicate (`UNIQUE(origin)`) surfaces as `DbError` —
    /// callers map it to a conflict response.
    async fn create_readarr_origin(&self, origin: &str) -> Result<ReadarrOrigin, DbError>;

    /// Delete one entry by id. A missing id is a no-op success.
    async fn delete_readarr_origin(&self, id: i64) -> Result<(), DbError>;

    /// True iff `origin` (already normalized by the caller) matches an
    /// approved entry exactly.
    async fn is_readarr_origin_approved(&self, origin: &str) -> Result<bool, DbError>;
}
