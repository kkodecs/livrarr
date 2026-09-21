use super::persistence::*;
use super::*;

fn assert_complete_facts(card: &Value, expected: &Value) {
    let facts = card
        .get("facts")
        .filter(|value| value.is_object())
        .unwrap_or_else(|| panic!("lookup dropped facts: {card}"));
    for key in [
        "provider",
        "subtitle",
        "language",
        "originalYear",
        "originalPublishDate",
        "editionPublishDate",
        "description",
        "descriptionTruncated",
        "publisher",
        "pageCount",
        "seriesName",
        "seriesPosition",
        "genres",
        "rating",
        "ratingCount",
        "coverUrl",
        "bareTitle",
        "decoratedTitle",
    ] {
        assert_eq!(
            facts[key], expected[key],
            "captured {key}, including honest absence"
        );
    }
    let contributors = facts["contributors"]
        .as_array()
        .expect("all supplied contributor names");
    let expected_contributors = expected["contributors"].as_array().unwrap();
    assert_eq!(contributors.len(), expected_contributors.len());
    for (actual, expected) in contributors.iter().zip(expected_contributors) {
        assert_eq!(actual["name"], expected["name"]);
        assert_eq!(actual["providerAuthorId"], expected["providerAuthorId"]);
    }
    let mut actual = facts["references"]
        .as_array()
        .expect("typed references")
        .clone();
    let mut expected = expected["references"].as_array().unwrap().clone();
    actual.sort_by_key(Value::to_string);
    expected.sort_by_key(Value::to_string);
    assert_eq!(
        actual, expected,
        "all captured identifiers, including original spellings"
    );
}

async fn capture_to_add_and_reopen(provider: MetadataProvider, name: &str) {
    let directory = tempfile::tempdir().unwrap();
    let (h, requests, gate) = file_harness(directory.path(), provider, true).await;
    let (source, key, value, fragment, language) = match provider {
        MetadataProvider::GoogleBooks => (
            "google_books",
            "isbn13",
            "9780441013593",
            "/books/v1/volumes?",
            "en",
        ),
        MetadataProvider::OpenLibrary => {
            ("openlibrary", "olKey", "OL893414W", "/search.json?", "fr")
        }
        MetadataProvider::Hardcover => ("hardcover", "hcKey", "312460", "api.hardcover.app", "en"),
        MetadataProvider::Goodreads => (
            "goodreads",
            "grKey",
            "44767458",
            "/book/auto_complete?",
            "en",
        ),
        _ => panic!("unsupported capture"),
    };
    let card = lookup_card_in_language(&h, source, key, value, language).await;
    assert_provider_fetched(&requests, fragment);
    let expected = expected_facts(name);
    assert_complete_facts(&card, &expected);
    if provider == MetadataProvider::OpenLibrary {
        assert_provider_fetched(&requests, "language=fre");
        assert!(
            card["language"].is_null(),
            "French search preference is not book language"
        );
    }
    if provider == MetadataProvider::GoogleBooks {
        assert!(
            card["year"].is_null(),
            "edition date cannot claim original year"
        );
    }
    // From here onward, every provider/cover response is unavailable. The only
    // facts sent into the production Add route are those actually returned.
    let body = add_from_card(&card);
    assert_eq!(body["facts"], card["facts"]);
    let selection_requests = requests.lock().unwrap().len();
    gate.store(false, Ordering::SeqCst);
    let added = post_add(&h, body).await;
    let id = created_id(&added);
    assert_saved_view(&added.json["work"], &expected);
    assert_no_image(&h, id, &added.json["work"]).await;
    assert_one_birth(&h, id).await;
    assert_primary_author_only(&h, id).await;

    wait_for_add(&h, id).await;
    assert!(
        requests.lock().unwrap().len() > selection_requests,
        "real background provider transport must exercise the unavailable barrier"
    );
    let read = detail(&h, id).await;
    assert_saved_view(&read, &expected);
    assert_no_image(&h, id, &read).await;
    if provider == MetadataProvider::Goodreads {
        let promoted: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM identity_routes WHERE user_id=?1 AND resolved_work_id=?2 AND provider_scoped_id='3634639'",
        ).bind(h.user_id).bind(id).fetch_one(h.db.pool()).await.unwrap();
        assert_eq!(
            promoted, 0,
            "Goodreads Work id is a source reference, not a Book route"
        );
        assert!(
            !requests
                .lock()
                .unwrap()
                .iter()
                .any(|url| url.contains("gr-assets.com")),
            "automatic Goodreads image remains excluded after the real Add continuation"
        );
    }

    // New pool, services and router; no cached response can satisfy this read.
    h.db.pool().close().await;
    drop(h);
    let (reopened, _, _) = file_harness(directory.path(), provider, false).await;
    let read = detail(&reopened, id).await;
    assert_saved_view(&read, &expected);
    assert_no_image(&reopened, id, &read).await;
    assert_one_birth(&reopened, id).await;
    assert_primary_author_only(&reopened, id).await;
    reopened.db.pool().close().await;
}

// SAVE-D1: REQ-001/002/003/004/006; AC-001/002/003.
#[tokio::test]
async fn google_capture_to_add_preserves_edition_facts_after_reopen() {
    let _breaker = lock_breaker().await;
    capture_to_add_and_reopen(MetadataProvider::GoogleBooks, "google_books").await;
}

// SAVE-D2: REQ-001/002/003; AC-001/002/003.
#[tokio::test]
async fn open_library_capture_to_add_preserves_all_references_and_unknown_language() {
    let _breaker = lock_breaker().await;
    capture_to_add_and_reopen(MetadataProvider::OpenLibrary, "open_library").await;
}

// SAVE-D3: REQ-001/002/003/006; AC-001/002/003.
#[tokio::test]
async fn hardcover_capture_to_add_preserves_original_date_and_excludes_subtitle() {
    let _breaker = lock_breaker().await;
    capture_to_add_and_reopen(MetadataProvider::Hardcover, "hardcover").await;
}

// SAVE-D4: REQ-001/002/004/006; AC-001/002/004/006.
#[tokio::test]
async fn goodreads_capture_to_add_preserves_titles_truncation_and_contains_cover() {
    let _breaker = lock_breaker().await;
    capture_to_add_and_reopen(MetadataProvider::Goodreads, "goodreads").await;
}

#[tokio::test]
async fn open_library_discovery_preserves_author_id_pairing_after_empty_key() {
    let _breaker = lock_breaker().await;
    let (mut transport, _, _) = captured_transport(MetadataProvider::OpenLibrary, true);
    let fallback = Arc::clone(&transport.scripted_transport);
    transport.scripted_transport = Arc::new(move |request| {
        if request.url.contains("/search.json?") {
            round13_response(
                include_bytes!("fixtures/open_library-author-pairing.synthetic.json").to_vec(),
            )
        } else {
            fallback(request)
        }
    });
    let h =
        build_route_harness_with_identity_http(None, Vec::new(), Some(transport), None, true).await;
    configure_discovery(&h.db).await;
    let card = lookup_card(&h, "openlibrary", "olKey", "OL893414W").await;
    let contributors: Vec<_> = card["facts"]["contributors"]
        .as_array()
        .expect("discovery contributor facts")
        .iter()
        .map(|contributor| {
            (
                contributor["name"].as_str().expect("contributor name"),
                contributor["providerAuthorId"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        contributors,
        vec![("Author One", None), ("Author Two", Some("OL222A"))],
        "an empty author key must not shift IDs between contributors"
    );
}
