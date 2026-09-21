//! Captured provider facts through the real authenticated lookup/Add/detail routes.
//! Raw captures are unchanged; backend tests use expected-facts.json only in assertions.

use super::*;
use livrarr_db::{ConfigDb, ProvenanceDb, UpdateMetadataConfigRequest};

mod discovery;
mod legacy;
mod persistence;
mod save_cases;

const GOOGLE: &str = include_str!("fixtures/google_books-dune-search.json");
const OPEN_LIBRARY: &str = include_str!("fixtures/open_library-dune-search.json");
const HARDCOVER: &str = include_str!("fixtures/hardcover-dune-search.json");
const GOODREADS: &str = include_str!("fixtures/goodreads-dune-search.json");
const EXPECTED_FACTS: &str = include_str!("fixtures/expected-facts.json");

type Requests = Arc<Mutex<Vec<String>>>;

/// Feed exactly one provider's raw response to the real parser. Other search
/// legs are empty; detail/cover requests fail and cannot supply missing facts.
fn captured_transport(
    provider: MetadataProvider,
    available: bool,
) -> (DiscoveryTransportFixture, Requests, Arc<AtomicBool>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let search_open = Arc::new(AtomicBool::new(available));
    let transport_open = Arc::clone(&search_open);
    let scripted = Arc::new(move |request: &livrarr_domain::services::FetchRequest| {
        observed.lock().unwrap().push(request.url.clone());
        if !transport_open.load(Ordering::SeqCst) {
            return round17_status_response(503, b"provider unavailable after selection".to_vec());
        }
        let response = if request.url.contains("/books/v1/volumes?") {
            if provider == MetadataProvider::GoogleBooks {
                GOOGLE
            } else {
                r#"{"items":[]}"#
            }
        } else if request.url.contains("/search.json?") {
            if provider == MetadataProvider::OpenLibrary {
                OPEN_LIBRARY
            } else {
                r#"{"docs":[]}"#
            }
        } else if request.url.contains("/book/auto_complete?") {
            if provider == MetadataProvider::Goodreads {
                GOODREADS
            } else {
                "[]"
            }
        } else if request.url.contains("api.hardcover.app") {
            let body: Value = serde_json::from_slice(request.body.as_deref().unwrap_or_default())
                .expect("Hardcover GraphQL request JSON");
            if body.pointer("/variables/query").is_some() {
                if provider == MetadataProvider::Hardcover {
                    HARDCOVER
                } else {
                    r#"{"data":{"search":{"results":{"hits":[]}}}}"#
                }
            } else {
                return round17_status_response(503, b"detail unavailable".to_vec());
            }
        } else {
            return round17_status_response(503, b"detail or cover unavailable".to_vec());
        };
        round13_response(response.as_bytes().to_vec())
    });
    let mut transport = round13_search_transport(scripted);
    transport.hardcover_search = true;
    (transport, requests, search_open)
}

async fn captured_harness(provider: MetadataProvider) -> (RouteHarness, Requests, Arc<AtomicBool>) {
    let (transport, requests, search_open) = captured_transport(provider, true);
    let harness =
        build_route_harness_with_identity_http(None, Vec::new(), Some(transport), None, true).await;
    configure_discovery(&harness.db).await;
    (harness, requests, search_open)
}

async fn configure_discovery(db: &SqliteDb) {
    // Discovery reads ConfigDb. These are private test-database settings;
    // the sentinel credentials can only reach the scripted transport above.
    db.update_metadata_config(UpdateMetadataConfigRequest {
        hardcover_enabled: Some(true),
        hardcover_api_token: Some(Some("amp-captured-hardcover".into())),
        google_books_api_key: Some(Some("amp-captured-google".into())),
        llm_enabled: Some(false),
        llm_provider: None,
        llm_endpoint: None,
        llm_api_key: None,
        llm_model: None,
        audnexus_url: None,
        languages: Some(vec!["en".into()]),
    })
    .await
    .expect("configure captured discovery on private SQLite");
}

