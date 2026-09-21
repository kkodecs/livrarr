use super::persistence::*;
use super::*;

/// Explicit synthetic request for persistence failures/duplicates. Discovery
/// coverage above always uses the returned card; it never uses this fixture.
fn save_request() -> Value {
    json!({
        "title":"Dune", "authorName":"Frank Herbert", "coverManual":false,
        "facts":{
            "provider":"google_books", "language":"en",
            "description":"Save regression description", "descriptionTruncated":false,
            "editionPublishDate":"2005-08", "publisher":"Save regression publisher",
            "pageCount":548, "seriesName":"Dune", "seriesPosition":1.0,
            "rating":4.5, "ratingCount":25, "genres":["Fiction"],
            "subtitle":"Save regression subtitle",
            "contributors":[{"name":"Frank Herbert"}],
            "references":[{"kind":"google_volume", "value":"save-regression-volume"}]
        }
    })
}

// SAVE-P1: REQ-002; AC-002. Existing table; no speculative schema setup.
#[tokio::test]
async fn provenance_failure_after_work_insert_rolls_back_then_retry_creates_once() {
    let _breaker = lock_breaker().await;
    let (h, _, gate) = captured_harness(MetadataProvider::GoogleBooks).await;
    gate.store(false, Ordering::SeqCst);
    sqlx::query(
        "CREATE TRIGGER amp_abort_required_provenance BEFORE INSERT ON work_metadata_provenance
         WHEN NEW.field='description' AND EXISTS (
             SELECT 1 FROM works WHERE user_id=NEW.user_id AND id=NEW.work_id
         )
         BEGIN SELECT RAISE(ABORT, 'amp_required_save_after_work_insert'); END",
    )
    .execute(h.db.pool())
    .await
    .expect("trigger on baseline provenance table must install before the behavioral assertion");
    let body = save_request();
    let failed = post_add(&h, body.clone()).await;
    assert_eq!(
        failed.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "required save failure must propagate: {}",
        failed.json
    );
    assert_ne!(failed.json["created"], true, "no partial success response");
    let works: i64 = sqlx::query_scalar("SELECT count(*) FROM works WHERE user_id=?1")
        .bind(h.user_id)
        .fetch_one(h.db.pool())
        .await
        .unwrap();
    let births: i64 =
        sqlx::query_scalar("SELECT count(*) FROM history WHERE user_id=?1 AND event_type='added'")
            .bind(h.user_id)
            .fetch_one(h.db.pool())
            .await
            .unwrap();
    let provenance: i64 =
        sqlx::query_scalar("SELECT count(*) FROM work_metadata_provenance WHERE user_id=?1")
            .bind(h.user_id)
            .fetch_one(h.db.pool())
            .await
            .unwrap();
    assert_eq!(
        (works, births, provenance),
        (0, 0, 0),
        "required save shares the Work/birth transaction"
    );
    sqlx::query("DROP TRIGGER amp_abort_required_provenance")
        .execute(h.db.pool())
        .await
        .unwrap();
    let retried = post_add(&h, body).await;
    let id = created_id(&retried);
    assert_eq!(
        retried.json["work"]["description"],
        "Save regression description"
    );
    assert_source(
        &retried.json["work"],
        "description",
        json!("google_books"),
        "provider",
    );
    assert_one_birth(&h, id).await;
}

fn named_metadata(work: &Value) -> Value {
    let mut selected = serde_json::Map::new();
    for key in [
        "description",
        "descriptionTruncated",
        "language",
        "year",
        "publishDate",
        "originalPublishDate",
        "publisher",
        "pageCount",
        "genres",
        "seriesName",
        "seriesPosition",
        "rating",
        "ratingCount",
        "coverUrl",
        "coverSource",
        "coverManual",
        "coverWidth",
        "coverHeight",
        "monitorEbook",
        "monitorAudiobook",
        "sourceReferences",
        "fieldSources",
    ] {
        let mut value = work
            .get(key)
            .unwrap_or_else(|| panic!("missing named read field {key}"))
            .clone();
        if matches!(key, "sourceReferences" | "fieldSources") {
            value
                .as_array_mut()
                .expect("read collection")
                .sort_by_key(Value::to_string);
        }
        selected.insert(key.into(), value);
    }
    Value::Object(selected)
}

