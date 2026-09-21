//! Original, edition, and unclassified dates at real provider and merge boundaries.
//! External fixtures are captured or explicitly synthetic; production merge output
//! is persisted unchanged. These cases do not exercise cache admission or Add.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{http::header, Router};
use livrarr_db::{
    create_test_db, sqlite::SqliteDb, CreateWorkDbRequest, ProvenanceDb, ProviderPolicyDb, WorkDb,
    WorkDbCreate,
};
use livrarr_domain::{
    services::RateBucket, AnchorQuery, ApplyMergeOutcome, FieldProvenance, MetadataProvider,
    OutcomeClass, ProvenanceSetter, RequestPriority, Work,
};
use livrarr_enrichment::{
    build_apply_request, DefaultMergeEngine, EnrichmentMode, MergeEngine, MergeInput,
    ReconstructedOutcome,
};
use livrarr_external_data::{
    live_config::LiveMetadataConfig, GoodreadsClient, GoogleBooksClient, HardcoverClient,
    NormalizedWorkDetail, ProviderClient, ProviderOutcome,
};
use livrarr_http::fetcher::{HttpFetcherImpl, ScriptedTransportOutcome};
use serde_json::{json, Value};

const GOOGLE: &str = include_str!("fixtures/date_google_books_dune_search.json");
const HARDCOVER: &str = include_str!("fixtures/date_hardcover_dune_search.json");
const GOODREADS_DETAIL: &str =
    include_str!("../../../crates/livrarr-external-data/fixtures/gr-book-23692271.html");
const REPAIR_HTML: &str = include_str!("fixtures/date_repair_unreadable.synthetic.html");
const REPAIR_COMPLETION: &str = include_str!("fixtures/date_repair_completion.synthetic.json");
const REPAIR_DATE: &str = "2007-09-03";
const REPAIR_DESCRIPTION: &str =
    "A cartographer returns to a village where every window reflects a different season.";

fn response(body: Vec<u8>) -> ScriptedTransportOutcome {
    ScriptedTransportOutcome::Response {
        delay: Duration::ZERO,
        response: livrarr_domain::services::FetchResponse {
            status: 200,
            headers: Vec::new(),
            body,
        },
    }
}

fn live_config() -> LiveMetadataConfig {
    LiveMetadataConfig::new(livrarr_db::MetadataConfig {
        hardcover_enabled: true,
        hardcover_api_token: Some("j3a-scripted-token".into()),
        google_books_api_key: Some("j3a-scripted-key".into()),
        llm_enabled: false,
        llm_provider: None,
        llm_endpoint: None,
        llm_api_key: None,
        llm_model: None,
        audnexus_url: "https://api.audnex.us".into(),
        languages: vec!["en".into()],
    })
}

fn success(outcome: ProviderOutcome<NormalizedWorkDetail>) -> NormalizedWorkDetail {
    match outcome {
        ProviderOutcome::Success(detail) => *detail,
        other => panic!("scripted provider response must normalize successfully: {other:?}"),
    }
}

async fn google_detail(isbn: &str) -> NormalizedWorkDetail {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let fetcher = HttpFetcherImpl::new()
        .unwrap()
        .with_scripted_transport(move |request| {
            assert_eq!(request.rate_bucket, RateBucket::GoogleBooks);
            assert!(request
                .url
                .starts_with("https://www.googleapis.com/books/v1/volumes?"));
            observed.lock().unwrap().push(request.url.clone());
            response(GOOGLE.as_bytes().to_vec())
        });
    let detail = success(
        GoogleBooksClient::new(fetcher, live_config())
            .fetch_by_isbn(isbn, RequestPriority::Interactive)
            .await,
    );
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "real ISBN client must consume the capture"
    );
    assert!(requests[0].contains(isbn));
    assert_eq!(detail.isbn_13.as_deref(), Some(isbn));
    detail
}

