use axum::extract::{Path, State};
use axum::Json;

use crate::middleware::RequireAdmin;
use crate::state::AppState;
use crate::{
    ApiError, CreateDownloadClientApiRequest, DownloadClientResponse,
    UpdateDownloadClientApiRequest,
};
use livrarr_db::{
    ConfigDb, CreateDownloadClientDbRequest, DownloadClientDb, UpdateDownloadClientDbRequest,
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
        // password intentionally omitted from response
        category: dc.category,
        enabled: dc.enabled,
        client_type,
        api_key_set: dc.api_key.is_some(),
        is_default_for_protocol: dc.is_default_for_protocol,
    }
}

/// GET /api/v1/downloadclient
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<DownloadClientResponse>>, ApiError> {
    let clients = state.db.list_download_clients().await?;
    Ok(Json(clients.into_iter().map(to_response).collect()))
}

/// GET /api/v1/downloadclient/:id
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<DownloadClientResponse>, ApiError> {
    let dc = state.db.get_download_client(id).await?;
    Ok(Json(to_response(dc)))
}

/// Strip scheme prefix from host if present; return (clean_host, use_ssl_override).
pub fn normalize_host(host: &str) -> (String, Option<bool>) {
    if let Some(h) = host.strip_prefix("https://") {
        (h.trim_end_matches('/').to_string(), Some(true))
    } else if let Some(h) = host.strip_prefix("http://") {
        (h.trim_end_matches('/').to_string(), Some(false))
    } else {
        (host.to_string(), None)
    }
}

