use axum::extract::State;
use axum::Json;

use crate::accessors::{LiveMetadataConfigAccessor, RssSyncAccessor};
use crate::context::{
    HasAppConfigService, HasEmailService, HasHttpClient, HasHttpFetcher, HasIndexerSettingsService,
    HasLiveConfig, HasProviderStats, HasRssSync, HasRssSyncWorkflow,
};

use crate::middleware::RequireAdmin;
use crate::{
    ApiError, AuthContext, EmailConfigResponse, MediaManagementConfigResponse,
    MetadataConfigResponse, NamingConfigResponse, UpdateEmailApiRequest,
    UpdateMediaManagementApiRequest, UpdateMetadataApiRequest,
};
use livrarr_domain::services::{
    AppConfigService, IndexerSettingsService, ProviderStatsService, RssSyncWorkflow,
};

struct RssSyncGuard<'a, R: RssSyncAccessor>(&'a R);
impl<R: RssSyncAccessor> Drop for RssSyncGuard<'_, R> {
    fn drop(&mut self) {
        self.0.release();
    }
}
use livrarr_domain::settings::{
    UpdateEmailParams, UpdateMediaManagementParams, UpdateMetadataParams, UpdateProwlarrParams,
};

/// Current per-provider error map for the metadata config page, derived from
/// the 24h call records (REQ-002): record-key → most recent error message,
/// present only when the error is newer than the last success.
async fn provider_error_map<S: HasProviderStats>(
    state: &S,
) -> Result<std::collections::HashMap<String, String>, ApiError> {
    let stats = state.provider_stats().provider_stats_24h().await?;
    Ok(stats
        .iter()
        .filter_map(|s| crate::system::current_error_of(s).map(|e| (s.provider.clone(), e)))
        .collect())
}

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
        google_books_api_key_set: cfg
            .google_books_api_key
            .as_ref()
            .is_some_and(|s| !s.is_empty()),
        provider_status,
    }
}

pub async fn get_naming<S: HasAppConfigService>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<NamingConfigResponse>, ApiError> {
    let cfg = state.app_config_service().get_naming_config().await?;
    Ok(Json(NamingConfigResponse {
        author_folder_format: cfg.author_folder_format,
        book_folder_format: cfg.book_folder_format,
        rename_files: cfg.rename_files,
        replace_illegal_chars: cfg.replace_illegal_chars,
    }))
}

pub async fn get_media_management<S: HasAppConfigService>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<MediaManagementConfigResponse>, ApiError> {
    let cfg = state
        .app_config_service()
        .get_media_management_config()
        .await?;
    Ok(Json(MediaManagementConfigResponse {
        cwa_ingest_path: cfg.cwa_ingest_path,
        preferred_ebook_formats: cfg.preferred_ebook_formats,
        preferred_audiobook_formats: cfg.preferred_audiobook_formats,
    }))
}

pub async fn update_media_management<S: HasAppConfigService>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<UpdateMediaManagementApiRequest>,
) -> Result<Json<MediaManagementConfigResponse>, ApiError> {
    let cfg = state
        .app_config_service()
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

pub async fn get_metadata<S: HasAppConfigService + HasProviderStats>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<MetadataConfigResponse>, ApiError> {
    let cfg = state.app_config_service().get_metadata_config().await?;
    let provider_status = provider_error_map(&state).await?;
    Ok(Json(metadata_to_response(cfg, provider_status)))
}

/// Validate an LLM endpoint URL: must be http/https, no embedded credentials,
/// no literal private IP addresses. Hostnames resolving to private IPs are
/// permitted — the LLM endpoint is admin-configured trusted infrastructure
/// (self-hosted LocalAI / vLLM / llama.cpp on a private LAN/Docker network
/// is a common and supported deployment). The literal-IP check stays as a
/// sanity rail against obvious admin mistakes like pasting `127.0.0.1`.
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

pub async fn update_metadata<S: HasAppConfigService + HasProviderStats + HasLiveConfig>(
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

    if let Some(Some(ref k)) = req.google_books_api_key {
        if k.is_empty() {
            return Err(ApiError::BadRequest(
                "googleBooksApiKey must not be empty string; use null to clear".into(),
            ));
        }
    }

    let hardcover_api_token = req
        .hardcover_api_token
        .map(|inner| inner.map(|t| clean_token(&t)));
    let llm_api_key = req.llm_api_key.map(|inner| inner.map(|t| clean_token(&t)));
    let google_books_api_key = req
        .google_books_api_key
        .map(|inner| inner.map(|t| clean_token(&t)));

    let validated_languages = if let Some(langs) = req.languages {
        let effective_key = match &llm_api_key {
            None => None,
            Some(None) => None,
            Some(Some(v)) => Some(v.as_str()),
        };
        let effective_gb_key = match &google_books_api_key {
            None => None,
            Some(None) => Some(""),
            Some(Some(v)) => Some(v.as_str()),
        };
        Some(
            state
                .app_config_service()
                .validate_metadata_languages(
                    &langs,
                    req.llm_enabled,
                    req.llm_endpoint.as_deref(),
                    effective_key,
                    req.llm_model.as_deref(),
                    effective_gb_key,
                )
                .await
                .map_err(ApiError::BadRequest)?,
        )
    } else {
        None
    };

    let cfg = state
        .app_config_service()
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
            google_books_api_key,
        })
        .await?;

    state.live_metadata_config().replace(cfg.clone());

    let provider_status = provider_error_map(&state).await?;
    Ok(Json(metadata_to_response(cfg, provider_status)))
}

