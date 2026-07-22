//! Import-intent crash-consistency record (Unit D2): `ImportIntentDb` trait.
//!
//! Storage only — the state machine policy (when to create/advance an
//! intent, and how startup recovery reconciles it) lives at the
//! `ImportWorkflow` call sites, never here (mirrors the
//! `ProviderResponseCacheDb` storage-only split).

use chrono::{DateTime, Utc};

use crate::{DbError, MediaType, RootFolderId, UserId, WorkId};

/// Where one file's crash-consistent import sequence currently stands.
///
/// `Staging`: the intent is persisted but the atomic rename to the target
/// path has not been confirmed durable yet. Recovery must still check the
/// filesystem in this state — a crash can land here even after the rename
/// actually succeeded, since the transition to `Renamed` is a separate
/// write that can itself be interrupted.
///
/// `Renamed`: the atomic rename + parent-directory fsync are durably
/// complete. Only the `LibraryItem` finalize write and clearing this intent
/// remain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportIntentState {
    Staging,
    Renamed,
}

/// One in-flight file import, tracked from before the first byte is staged
/// until the `LibraryItem` row is finalized and the intent is cleared.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportIntent {
    pub id: i64,
    pub user_id: UserId,
    pub work_id: WorkId,
    pub root_folder_id: RootFolderId,
    pub media_type: MediaType,
    /// Relative to the root folder — matches the convention every other
    /// path column uses (e.g. `LibraryItem::path`).
    pub target_relative: String,
    /// Absolute path to the reserved staging file (a `tempfile::Builder`
    /// name in the destination directory, never predictable).
    pub staging_path: String,
    pub expected_size: i64,
    pub import_id: Option<String>,
    pub state: ImportIntentState,
    pub created_at: DateTime<Utc>,
}

pub struct CreateImportIntentDbRequest {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub root_folder_id: RootFolderId,
    pub media_type: MediaType,
    pub target_relative: String,
    pub staging_path: String,
    pub expected_size: i64,
    pub import_id: Option<String>,
}

/// Persistent storage for import-intent crash-consistency records
/// (Unit D2).
#[trait_variant::make(Send)]
pub trait ImportIntentDb: Send + Sync {
    /// Insert a new intent in `Staging` state.
    async fn create_import_intent(
        &self,
        req: CreateImportIntentDbRequest,
    ) -> Result<ImportIntent, DbError>;

    /// Advance an existing intent to `Renamed` — the atomic rename and the
    /// parent-directory fsync are durably complete.
    async fn mark_import_intent_renamed(&self, id: i64) -> Result<(), DbError>;

    /// All outstanding intents, across every user (a startup-recovery-only
    /// read — there is no "current user" at that point). Order is not
    /// load-bearing: every row is reconciled independently under its own
    /// work's import lock.
    async fn list_import_intents(&self) -> Result<Vec<ImportIntent>, DbError>;

    /// Delete one intent by id. A missing id is a no-op success — recovery
    /// relies on this to be safely retryable.
    async fn delete_import_intent(&self, id: i64) -> Result<(), DbError>;
}
