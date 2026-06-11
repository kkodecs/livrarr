use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");

use librarr::http::{
    ClientKind, DownloadClient, ForegroundClient, HealthCheckClient, HttpClientContract,
    HttpErrorKind, RetryDisposition, MAX_RESPONSE_BODY_BYTES,
};
use librarr::rate_limit::{DefaultRateLimiter, ProviderKind, RateLimitContract};

fn make_foreground_client() -> ForegroundClient {
    ForegroundClient
}

fn make_background_client() -> librarr::http::BackgroundClient {
    librarr::http::BackgroundClient
}

fn make_health_check_client() -> HealthCheckClient {
    HealthCheckClient
}

fn make_download_client(skip_ssl_validation: bool) -> DownloadClient {
    DownloadClient::new(skip_ssl_validation)
}

fn make_default_rate_limiter() -> DefaultRateLimiter {
    DefaultRateLimiter
}

fn assert_standard_user_agent<C: HttpClientContract>(client: &C) {
    assert_eq!(client.user_agent(), format!("Librarr/{}", VERSION));
}

fn assert_standard_retry_backoff<C: HttpClientContract>(client: &C) {
    assert_eq!(client.max_attempts(), 3);
    assert_eq!(
        client.backoff_schedule(),
        &[
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
        ]
    );
}

fn assert_no_retry_for_4xx<C: HttpClientContract>(client: &C) {
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Status4xx),
        RetryDisposition::NoRetry
    );
}

fn assert_no_retry_for_429<C: HttpClientContract>(client: &C) {
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Status429),
        RetryDisposition::NoRetry
    );
}

fn assert_retryable_for_transient_transport_failures<C: HttpClientContract>(client: &C) {
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Status5xx),
        RetryDisposition::Retryable
    );
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Connection),
        RetryDisposition::Retryable
    );
}

fn assert_non_retryable_for_timeout_and_tls<C: HttpClientContract>(client: &C) {
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Timeout),
        RetryDisposition::NoRetry
    );
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Tls),
        RetryDisposition::NoRetry
    );
}

/// REQ-ID: HTTP-v2.1
/// IR: foreground client uses foreground kind, 3s timeout, and disables HTTP retries entirely.
#[test]
fn foreground_client_contracts_are_synchronous_low_latency_and_non_retrying() {
    let client = make_foreground_client();

    assert_eq!(client.kind(), ClientKind::Foreground);
    assert_eq!(client.timeout(), Duration::from_secs(3));
    assert!(!client.retry_enabled());
    assert_eq!(client.max_attempts(), 1);
    assert!(client.backoff_schedule().is_empty());
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Status5xx),
        RetryDisposition::NoRetry
    );
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Connection),
        RetryDisposition::NoRetry
    );
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Timeout),
        RetryDisposition::NoRetry
    );
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Tls),
        RetryDisposition::NoRetry
    );
}

/// REQ-ID: HTTP-v2.1
/// IR: background client uses background kind, 30s timeout, and standard retry policy for transient failures.
#[test]
fn background_client_contracts_are_retrying_with_standard_backoff() {
    let client = make_background_client();

    assert_eq!(client.kind(), ClientKind::Background);
    assert_eq!(client.timeout(), Duration::from_secs(30));
    assert!(client.retry_enabled());
    assert_standard_retry_backoff(&client);
    assert_retryable_for_transient_transport_failures(&client);
    assert_non_retryable_for_timeout_and_tls(&client);
}

/// REQ-ID: HTTP-v2.1
/// IR: health-check client uses health-check kind, 5s timeout, and does not retry failed probes.
#[test]
fn health_check_client_contracts_are_fast_and_non_retrying() {
    let client = make_health_check_client();

    assert_eq!(client.kind(), ClientKind::HealthCheck);
    assert_eq!(client.timeout(), Duration::from_secs(5));
    assert!(!client.retry_enabled());
    assert_eq!(client.max_attempts(), 1);
    assert!(client.backoff_schedule().is_empty());
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Status5xx),
        RetryDisposition::NoRetry
    );
    assert_eq!(
        client.retry_disposition(HttpErrorKind::Connection),
        RetryDisposition::NoRetry
    );
}

