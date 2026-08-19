//! Behavioral tests for metadata-correctness filters, covers, and regression pins.

use livrarr_behavioral::stubs::{create_test_user, StubEnrichmentWorkflow, StubHttpFetcher};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{
    ConfigDb, CreateWorkDbRequest, UpdateMetadataConfigRequest, UpdateWorkEnrichmentDbRequest,
    WorkDb, WorkDbCreate,
};
use livrarr_domain::services::{
    CoverSlotState, DiscoveryService, LookupRequest, MaterializeRequest, MaterializeService,
    MaterializeTags, RefreshSurface, WorkFilter, WorkService,
};
use livrarr_domain::{normalize_for_matching, EnrichmentStatus, Work};
use livrarr_materialize::LiveMaterializeService;
use livrarr_metadata::work_service::WorkServiceImpl;

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

fn service(db: SqliteDb, workflow: StubEnrichmentWorkflow) -> TestWorkService {
    WorkServiceImpl::new(
        db,
        workflow,
        StubHttpFetcher::new(),
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    )
}

fn filter() -> WorkFilter {
    WorkFilter {
        author_id: None,
        monitored: None,
        enrichment_status: None,
        media_type: None,
        language: None,
        sort_by: None,
        sort_dir: None,
    }
}

fn work_req(user_id: i64, title: &str, language: &str, monitored: bool) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: "Metadata Correctness Author".to_string(),
        normalized_title: normalize_for_matching(title),
        normalized_author: normalize_for_matching("Metadata Correctness Author"),
        language: Some(language.to_string()),
        monitor_ebook: monitored,
        monitor_audiobook: false,
        ..Default::default()
    }
}

async fn seed_work(
    db: &SqliteDb,
    user_id: i64,
    title: &str,
    language: &str,
    monitored: bool,
) -> Work {
    let (work, created) = db
        .create_work(work_req(user_id, title, language, monitored))
        .await
        .expect("seed work");
    assert!(created);
    work
}

#[tokio::test]
async fn list_language_filter_returns_exact_language_slice() {
    // REQ-015/AC-017: language=fr returns exactly the French works.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let french = seed_work(&db, user_id, "Le Comte de Monte-Cristo", "fr", true).await;
    let english = seed_work(&db, user_id, "Dune", "en", true).await;
    let polish = seed_work(&db, user_id, "Pan Tadeusz", "pl", true).await;
    let svc = service(db, StubEnrichmentWorkflow::succeeding());

    let mut f = filter();
    f.language = Some("fr".to_string());
    let listed = svc.list(user_id, f).await.expect("list works");
    let ids = listed.into_iter().map(|w| w.id).collect::<Vec<_>>();

    assert_eq!(ids, vec![french.id]);
    assert!(!ids.contains(&english.id));
    assert!(!ids.contains(&polish.id));
}

#[tokio::test]
async fn list_language_and_monitored_filters_intersect() {
    // REQ-015/AC-017: combined active facets are intersected, not unioned.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let wanted = seed_work(&db, user_id, "French Monitored", "fr", true).await;
    let unmonitored_fr = seed_work(&db, user_id, "French Unmonitored", "fr", false).await;
    let monitored_en = seed_work(&db, user_id, "English Monitored", "en", true).await;
    let svc = service(db, StubEnrichmentWorkflow::succeeding());

    let mut f = filter();
    f.language = Some("fr".to_string());
    f.monitored = Some(true);
    let listed = svc.list(user_id, f).await.expect("list works");
    let ids = listed.into_iter().map(|w| w.id).collect::<Vec<_>>();

    assert_eq!(ids, vec![wanted.id]);
    assert!(!ids.contains(&unmonitored_fr.id));
    assert!(!ids.contains(&monitored_en.id));
}

