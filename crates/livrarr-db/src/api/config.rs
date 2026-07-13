//! Configuration data access (DB singletons): `ConfigDb` trait + request types.

use serde::Deserialize;

use livrarr_domain::settings::{
    UpdateEmailParams, UpdateIndexerConfigParams, UpdateMediaManagementParams,
    UpdateMetadataParams, UpdateProwlarrParams,
};

use crate::{
    DbError, EmailConfig, IndexerConfig, LlmProvider, MediaManagementConfig, MetadataConfig,
    NamingConfig, ProwlarrConfig,
};

/// Configuration data access (DB singletons).
/// Shared infrastructure: admin-managed, visible to all users.
///
/// Satisfies: CONFIG-001, CONFIG-002, CONFIG-003, CONFIG-004, CONFIG-005, AUTH-004, RSS-CONFIG-001
#[trait_variant::make(Send)]
pub trait ConfigDb: Send + Sync {
    /// Get naming config (read-only singleton).
    async fn get_naming_config(&self) -> Result<NamingConfig, DbError>;

    /// Get media management config.
    async fn get_media_management_config(&self) -> Result<MediaManagementConfig, DbError>;

    /// Update media management config.
    async fn update_media_management_config(
        &self,
        req: UpdateMediaManagementConfigRequest,
    ) -> Result<MediaManagementConfig, DbError>;

    /// Get Prowlarr config.
    async fn get_prowlarr_config(&self) -> Result<ProwlarrConfig, DbError>;

    /// Update Prowlarr config.
    async fn update_prowlarr_config(
        &self,
        req: UpdateProwlarrConfigRequest,
    ) -> Result<ProwlarrConfig, DbError>;

    /// Get metadata config.
    async fn get_metadata_config(&self) -> Result<MetadataConfig, DbError>;

    /// Update metadata config.
    async fn update_metadata_config(
        &self,
        req: UpdateMetadataConfigRequest,
    ) -> Result<MetadataConfig, DbError>;

    /// Get the default language for newly added works.
    async fn get_default_language(&self) -> Result<String, DbError>;

    /// Update the default language for newly added works.
    async fn update_default_language(&self, language: &str) -> Result<String, DbError>;

    /// Get email config.
    async fn get_email_config(&self) -> Result<EmailConfig, DbError>;

    /// Update email config.
    async fn update_email_config(
        &self,
        req: UpdateEmailConfigRequest,
    ) -> Result<EmailConfig, DbError>;

    /// Get indexer config singleton (RSS sync settings).
    ///
    /// Satisfies: RSS-CONFIG-001
    async fn get_indexer_config(&self) -> Result<IndexerConfig, DbError>;

    /// Update indexer config singleton.
    ///
    /// Satisfies: RSS-CONFIG-001
    async fn update_indexer_config(
        &self,
        req: UpdateIndexerConfigRequest,
    ) -> Result<IndexerConfig, DbError>;
}

// NamingConfig, MediaManagementConfig, ProwlarrConfig, MetadataConfig
// re-exported from livrarr_domain::settings above.

pub struct UpdateMediaManagementConfigRequest {
    pub cwa_ingest_path: Option<String>,
    pub preferred_ebook_formats: Vec<String>,
    pub preferred_audiobook_formats: Vec<String>,
}

pub struct UpdateProwlarrConfigRequest {
    pub url: Option<String>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub api_key: Option<Option<String>>,
    pub enabled: Option<bool>,
}

// EmailConfig re-exported from livrarr_domain::settings above.

pub struct UpdateEmailConfigRequest {
    pub enabled: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i32>,
    pub encryption: Option<String>,
    pub username: Option<String>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub password: Option<Option<String>>,
    pub from_address: Option<String>,
    pub recipient_email: Option<String>,
    pub send_on_import: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIndexerConfigRequest {
    pub rss_sync_interval_minutes: Option<i32>,
    pub rss_match_threshold: Option<f64>,
    pub rss_grab_failure_limit: Option<i32>,
}

pub struct UpdateMetadataConfigRequest {
    pub hardcover_enabled: Option<bool>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub hardcover_api_token: Option<Option<String>>,
    pub llm_enabled: Option<bool>,
    pub llm_provider: Option<LlmProvider>,
    pub llm_endpoint: Option<String>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub llm_api_key: Option<Option<String>>,
    pub llm_model: Option<String>,
    pub audnexus_url: Option<String>,
    pub languages: Option<Vec<String>>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub google_books_api_key: Option<Option<String>>,
}

impl From<UpdateMediaManagementParams> for UpdateMediaManagementConfigRequest {
    fn from(p: UpdateMediaManagementParams) -> Self {
        Self {
            cwa_ingest_path: p.cwa_ingest_path,
            preferred_ebook_formats: p.preferred_ebook_formats,
            preferred_audiobook_formats: p.preferred_audiobook_formats,
        }
    }
}

impl From<UpdateMetadataParams> for UpdateMetadataConfigRequest {
    fn from(p: UpdateMetadataParams) -> Self {
        Self {
            hardcover_enabled: p.hardcover_enabled,
            hardcover_api_token: p.hardcover_api_token,
            llm_enabled: p.llm_enabled,
            llm_provider: p.llm_provider,
            llm_endpoint: p.llm_endpoint,
            llm_api_key: p.llm_api_key,
            llm_model: p.llm_model,
            audnexus_url: p.audnexus_url,
            languages: p.languages,
            google_books_api_key: p.google_books_api_key,
        }
    }
}

impl From<UpdateProwlarrParams> for UpdateProwlarrConfigRequest {
    fn from(p: UpdateProwlarrParams) -> Self {
        Self {
            url: p.url,
            api_key: p.api_key,
            enabled: p.enabled,
        }
    }
}

impl From<UpdateEmailParams> for UpdateEmailConfigRequest {
    fn from(p: UpdateEmailParams) -> Self {
        Self {
            enabled: p.enabled,
            smtp_host: p.smtp_host,
            smtp_port: p.smtp_port,
            encryption: p.encryption,
            username: p.username,
            password: p.password,
            from_address: p.from_address,
            recipient_email: p.recipient_email,
            send_on_import: p.send_on_import,
        }
    }
}

impl From<UpdateIndexerConfigParams> for UpdateIndexerConfigRequest {
    fn from(p: UpdateIndexerConfigParams) -> Self {
        Self {
            rss_sync_interval_minutes: p.rss_sync_interval_minutes,
            rss_match_threshold: p.rss_match_threshold,
            rss_grab_failure_limit: p.rss_grab_failure_limit,
        }
    }
}
