use axum::extract::State;
use axum::Json;

use crate::accessors::{LiveMetadataConfigAccessor, ProviderHealthAccessor, RssSyncAccessor};
use crate::context::AppContext;
use crate::middleware::RequireAdmin;
use crate::{
    ApiError, AuthContext, EmailConfigResponse, MediaManagementConfigResponse,
    MetadataConfigResponse, NamingConfigResponse, UpdateEmailApiRequest,
    UpdateMediaManagementApiRequest, UpdateMetadataApiRequest,
};
use livrarr_domain::services::{RssSyncWorkflow, SettingsService};

struct RssSyncGuard<'a, R: RssSyncAccessor>(&'a R);
impl<R: RssSyncAccessor> Drop for RssSyncGuard<'_, R> {
    fn drop(&mut self) {
        self.0.release();
    }
}
use livrarr_domain::settings::{
    UpdateEmailParams, UpdateMediaManagementParams, UpdateMetadataParams, UpdateProwlarrParams,
};

fn clean_token(token: &str) -> String {
    let trimmed = token.trim();
    trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

fn metadata_to_response(
    cfg: livrarr_domain::settings::MetadataConfig,
    provider_status: std::collections::HashMap<String, String>,
) -> MetadataConfigResponse {
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
        provider_status,
    }
}

pub async fn get_naming<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<NamingConfigResponse>, ApiError> {
    let cfg = state.settings_service().get_naming_config().await?;
    Ok(Json(NamingConfigResponse {
        author_folder_format: cfg.author_folder_format,
        book_folder_format: cfg.book_folder_format,
        rename_files: cfg.rename_files,
        replace_illegal_chars: cfg.replace_illegal_chars,
    }))
}

pub async fn get_media_management<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<MediaManagementConfigResponse>, ApiError> {
    let cfg = state
        .settings_service()
        .get_media_management_config()
        .await?;
    Ok(Json(MediaManagementConfigResponse {
        cwa_ingest_path: cfg.cwa_ingest_path,
        preferred_ebook_formats: cfg.preferred_ebook_formats,
        preferred_audiobook_formats: cfg.preferred_audiobook_formats,
    }))
}

pub async fn update_media_management<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<UpdateMediaManagementApiRequest>,
) -> Result<Json<MediaManagementConfigResponse>, ApiError> {
    let cfg = state
        .settings_service()
        .update_media_management_config(UpdateMediaManagementParams {
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

pub async fn get_metadata<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<MetadataConfigResponse>, ApiError> {
    let cfg = state.settings_service().get_metadata_config().await?;
    let provider_status = state.provider_health().statuses().await;
    Ok(Json(metadata_to_response(cfg, provider_status)))
}

/// Validate an LLM endpoint URL: must be http/https, no embedded credentials,
/// no private IP addresses.
fn validate_llm_endpoint(endpoint: &str) -> Result<(), ApiError> {
    let parsed = reqwest::Url::parse(endpoint)
        .map_err(|e| ApiError::BadRequest(format!("invalid LLM endpoint URL: {e}")))?;

    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(ApiError::BadRequest(format!(
                "LLM endpoint must use http or https scheme, got: {other}"
            )));
        }
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ApiError::BadRequest(
            "LLM endpoint must not contain embedded credentials".into(),
        ));
    }

    if let Some(host) = parsed.host_str() {
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if livrarr_http::ssrf::is_private_ip(ip) {
                return Err(ApiError::BadRequest(
                    "LLM endpoint must not point to a private IP address".into(),
                ));
            }
        }
    } else {
        return Err(ApiError::BadRequest("LLM endpoint must have a host".into()));
    }

    Ok(())
}

pub async fn update_metadata<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<UpdateMetadataApiRequest>,
) -> Result<Json<MetadataConfigResponse>, ApiError> {
    if let Some(Some(ref t)) = req.hardcover_api_token {
        if t.is_empty() {
            return Err(ApiError::BadRequest(
                "hardcoverApiToken must not be empty string; use null to clear".into(),
            ));
        }
    }
    if let Some(Some(ref k)) = req.llm_api_key {
        if k.is_empty() {
            return Err(ApiError::BadRequest(
                "llmApiKey must not be empty string; use null to clear".into(),
            ));
        }
    }

    // Validate LLM endpoint URL if provided
    if let Some(ref endpoint) = req.llm_endpoint {
        if !endpoint.is_empty() {
            validate_llm_endpoint(endpoint)?;
        }
    }

    let hardcover_api_token = req
        .hardcover_api_token
        .map(|inner| inner.map(|t| clean_token(&t)));
    let llm_api_key = req.llm_api_key.map(|inner| inner.map(|t| clean_token(&t)));

    let validated_languages = if let Some(langs) = req.languages {
        let effective_key = match &llm_api_key {
            None => None,
            Some(None) => None,
            Some(Some(v)) => Some(v.as_str()),
        };
        Some(
            state
                .settings_service()
                .validate_metadata_languages(
                    &langs,
                    req.llm_enabled,
                    req.llm_endpoint.as_deref(),
                    effective_key,
                    req.llm_model.as_deref(),
                )
                .await
                .map_err(ApiError::BadRequest)?,
        )
    } else {
        None
    };

    let cfg = state
        .settings_service()
        .update_metadata_config(UpdateMetadataParams {
            hardcover_enabled: req.hardcover_enabled,
            hardcover_api_token,
            llm_enabled: req.llm_enabled,
            llm_provider: req.llm_provider,
            llm_endpoint: req.llm_endpoint,
            llm_api_key,
            llm_model: req.llm_model,
            audnexus_url: req.audnexus_url,
            languages: validated_languages,
        })
        .await?;

    state.live_metadata_config().replace(cfg.clone());

    let provider_status = state.provider_health().statuses().await;
    Ok(Json(metadata_to_response(cfg, provider_status)))
}