async fn hardcover_detail() -> NormalizedWorkDetail {
    let legs = Arc::new(Mutex::new(Vec::new()));
    let observed = legs.clone();
    let fetcher = HttpFetcherImpl::new()
        .unwrap()
        .with_scripted_transport(move |request| {
            assert_eq!(request.rate_bucket, RateBucket::Hardcover);
            assert!(request.url.starts_with("https://api.hardcover.app/"));
            let body: Value = serde_json::from_slice(request.body.as_deref().unwrap()).unwrap();
            if body.pointer("/variables/query").is_some() {
                observed.lock().unwrap().push("search");
                response(HARDCOVER.as_bytes().to_vec())
            } else {
                assert!(body["query"].as_str().unwrap().contains("GetEditions"));
                assert_eq!(body["variables"]["bookId"], 312460);
                observed.lock().unwrap().push("editions");
                // Controlled nonempty response to the EXISTING identifier-only
                // query. No edition date is requested or supplied by this leg.
                response(br#"{"data":{"editions":[{"isbn_13":"9780441013593","language":{"language":"English"}}]}}"#.to_vec())
            }
        });
    let detail = success(
        ProviderClient::Hardcover(HardcoverClient::new(fetcher, live_config()))
            .fetch(
                &Work {
                    title: "Dune".into(),
                    author_name: "Frank Herbert".into(),
                    ..Default::default()
                },
                RequestPriority::Interactive,
            )
            .await,
    );
    assert_eq!(*legs.lock().unwrap(), vec!["search", "editions"]);
    assert_eq!(detail.hc_key.as_deref(), Some("312460"));
    assert_eq!(detail.page_count, Some(896));
    assert_eq!(detail.isbn_13.as_deref(), Some("9780441013593"));
    detail
}

async fn goodreads_detail() -> NormalizedWorkDetail {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let fetcher = HttpFetcherImpl::new()
        .unwrap()
        .with_scripted_transport(move |request| {
            assert_eq!(request.rate_bucket, RateBucket::Goodreads);
            assert_eq!(request.url, "https://www.goodreads.com/book/show/23692271");
            observed.lock().unwrap().push(request.url.clone());
            response(GOODREADS_DETAIL.as_bytes().to_vec())
        });
    let client = ProviderClient::Goodreads(GoodreadsClient::new(
        fetcher,
        livrarr_http::HttpClient::builder().build().unwrap(),
        "https://www.goodreads.com",
    ));
    let detail = success(
        client
            .fetch_by_anchor(
                AnchorQuery::GrKey("23692271".into()),
                None,
                RequestPriority::Interactive,
            )
            .await,
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(detail.gr_key.as_deref(), Some("23692271"));
    assert_eq!(detail.gr_work_key.as_deref(), Some("18962767"));
    assert_eq!(detail.page_count, Some(512));
    detail
}

/// Both external responses are labeled synthetic fixtures. Only the page HTTP
/// transport is intercepted: the real repair client posts to a local axum server.
async fn goodreads_repaired_detail() -> NormalizedWorkDetail {
    assert!(livrarr_external_data::goodreads::parse_detail_html(REPAIR_HTML).is_none());
    let cleaned = livrarr_external_data::provider_util::clean_html_for_llm(REPAIR_HTML);
    assert!(cleaned.contains("The Glass Orchard") && cleaned.contains(REPAIR_DATE));

    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let envelope: Value = serde_json::from_str(REPAIR_COMPLETION).unwrap();
    let app = Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                let observed = observed.clone();
                let envelope = envelope.clone();
                async move {
                    observed.lock().unwrap().push((headers, body));
                    axum::Json(envelope)
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("repair fixture needs host execution with localhost sockets");
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let pages = Arc::new(Mutex::new(Vec::new()));
    let observed_pages = pages.clone();
    let fetcher = HttpFetcherImpl::new()
        .unwrap()
        .with_scripted_transport(move |request| {
            assert_eq!(request.rate_bucket, RateBucket::Goodreads);
            assert_eq!(request.url, "https://www.goodreads.com/book/show/999000007");
            observed_pages.lock().unwrap().push(request.url.clone());
            response(REPAIR_HTML.as_bytes().to_vec())
        });
    let mut config = (*live_config().snapshot()).clone();
    config.llm_enabled = true;
    config.llm_endpoint = Some(endpoint);
    config.llm_api_key = Some("synthetic-j3a-repair-key".into());
    config.llm_model = Some("synthetic-j3a-repair-model".into());
    let client = ProviderClient::Goodreads(
        GoodreadsClient::new(
            fetcher,
            livrarr_http::HttpClient::builder().build().unwrap(),
            "https://www.goodreads.com",
        )
        .with_live_config(LiveMetadataConfig::new(config)),
    );
    let outcome = tokio::time::timeout(
        Duration::from_secs(30),
        client.fetch_by_anchor(
            AnchorQuery::GrKey("999000007".into()),
            Some("en"),
            RequestPriority::Interactive,
        ),
    )
    .await;
    server.abort();
    let _ = server.await;
    let detail = success(outcome.expect("bounded real Goodreads repair request"));
    assert_eq!(pages.lock().unwrap().len(), 1);
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "actual configured repair must run exactly once"
    );
    let (headers, body) = &requests[0];
    assert_eq!(
        headers[header::AUTHORIZATION],
        "Bearer synthetic-j3a-repair-key"
    );
    assert_eq!(body["model"], "synthetic-j3a-repair-model");
    assert_eq!(body["response_format"]["type"], "json_object");
    assert!(body["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains(&cleaned));
    assert_eq!(detail.title.as_deref(), Some("The Glass Orchard"));
    assert_eq!(detail.author_name.as_deref(), Some("Mira Vale"));
    assert_eq!(detail.description.as_deref(), Some(REPAIR_DESCRIPTION));
    assert_eq!(detail.page_count, Some(312));
    assert_eq!(detail.publisher.as_deref(), Some("Lantern Press"));
    assert_eq!(detail.language.as_deref(), Some("en"));
    assert_eq!(detail.gr_key.as_deref(), Some("999000007"));
    detail
}

/// Reconstruct the deployed cache shape from the real client's non-date facts.
/// Both old Google/Hardcover dates and ambiguous Goodreads parser/repair dates
/// had only year + publish_date. Omit the new fields entirely, even after the
/// live producer is fixed; this is not a serialization of a newly classified row.
fn historical_date_payload(
    detail: &NormalizedWorkDetail,
    captured_date: &str,
) -> NormalizedWorkDetail {
    let mut value = serde_json::to_value(detail).unwrap();
    let fields = value.as_object_mut().unwrap();
    fields.remove("original_publish_date");
    fields.remove("unclassified_publish_date");
    fields.insert(
        "year".into(),
        json!(captured_date[..4].parse::<i32>().unwrap()),
    );
    fields.insert("publish_date".into(), json!(captured_date));
    assert!(!fields.contains_key("original_publish_date"));
    assert!(!fields.contains_key("unclassified_publish_date"));
    serde_json::from_value(value).expect("the real historical normalized cache format")
}

async fn date_fixture(detail: &NormalizedWorkDetail) -> (SqliteDb, Work) {
    let db = create_test_db().await;
    let user_id = livrarr_behavioral::stubs::create_test_user(&db).await;
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            language: detail.language.clone(),
            ..super::work_req(
                user_id,
                detail.title.as_deref().expect("real provider title"),
                detail.author_name.as_deref().expect("real provider author"),
            )
        })
        .await
        .unwrap();
    assert!(created);
    (db, work)
}

async fn merge_and_save(
    db: &SqliteDb,
    work: &Work,
    provider: MetadataProvider,
    detail: &NormalizedWorkDetail,
    cached: bool,
) -> Work {
    let current_work = db.get_work(work.user_id, work.id).await.unwrap();
    let current_provenance = db
        .list_work_provenance(work.user_id, work.id)
        .await
        .unwrap();
    let generation = db
        .get_merge_generation(work.user_id, work.id)
        .await
        .unwrap();
    let engine = DefaultMergeEngine::from_policy(Arc::new(
        db.load_provider_policy_snapshot().await.unwrap(),
    ));
    let language = current_work.language.clone();
    let output = if cached {
        engine
            .merge_from_cached(
                current_work,
                HashMap::from([(provider, detail.clone())]),
                current_provenance,
                language.as_deref(),
            )
            .await
    } else {
        engine
            .merge(MergeInput {
                priority_model: engine.priority_model(language.as_deref()),
                current_work,
                current_provenance,
                provider_results: HashMap::from([(
                    provider,
                    ReconstructedOutcome {
                        class: OutcomeClass::Success,
                        payload: Some(detail.clone()),
                    },
                )]),
                mode: EnrichmentMode::Manual,
            })
            .await
    }
    .expect("production date merge");
    assert_eq!(
        db.apply_enrichment_merge(build_apply_request(
            &output,
            work.user_id,
            work.id,
            generation
        ))
        .await
        .unwrap(),
        ApplyMergeOutcome::Applied,
    );
    db.get_work(work.user_id, work.id).await.unwrap()
}

// These domain/cache types serialize with snake_case. JSON access lets this
// suite compile before the optional original_publish_date field is implemented;
// the red assertion is on the actual normalized/persisted value, not a new symbol.
fn assert_dates(value: &Value, year: Option<i32>, original: Option<&str>, edition: Option<&str>) {
    assert_eq!(value["year"].as_i64(), year.map(i64::from), "original year");
    assert_eq!(
        value["publish_date"].as_str(),
        edition,
        "edition date and precision"
    );
    assert_eq!(
        value["original_publish_date"].as_str(),
        original,
        "original date and precision"
    );
}

async fn field_provenance(db: &SqliteDb, work: &Work, field: &str) -> Option<FieldProvenance> {
    db.list_work_provenance(work.user_id, work.id)
        .await
        .unwrap()
        .into_iter()
        .find(|row| serde_json::to_value(row.field).unwrap().as_str() == Some(field))
}

async fn assert_date_provenance(
    db: &SqliteDb,
    work: &Work,
    provider: MetadataProvider,
    fields: &[(&str, bool)],
) {
    for &(field, offered) in fields {
        let row = field_provenance(db, work, field).await;
        if offered {
            let row = row.unwrap_or_else(|| panic!("missing {field} provenance"));
            assert_eq!(
                (row.source, row.setter, row.cleared),
                (Some(provider), ProvenanceSetter::Provider, false)
            );
        } else {
            assert!(
                row.is_none(),
                "a rejected {field} offer must not claim provenance"
            );
        }
    }
}

fn assert_non_dates(saved: &Work, detail: &NormalizedWorkDetail) {
    assert!(
        detail.description.is_some(),
        "the response supplies a non-date control"
    );
    assert_eq!(saved.description, detail.description);
    assert_eq!(saved.page_count, detail.page_count);
    assert_eq!(saved.publisher, detail.publisher);
}

async fn assert_merge_dates(
    provider: MetadataProvider,
    detail: NormalizedWorkDetail,
    cached: bool,
    expected: (Option<i32>, Option<&str>, Option<&str>),
) {
    let (db, work) = date_fixture(&detail).await;
    let saved = merge_and_save(&db, &work, provider, &detail, cached).await;
    let (year, original, edition) = expected;
    assert_dates(
        &serde_json::to_value(&saved).unwrap(),
        year,
        original,
        edition,
    );
    assert_non_dates(&saved, &detail);
    assert_date_provenance(
        &db,
        &saved,
        provider,
        &[
            ("year", year.is_some()),
            ("original_publish_date", original.is_some()),
            ("publish_date", edition.is_some()),
        ],
    )
    .await;
    if !cached {
        assert_dates(
            &serde_json::to_value(&detail).unwrap(),
            year,
            original,
            edition,
        );
    }
}

// REQ-003 / AC-003; selected archived T1.
#[tokio::test]
async fn google_live_month_keeps_edition_date_and_no_original_year() {
    let detail = google_detail("9781847394286").await;
    assert_merge_dates(
        MetadataProvider::GoogleBooks,
        detail,
        false,
        (None, None, Some("2009-01")),
    )
    .await;
}

// REQ-003 / AC-003; selected archived T2.
#[tokio::test]
async fn google_old_cache_keeps_month_edition_date_and_rejects_poisoned_year() {
    let detail = google_detail("9781847394286").await;
    let old = historical_date_payload(&detail, "2009-01");
    assert_eq!(
        (old.year, old.publish_date.as_deref()),
        (Some(2009), Some("2009-01"))
    );
    assert_merge_dates(
        MetadataProvider::GoogleBooks,
        old,
        true,
        (None, None, Some("2009-01")),
    )
    .await;
}

// REQ-003 / AC-003; selected archived T3. The existing editions leg supplies
// an ISBN only, while the captured book release supplies the original date.
#[tokio::test]
async fn hardcover_live_editions_isbn_keeps_original_year_and_date() {
    let detail = hardcover_detail().await;
    assert_merge_dates(
        MetadataProvider::Hardcover,
        detail,
        false,
        (Some(1965), Some("1965-06-01"), None),
    )
    .await;
}

// REQ-003 / AC-003; selected archived T5. Old cache must still offer its
// trustworthy year. Its old book date is never an edition date, and it cannot
// erase an original date that a classified provider pass has already saved.
#[tokio::test]
async fn hardcover_old_cache_keeps_original_year_without_relabeling_edition_date() {
    let detail = hardcover_detail().await;
    let old = historical_date_payload(&detail, "1965-06-01");
    let (db, work) = date_fixture(&old).await;
    let saved = merge_and_save(&db, &work, MetadataProvider::Hardcover, &old, true).await;
    assert_eq!(saved.year, Some(1965));
    assert_eq!(
        saved.publish_date, None,
        "the cached book date is not an edition date"
    );
    // R2 permits retaining the known year alone until a fresh classified fetch.
    // If full original precision is recovered, it must be the supplied date.
    let saved_json = serde_json::to_value(&saved).unwrap();
    if !saved_json["original_publish_date"].is_null() {
        assert_eq!(
            saved_json["original_publish_date"].as_str(),
            Some("1965-06-01")
        );
    }
    assert_non_dates(&saved, &old);
    assert_date_provenance(
        &db,
        &saved,
        MetadataProvider::Hardcover,
        &[("year", true), ("publish_date", false)],
    )
    .await;

    // Feed the existing merger the classified shape planned in R2. Baseline
    // serde ignores the optional new field; no speculative Rust type is needed.
    // Only date meaning changes; all facts still come from the actual HC client.
    let mut classified = serde_json::to_value(&detail).unwrap();
    classified["year"] = json!(1965);
    classified["publish_date"] = Value::Null;
    classified["original_publish_date"] = json!("1965-06-01");
    let classified: NormalizedWorkDetail = serde_json::from_value(classified).unwrap();
    let known = merge_and_save(&db, &work, MetadataProvider::Hardcover, &classified, false).await;
    assert_dates(
        &serde_json::to_value(&known).unwrap(),
        Some(1965),
        Some("1965-06-01"),
        None,
    );
    assert_date_provenance(
        &db,
        &known,
        MetadataProvider::Hardcover,
        &[("original_publish_date", true)],
    )
    .await;
    let retained = merge_and_save(&db, &work, MetadataProvider::Hardcover, &old, true).await;
    assert_dates(
        &serde_json::to_value(&retained).unwrap(),
        Some(1965),
        Some("1965-06-01"),
        None,
    );
    assert_date_provenance(
        &db,
        &retained,
        MetadataProvider::Hardcover,
        &[
            ("year", true),
            ("original_publish_date", true),
            ("publish_date", false),
        ],
    )
    .await;
}

// REQ-003 / AC-003; selected archived T6. Captured Work.publicationTime is
// 2011-01-01; Book.details.publicationTime is the different 2015 edition date.
#[tokio::test]
async fn goodreads_live_recorded_detail_keeps_original_2011_year_and_date() {
    let detail = goodreads_detail().await;
    assert_merge_dates(
        MetadataProvider::Goodreads,
        detail,
        false,
        (Some(2011), Some("2011-01-01"), None),
    )
    .await;
}

// REQ-003 / AC-003; selected archived T8. Actual parser miss -> configured
// local HTTP repair -> production decoder/normalizer -> merger -> SQLite.
#[tokio::test]
async fn goodreads_live_synthetic_llm_repair_preserves_non_dates_without_date_offers() {
    let detail = goodreads_repaired_detail().await;
    let normalized = serde_json::to_value(&detail).unwrap();
    assert_merge_dates(
        MetadataProvider::Goodreads,
        detail,
        false,
        (None, None, None),
    )
    .await;
    assert_eq!(
        normalized["unclassified_publish_date"].as_str(),
        Some(REPAIR_DATE),
        "retain the supplied date without inventing its meaning"
    );
}

// REQ-003 / AC-003; selected archived T14. Historical parser and repair output
// shared this shape; the cache does not establish which origin supplied a date.
#[tokio::test]
async fn goodreads_ambiguous_old_cache_offers_neither_date_and_retains_non_dates() {
    let detail = goodreads_detail().await;
    let old = historical_date_payload(&detail, "2011-01-01");
    assert_eq!(
        (old.year, old.publish_date.as_deref()),
        (Some(2011), Some("2011-01-01"))
    );
    assert_merge_dates(MetadataProvider::Goodreads, old, true, (None, None, None)).await;
}
