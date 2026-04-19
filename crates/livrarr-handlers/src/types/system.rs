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
    pub log_file: String,
    pub startup_time: DateTime<Utc>,
    pub log_level: String,
}

#[trait_variant::make(Send)]
pub trait SystemApi: Send + Sync {
    async fn health(&self) -> Result<Vec<HealthCheckResult>, ApiError>;
    async fn status(&self) -> Result<SystemStatus, ApiError>;
}
