use crate::{DownloadClientImplementation, LlmProvider};

// =============================================================================
// Config return types (read from DB, returned by SettingsService)
// =============================================================================

pub struct NamingConfig {
    pub author_folder_format: String,
    pub book_folder_format: String,
    pub rename_files: bool,
    pub replace_illegal_chars: bool,
}

pub struct MediaManagementConfig {
    pub cwa_ingest_path: Option<String>,
    pub preferred_ebook_formats: Vec<String>,
    pub preferred_audiobook_formats: Vec<String>,
}

#[derive(Default)]
pub struct ProwlarrConfig {
    pub url: Option<String>,
    pub api_key: Option<String>,
    pub enabled: bool,
}

impl std::fmt::Debug for ProwlarrConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProwlarrConfig")
            .field("url", &self.url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[derive(Clone)]
pub struct MetadataConfig {
    pub hardcover_enabled: bool,
    pub hardcover_api_token: Option<String>,
    pub llm_enabled: bool,
    pub llm_provider: Option<LlmProvider>,
    pub llm_endpoint: Option<String>,
    pub llm_api_key: Option<String>,
    pub llm_model: Option<String>,
    pub audnexus_url: String,
    pub languages: Vec<String>,
}

impl std::fmt::Debug for MetadataConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetadataConfig")
            .field("hardcover_enabled", &self.hardcover_enabled)
            .field(
                "hardcover_api_token",
                &self.hardcover_api_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("llm_enabled", &self.llm_enabled)
            .field("llm_provider", &self.llm_provider)
            .field("llm_endpoint", &self.llm_endpoint)
            .field(
                "llm_api_key",
                &self.llm_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("llm_model", &self.llm_model)
            .field("audnexus_url", &self.audnexus_url)
            .field("languages", &self.languages)
            .finish()
    }
}

#[derive(Default)]
pub struct EmailConfig {
    pub enabled: bool,
    pub smtp_host: String,
    pub smtp_port: i32,
    pub encryption: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from_address: Option<String>,
    pub recipient_email: Option<String>,
    pub send_on_import: bool,
}

impl std::fmt::Debug for EmailConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailConfig")
            .field("enabled", &self.enabled)
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("encryption", &self.encryption)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("from_address", &self.from_address)
            .field("recipient_email", &self.recipient_email)
            .field("send_on_import", &self.send_on_import)
            .finish()
    }
}

// =============================================================================
// Config update param types (passed into SettingsService mutators)
// =============================================================================

pub struct UpdateMediaManagementParams {
    pub cwa_ingest_path: Option<String>,
    pub preferred_ebook_formats: Vec<String>,
    pub preferred_audiobook_formats: Vec<String>,
}

pub struct UpdateMetadataParams {
    pub hardcover_enabled: Option<bool>,
    pub hardcover_api_token: Option<Option<String>>,
    pub llm_enabled: Option<bool>,
    pub llm_provider: Option<LlmProvider>,
    pub llm_endpoint: Option<String>,
    pub llm_api_key: Option<Option<String>>,
    pub llm_model: Option<String>,
    pub audnexus_url: Option<String>,
    pub languages: Option<Vec<String>>,
}

pub struct UpdateProwlarrParams {
    pub url: Option<String>,
    pub api_key: Option<Option<String>>,
    pub enabled: Option<bool>,
}

pub struct UpdateEmailParams {
    pub enabled: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i32>,
    pub encryption: Option<String>,
    pub username: Option<String>,
    pub password: Option<Option<String>>,
    pub from_address: Option<String>,
    pub recipient_email: Option<String>,
    pub send_on_import: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIndexerConfigParams {
    pub rss_sync_interval_minutes: Option<i32>,
    pub rss_match_threshold: Option<f64>,
}

// =============================================================================
// Download client param types
// =============================================================================

pub struct CreateDownloadClientParams {
    pub name: String,
    pub implementation: DownloadClientImplementation,
    pub host: String,
    pub port: u16,
    pub use_ssl: bool,
    pub skip_ssl_validation: bool,
    pub url_base: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub category: String,
    pub enabled: bool,
    pub api_key: Option<String>,
}

#[derive(Default)]
pub struct UpdateDownloadClientParams {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub use_ssl: Option<bool>,
    pub skip_ssl_validation: Option<bool>,
    pub url_base: Option<String>,
    pub username: Option<String>,
    pub password: Option<Option<String>>,
    pub category: Option<String>,
    pub enabled: Option<bool>,
    pub api_key: Option<Option<String>>,
    pub is_default_for_protocol: Option<bool>,
}

// =============================================================================
// Indexer param types
// =============================================================================

pub struct CreateIndexerParams {
    pub name: String,
    pub protocol: String,
    pub url: String,
    pub api_path: String,
    pub api_key: Option<String>,
    pub categories: Vec<i32>,
    pub priority: i32,
    pub enable_automatic_search: bool,
    pub enable_interactive_search: bool,
    pub enable_rss: bool,
    pub enabled: bool,
}

pub struct UpdateIndexerParams {
    pub name: Option<String>,
    pub url: Option<String>,
    pub api_path: Option<String>,
    pub api_key: Option<Option<String>>,
    pub categories: Option<Vec<i32>>,
    pub priority: Option<i32>,
    pub enable_automatic_search: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enable_rss: Option<bool>,
    pub enabled: Option<bool>,
}