#[tokio::test]
async fn materialize_saved_cover_reports_decoded_dimensions() {
    // REQ-017/AC-019: a newly saved cover returns width/height for the orchestrator to persist.
    let png_1x1 = vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D',
        b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8,
        0xcf, 0xc0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xdd, 0x8d, 0xb0, 0x00, 0x00, 0x00,
        0x00, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
    ];
    let http = std::sync::Arc::new(StubHttpFetcher::with_ok(200, png_1x1));
    let svc = LiveMaterializeService::new(http);
    let temp = tempfile::tempdir().expect("covers tempdir");

    let outcome = svc
        .materialize(MaterializeRequest {
            work_id: 9001,
            changed: true,
            tag_fields_changed: false,
            ebook_cover: CoverSlotState {
                chosen_new_url: Some("https://images.example/cover.png".to_string()),
                current_url: None,
                current_path: None,
                user_locked: false,
                ..Default::default()
            },
            audiobook_cover: CoverSlotState::default(),
            file_paths: vec![],
            tags: MaterializeTags::default(),
            covers_dir: temp.path().to_path_buf(),
        })
        .await
        .expect("materialize cover");

    let saved = outcome
        .saved_cover
        .expect("REQ-017/AC-019: saved_cover should be populated");
    assert!(saved.path.exists());
    assert_eq!((saved.width, saved.height), (1, 1));
}

#[tokio::test]
async fn user_set_cover_survives_refresh_byte_identically() {
    // REQ-019/AC-021: a user-set cover URL and trust survive refresh unchanged.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let user_id = create_test_user(&db).await;
    let work = seed_work(&db, user_id, "Manual Cover Work", "en", true).await;
    db.update_work_enrichment(
        user_id,
        work.id,
        UpdateWorkEnrichmentDbRequest {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("test".to_string()),
            cover_url: Some("https://covers.example/manual.jpg".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("set cover url");
    db.update_cover_metadata(
        user_id,
        work.id,
        Some("https://covers.example/manual.jpg"),
        "user",
        true,
        640,
        960,
    )
    .await
    .expect("set user cover trust");
    let svc = service(db.clone(), StubEnrichmentWorkflow::succeeding());

    svc.refresh(user_id, work.id, RefreshSurface::Interactive)
        .await
        .expect("refresh work");
    let refreshed = db
        .get_work(user_id, work.id)
        .await
        .expect("read refreshed work");

    assert_eq!(
        refreshed.cover_url.as_deref(),
        Some("https://covers.example/manual.jpg")
    );
    assert!(refreshed.cover_manual);
}

#[tokio::test]
async fn cjk_discovery_result_does_not_inherit_query_language() {
    // REQ-011/AC-013: the #11 case shape — the real lookup road must not stamp
    // the query term's language onto discovery results. This stays red while
    // Google Books falls back from missing payload language to query language.
    let db = livrarr_db::test_helpers::create_test_db().await;
    let _user = create_test_user(&db).await;
    // Enable Google Books so the fan-out actually queries it (keyless = zero calls).
    db.update_metadata_config(UpdateMetadataConfigRequest {
        hardcover_enabled: None,
        hardcover_api_token: None,
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
    .expect("enable google books");

    // Every HTTP call replays a GB-shaped body carrying NO language: any
    // Some(language) on a result could only come from query-side stamping —
    // exactly the #11 regression this pins.
    let body = serde_json::to_vec(&serde_json::json!({
        "items": [{
            "id": "gb-1",
            "volumeInfo": {
                "title": "三体",
                "authors": ["刘慈欣"],
                "publishedDate": "2008",
            }
        }]
    }))
    .expect("gb body");
    let http = StubHttpFetcher::with_ok(200, body);
    let svc = livrarr_metadata::discovery_service::DiscoveryServiceImpl::new(
        db,
        http,
        livrarr_metadata::discovery_service::StubNoLlm,
    );

    let results = svc
        .lookup(LookupRequest {
            term: "三体".into(),
            lang_override: None,
        })
        .await
        .expect("lookup");

    assert!(
        !results.is_empty(),
        "the GB-shaped body should parse to at least one result"
    );
    assert!(
        results.iter().all(|r| r.language.is_none()),
        "AC-013: no discovery result may carry a language inferred from the query term"
    );
}
