use axum::extract::{Path, State};
use axum::Json;

use crate::context::{
    HasDownloadClientCredentialService, HasDownloadClientSettingsService, HasHttpClient,
    HasIndexerSettingsService,
};
use crate::middleware::RequireAdmin;
use crate::{
    ApiError, CreateDownloadClientApiRequest, DownloadClientResponse,
    UpdateDownloadClientApiRequest,
};
use livrarr_domain::services::{
    DownloadClientCredentialService, DownloadClientSettingsService, IndexerSettingsService,
};
use livrarr_domain::settings::{
    CreateDownloadClientParams, UpdateDownloadClientParams, UpdateProwlarrParams,
};
use livrarr_domain::{DownloadClient, DownloadClientImplementation};

fn to_response(dc: DownloadClient) -> DownloadClientResponse {
    let client_type = dc.client_type().to_string();
    DownloadClientResponse {
        id: dc.id,
        name: dc.name,
        implementation: dc.implementation,
        host: dc.host,
        port: dc.port,
        use_ssl: dc.use_ssl,
        skip_ssl_validation: dc.skip_ssl_validation,
        url_base: dc.url_base,
        username: dc.username,
        category: dc.category,
        enabled: dc.enabled,
        client_type,
        api_key_set: dc.api_key.is_some(),
        is_default_for_protocol: dc.is_default_for_protocol,
    }
}

pub fn normalize_host(host: &str) -> (String, Option<bool>) {
    if let Some(h) = host.strip_prefix("https://") {
        (h.trim_end_matches('/').to_string(), Some(true))
    } else if let Some(h) = host.strip_prefix("http://") {
        (h.trim_end_matches('/').to_string(), Some(false))
    } else {
        (host.to_string(), None)
    }
}

pub fn build_base_url(host: &str, port: u16, use_ssl: bool, url_base: Option<&str>) -> String {
    let scheme = if use_ssl { "https" } else { "http" };
    let raw = url_base.unwrap_or("");
    let url_base_part = if raw.is_empty() {
        String::new()
    } else {
        let trimmed = raw.trim_end_matches('/');
        if trimmed.starts_with('/') {
            trimmed.to_string()
        } else {
            format!("/{trimmed}")
        }
    };
    if host.starts_with("http://") || host.starts_with("https://") {
        format!("{}{url_base_part}", host.trim_end_matches('/'))
    } else if port == 80 || port == 443 {
        format!("{scheme}://{host}{url_base_part}")
    } else {
        format!("{scheme}://{host}:{port}{url_base_part}")
    }
}

pub fn client_base_url(client: &DownloadClient) -> String {
    build_base_url(
        &client.host,
        client.port,
        client.use_ssl,
        client.url_base.as_deref(),
    )
}

fn request_base_url(req: &CreateDownloadClientApiRequest) -> String {
    build_base_url(&req.host, req.port, req.use_ssl, req.url_base.as_deref())
}

pub async fn list<S: HasDownloadClientSettingsService>(
    State(state): State<S>,
) -> Result<Json<Vec<DownloadClientResponse>>, ApiError> {
    let clients = state
        .download_client_settings_service()
        .list_download_clients()
        .await?;
    Ok(Json(clients.into_iter().map(to_response).collect()))
}

pub async fn get<S: HasDownloadClientSettingsService>(
    State(state): State<S>,
    Path(id): Path<i64>,
) -> Result<Json<DownloadClientResponse>, ApiError> {
    let dc = state
        .download_client_settings_service()
        .get_download_client(id)
        .await?;
    Ok(Json(to_response(dc)))
}

