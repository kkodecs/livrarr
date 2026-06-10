use std::time::Duration;

use livrarr_db::{create_test_db, CreateWorkDbRequest, MetadataCacheDb, WorkDbCreate};
use livrarr_domain::{normalize_for_matching, services::EnrichmentMode, MetadataProvider};
use livrarr_external_data::NormalizedWorkDetail;

fn work_req(user_id: i64) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: "Cache Contract".to_string(),
        author_name: "Cache Author".to_string(),
        normalized_title: normalize_for_matching("Cache Contract"),
        normalized_author: normalize_for_matching("Cache Author"),
        language: Some("en".to_string()),
        ..Default::default()
    }
}

#[tokio::test]
async fn metadata_cache_get_respects_ttl_and_hard_refresh_bypasses_cache() {
    // AC-011
    let db = create_test_db().await;
    let (work, created) = db
        .create_work(work_req(1))
        .await
        .expect("work row should be created for cache test");
    assert!(created);

    let payload = serde_json::to_string(&NormalizedWorkDetail {
        title: Some("Fresh cached title".to_string()),
        author_name: Some("Cache Author".to_string()),
        ..Default::default()
    })
    .expect("payload should serialize");

    db.metadata_cache_put(work.id, MetadataProvider::GoogleBooks, &payload)
        .await
        .expect("cache put should succeed");

    let fresh = db
        .metadata_cache_get(
            work.id,
            MetadataProvider::GoogleBooks,
            Duration::from_secs(24 * 60 * 60),
        )
        .await
        .expect("fresh cache lookup should not error");
    assert!(
        fresh.is_some(),
        "non-refresh enrichment should use a provider payload cached within 24h"
    );

    assert!(
        EnrichmentMode::HardRefresh.bypasses_cache(),
        "HardRefresh must bypass the provider cache before refetching"
    );

    let stale = db
        .metadata_cache_get(
            work.id,
            MetadataProvider::GoogleBooks,
            Duration::from_secs(0),
        )
        .await
        .expect("stale cache lookup should not error");
    assert!(
        stale.is_none(),
        "payloads older than the max_age are stale and must be refetched"
    );
}
