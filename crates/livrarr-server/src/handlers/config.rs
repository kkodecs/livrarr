use axum::extract::State;
use axum::Json;

/// Strip whitespace and "Bearer " prefix from token inputs.
fn clean_token(token: &str) -> String {
    let trimmed = token.trim();
    trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

use crate::middleware::RequireAdmin;
use crate::state::AppState;
use crate::{
    ApiError, MediaManagementConfigResponse, MetadataConfigResponse, NamingConfigResponse,
    UpdateMediaManagementApiRequest, UpdateMetadataApiRequest,
};
use livrarr_db::{ConfigDb, UpdateMediaManagementConfigRequest, UpdateMetadataConfigRequest};

/// GET /api/v1/config/naming
pub async fn get_naming(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<NamingConfigResponse>, ApiError> {
    let cfg = state.db.get_naming_config().await?;
    Ok(Json(NamingConfigResponse {
        author_folder_format: cfg.author_folder_format,
        book_folder_format: cfg.book_folder_format,
        rename_files: cfg.rename_files,
        replace_illegal_chars: cfg.replace_illegal_chars,
    }))
}

/// GET /api/v1/config/mediamanagement
pub async fn get_media_management(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<MediaManagementConfigResponse>, ApiError> {
    let cfg = state.db.get_media_management_config().await?;
    Ok(Json(MediaManagementConfigResponse {
        cwa_ingest_path: cfg.cwa_ingest_path,
        preferred_ebook_formats: cfg.preferred_ebook_formats,
        preferred_audiobook_formats: cfg.preferred_audiobook_formats,
    }))
}

/// PUT /api/v1/config/mediamanagement
pub async fn update_media_management(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(req): Json<UpdateMediaManagementApiRequest>,
) -> Result<Json<MediaManagementConfigResponse>, ApiError> {
    let cfg = state
        .db
        .update_media_management_config(UpdateMediaManagementConfigRequest {
            cwa_ingest_path: req.cwa_ingest_path,
            preferred_ebook_formats: req.preferred_ebook_formats,
            preferred_audiobook_formats: req.preferred_audiobook_formats,
        })
        .await?;
    Ok(Json(MediaManagementConfigResponse {
        cwa_ingest_path: cfg.cwa_ingest_path,
        preferred_ebook_formats: cfg.preferred_ebook_formats,
        preferred_audiobook_formats: cfg.preferred_audiobook_formats,
    }))
}

/// GET /api/v1/config/metadata
pub async fn get_metadata(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<MetadataConfigResponse>, ApiError> {
    let cfg = state.db.get_metadata_config().await?;
    Ok(Json(metadata_to_response(cfg)))
}

fn metadata_to_response(cfg: livrarr_db::MetadataConfig) -> MetadataConfigResponse {
    MetadataConfigResponse {
        hardcover_enabled: cfg.hardcover_enabled,
        hardcover_api_token_set: cfg.hardcover_api_token.is_some(),
        llm_enabled: cfg.llm_enabled,
        llm_provider: cfg.llm_provider,
        llm_endpoint: cfg.llm_endpoint,
        llm_api_key_set: cfg.llm_api_key.is_some(),
        llm_model: cfg.llm_model,
        audnexus_url: cfg.audnexus_url,
        languages: cfg.languages,
    }
}

/// PUT /api/v1/config/metadata
pub async fn update_metadata(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(req): Json<UpdateMetadataApiRequest>,
) -> Result<Json<MetadataConfigResponse>, ApiError> {
    let cfg = state
        .db
        .update_metadata_config(UpdateMetadataConfigRequest {
            hardcover_enabled: req.hardcover_enabled,
            hardcover_api_token: req.hardcover_api_token.map(|t| clean_token(&t)),
            llm_enabled: req.llm_enabled,
            llm_provider: req.llm_provider,
            llm_endpoint: req.llm_endpoint,
            llm_api_key: req.llm_api_key.map(|t| clean_token(&t)),
            llm_model: req.llm_model,
            audnexus_url: req.audnexus_url,
            languages: req.languages,
        })
        .await?;
    Ok(Json(metadata_to_response(cfg)))
}

/// POST /api/v1/config/metadata/test/hardcover
pub async fn test_hardcover(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<(), ApiError> {
    let cfg = state.db.get_metadata_config().await?;
    let token = cfg
        .hardcover_api_token
        .ok_or_else(|| ApiError::BadRequest("Hardcover API token not configured".into()))?;

    let clean = clean_token(&token);
    let resp = state
        .http_client
        .post("https://api.hardcover.app/v1/graphql")
        .header("Authorization", format!("Bearer {clean}"))
        .header("Content-Type", "application/json")
        .body(r#"{"query":"{ me { id } }"}"#)
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("Hardcover connection failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "Hardcover returned {} — check API token",
            resp.status()
        )));
    }
    Ok(())
}

/// POST /api/v1/config/metadata/test/audnexus
pub async fn test_audnexus(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<(), ApiError> {
    let cfg = state.db.get_metadata_config().await?;
    let url = format!(
        "{}/authors/B000AQ0842",
        cfg.audnexus_url.trim_end_matches('/')
    );

    let resp = state
        .http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("Audnexus connection failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "Audnexus returned {}",
            resp.status()
        )));
    }
    Ok(())
}

/// POST /api/v1/config/metadata/test/llm
pub async fn test_llm(State(state): State<AppState>, _admin: RequireAdmin) -> Result<(), ApiError> {
    let cfg = state.db.get_metadata_config().await?;
    let endpoint = cfg
        .llm_endpoint
        .ok_or_else(|| ApiError::BadRequest("LLM endpoint not configured".into()))?;
    let api_key = cfg
        .llm_api_key
        .ok_or_else(|| ApiError::BadRequest("LLM API key not configured".into()))?;
    let model = cfg
        .llm_model
        .ok_or_else(|| ApiError::BadRequest("LLM model not configured".into()))?;

    let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Say hi"}],
        "max_tokens": 5
    });

    let resp = state
        .http_client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("LLM connection failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::BadGateway(format!(
            "LLM returned {status}: {text}"
        )));
    }
    Ok(())
}

