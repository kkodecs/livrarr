use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::{FromRequest, Multipart, Path, Query, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use livrarr_domain::services::{CoverService, CoverServiceError};
use livrarr_domain::{
    AuthType, CoverCandidate, CoverMediaType, SelectCoverRequest, User, UserRole,
};
use livrarr_handlers::context::HasCoverService;
use livrarr_handlers::cover::{
    get_cover_alternatives, select_cover_handler, upload_cover_handler, UploadCoverQuery,
};
use livrarr_handlers::AuthContext;
use livrarr_matching::string_similarity;

#[test]
fn qw2_m4_normalize_latin_diacritic_control_stays_folded() {
    assert_eq!(string_similarity("Caf\u{00E9}", "Cafe"), 1.0);
}

#[test]
fn qw2_m4_normalize_strips_thai_combining_mark_that_survives_current_filter() {
    assert_eq!(string_similarity("\u{0E01}\u{0E35}", "\u{0E01}"), 1.0);
}

#[test]
fn qw2_m4_normalize_preserves_cjk_cyrillic_and_arabic_base_characters() {
    assert_eq!(string_similarity("三体", "三体"), 1.0);
    assert_eq!(string_similarity("Преступление", "преступление"), 1.0);
    assert_eq!(string_similarity("كتاب", "كتاب"), 1.0);
}

#[derive(Clone)]
struct CoverState {
    service: Arc<FailingCoverService>,
}

impl HasCoverService for CoverState {
    type CoverSvc = FailingCoverService;

    fn cover_service(&self) -> &Self::CoverSvc {
        &self.service
    }
}

struct FailingCoverService {
    alternatives_error: CoverServiceError,
    select_error: CoverServiceError,
    upload_error: CoverServiceError,
}

impl FailingCoverService {
    fn with_errors(
        alternatives_error: CoverServiceError,
        select_error: CoverServiceError,
        upload_error: CoverServiceError,
    ) -> CoverState {
        CoverState {
            service: Arc::new(Self {
                alternatives_error,
                select_error,
                upload_error,
            }),
        }
    }
}

impl CoverService for FailingCoverService {
    async fn fetch_alternatives(
        &self,
        _user_id: i64,
        _work_id: i64,
    ) -> Result<Vec<CoverCandidate>, CoverServiceError> {
        Err(match &self.alternatives_error {
            CoverServiceError::NotFound => CoverServiceError::NotFound,
            CoverServiceError::InvalidCandidate(msg) => {
                CoverServiceError::InvalidCandidate(msg.clone())
            }
            CoverServiceError::UploadValidation(msg) => {
                CoverServiceError::UploadValidation(msg.clone())
            }
            CoverServiceError::Internal(msg) => CoverServiceError::Internal(msg.clone()),
        })
    }

    async fn select_cover(
        &self,
        _user_id: i64,
        _work_id: i64,
        _candidate_id: &str,
        _media_type: CoverMediaType,
    ) -> Result<(), CoverServiceError> {
        Err(match &self.select_error {
            CoverServiceError::NotFound => CoverServiceError::NotFound,
            CoverServiceError::InvalidCandidate(msg) => {
                CoverServiceError::InvalidCandidate(msg.clone())
            }
            CoverServiceError::UploadValidation(msg) => {
                CoverServiceError::UploadValidation(msg.clone())
            }
            CoverServiceError::Internal(msg) => CoverServiceError::Internal(msg.clone()),
        })
    }

    async fn upload_cover(
        &self,
        _user_id: i64,
        _work_id: i64,
        _data: &[u8],
        _media_type: CoverMediaType,
    ) -> Result<(), CoverServiceError> {
        Err(match &self.upload_error {
            CoverServiceError::NotFound => CoverServiceError::NotFound,
            CoverServiceError::InvalidCandidate(msg) => {
                CoverServiceError::InvalidCandidate(msg.clone())
            }
            CoverServiceError::UploadValidation(msg) => {
                CoverServiceError::UploadValidation(msg.clone())
            }
            CoverServiceError::Internal(msg) => CoverServiceError::Internal(msg.clone()),
        })
    }
}

fn auth_context() -> AuthContext {
    let now = Utc::now();
    AuthContext {
        user: User {
            id: 7,
            username: "qw2-cover".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
            api_key_hash: "api".to_string(),
            setup_pending: false,
            created_at: now,
            updated_at: now,
        },
        auth_type: AuthType::ApiKey,
        session_token_hash: None,
    }
}

async fn response_json(response: Response) -> (StatusCode, serde_json::Value) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    let body = serde_json::from_slice(&bytes).expect("error response should be JSON");
    (status, body)
}

