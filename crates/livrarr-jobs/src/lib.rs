use livrarr_domain::{AuthorId, GrabId, NotificationId, RootFolderId, UserId, WorkId};

// =============================================================================
// CRATE: livrarr-jobs
// =============================================================================
// Background tasks.

// ---------------------------------------------------------------------------
// Job Service (trigger interface for server)
// ---------------------------------------------------------------------------

/// Background job trigger and status.
#[trait_variant::make(Send)]
pub trait JobService: Send + Sync {
    /// Trigger bulk re-enrichment for all user's works. Returns immediately (202).
    async fn trigger_bulk_enrichment(&self, user_id: UserId) -> Result<(), JobError>;

    /// Trigger author monitoring check for all monitored authors. Returns immediately (202).
    async fn trigger_author_search(&self) -> Result<(), JobError>;

    /// Trigger manual scan of a root folder. Returns immediately (202).
    async fn trigger_scan(
        &self,
        user_id: UserId,
        root_folder_id: RootFolderId,
    ) -> Result<(), JobError>;
}

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("job already running")]
    AlreadyRunning,
    #[error("job failed: {message}")]
    Failed {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

// ---------------------------------------------------------------------------
// Download Poller
// ---------------------------------------------------------------------------

/// Background task: polls qBit clients for completed downloads, triggers import.
#[trait_variant::make(Send)]
pub trait DownloadPoller: Send + Sync {
    /// Run one poll cycle.
    async fn poll(&self) -> Result<Vec<PollResult>, JobError>;
}

pub struct PollResult {
    pub grab_id: GrabId,
    pub action: PollAction,
}

pub enum PollAction {
    ImportTriggered,
    MarkedFailed { reason: String },
    Skipped { reason: String },
}

// ---------------------------------------------------------------------------
// Author Monitor
// ---------------------------------------------------------------------------

/// Background task: checks monitored authors for new works.
#[trait_variant::make(Send)]
pub trait AuthorMonitor: Send + Sync {
    /// Run one monitoring cycle for all monitored authors.
    async fn check_all(&self) -> Result<Vec<AuthorMonitorResult>, JobError>;
}

pub struct AuthorMonitorResult {
    pub author_id: AuthorId,
    pub new_works_detected: Vec<DetectedWork>,
    pub auto_added: Vec<WorkId>,
    pub notifications_created: Vec<NotificationId>,
    pub warnings: Vec<String>,
}

pub struct DetectedWork {
    pub ol_key: String,
    pub title: String,
    pub publish_year: Option<i32>,
}