pub async fn create<S: HasDownloadClientSettingsService>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<CreateDownloadClientApiRequest>,
) -> Result<Json<DownloadClientResponse>, ApiError> {
    if req.name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    if req.host.is_empty() {
        return Err(ApiError::BadRequest("host is required".into()));
    }
    if req.category.contains('/') || req.category.contains('\\') {
        return Err(ApiError::BadRequest(
            "category must not contain path separators".into(),
        ));
    }

    let (host, ssl_override) = normalize_host(&req.host);
    let use_ssl = ssl_override.unwrap_or(req.use_ssl);

    let dc = state
        .download_client_settings_service()
        .create_download_client(CreateDownloadClientParams {
            name: req.name,
            implementation: req.implementation,
            host,
            port: req.port,
            use_ssl,
            skip_ssl_validation: req.skip_ssl_validation,
            url_base: req.url_base,
            username: req.username,
            password: req.password,
            category: req.category,
            enabled: req.enabled,
            api_key: req.api_key,
        })
        .await?;

    Ok(Json(to_response(dc)))
}

pub async fn update<S: HasDownloadClientSettingsService>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
    Json(req): Json<UpdateDownloadClientApiRequest>,
) -> Result<Json<DownloadClientResponse>, ApiError> {
    if let Some(ref cat) = req.category {
        if cat.contains('/') || cat.contains('\\') {
            return Err(ApiError::BadRequest(
                "category must not contain path separators".into(),
            ));
        }
    }
    if let Some(Some(ref k)) = req.api_key {
        if k.is_empty() {
            return Err(ApiError::BadRequest(
                "api_key must not be empty string; use null to clear".into(),
            ));
        }
    }
    if let Some(Some(ref p)) = req.password {
        if p.is_empty() {
            return Err(ApiError::BadRequest(
                "password must not be empty string; use null to clear".into(),
            ));
        }
    }
    let api_key = req.api_key;

    if req.is_default_for_protocol == Some(false) {
        let existing = state
            .download_client_settings_service()
            .get_download_client(id)
            .await?;
        if existing.is_default_for_protocol {
            let clients = state
                .download_client_settings_service()
                .list_download_clients()
                .await?;
            let other_defaults = clients.iter().any(|c| {
                c.id != id
                    && c.client_type() == existing.client_type()
                    && c.enabled
                    && c.is_default_for_protocol
            });
            if !other_defaults {
                return Err(ApiError::BadRequest(
                    "Cannot clear the only default client for this protocol".into(),
                ));
            }
        }
    }

    let (host, ssl_override) = match &req.host {
        Some(h) => {
            let (clean, ssl) = normalize_host(h);
            (Some(clean), ssl)
        }
        None => (None, None),
    };
    let use_ssl = ssl_override.or(req.use_ssl);

    let dc = state
        .download_client_settings_service()
        .update_download_client(
            id,
            UpdateDownloadClientParams {
                name: req.name,
                host,
                port: req.port,
                use_ssl,
                skip_ssl_validation: req.skip_ssl_validation,
                url_base: req.url_base,
                username: req.username,
                password: req.password,
                category: req.category,
                enabled: req.enabled,
                api_key,
                is_default_for_protocol: req.is_default_for_protocol,
            },
        )
        .await?;

    if req.enabled == Some(false) && dc.is_default_for_protocol {
        auto_promote_default(&state, dc.client_type(), dc.id).await;
    }

    Ok(Json(to_response(dc)))
}

pub async fn delete<S: HasDownloadClientSettingsService>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    let existing = state
        .download_client_settings_service()
        .get_download_client(id)
        .await?;
    state
        .download_client_settings_service()
        .delete_download_client(id)
        .await?;

    if existing.is_default_for_protocol {
        auto_promote_default(&state, existing.client_type(), existing.id).await;
    }

    Ok(())
}

async fn auto_promote_default<S: HasDownloadClientSettingsService>(
    state: &S,
    client_type: &str,
    exclude_id: i64,
) {
    if let Ok(clients) = state
        .download_client_settings_service()
        .list_download_clients()
        .await
    {
        if let Some(candidate) = clients
            .iter()
            .find(|c| c.client_type() == client_type && c.enabled && c.id != exclude_id)
        {
            if let Err(e) = state
                .download_client_settings_service()
                .update_download_client(
                    candidate.id,
                    UpdateDownloadClientParams {
                        is_default_for_protocol: Some(true),
                        ..Default::default()
                    },
                )
                .await
            {
                tracing::warn!("update_download_client failed: {e}");
            }
        }
    }
}