async fn lookup_card(harness: &RouteHarness, provider: &str, key: &str, value: &str) -> Value {
    lookup_card_in_language(harness, provider, key, value, "en").await
}

async fn lookup_card_in_language(
    harness: &RouteHarness,
    provider: &str,
    key: &str,
    value: &str,
    language: &str,
) -> Value {
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        call_router_json(
            harness,
            Method::GET,
            format!("/api/v1/work/lookup?term=Dune&lang={language}&raw=true"),
            None,
        ),
    )
    .await
    .expect("bounded production lookup");
    assert_eq!(response.status, StatusCode::OK, "lookup: {}", response.json);
    response.json["results"]
        .as_array()
        .expect("lookup results")
        .iter()
        .find(|card| card["source"] == provider && card[key] == value)
        .unwrap_or_else(|| {
            panic!(
                "captured {provider} record {value} missing: {}",
                response.json
            )
        })
        .clone()
}

fn assert_provider_fetched(requests: &Requests, fragment: &str) {
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .any(|url| url.contains(fragment)),
        "the actual provider request must reach HttpFetcherImpl's scripted transport"
    );
}

async fn post_add(harness: &RouteHarness, body: Value) -> RouteResponse {
    tokio::time::timeout(
        Duration::from_secs(10),
        call_router_json(harness, Method::POST, "/api/v1/work".into(), Some(body)),
    )
    .await
    .expect("bounded authenticated Add")
}

fn expected_facts(provider: &str) -> Value {
    let records: Value =
        serde_json::from_str(EXPECTED_FACTS).expect("capture-based assertion values");
    records
        .get(provider)
        .expect("named captured provider")
        .clone()
}

/// Echo the actual lookup facts. A missing facts object is a behavioral failure;
/// never fill it from the expected capture values before driving Add.
fn add_from_card(card: &Value) -> Value {
    let facts = card
        .get("facts")
        .filter(|v| v.is_object())
        .unwrap_or_else(|| panic!("lookup dropped facts: {card}"));
    let mut body = json!({"coverManual": false, "facts": facts});
    if let Some(source) = card.get("source") {
        body["metadataSource"] = source.clone();
    }
    for key in [
        "olKey",
        "title",
        "authorName",
        "authorOlKey",
        "year",
        "coverUrl",
        "language",
        "detailUrl",
        "isbn13",
        "candidateId",
        "hcKey",
        "grKey",
        "asin",
    ] {
        if let Some(value) = card.get(key) {
            body[key] = value.clone();
        }
    }
    body
}

async fn detail(h: &RouteHarness, id: i64) -> Value {
    let response = call_router_json(h, Method::GET, format!("/api/v1/work/{id}"), None).await;
    assert_eq!(response.status, StatusCode::OK, "detail: {}", response.json);
    response.json
}

fn created_id(response: &RouteResponse) -> i64 {
    assert_eq!(response.status, StatusCode::OK, "Add: {}", response.json);
    assert_eq!(response.json["created"], true);
    response.json["work"]["id"]
        .as_i64()
        .expect("created Work id")
}

async fn assert_one_birth(h: &RouteHarness, id: i64) {
    let works: i64 = sqlx::query_scalar("SELECT count(*) FROM works WHERE user_id=?1")
        .bind(h.user_id)
        .fetch_one(h.db.pool())
        .await
        .unwrap();
    let births: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM history WHERE user_id=?1 AND work_id=?2 AND event_type='added'",
    )
    .bind(h.user_id)
    .bind(id)
    .fetch_one(h.db.pool())
    .await
    .unwrap();
    assert_eq!(works, 1, "one created Work");
    assert_eq!(births, 1, "one committed birth");
}

async fn wait_for_add(h: &RouteHarness, id: i64) {
    // Observe the real Add task's lifecycle, not a fixed sleep or a second call
    // to complete_add. This includes its existing delayed follow-up, but makes
    // no retry/recovery assertions.
    tokio::time::timeout(Duration::from_secs(45), async {
        while h.state.work_service.is_enriching(h.user_id, id) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("real Add continuation must finish within the harness deadline");
}