// SAVE-P2: REQ-007; AC-006. Same real route adopts its existing Work.
#[tokio::test]
async fn duplicate_add_preserves_named_metadata_provenance_and_personal_edits() {
    let _breaker = lock_breaker().await;
    let (h, _, gate) = captured_harness(MetadataProvider::GoogleBooks).await;
    gate.store(false, Ordering::SeqCst);
    let body = save_request();
    let added = post_add(&h, body.clone()).await;
    let id = created_id(&added);
    assert_eq!(
        added.json["work"]["description"],
        body["facts"]["description"]
    );
    assert_source(
        &added.json["work"],
        "description",
        json!("google_books"),
        "provider",
    );
    assert_reference(
        &added.json["work"],
        "google_books",
        "subtitle",
        "Save regression subtitle",
    );
    assert!(
        added.json["work"]["subtitle"].is_null(),
        "reference subtitle does not alter identity"
    );
    wait_for_add(&h, id).await;
    let edited = call_router_json(
        &h,
        Method::PUT,
        format!("/api/v1/work/{id}"),
        Some(json!({
            "seriesName":"My shelf sequence", "seriesPosition":7.0,
            "monitorEbook":false, "monitorAudiobook":true
        })),
    )
    .await;
    assert_eq!(
        edited.status,
        StatusCode::OK,
        "personal edit: {}",
        edited.json
    );
    let before = detail(&h, id).await;
    assert_eq!(before["seriesName"], "My shelf sequence");
    assert_eq!(before["seriesPosition"], 7.0);
    assert_eq!(before["monitorEbook"], false);
    assert_eq!(before["monitorAudiobook"], true);
    assert_source(&before, "series_name", Value::Null, "user");
    assert_source(&before, "series_position", Value::Null, "user");
    let provenance = h.db.list_work_provenance(h.user_id, id).await.unwrap();
    let mut duplicate = body;
    duplicate["facts"]["description"] = json!("Re-add must not replace this");
    duplicate["facts"]["descriptionTruncated"] = json!(true);
    duplicate["facts"]["pageCount"] = json!(999);
    duplicate["facts"]["language"] = json!("fr");
    duplicate["facts"]["editionPublishDate"] = json!("2026");
    duplicate["facts"]["seriesName"] = json!("Incoming series");
    duplicate["facts"]["seriesPosition"] = json!(2.0);
    duplicate["facts"]["rating"] = json!(1.5);
    duplicate["facts"]["ratingCount"] = json!(9);
    duplicate["facts"]["references"] = json!([{"kind":"google_volume", "value":"incoming-volume"}]);
    duplicate["facts"]["coverUrl"] = json!("https://covers.openlibrary.org/b/id/11481354-L.jpg");
    duplicate["coverUrl"] = duplicate["facts"]["coverUrl"].clone();
    duplicate["coverManual"] = json!(true);
    let adopted = post_add(&h, duplicate).await;
    assert_eq!(
        adopted.status,
        StatusCode::OK,
        "duplicate response: {}",
        adopted.json
    );
    assert_eq!(adopted.json["created"], false);
    assert_eq!(adopted.json["authorCreated"], false);
    assert_eq!(adopted.json["work"]["id"], id);
    assert_eq!(
        named_metadata(&adopted.json["work"]),
        named_metadata(&before)
    );
    assert_eq!(
        named_metadata(&detail(&h, id).await),
        named_metadata(&before)
    );
    assert_eq!(
        h.db.list_work_provenance(h.user_id, id).await.unwrap(),
        provenance
    );
    assert_one_birth(&h, id).await;
    assert_primary_author_only(&h, id).await;
}

// SAVE-C1: REQ-004; AC-004. Exactly the initial existing Add continuation.
#[tokio::test]
async fn failed_initial_explicit_image_attempt_keeps_url_source_and_honest_image_state() {
    let _breaker = lock_breaker().await;
    let (h, requests, gate) = captured_harness(MetadataProvider::OpenLibrary).await;
    let card = lookup_card(&h, "openlibrary", "olKey", "OL893414W").await;
    let mut body = add_from_card(&card);
    body["coverManual"] = json!(true);
    let url = body["facts"]["coverUrl"]
        .as_str()
        .expect("captured cover address")
        .to_string();
    // Current explicit choice uses the card URL; only the user's manual intent
    // changes. All transport responses after selection, including this URL,
    // are controlled 503s through the real HttpFetcher and image writer.
    gate.store(false, Ordering::SeqCst);
    let added = post_add(&h, body).await;
    let id = created_id(&added);
    assert_eq!(added.json["work"]["coverUrl"], url);
    assert_reference(
        &added.json["work"],
        "open_library",
        "cover_url_explicit",
        &url,
    );
    assert_no_image(&h, id, &added.json["work"]).await;
    wait_for_add(&h, id).await;
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .any(|requested| requested == &url),
        "real initial image attempt must reach the controlled failing transport"
    );
    let read = detail(&h, id).await;
    assert_eq!(read["coverUrl"], url);
    assert_reference(&read, "open_library", "cover_url_explicit", &url);
    assert_no_image(&h, id, &read).await;
    let saved = h.db.get_work(h.user_id, id).await.unwrap();
    assert_eq!(saved.cover_url.as_deref(), Some(url.as_str()));
}

// SAVE-P3: REQ-001/006; AC-006. One incomplete-pair fixture, no merge matrix.
#[tokio::test]
async fn initial_incomplete_pairs_keep_missing_partners_and_contributor_associations() {
    let _breaker = lock_breaker().await;
    let (h, _, gate) = captured_harness(MetadataProvider::OpenLibrary).await;
    gate.store(false, Ordering::SeqCst);
    // Synthetic request is intentional: the named captures have complete pairs
    // and one author each. Duplicate names with different ids exercise the
    // settled ordinal association without creating any additional Authors.
    let facts = json!({
        "provider":"open_library", "descriptionTruncated":false, "genres":[],
        "seriesName":"Dune", "seriesPosition":null, "rating":4.25, "ratingCount":null,
        "contributors":[
            {"name":"Frank Herbert", "providerAuthorId":"OL79034A"},
            {"name":"Repeated Contributor", "providerAuthorId":"OL100001A"},
            {"name":"Repeated Contributor", "providerAuthorId":"OL100002A"}
        ], "references":[]
    });
    let added = post_add(
        &h,
        json!({
            "title":"Dune", "authorName":"Frank Herbert", "facts":facts, "coverManual":false
        }),
    )
    .await;
    let id = created_id(&added);
    for work in [&added.json["work"], &detail(&h, id).await] {
        assert_eq!(work["seriesName"], "Dune");
        assert!(work["seriesPosition"].is_null());
        assert_eq!(work["rating"], 4.25);
        assert!(work["ratingCount"].is_null());
        assert_source(work, "series_name", json!("open_library"), "provider");
        assert_source(work, "rating", json!("open_library"), "provider");
        assert_no_source(work, "series_position");
        assert_no_source(work, "rating_count");
        assert_contributors(work, &facts);
        assert!(
            work["language"].is_null(),
            "configured English default is not book language"
        );
        assert_no_source(work, "language");
    }
    assert_primary_author_only(&h, id).await;
    assert_one_birth(&h, id).await;
}
