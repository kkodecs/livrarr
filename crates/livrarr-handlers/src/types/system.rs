use chrono::{DateTime, Utc};
use livrarr_domain::HealthCheckType;
use serde::{Deserialize, Serialize};

use super::api_error::ApiError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckResult {
    pub source: String,
    pub check_type: HealthCheckType,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    pub version: String,
    pub os_info: String,
    pub data_directory: String,
    /// The daily rolling log file the appender actually writes (REQ-003).
    pub log_file: String,
    /// Last write time of the active log file; `None` when it doesn't exist.
    #[serde(default)]
    pub log_last_write: Option<DateTime<Utc>>,
    /// Log-dir creation/write failure captured at startup — surfaced loudly
    /// instead of swallowed (REQ-003, #102's vector).
    #[serde(default)]
    pub log_init_error: Option<String>,
    pub startup_time: DateTime<Utc>,
    pub log_level: String,
    /// Current process resident memory (physical RAM) in bytes; `None` when
    /// unavailable (non-Linux or read failure).
    #[serde(default)]
    pub rss_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthSummaryResponse {
    pub llm: LlmStatus,
    pub indexers: Vec<InfraItemStatus>,
    pub download_clients: Vec<InfraItemStatus>,
    pub rss_sync: RssSyncStatus,
    pub metadata_providers: Vec<ProviderStatus>,
    pub library: LibraryStats,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmStatus {
    pub configured: bool,
    pub enabled: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfraItemStatus {
    pub id: i64,
    pub name: String,
    pub implementation: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RssSyncStatus {
    pub running: bool,
    pub last_run_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub name: String,
    pub status: &'static str,
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    pub work_count: i64,
    pub library_item_count: i64,
    pub total_size_bytes: i64,
}

#[trait_variant::make(Send)]
pub trait SystemApi: Send + Sync {
    async fn health(&self) -> Result<Vec<HealthCheckResult>, ApiError>;
    async fn status(&self) -> Result<SystemStatus, ApiError>;
}