/// GET /api/v1/config/prowlarr
pub async fn get_prowlarr(
    State(state): State<AppState>,
) -> Result<Json<crate::ProwlarrConfigResponse>, ApiError> {
    let c = state.db.get_prowlarr_config().await?;
    Ok(Json(crate::ProwlarrConfigResponse {
        url: c.url,
        api_key_set: c.api_key.is_some(),
        enabled: c.enabled,
    }))
}

/// PUT /api/v1/config/prowlarr
pub async fn update_prowlarr(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(req): Json<crate::UpdateProwlarrApiRequest>,
) -> Result<Json<crate::ProwlarrConfigResponse>, ApiError> {
    let c = state
        .db
        .update_prowlarr_config(livrarr_db::UpdateProwlarrConfigRequest {
            url: req.url,
            api_key: req.api_key,
            enabled: req.enabled,
        })
        .await?;
    Ok(Json(crate::ProwlarrConfigResponse {
        url: c.url,
        api_key_set: c.api_key.is_some(),
        enabled: c.enabled,
    }))
}

/// GET /api/v1/config/email
pub async fn get_email(
    State(state): State<AppState>,
) -> Result<Json<crate::EmailConfigResponse>, ApiError> {
    let c = state.db.get_email_config().await?;
    Ok(Json(crate::EmailConfigResponse {
        enabled: c.enabled,
        smtp_host: c.smtp_host,
        smtp_port: c.smtp_port,
        encryption: c.encryption,
        username: c.username,
        password_set: c.password.is_some(),
        from_address: c.from_address,
        recipient_email: c.recipient_email,
        send_on_import: c.send_on_import,
    }))
}

/// PUT /api/v1/config/email
pub async fn update_email(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(req): Json<crate::UpdateEmailApiRequest>,
) -> Result<Json<crate::EmailConfigResponse>, ApiError> {
    let c = state
        .db
        .update_email_config(livrarr_db::UpdateEmailConfigRequest {
            enabled: req.enabled,
            smtp_host: req.smtp_host,
            smtp_port: req.smtp_port,
            encryption: req.encryption,
            username: req.username,
            password: req.password,
            from_address: req.from_address,
            recipient_email: req.recipient_email,
            send_on_import: req.send_on_import,
        })
        .await?;
    Ok(Json(crate::EmailConfigResponse {
        enabled: c.enabled,
        smtp_host: c.smtp_host,
        smtp_port: c.smtp_port,
        encryption: c.encryption,
        username: c.username,
        password_set: c.password.is_some(),
        from_address: c.from_address,
        recipient_email: c.recipient_email,
        send_on_import: c.send_on_import,
    }))
}

/// POST /api/v1/config/email/test
pub async fn test_email(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<serde_json::Value>, ApiError> {
    let cfg = state.db.get_email_config().await?;
    super::email::send_test(&cfg)
        .await
        .map_err(ApiError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}