pub async fn test<S: HasDownloadClientSettingsService + HasHttpClient>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<CreateDownloadClientApiRequest>,
) -> Result<(), ApiError> {
    run_connection_test(&state, &req).await
}

pub async fn test_saved<S: HasDownloadClientCredentialService + HasHttpClient>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    let dc = state
        .download_client_credential_service()
        .get_download_client_with_credentials(id)
        .await?;
    let req = CreateDownloadClientApiRequest {
        name: dc.name,
        implementation: dc.implementation,
        host: dc.host,
        port: dc.port,
        use_ssl: dc.use_ssl,
        skip_ssl_validation: dc.skip_ssl_validation,
        url_base: dc.url_base,
        username: dc.username,
        password: dc.password,
        category: dc.category,
        enabled: dc.enabled,
        api_key: dc.api_key,
    };
    run_connection_test(&state, &req).await
}

async fn run_connection_test<S: HasHttpClient>(
    state: &S,
    req: &CreateDownloadClientApiRequest,
) -> Result<(), ApiError> {
    match req.implementation {
        DownloadClientImplementation::SABnzbd => test_sabnzbd(state, req).await,
        DownloadClientImplementation::QBittorrent => test_qbittorrent(state, req).await,
    }
}

async fn test_sabnzbd<S: HasHttpClient>(
    state: &S,
    req: &CreateDownloadClientApiRequest,
) -> Result<(), ApiError> {
    let base_url = request_base_url(req);
    let api_key = req.api_key.as_deref().unwrap_or("");

    let version_url = format!("{base_url}/api?mode=version&apikey={api_key}&output=json");
    let resp = state
        .http_client_safe()
        .get(&version_url)
        .send()
        .await
        .map_err(|e| {
            ApiError::BadGateway(format!("SABnzbd connection failed: {}", e.without_url()))
        })?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "SABnzbd API returned {}",
            resp.status()
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ApiError::BadGateway(format!("SABnzbd parse error: {e}")))?;

    if body.get("error").is_some() {
        return Err(ApiError::BadGateway(format!(
            "SABnzbd API key invalid: {}",
            body["error"].as_str().unwrap_or("unknown error")
        )));
    }

    let cat_url =
        format!("{base_url}/api?mode=get_config&section=categories&apikey={api_key}&output=json");
    let resp = state
        .http_client_safe()
        .get(&cat_url)
        .send()
        .await
        .map_err(|e| {
            ApiError::BadGateway(format!(
                "SABnzbd categories request failed: {}",
                e.without_url()
            ))
        })?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "SABnzbd categories request returned {}",
            resp.status()
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ApiError::BadGateway(format!("SABnzbd categories parse error: {e}")))?;

    let cats = body
        .get("config")
        .and_then(|c| c.get("categories"))
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            ApiError::BadGateway(
                "SABnzbd returned unexpected categories response format".to_string(),
            )
        })?;

    let category = &req.category;
    if !category.is_empty() {
        let found = cats.iter().any(|c| {
            c.get("name")
                .and_then(|n| n.as_str())
                .is_some_and(|n| n == category)
        });
        if !found {
            return Err(ApiError::BadGateway(format!(
                "SABnzbd category '{}' does not exist",
                category
            )));
        }
    }

    Ok(())
}