/// POST /api/v1/downloadclient
pub async fn create(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(req): Json<CreateDownloadClientApiRequest>,
) -> Result<Json<DownloadClientResponse>, ApiError> {
    if req.name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    if req.host.is_empty() {
        return Err(ApiError::BadRequest("host is required".into()));
    }

    let (host, ssl_override) = normalize_host(&req.host);
    let use_ssl = ssl_override.unwrap_or(req.use_ssl);

    let dc = state
        .db
        .create_download_client(CreateDownloadClientDbRequest {
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

/// PUT /api/v1/downloadclient/:id
pub async fn update(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
    Json(req): Json<UpdateDownloadClientApiRequest>,
) -> Result<Json<DownloadClientResponse>, ApiError> {
    // USE-DLC-005: preserve existing api_key if incoming is empty/omitted.
    let api_key = match req.api_key {
        Some(ref k) if !k.is_empty() => Some(k.clone()),
        _ => None,
    };

    // Prevent clearing the last default for a protocol.
    if req.is_default_for_protocol == Some(false) {
        let existing = state.db.get_download_client(id).await?;
        if existing.is_default_for_protocol {
            let clients = state.db.list_download_clients().await?;
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
        .db
        .update_download_client(
            id,
            UpdateDownloadClientDbRequest {
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

    // Auto-promote: if disabling the default, promote another enabled client.
    if req.enabled == Some(false) && dc.is_default_for_protocol {
        auto_promote_default(&state, dc.client_type(), dc.id).await;
    }

    Ok(Json(to_response(dc)))
}

/// DELETE /api/v1/downloadclient/:id
pub async fn delete(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    let existing = state.db.get_download_client(id).await?;
    state.db.delete_download_client(id).await?;

    // Auto-promote: if deleting the default, promote another enabled client.
    if existing.is_default_for_protocol {
        auto_promote_default(&state, existing.client_type(), existing.id).await;
    }

    Ok(())
}

/// Promote the first enabled client of this type as default (if any exist).
async fn auto_promote_default(state: &AppState, client_type: &str, exclude_id: i64) {
    if let Ok(clients) = state.db.list_download_clients().await {
        if let Some(candidate) = clients
            .iter()
            .find(|c| c.client_type() == client_type && c.enabled && c.id != exclude_id)
        {
            if let Err(e) = state
                .db
                .update_download_client(
                    candidate.id,
                    UpdateDownloadClientDbRequest {
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

/// Build base URL for a download client (works for both qBit and SABnzbd).
pub fn client_base_url(client: &DownloadClient) -> String {
    let scheme = if client.use_ssl { "https" } else { "http" };
    // Normalize url_base: ensure leading slash if non-empty, strip trailing slash.
    let raw = client.url_base.as_deref().unwrap_or("");
    let url_base = if raw.is_empty() {
        String::new()
    } else {
        let trimmed = raw.trim_end_matches('/');
        if trimmed.starts_with('/') {
            trimmed.to_string()
        } else {
            format!("/{trimmed}")
        }
    };
    if client.host.starts_with("http://") || client.host.starts_with("https://") {
        format!("{}{url_base}", client.host.trim_end_matches('/'))
    } else if client.port == 80 || client.port == 443 {
        format!("{scheme}://{}{url_base}", client.host)
    } else {
        format!("{scheme}://{}:{}{url_base}", client.host, client.port)
    }
}

/// Build base URL from request fields (for test endpoint, before client is persisted).
fn request_base_url(req: &CreateDownloadClientApiRequest) -> String {
    let scheme = if req.use_ssl { "https" } else { "http" };
    let raw = req.url_base.as_deref().unwrap_or("");
    let url_base = if raw.is_empty() {
        String::new()
    } else {
        let trimmed = raw.trim_end_matches('/');
        if trimmed.starts_with('/') {
            trimmed.to_string()
        } else {
            format!("/{trimmed}")
        }
    };
    if req.host.starts_with("http://") || req.host.starts_with("https://") {
        format!("{}{url_base}", req.host.trim_end_matches('/'))
    } else if req.port == 80 || req.port == 443 {
        format!("{scheme}://{}{url_base}", req.host)
    } else {
        format!("{scheme}://{}:{}{url_base}", req.host, req.port)
    }
}

/// POST /api/v1/downloadclient/test — test connection for qBit or SABnzbd
pub async fn test(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(req): Json<CreateDownloadClientApiRequest>,
) -> Result<(), ApiError> {
    run_connection_test(&state, &req).await
}

/// POST /api/v1/downloadclient/:id/test — test saved download client using DB creds
pub async fn test_saved(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<(), ApiError> {
    let dc = state.db.get_download_client(id).await?;
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

/// Shared implementation for both test endpoints.
async fn run_connection_test(
    state: &AppState,
    req: &CreateDownloadClientApiRequest,
) -> Result<(), ApiError> {
    match req.implementation {
        DownloadClientImplementation::SABnzbd => test_sabnzbd(state, req).await,
        DownloadClientImplementation::QBittorrent => test_qbittorrent(state, req).await,
    }
}

/// USE-DLC-003: Test SABnzbd — verify API key + category exists.
async fn test_sabnzbd(
    state: &AppState,
    req: &CreateDownloadClientApiRequest,
) -> Result<(), ApiError> {
    let base_url = request_base_url(req);

    let api_key = req.api_key.as_deref().unwrap_or("");

    // Test 1: Check version (validates API reachability + key).
    let version_url = format!("{base_url}/api?mode=version&apikey={api_key}&output=json");
    let resp = state
        .http_client
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

    // SABnzbd returns {"version": "4.x.x"} on success, or error on bad key.
    if body.get("error").is_some() {
        return Err(ApiError::BadGateway(format!(
            "SABnzbd API key invalid: {}",
            body["error"].as_str().unwrap_or("unknown error")
        )));
    }

    // Test 2: Check category exists.
    let cat_url =
        format!("{base_url}/api?mode=get_config&section=categories&apikey={api_key}&output=json");
    let resp = state.http_client.get(&cat_url).send().await.map_err(|e| {
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

/// Test qBittorrent connection (existing logic).
async fn test_qbittorrent(
    state: &AppState,
    req: &CreateDownloadClientApiRequest,
) -> Result<(), ApiError> {
    let base_url = request_base_url(req);

    // Authenticate.
    let username = req.username.as_deref().unwrap_or("");
    let password = req.password.as_deref().unwrap_or("");
    let mut sid: Option<String> = None;
    if !username.is_empty() || !password.is_empty() {
        let login_url = format!("{base_url}/api/v2/auth/login");
        let resp = state
            .http_client
            .post(&login_url)
            .form(&[("username", username), ("password", password)])
            .send()
            .await
            .map_err(|e| ApiError::BadGateway(format!("Connection failed: {e}")))?;
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

    // Check API version.
    let version_url = format!("{base_url}/api/v2/app/webapiVersion");
    let mut version_req = state.http_client.get(&version_url);
    if let Some(ref s) = sid {
        version_req = version_req.header("Cookie", format!("SID={s}"));
    }
    let resp = version_req
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("Failed to reach qBittorrent API: {e}")))?;
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

/// Prowlarr API download client response shape.
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

/// POST /api/v1/downloadclient/import/prowlarr
pub async fn import_from_prowlarr(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(req): Json<crate::ProwlarrImportRequest>,
) -> Result<Json<crate::ProwlarrImportResponse>, ApiError> {
    use std::time::Duration;

    // Fall back to saved Prowlarr config for missing fields.
    let saved = state.db.get_prowlarr_config().await.unwrap_or_default();
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
        .http_client
        .inner()
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

    // Read raw body for debugging, then parse.
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
    // NOTE: raw body intentionally not logged — may contain credential fields.

    let prowlarr_clients: Vec<ProwlarrDownloadClient> = serde_json::from_str(&body_text)
        .map_err(|e| ApiError::BadGateway(format!("Failed to parse Prowlarr response: {e}")))?;

    tracing::info!(
        parsed_count = prowlarr_clients.len(),
        "Parsed Prowlarr download clients"
    );

    // Load existing clients for duplicate detection (by host + port + implementation).
    let existing = state.db.list_download_clients().await?;
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
        // Map implementation.
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
        // Prowlarr masks secrets as "********" — filter those out so we store None
        // instead of the mask string. Users must enter credentials manually after import.
        let is_masked = |s: &str| s.chars().all(|c| c == '*');
        let password = pc
            .field_str("password")
            .filter(|s| !s.is_empty() && !is_masked(s));
        let api_key = pc
            .field_str("apiKey")
            .filter(|s| !s.is_empty() && !is_masked(s));
        let category = pc
            .field_str("category")
            .unwrap_or_else(|| "livrarr".to_string());

        // Disable if credentials are missing (Prowlarr masks them as "********").
        let has_creds = match impl_enum {
            DownloadClientImplementation::QBittorrent => password.is_some(),
            DownloadClientImplementation::SABnzbd => api_key.is_some(),
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
            .db
            .create_download_client(CreateDownloadClientDbRequest {
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
                api_key,
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

    // Persist Prowlarr creds on successful import so the form can be pre-filled.
    if imported > 0 || skipped > 0 {
        let _ = state
            .db
            .update_prowlarr_config(livrarr_db::UpdateProwlarrConfigRequest {
                url: Some(url),
                api_key: Some(api_key),
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
