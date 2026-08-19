// tests/behavioral/test_wcc_discovery_fanout.rs
#![allow(dead_code, unused_imports)]

//! #97: WorkService discovery (`lookup`) must fan out across providers instead
//! of returning the first provider that answers. Otherwise a book present only
//! on a later provider (e.g. Hardcover, behind a non-empty Google Books) is
//! silently dropped from search results.
//!
//! `StubHttpFetcher::with_ok` replays the same body on every call, so a
//! `call_count()` assertion measures how many providers were queried regardless
//! of completion order (the fan-out runs them concurrently).

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::*;
use livrarr_domain::services::*;
use livrarr_domain::{RequestPriority, UserRole};
use livrarr_metadata::discovery_service::{DiscoveryServiceImpl, StubNoLlm};
use std::sync::{Arc, Mutex};
use tracing_test::traced_test;

#[derive(Clone)]
struct RecordingHttpFetcher {
    body: Vec<u8>,
    requests: Arc<Mutex<Vec<(RateBucket, RequestPriority)>>>,
}

impl RecordingHttpFetcher {
    fn new(body: Vec<u8>) -> Self {
        Self {
            body,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<(RateBucket, RequestPriority)> {
        self.requests.lock().expect("recorded requests").clone()
    }
}

impl HttpFetcher for RecordingHttpFetcher {
    async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.requests
            .lock()
            .expect("record request")
            .push((req.rate_bucket, req.priority));
        Ok(FetchResponse {
            status: 200,
            headers: Vec::new(),
            body: self.body.clone(),
        })
    }

    async fn fetch_ssrf_safe(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.fetch(req).await
    }
}

#[derive(Clone)]
struct GoodreadsDropFetcher;

impl HttpFetcher for GoodreadsDropFetcher {
    async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        if req.rate_bucket == RateBucket::Goodreads {
            return Err(FetchError::QueueFull {
                retry_after: std::time::Duration::from_secs(1),
            });
        }
        Ok(FetchResponse {
            status: 200,
            headers: Vec::new(),
            body: serde_json::to_vec(&serde_json::json!({
                "docs": [{
                    "key": "/works/OL-DISCOVERY-SURVIVES-W",
                    "title": "Other Provider Survives",
                    "author_name": ["Search Author"]
                }]
            }))
            .expect("OpenLibrary fixture"),
        })
    }

    async fn fetch_ssrf_safe(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.fetch(req).await
    }
}

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-test-fanout-{}", std::process::id()))
}

async fn setup_user(db: &SqliteDb) -> i64 {
    db.create_user(CreateUserDbRequest {
        username: "fanoutuser".into(),
        password_hash: "hash".into(),
        role: UserRole::Admin,
        api_key_hash: "fanouthash".into(),
    })
    .await
    .unwrap()
    .id
}

/// Enable Google Books + Hardcover so both are actually queried (each
/// short-circuits to empty without its credential).
async fn enable_providers(db: &SqliteDb) {
    db.update_metadata_config(UpdateMetadataConfigRequest {
        hardcover_enabled: Some(true),
        hardcover_api_token: Some(Some("test-hc-token".into())),
        llm_enabled: None,
        llm_provider: None,
        llm_endpoint: None,
        llm_api_key: None,
        llm_model: None,
        audnexus_url: None,
        languages: None,
        google_books_api_key: Some(Some("test-gb-key".into())),
    })
    .await
    .unwrap();
}

/// A Google Books `volumes` body that parses to one result — so the OLD
/// first-hit cascade would STOP at Google Books after a single HTTP call.
fn gb_one_result() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "items": [{
            "id": "gb-1",
            "volumeInfo": {
                "title": "Google Result",
                "authors": ["Author A"],
                "publishedDate": "1965",
                "industryIdentifiers": [{"type": "ISBN_13", "identifier": "9780000000001"}],
            }
        }]
    }))
    .unwrap()
}

#[tokio::test]
async fn test_lookup_fans_out_across_providers_not_first_hit() {
    let db = create_test_db().await;
    let _user = setup_user(&db).await;
    enable_providers(&db).await;

    // Every HTTP call replays the same Google Books body: Google Books parses it
    // (non-empty), the other providers parse it as their own format and yield
    // nothing — but all of them are still queried by the fan-out.
    // Bug reproduction: identity-layer-rewrite S-17 — a user-facing lookup is
    // Interactive work in the shared outbound queue. Normal-priority provider
    // legs can sit behind High-priority add traffic until an outer leg budget
    // cancels them before HTTP.
    let http = RecordingHttpFetcher::new(gb_one_result());
    let svc = DiscoveryServiceImpl::new(db, http.clone(), StubNoLlm);

    let _ = svc
        .lookup(LookupRequest {
            term: "dune".into(),
            lang_override: None,
        })
        .await
        .unwrap();

    // First-hit would stop after Google Books returned a result (1 call). The
    // fan-out queries every eligible provider regardless of who answers first.
    // GB + OpenLibrary + Hardcover are enabled and Goodreads is always queried
    // (scrape-based, needs no credential), so the fan-out issues exactly four
    // provider calls; the old first-hit cascade would have stopped at one.
    let requests = http.requests();
    assert_eq!(
        requests.len(),
        4,
        "lookup must query every provider (GB+OL+HC+Goodreads), not stop at the first hit; got {} call(s)",
        requests.len()
    );
    assert!(
        requests
            .iter()
            .all(|(_, priority)| *priority == RequestPriority::Interactive),
        "every user-search leg must enter the real outbound queue as Interactive; got {requests:?}"
    );
}

#[tokio::test]
#[traced_test]
async fn test_failed_goodreads_leg_warns_once_and_other_provider_results_serve() {
    // Bug reproduction: identity-layer-rewrite S-17 — every dropped discovery
    // leg has one provider+cause WARN, while healthy sibling legs still serve.
    let db = create_test_db().await;
    let _user = setup_user(&db).await;
    let svc = DiscoveryServiceImpl::new(db, GoodreadsDropFetcher, StubNoLlm);

    let results = svc
        .lookup(LookupRequest {
            term: "other provider survives".into(),
            lang_override: None,
        })
        .await
        .expect("one provider failure must not fail discovery");

    assert!(
        results
            .iter()
            .any(|result| result.ol_key.as_deref() == Some("OL-DISCOVERY-SURVIVES-W")),
        "OpenLibrary results must survive a dropped Goodreads leg: {results:?}"
    );
    logs_assert(|lines: &[&str]| {
        let drops = lines
            .iter()
            .filter(|line| {
                line.contains("discovery provider leg dropped")
                    && line.contains("Goodreads")
                    && line.contains("cause")
            })
            .count();
        if drops == 1 {
            Ok(())
        } else {
            Err(format!(
                "expected exactly one standardized Goodreads drop WARN, got {drops}: {lines:?}"
            ))
        }
    });
}
