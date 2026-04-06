use axum::extract::{Path, State};
use axum::Json;

use crate::state::AppState;
use crate::{
    ApiError, CreateDownloadClientApiRequest, DownloadClientResponse,
    UpdateDownloadClientApiRequest,
};
use livrarr_db::{CreateDownloadClientDbRequest, DownloadClientDb, UpdateDownloadClientDbRequest};
use livrarr_domain::{DownloadClient, DownloadClientImplementation};

fn to_response(dc: DownloadClient) -> DownloadClientResponse {
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
        client_type: dc.client_type.clone(),
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
                    && c.client_type == existing.client_type
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
        auto_promote_default(&state, &dc.client_type, dc.id).await;
    }

    Ok(Json(to_response(dc)))
}

/// DELETE /api/v1/downloadclient/:id
pub async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> Result<(), ApiError> {
    let existing = state.db.get_download_client(id).await?;
    state.db.delete_download_client(id).await?;

    // Auto-promote: if deleting the default, promote another enabled client.
    if existing.is_default_for_protocol {
        auto_promote_default(&state, &existing.client_type, existing.id).await;
    }

    Ok(())
}

/// Promote the first enabled client of this type as default (if any exist).
async fn auto_promote_default(state: &AppState, client_type: &str, exclude_id: i64) {
    if let Ok(clients) = state.db.list_download_clients().await {
        if let Some(candidate) = clients
            .iter()
            .find(|c| c.client_type == client_type && c.enabled && c.id != exclude_id)
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
    Json(req): Json<CreateDownloadClientApiRequest>,
) -> Result<(), ApiError> {
    match req.implementation {
        DownloadClientImplementation::SABnzbd => test_sabnzbd(&state, &req).await,
        DownloadClientImplementation::QBittorrent => test_qbittorrent(&state, &req).await,
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
        .map_err(|e| ApiError::BadGateway(format!("SABnzbd connection failed: {e}")))?;

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
    let resp = state
        .http_client
        .get(&cat_url)
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("SABnzbd categories request failed: {e}")))?;

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
