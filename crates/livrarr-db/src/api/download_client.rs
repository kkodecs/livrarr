//! Download client config data access: `DownloadClientDb` trait + request types.

use livrarr_domain::settings::{CreateDownloadClientParams, UpdateDownloadClientParams};

use crate::{DbError, DownloadClient, DownloadClientId, DownloadClientImplementation};

/// Download client config data access.
/// Shared infrastructure: admin-managed, visible to all users.
///
/// Satisfies: DLC-001, DLC-003, DLC-005, AUTH-004, USE-DLC-001, USE-DLC-004
#[trait_variant::make(Send)]
pub trait DownloadClientDb: Send + Sync {
    async fn get_download_client(&self, id: DownloadClientId) -> Result<DownloadClient, DbError>;

    /// Get download client with credentials (password and api_key populated).
    /// Use for outbound connections (test, grab, import poll). Default get_download_client
    /// is equivalent but callers making outbound calls should use this variant to signal intent.
    async fn get_download_client_with_credentials(
        &self,
        id: DownloadClientId,
    ) -> Result<DownloadClient, DbError>;
    async fn list_download_clients(&self) -> Result<Vec<DownloadClient>, DbError>;
    async fn create_download_client(
        &self,
        req: CreateDownloadClientDbRequest,
    ) -> Result<DownloadClient, DbError>;
    async fn update_download_client(
        &self,
        id: DownloadClientId,
        req: UpdateDownloadClientDbRequest,
    ) -> Result<DownloadClient, DbError>;
    async fn delete_download_client(&self, id: DownloadClientId) -> Result<(), DbError>;

    /// Get the default download client for a given protocol (client_type).
    ///
    /// Satisfies: DLC-005, USE-DLC-004
    async fn get_default_download_client(
        &self,
        client_type: &str,
    ) -> Result<Option<DownloadClient>, DbError>;
}

pub struct CreateDownloadClientDbRequest {
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
    pub download_dir: Option<String>,
    pub enabled: bool,
    pub api_key: Option<String>,
}

#[derive(Default)]
pub struct UpdateDownloadClientDbRequest {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub use_ssl: Option<bool>,
    pub skip_ssl_validation: Option<bool>,
    pub url_base: Option<String>,
    pub username: Option<String>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub password: Option<Option<String>>,
    pub category: Option<String>,
    pub download_dir: Option<Option<String>>,
    pub enabled: Option<bool>,
    /// Tri-state: None = keep existing, Some(None) = clear, Some(Some(v)) = set.
    pub api_key: Option<Option<String>>,
    pub is_default_for_protocol: Option<bool>,
}

impl From<CreateDownloadClientParams> for CreateDownloadClientDbRequest {
    fn from(p: CreateDownloadClientParams) -> Self {
        Self {
            name: p.name,
            implementation: p.implementation,
            host: p.host,
            port: p.port,
            use_ssl: p.use_ssl,
            skip_ssl_validation: p.skip_ssl_validation,
            url_base: p.url_base,
            username: p.username,
            password: p.password,
            category: p.category,
            download_dir: p.download_dir,
            enabled: p.enabled,
            api_key: p.api_key,
        }
    }
}

impl From<UpdateDownloadClientParams> for UpdateDownloadClientDbRequest {
    fn from(p: UpdateDownloadClientParams) -> Self {
        Self {
            name: p.name,
            host: p.host,
            port: p.port,
            use_ssl: p.use_ssl,
            skip_ssl_validation: p.skip_ssl_validation,
            url_base: p.url_base,
            username: p.username,
            password: p.password,
            category: p.category,
            download_dir: p.download_dir,
            enabled: p.enabled,
            api_key: p.api_key,
            is_default_for_protocol: p.is_default_for_protocol,
        }
    }
}
