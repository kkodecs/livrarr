#![allow(dead_code, unused_imports)]

//! Behavioral tests for LlmCaller trait (PRIM-LLM-001..005).
//! Covers: fn.llm_caller.call
//! Test obligations: test.llm.disallowed_field, test.llm.not_configured

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::{routing::post, Router};
use livrarr_domain::services::*;
use livrarr_domain::settings::MetadataConfig;
use livrarr_external_data::live_config::LiveMetadataConfig;
use livrarr_external_data::llm_caller_service::LlmCallerImpl;
use livrarr_http::HttpClient;

/// Helper: build a LiveMetadataConfig for tests with given endpoint/key/model.
fn test_live_config(endpoint: &str, api_key: &str, model: &str) -> LiveMetadataConfig {
    LiveMetadataConfig::new(MetadataConfig {
        hardcover_enabled: false,
        hardcover_api_token: None,
        llm_enabled: true,
        llm_provider: None,
        llm_endpoint: Some(endpoint.to_string()),
        llm_api_key: Some(api_key.to_string()),
        llm_model: Some(model.to_string()),
        audnexus_url: String::new(),
        languages: vec![],
        google_books_api_key: None,
    })
}

/// Helper: spin up an axum test server, return the base URL (e.g. "http://127.0.0.1:PORT/v1/").
async fn spawn_test_server(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}/v1/", addr.port())
}

/// Helper: build a minimal valid LlmCallRequest.
fn valid_request() -> LlmCallRequest {
    let mut context = HashMap::new();
    context.insert(LlmField::Title, LlmValue::Text("Dune".into()));
    LlmCallRequest {
        system_template: "You are a book validator. Title: {title}".to_string(),
        user_template: "Validate this title: {title}".to_string(),
        context,
        allowed_fields: &[LlmField::Title],
        timeout: Duration::from_secs(5),
        purpose: LlmPurpose::IdentityValidation,
    }
}

#[tokio::test]
async fn test_llm_disallowed_field_returns_error_before_network() {
    // PRIM-LLM-002, test.llm.disallowed_field: Given context with field not in allowed_fields, returns DisallowedField before any network call
    let caller = LlmCallerImpl::new(
        test_live_config("http://localhost:9999/v1/", "test-key", "test-model"),
        HttpClient::builder().build().unwrap(),
    );

    let mut context = HashMap::new();
    context.insert(LlmField::Title, LlmValue::Text("Dune".into()));

    let req = LlmCallRequest {
        system_template: "test".to_string(),
        user_template: "test".to_string(),
        context,
        allowed_fields: &[LlmField::AuthorName], // Title not allowed
        timeout: Duration::from_secs(5),
        purpose: LlmPurpose::IdentityValidation,
    };

    let result = caller.call(req).await;
    match result {
        Err(LlmError::DisallowedField { field }) => {
            assert_eq!(field, LlmField::Title);
        }
        other => panic!("expected DisallowedField, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_llm_not_configured_returns_not_configured() {
    // PRIM-LLM-001, test.llm.not_configured: Given no LLM configured, returns NotConfigured
    let caller = LlmCallerImpl::not_configured();

    let req = valid_request();
    let result = caller.call(req).await;

    match result {
        Err(LlmError::NotConfigured) => {} // expected
        other => panic!("expected NotConfigured, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_llm_valid_request_returns_content() {
    // PRIM-LLM-001: Given valid request, returns Ok with content
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            axum::Json(serde_json::json!({
                "choices": [{"message": {"content": "test response"}}],
                "model": "test-model"
            }))
        }),
    );

    let base_url = spawn_test_server(app).await;

    let caller = LlmCallerImpl::new(
        test_live_config(&base_url, "test-key", "test-model"),
        HttpClient::builder().build().unwrap(),
    );

    let req = valid_request();
    let result = caller.call(req).await.expect("should succeed");

    assert_eq!(result.content, "test response");
    assert_eq!(result.model_used, "test-model");
    assert!(!result.elapsed.is_zero());
}

#[tokio::test]
async fn test_llm_provider_timeout_returns_timeout() {
    // PRIM-LLM-001: Given provider timeout, returns Timeout
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            axum::Json(serde_json::json!({
                "choices": [{"message": {"content": "too late"}}],
                "model": "test-model"
            }))
        }),
    );

    let base_url = spawn_test_server(app).await;

    let caller = LlmCallerImpl::new(
        test_live_config(&base_url, "test-key", "test-model"),
        HttpClient::builder().build().unwrap(),
    );

    let mut context = HashMap::new();
    context.insert(LlmField::Title, LlmValue::Text("Dune".into()));

    let req = LlmCallRequest {
        system_template: "test".to_string(),
        user_template: "test".to_string(),
        context,
        allowed_fields: &[LlmField::Title],
        timeout: Duration::from_millis(100),
        purpose: LlmPurpose::IdentityValidation,
    };

    let result = caller.call(req).await;
    match result {
        Err(LlmError::Timeout) => {} // expected
        other => panic!("expected Timeout, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_llm_empty_response_returns_invalid_response() {
    // PRIM-LLM-001: Given empty response from provider, returns InvalidResponse
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            axum::Json(serde_json::json!({
                "choices": [{"message": {"content": ""}}],
                "model": "test-model"
            }))
        }),
    );

    let base_url = spawn_test_server(app).await;

    let caller = LlmCallerImpl::new(
        test_live_config(&base_url, "test-key", "test-model"),
        HttpClient::builder().build().unwrap(),
    );

    let req = valid_request();
    let result = caller.call(req).await;

    match result {
        Err(LlmError::InvalidResponse(_)) => {} // expected
        other => panic!("expected InvalidResponse, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_llm_provider_http_error_returns_without_retry() {
    // PRIM-LLM-001: must not retry on provider error (caller decides retry policy)
    let call_count = Arc::new(AtomicUsize::new(0));
    let count = call_count.clone();

    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error",
                )
            }
        }),
    );

    let base_url = spawn_test_server(app).await;

    let caller = LlmCallerImpl::new(
        test_live_config(&base_url, "test-key", "test-model"),
        HttpClient::builder().build().unwrap(),
    );

    let req = valid_request();
    let result = caller.call(req).await;

    match result {
        Err(LlmError::Provider(msg)) => {
            assert!(
                msg.contains("500"),
                "error should mention status code: {msg}"
            );
        }
        other => panic!("expected Provider error, got: {other:?}"),
    }

    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "provider should be called exactly once (no retry)"
    );
}

#[tokio::test]
#[ignore = "requires tracing-test subscriber infrastructure"]
async fn test_llm_redacts_prompt_in_info_logs() {
    // PRIM-LLM-004: must not log the full LLM request or response at info level or above
    // Testing log redaction requires injecting a tracing test subscriber and capturing
    // output. Marking #[ignore] since the tracing-test infrastructure isn't in scope
    // for this implementation pass.
}