/// REQ-ID: HTTP-v2.1
/// IR: download client uses download kind, 30s timeout, and standard retry policy for transient failures.
#[test]
fn download_client_core_http_contract_matches_background_retry_profile() {
    let client = make_download_client(false);

    assert_eq!(client.kind(), ClientKind::Download);
    assert_eq!(client.timeout(), Duration::from_secs(30));
    assert!(client.retry_enabled());
    assert_standard_retry_backoff(&client);
    assert_retryable_for_transient_transport_failures(&client);
    assert_non_retryable_for_timeout_and_tls(&client);
}

/// REQ-ID: HTTP-v2.1
/// IR: all HTTP client types send a Librarr/version user-agent header.
#[test]
fn all_http_clients_use_librarr_version_user_agent() {
    let foreground = make_foreground_client();
    let background = make_background_client();
    let health = make_health_check_client();
    let download = make_download_client(false);

    assert_standard_user_agent(&foreground);
    assert_standard_user_agent(&background);
    assert_standard_user_agent(&health);
    assert_standard_user_agent(&download);
}

/// REQ-ID: HTTP-v2.1
/// IR: HTTP layer must not retry generic 4xx responses for any client type.
#[test]
fn http_4xx_responses_are_not_retried_for_any_client_type() {
    let foreground = make_foreground_client();
    let background = make_background_client();
    let health = make_health_check_client();
    let download = make_download_client(false);

    assert_no_retry_for_4xx(&foreground);
    assert_no_retry_for_4xx(&background);
    assert_no_retry_for_4xx(&health);
    assert_no_retry_for_4xx(&download);
}

/// REQ-ID: HTTP-v2.1
/// IR: HTTP layer must not retry 429 responses for any client type because rate limiting is handled outside generic HTTP retry policy.
#[test]
fn http_429_responses_are_not_retried_for_any_client_type() {
    let foreground = make_foreground_client();
    let background = make_background_client();
    let health = make_health_check_client();
    let download = make_download_client(false);

    assert_no_retry_for_429(&foreground);
    assert_no_retry_for_429(&background);
    assert_no_retry_for_429(&health);
    assert_no_retry_for_429(&download);
}

/// REQ-ID: HTTP-v2.1
/// IR: response bodies are capped at exactly 10 MiB to bound memory usage.
#[test]
fn max_response_body_bytes_is_exactly_10_mib() {
    assert_eq!(MAX_RESPONSE_BODY_BYTES, 10 * 1024 * 1024);
    assert_eq!(MAX_RESPONSE_BODY_BYTES, 10_485_760);
}

/// REQ-ID: HTTP-v2.1
/// IR: only the download client exposes optional SSL validation skipping; all other clients always validate TLS.
#[test]
fn skip_ssl_validation_is_supported_only_by_download_client_configuration() {
    let foreground = make_foreground_client();
    let background = make_background_client();
    let health = make_health_check_client();
    let download_default = make_download_client(false);
    let download_insecure = make_download_client(true);

    assert!(!foreground.skip_ssl_validation());
    assert!(!background.skip_ssl_validation());
    assert!(!health.skip_ssl_validation());
    assert!(!download_default.skip_ssl_validation());
    assert!(download_insecure.skip_ssl_validation());
}

/// REQ-ID: HTTP-v2.1
/// IR: provider-specific default rate limits are Hardcover=1.0 rps, Audnexus=0.5 rps, and unspecified providers have no default cap.
#[test]
fn default_rate_limiter_exposes_provider_specific_request_rates() {
    let limiter = make_default_rate_limiter();

    assert_eq!(
        limiter.requests_per_second(ProviderKind::Hardcover),
        Some(1.0)
    );
    assert_eq!(
        limiter.requests_per_second(ProviderKind::Audnexus),
        Some(0.5)
    );
    assert_eq!(limiter.requests_per_second(ProviderKind::Other), None);
}