pub async fn test_hardcover<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<(), ApiError> {
    let cfg = state.settings_service().get_metadata_config().await?;
    let token = cfg
        .hardcover_api_token
        .ok_or_else(|| ApiError::BadRequest("Hardcover API token not configured".into()))?;

    let clean = clean_token(&token);
    let resp = state
        .http_client()
        .post("https://api.hardcover.app/v1/graphql")
        .header("Authorization", format!("Bearer {clean}"))
        .header("Content-Type", "application/json")
        .body(r#"{"query":"{ me { id } }"}"#)
        .send()
        .await
        .map_err(|e| {
            ApiError::BadGateway(format!("Hardcover connection failed: {}", e.without_url()))
        })?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "Hardcover returned {} — check API token",
            resp.status()
        )));
    }
    Ok(())
}

pub async fn test_audnexus<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<(), ApiError> {
    let cfg = state.settings_service().get_metadata_config().await?;
    let url = format!(
        "{}/authors/B000AQ0842",
        cfg.audnexus_url.trim_end_matches('/')
    );

    let resp = state.http_client().get(&url).send().await.map_err(|e| {
        ApiError::BadGateway(format!("Audnexus connection failed: {}", e.without_url()))
    })?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "Audnexus returned {}",
            resp.status()
        )));
    }
    Ok(())
}

pub async fn test_llm<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<(), ApiError> {
    let cfg = state.settings_service().get_metadata_config().await?;
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
        .http_client()
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("LLM connection failed: {}", e.without_url())))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::BadGateway(format!(
            "LLM returned {status}: {text}"
        )));
    }
    Ok(())
}

pub async fn get_prowlarr<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<crate::ProwlarrConfigResponse>, ApiError> {
    let c = state.settings_service().get_prowlarr_config().await?;
    Ok(Json(crate::ProwlarrConfigResponse {
        url: c.url,
        api_key_set: c.api_key.is_some(),
        enabled: c.enabled,
    }))
}

pub async fn update_prowlarr<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<crate::UpdateProwlarrApiRequest>,
) -> Result<Json<crate::ProwlarrConfigResponse>, ApiError> {
    if let Some(Some(ref k)) = req.api_key {
        if k.is_empty() {
            return Err(ApiError::BadRequest(
                "api_key must not be empty string; use null to clear".into(),
            ));
        }
    }
    let c = state
        .settings_service()
        .update_prowlarr_config(UpdateProwlarrParams {
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

pub async fn get_email<S: AppContext>(
    _admin: RequireAdmin,
    State(state): State<S>,
) -> Result<Json<EmailConfigResponse>, ApiError> {
    let c = state.settings_service().get_email_config().await?;
    Ok(Json(EmailConfigResponse {
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

pub async fn update_email<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<UpdateEmailApiRequest>,
) -> Result<Json<EmailConfigResponse>, ApiError> {
    if let Some(Some(ref p)) = req.password {
        if p.is_empty() {
            return Err(ApiError::BadRequest(
                "password must not be empty string; use null to clear".into(),
            ));
        }
    }
    let c = state
        .settings_service()
        .update_email_config(UpdateEmailParams {
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
    Ok(Json(EmailConfigResponse {
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

pub async fn get_indexer_config<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<livrarr_domain::IndexerConfig>, ApiError> {
    let c = state.settings_service().get_indexer_config().await?;
    Ok(Json(c))
}

pub async fn update_indexer_config<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<livrarr_domain::settings::UpdateIndexerConfigParams>,
) -> Result<Json<livrarr_domain::IndexerConfig>, ApiError> {
    if let Some(interval) = req.rss_sync_interval_minutes {
        if interval != 0 && !(10..=1440).contains(&interval) {
            return Err(ApiError::BadRequest(
                "rss_sync_interval_minutes must be 0 (disabled) or between 10 and 1440".into(),
            ));
        }
    }
    if let Some(threshold) = req.rss_match_threshold {
        if !(0.50..=0.95).contains(&threshold) {
            return Err(ApiError::BadRequest(
                "rss_match_threshold must be between 0.50 and 0.95".into(),
            ));
        }
    }
    let c = state.settings_service().update_indexer_config(req).await?;
    Ok(Json(c))
}

pub async fn trigger_rss_sync<S: AppContext>(
    State(state): State<S>,
    _auth: AuthContext,
) -> Result<axum::http::StatusCode, ApiError> {
    if !state.rss_sync().try_acquire() {
        return Err(ApiError::Conflict {
            reason: "RSS sync already running".into(),
        });
    }

    let s = state.clone();
    tokio::spawn(async move {
        let _guard = RssSyncGuard(s.rss_sync());

        match s.rss_sync_workflow().run_sync().await {
            Ok(report) => {
                s.rss_sync().set_last_run(chrono::Utc::now().timestamp());
                for w in &report.warnings {
                    tracing::warn!("RSS sync: {w}");
                }
            }
            Err(e) => {
                tracing::warn!("trigger rss_sync failed: {e}");
            }
        }
    });

    Ok(axum::http::StatusCode::OK)
}

pub async fn test_email<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<serde_json::Value>, ApiError> {
    use livrarr_domain::services::EmailService;
    state
        .email_service()
        .send_test()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}