async fn test_qbittorrent<S: HasHttpClient>(
    state: &S,
    req: &CreateDownloadClientApiRequest,
) -> Result<(), ApiError> {
    let base_url = request_base_url(req);

    let username = req.username.as_deref().unwrap_or("");
    let password = req.password.as_deref().unwrap_or("");
    let mut sid: Option<String> = None;
    if !username.is_empty() || !password.is_empty() {
        let login_url = format!("{base_url}/api/v2/auth/login");
        let resp = state
            .http_client_safe()
            .post(&login_url)
            .form(&[("username", username), ("password", password)])
            .send()
            .await
            .map_err(|e| ApiError::BadGateway(format!("Connection failed: {}", e.without_url())))?;
        if let Some(cookie) = resp
            .headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
        {
            if let Some(s) = cookie
                .split(';')
                .next()
                .and_then(|c| c.strip_prefix("SID="))
            {
                sid = Some(s.to_string());
            }
        }
        let body = resp.text().await.unwrap_or_default();
        if body.contains("Fails") {
            return Err(ApiError::BadGateway(
                "Authentication failed — check username/password".into(),
            ));
        }
    }

    let version_url = format!("{base_url}/api/v2/app/webapiVersion");
    let mut version_req = state.http_client_safe().get(&version_url);
    if let Some(ref s) = sid {
        version_req = version_req.header("Cookie", format!("SID={s}"));
    }
    let resp = version_req.send().await.map_err(|e| {
        ApiError::BadGateway(format!(
            "Failed to reach qBittorrent API: {}",
            e.without_url()
        ))
    })?;
    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "qBittorrent API returned {}",
            resp.status()
        )));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Prowlarr Import
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProwlarrDownloadClient {
    name: String,
    #[serde(default)]
    implementation: String,
    #[serde(default)]
    enable: bool,
    #[serde(default)]
    fields: Vec<ProwlarrField>,
}

#[derive(serde::Deserialize)]
struct ProwlarrField {
    name: String,
    #[serde(default)]
    value: serde_json::Value,
}

