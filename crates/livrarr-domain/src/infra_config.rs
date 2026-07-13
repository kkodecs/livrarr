//! Configuration for external systems the app talks to: download clients
//! (qBittorrent/SABnzbd/Transmission), Torznab/Newznab indexers, remote path
//! mappings, and LLM provider settings.

use crate::entities::{DownloadClientId, IndexerId, RemotePathMappingId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Download client implementation type.
///
/// Satisfies: DLC-002, USE-DLC-001
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DownloadClientImplementation {
    #[serde(rename = "qBittorrent")]
    #[default]
    QBittorrent,
    #[serde(rename = "sabnzbd")]
    SABnzbd,
    #[serde(rename = "transmission")]
    Transmission,
}

impl DownloadClientImplementation {
    /// Canonical client_type string for DB storage and protocol routing.
    pub fn client_type(&self) -> &'static str {
        match self {
            Self::QBittorrent => "qbittorrent",
            Self::SABnzbd => "sabnzbd",
            Self::Transmission => "transmission",
        }
    }

    pub fn protocol(&self) -> crate::services::DownloadProtocol {
        match self {
            Self::QBittorrent => crate::services::DownloadProtocol::Torrent,
            Self::SABnzbd => crate::services::DownloadProtocol::Usenet,
            Self::Transmission => crate::services::DownloadProtocol::Torrent,
        }
    }
}

/// LLM chat message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmRole {
    System,
    User,
    Assistant,
}

/// LLM provider presets.
///
/// Satisfies: CONFIG-004
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    Groq,
    Gemini,
    Openai,
    Custom,
}

/// Download client configuration.
///
/// Satisfies: DLC-001, DLC-002, USE-DLC-001, USE-DLC-004
#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DownloadClient {
    pub id: DownloadClientId,
    pub name: String,
    pub implementation: DownloadClientImplementation,
    pub host: String,
    pub port: u16,
    pub use_ssl: bool,
    pub skip_ssl_validation: bool,
    pub url_base: Option<String>,
    pub username: Option<String>,
    #[serde(skip_serializing)]
    pub password: Option<String>,
    pub category: String,
    pub download_dir: Option<String>,
    pub enabled: bool,
    #[serde(skip_serializing)]
    pub api_key: Option<String>,
    pub is_default_for_protocol: bool,
}

impl DownloadClient {
    /// Canonical client_type string derived from implementation — single source of truth.
    pub fn client_type(&self) -> &'static str {
        self.implementation.client_type()
    }
}

impl std::fmt::Debug for DownloadClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadClient")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("implementation", &self.implementation)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("use_ssl", &self.use_ssl)
            .field("skip_ssl_validation", &self.skip_ssl_validation)
            .field("url_base", &self.url_base)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("category", &self.category)
            .field("enabled", &self.enabled)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("is_default_for_protocol", &self.is_default_for_protocol)
            .finish()
    }
}

/// Remote path mapping.
///
/// Satisfies: DLC-013
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RemotePathMapping {
    pub id: RemotePathMappingId,
    pub host: String,
    pub remote_path: String,
    pub local_path: String,
}

/// Torznab/Newznab indexer configuration.
///
/// Satisfies: IDX-001, IDX-002, IDX-004, IDX-005, IDX-006, IDX-007
#[derive(Clone, Serialize, Deserialize)]
pub struct Indexer {
    pub id: IndexerId,
    pub name: String,
    pub protocol: String,
    pub url: String,
    pub api_path: String,
    #[serde(skip_serializing)]
    pub api_key: Option<String>,
    pub categories: Vec<i32>,
    pub priority: i32,
    pub enable_automatic_search: bool,
    pub enable_interactive_search: bool,
    pub supports_book_search: bool,
    pub enable_rss: bool,
    pub enabled: bool,
    pub added_at: DateTime<Utc>,
}

impl std::fmt::Debug for Indexer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Indexer")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("url", &self.url)
            .field("api_path", &self.api_path)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("categories", &self.categories)
            .field("priority", &self.priority)
            .field("enable_automatic_search", &self.enable_automatic_search)
            .field("enable_interactive_search", &self.enable_interactive_search)
            .field("supports_book_search", &self.supports_book_search)
            .field("enable_rss", &self.enable_rss)
            .field("enabled", &self.enabled)
            .field("added_at", &self.added_at)
            .finish()
    }
}

/// Per-indexer RSS sync state for gap detection.
///
/// Satisfies: RSS-GAP-001
#[derive(Debug, Clone)]
pub struct IndexerRssState {
    pub indexer_id: IndexerId,
    pub last_publish_date: Option<String>,
    pub last_guid: Option<String>,
}

/// Indexer config singleton (RSS sync settings).
///
/// Satisfies: RSS-CONFIG-001
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexerConfig {
    pub rss_sync_interval_minutes: i32,
    pub rss_match_threshold: f64,
    /// Max terminal-failed (importFailed/failed) grabs per (work, media_type)
    /// in a 30-day window before RSS auto-grab suppresses further attempts.
    /// 0 disables the cap. Default 3. Satisfies: 114a.
    pub rss_grab_failure_limit: i32,
}
