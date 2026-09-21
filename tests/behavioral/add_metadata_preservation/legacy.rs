use super::persistence::*;
use super::*;

fn assert_legacy_reference(work: &Value, kind: &str, value: &str) {
    let references = work["sourceReferences"]
        .as_array()
        .expect("sourceReferences");
    let reference = references
        .iter()
        .find(|row| row["kind"] == kind && row["value"] == value)
        .unwrap_or_else(|| panic!("missing unclassified legacy {kind}: {work}"));
    // R2 permits a nullable provider in the read representation; either honest
    // legacy spelling carries no claim that Google supplied this input.
    assert!(reference["provider"].is_null() || reference["provider"] == "legacy");
}

// SAVE-L1: REQ-003/004/007; AC-003/006.
#[tokio::test]
async fn legacy_add_saves_supplied_language_cover_and_unclassified_year() {
    let _breaker = lock_breaker().await;
    let (h, _, gate) = captured_harness(MetadataProvider::GoogleBooks).await;
    gate.store(false, Ordering::SeqCst);
    let url = "https://covers.openlibrary.org/b/id/11481354-L.jpg";
    // Explicit input remains a book fact even when it equals the configured en default.
    let added = post_add(
        &h,
        json!({
            "title":"Dune", "authorName":"Frank Herbert", "year":2005,
            "language":"en", "coverUrl":url, "coverManual":false,
            "metadataSource":"google_books",
            "candidateId":"cbb97bcb-df48-4e9a-831a-bf367a063a02"
        }),
    )
    .await;
    let id = created_id(&added);
    let read = detail(&h, id).await;
    for work in [&added.json["work"], &read] {
        assert_eq!(
            work["language"], "en",
            "save explicitly supplied language even when it equals the search default"
        );
        assert_source(work, "language", Value::Null, "import");
        assert_eq!(work["coverUrl"], url);
        assert_legacy_reference(work, "cover_url", url);
        assert_legacy_reference(work, "unclassified_year", "2005");
        for (field, source) in [
            ("year", "year"),
            ("originalPublishDate", "original_publish_date"),
            ("publishDate", "publish_date"),
        ] {
            assert!(work
                .get(field)
                .expect("date field in read response")
                .is_null());
            assert_no_source(work, source);
        }
        assert_eq!(work["descriptionTruncated"], false);
        for source in work["fieldSources"].as_array().unwrap() {
            if matches!(
                source["field"].as_str(),
                Some("language" | "cover_url" | "year" | "publish_date" | "original_publish_date")
            ) {
                assert!(
                    source["source"].is_null(),
                    "legacy hints do not prove a provider"
                );
                assert_ne!(
                    source["setter"], "user",
                    "copied legacy facts are not personal edits"
                );
            }
        }
    }
    let saved = h.db.get_work(h.user_id, id).await.unwrap();
    assert_eq!(saved.language.as_deref(), Some("en"));
    assert_eq!(saved.cover_url.as_deref(), Some(url));
    assert_eq!(saved.year, None);
    assert_eq!(saved.publish_date, None);
    assert_one_birth(&h, id).await;

    // Required empty-legacy control: omission must not save the configured
    // search/default language. A fresh account keeps its birth count isolated.
    let (empty, _, empty_gate) = captured_harness(MetadataProvider::GoogleBooks).await;
    empty_gate.store(false, Ordering::SeqCst);
    let response = post_add(
        &empty,
        json!({"title":"Dune", "authorName":"Frank Herbert"}),
    )
    .await;
    let empty_id = created_id(&response);
    let read = detail(&empty, empty_id).await;
    for work in [&response.json["work"], &read] {
        for field in [
            "language",
            "year",
            "publishDate",
            "originalPublishDate",
            "coverUrl",
        ] {
            assert!(
                work.get(field).expect("ordinary read field").is_null(),
                "omitted legacy {field}"
            );
        }
        for field in [
            "language",
            "year",
            "publish_date",
            "original_publish_date",
            "cover_url",
        ] {
            assert_no_source(work, field);
        }
        assert!(work["sourceReferences"]
            .as_array()
            .expect("sourceReferences")
            .is_empty());
    }
    assert_one_birth(&empty, empty_id).await;
}