pub async fn test_hardcover<S: HasAppConfigService + HasHttpFetcher>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<(), ApiError> {
    use livrarr_domain::services::{
        FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
    };

    let cfg = state.app_config_service().get_metadata_config().await?;
    let token = cfg
        .hardcover_api_token
        .ok_or_else(|| ApiError::BadRequest("Hardcover API token not configured".into()))?;

    let clean = clean_token(&token);
    let req = FetchRequest {
        url: "https://api.hardcover.app/v1/graphql".to_string(),
        method: HttpMethod::Post,
        headers: vec![
            ("Authorization".into(), format!("Bearer {clean}")),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: Some(br#"{"query":"{ me { id } }"}"#.to_vec()),
        timeout: std::time::Duration::from_secs(10),
        rate_bucket: RateBucket::Hardcover,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        // Interactive: an admin's "Test Connection" click, B4 table.
        priority: livrarr_domain::RequestPriority::Interactive,
    };

    let resp = state
        .http_fetcher()
        .fetch(req)
        .await
        .map_err(|e| ApiError::BadGateway(format!("Hardcover connection failed: {e}")))?;

    if !(200..300).contains(&resp.status) {
        return Err(ApiError::BadGateway(format!(
            "Hardcover returned {} — check API token",
            resp.status
        )));
    }
    Ok(())
}

pub async fn test_audnexus<S: HasAppConfigService + HasHttpFetcher>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<(), ApiError> {
    use livrarr_domain::services::{
        FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
    };

    let cfg = state.app_config_service().get_metadata_config().await?;
    let url = format!(
        "{}/authors/B000AQ0842",
        cfg.audnexus_url.trim_end_matches('/')
    );

    let req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(30),
        rate_bucket: RateBucket::Audnexus,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        // Interactive: an admin's "Test Connection" click, B4 table.
        priority: livrarr_domain::RequestPriority::Interactive,
    };

    let resp = state
        .http_fetcher()
        .fetch(req)
        .await
        .map_err(|e| ApiError::BadGateway(format!("Audnexus connection failed: {e}")))?;

    if !(200..300).contains(&resp.status) {
        return Err(ApiError::BadGateway(format!(
            "Audnexus returned {}",
            resp.status
        )));
    }
    Ok(())
}

pub async fn test_llm<S: HasAppConfigService + HasHttpClient>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<(), ApiError> {
    let cfg = state.app_config_service().get_metadata_config().await?;
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
        tracing::warn!(status = %status, body = %text, "LLM test endpoint returned non-success");
        return Err(ApiError::BadGateway(format!(
            "LLM returned {status} (see server logs for details)"
        )));
    }
    Ok(())
}

pub async fn get_prowlarr<S: HasIndexerSettingsService>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<crate::ProwlarrConfigResponse>, ApiError> {
    let c = state
        .indexer_settings_service()
        .get_prowlarr_config()
        .await?;
    Ok(Json(crate::ProwlarrConfigResponse {
        url: c.url,
        api_key_set: c.api_key.is_some(),
        enabled: c.enabled,
    }))
}

pub async fn update_prowlarr<S: HasIndexerSettingsService>(
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
        .indexer_settings_service()
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

pub async fn get_email<S: HasAppConfigService>(
    _admin: RequireAdmin,
    State(state): State<S>,
) -> Result<Json<EmailConfigResponse>, ApiError> {
    let c = state.app_config_service().get_email_config().await?;
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

pub async fn update_email<S: HasAppConfigService>(
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
        .app_config_service()
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

pub async fn get_indexer_config<S: HasIndexerSettingsService>(
    State(state): State<S>,
    _admin: RequireAdmin,
) -> Result<Json<livrarr_domain::IndexerConfig>, ApiError> {
    let c = state
        .indexer_settings_service()
        .get_indexer_config()
        .await?;
    Ok(Json(c))
}

pub async fn update_indexer_config<S: HasIndexerSettingsService>(
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
    let c = state
        .indexer_settings_service()
        .update_indexer_config(req)
        .await?;
    Ok(Json(c))
}

pub async fn trigger_rss_sync<S: HasRssSync + HasRssSyncWorkflow>(
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

pub async fn test_email<S: HasEmailService>(
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