impl ProwlarrDownloadClient {
    fn field_str(&self, name: &str) -> Option<String> {
        self.fields
            .iter()
            .find(|f| f.name == name)
            .and_then(|f| match &f.value {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
    }

    fn field_bool(&self, name: &str) -> Option<bool> {
        self.fields
            .iter()
            .find(|f| f.name == name)
            .and_then(|f| f.value.as_bool())
    }

    fn field_u16(&self, name: &str) -> Option<u16> {
        self.fields
            .iter()
            .find(|f| f.name == name)
            .and_then(|f| f.value.as_u64())
            .and_then(|n| u16::try_from(n).ok())
    }
}

pub async fn import_from_prowlarr<
    S: HasDownloadClientSettingsService + HasIndexerSettingsService + HasHttpClient,
>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<crate::ProwlarrImportRequest>,
) -> Result<Json<crate::ProwlarrImportResponse>, ApiError> {
    use std::time::Duration;

    let saved = state
        .indexer_settings_service()
        .get_prowlarr_config()
        .await
        .unwrap_or_default();
    let url = if req.url.is_empty() {
        saved
            .url
            .ok_or_else(|| ApiError::BadRequest("url is required".into()))?
    } else {
        req.url.clone()
    };
    let api_key = if req.api_key.is_empty() {
        saved
            .api_key
            .ok_or_else(|| ApiError::BadRequest("apiKey is required".into()))?
    } else {
        req.api_key.clone()
    };

    let base = url.trim_end_matches('/');
    let fetch_url = format!("{base}/api/v1/downloadclient");

    let resp = state
        .http_client_safe()
        .get(&fetch_url)
        .header("X-Api-Key", &api_key)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| {
            ApiError::BadGateway(format!(
                "Failed to connect to Prowlarr: {}",
                e.without_url()
            ))
        })?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "Prowlarr returned HTTP {}",
            resp.status()
        )));
    }

    let body_text = resp.text().await.map_err(|e| {
        ApiError::BadGateway(format!(
            "Failed to read Prowlarr response: {}",
            e.without_url()
        ))
    })?;

    tracing::info!(
        count = body_text.matches("\"name\"").count(),
        "Prowlarr download client response received, raw length={}",
        body_text.len()
    );

    let prowlarr_clients: Vec<ProwlarrDownloadClient> = serde_json::from_str(&body_text)
        .map_err(|e| ApiError::BadGateway(format!("Failed to parse Prowlarr response: {e}")))?;

    tracing::info!(
        parsed_count = prowlarr_clients.len(),
        "Parsed Prowlarr download clients"
    );

    let existing = state
        .download_client_settings_service()
        .list_download_clients()
        .await?;
    let existing_keys: std::collections::HashSet<(String, u16, String)> = existing
        .iter()
        .map(|c| (c.host.to_lowercase(), c.port, c.client_type().to_string()))
        .collect();

    tracing::info!(
        existing_count = existing.len(),
        "Existing download clients loaded for dedup"
    );

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors = Vec::new();

    for pc in &prowlarr_clients {
        tracing::info!(
            name = %pc.name,
            implementation = %pc.implementation,
            enable = pc.enable,
            field_count = pc.fields.len(),
            host = ?pc.field_str("host"),
            port = ?pc.field_u16("port"),
            "Processing Prowlarr download client"
        );
    }

    for pc in prowlarr_clients {
        let impl_enum = match pc.implementation.as_str() {
            "QBittorrent" => DownloadClientImplementation::QBittorrent,
            "Sabnzbd" => DownloadClientImplementation::SABnzbd,
            other => {
                errors.push(format!("{}: unsupported implementation '{other}'", pc.name));
                continue;
            }
        };

        let host = match pc.field_str("host") {
            Some(h) => h,
            None => {
                errors.push(format!("{}: missing host field", pc.name));
                continue;
            }
        };

        let port = pc.field_u16("port").unwrap_or(match impl_enum {
            DownloadClientImplementation::QBittorrent => 8080,
            DownloadClientImplementation::SABnzbd => 8080,
        });

        let (clean_host, ssl_override) = normalize_host(&host);
        let use_ssl = ssl_override.unwrap_or_else(|| pc.field_bool("useSsl").unwrap_or(false));

        if existing_keys.contains(&(
            clean_host.to_lowercase(),
            port,
            impl_enum.client_type().to_string(),
        )) {
            skipped += 1;
            continue;
        }

        let url_base = pc.field_str("urlBase").filter(|s| !s.is_empty());
        let username = pc.field_str("username").filter(|s| !s.is_empty());
        let is_masked = |s: &str| s.chars().all(|c| c == '*');
        let password = pc
            .field_str("password")
            .filter(|s| !s.is_empty() && !is_masked(s));
        let api_key_field = pc
            .field_str("apiKey")
            .filter(|s| !s.is_empty() && !is_masked(s));
        let category = "livrarr".to_string();

        let has_creds = match impl_enum {
            DownloadClientImplementation::QBittorrent => password.is_some(),
            DownloadClientImplementation::SABnzbd => api_key_field.is_some(),
        };

        tracing::info!(
            name = %pc.name,
            host = %clean_host,
            port,
            use_ssl,
            has_creds,
            impl_type = ?impl_enum,
            "Creating download client from Prowlarr"
        );

        match state
            .download_client_settings_service()
            .create_download_client(CreateDownloadClientParams {
                name: pc.name.clone(),
                implementation: impl_enum,
                host: clean_host,
                port,
                use_ssl,
                skip_ssl_validation: false,
                url_base,
                username,
                password,
                category,
                enabled: has_creds,
                api_key: api_key_field,
            })
            .await
        {
            Ok(dc) => {
                tracing::info!(id = dc.id, name = %dc.name, "Download client imported successfully");
                imported += 1;
            }
            Err(e) => {
                tracing::warn!(name = %pc.name, error = %e, "Failed to import download client");
                errors.push(format!("{}: {e}", pc.name));
            }
        }
    }

    tracing::info!(
        imported,
        skipped,
        error_count = errors.len(),
        "Prowlarr download client import complete"
    );

    if imported > 0 || skipped > 0 {
        let _ = state
            .indexer_settings_service()
            .update_prowlarr_config(UpdateProwlarrParams {
                url: Some(url),
                api_key: Some(Some(api_key)),
                enabled: Some(true),
            })
            .await;
    }

    Ok(Json(crate::ProwlarrImportResponse {
        imported,
        skipped,
        errors,
    }))
}