fn assert_api_error_envelope(body: &serde_json::Value, status: u16, error: &str) {
    assert_eq!(
        body.get("status").and_then(|v| v.as_u64()),
        Some(status.into())
    );
    assert_eq!(body.get("error").and_then(|v| v.as_str()), Some(error));
    assert!(body.get("message").and_then(|v| v.as_str()).is_some());
}

async fn multipart_with_image() -> Multipart {
    let boundary = "qw2boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"cover.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nabc\r\n--{boundary}--\r\n"
    );
    let req = Request::builder()
        .method("POST")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .expect("multipart request should build");
    Multipart::from_request(req, &())
        .await
        .expect("multipart should parse")
}

#[tokio::test]
async fn qw2_cover_alternatives_internal_error_returns_api_error_envelope() {
    let state = FailingCoverService::with_errors(
        CoverServiceError::Internal("provider failed".to_string()),
        CoverServiceError::NotFound,
        CoverServiceError::NotFound,
    );

    let response = get_cover_alternatives(State(state), auth_context(), Path(42))
        .await
        .into_response();
    let (status, body) = response_json(response).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_api_error_envelope(&body, 500, "internal");
}

#[tokio::test]
async fn qw2_select_cover_not_found_returns_api_error_envelope() {
    let state = FailingCoverService::with_errors(
        CoverServiceError::Internal("unused".to_string()),
        CoverServiceError::NotFound,
        CoverServiceError::NotFound,
    );
    let req = SelectCoverRequest {
        candidate_id: "candidate".to_string(),
        media_type: CoverMediaType::Ebook,
    };

    let response = select_cover_handler(State(state), auth_context(), Path(42), Json(req))
        .await
        .into_response();
    let (status, body) = response_json(response).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_api_error_envelope(&body, 404, "not_found");
}

#[tokio::test]
async fn qw2_select_cover_invalid_candidate_returns_api_error_envelope() {
    let state = FailingCoverService::with_errors(
        CoverServiceError::Internal("unused".to_string()),
        CoverServiceError::InvalidCandidate("candidate expired".to_string()),
        CoverServiceError::NotFound,
    );
    let req = SelectCoverRequest {
        candidate_id: "candidate".to_string(),
        media_type: CoverMediaType::Ebook,
    };

    let response = select_cover_handler(State(state), auth_context(), Path(42), Json(req))
        .await
        .into_response();
    let (status, body) = response_json(response).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_api_error_envelope(&body, 400, "bad_request");
}

#[tokio::test]
async fn qw2_upload_cover_validation_error_returns_api_error_envelope() {
    let state = FailingCoverService::with_errors(
        CoverServiceError::Internal("unused".to_string()),
        CoverServiceError::NotFound,
        CoverServiceError::UploadValidation("not an image".to_string()),
    );
    let multipart = multipart_with_image().await;

    let response = upload_cover_handler(
        State(state),
        auth_context(),
        Path(42),
        Query(UploadCoverQuery {
            media_type: CoverMediaType::Ebook,
        }),
        multipart,
    )
    .await
    .into_response();
    let (status, body) = response_json(response).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_api_error_envelope(&body, 400, "bad_request");
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        message.contains("not an image"),
        "validation detail must reach the client, got: {message}"
    );
}

#[cfg(feature = "qw2_compile_red")]
mod cancellation_signature_pin {
    use livrarr_metadata::series_query_service::fetch_series_roster_pages;
    use tokio_util::sync::CancellationToken;

    #[tokio::test(start_paused = true)]
    async fn qw2_series_roster_pagination_accepts_cancellation_token() {
        let fetcher = livrarr_behavioral::stubs::StubHttpFetcher::new();
        let token = CancellationToken::new();
        token.cancel();
        let _ = fetch_series_roster_pages(&fetcher, "108562", token).await;
    }
}
