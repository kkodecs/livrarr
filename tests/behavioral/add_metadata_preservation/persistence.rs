use super::*;
use std::path::Path;

pub(super) async fn file_harness(
    directory: &Path,
    provider: MetadataProvider,
    available: bool,
) -> (RouteHarness, Requests, Arc<AtomicBool>) {
    let pool = livrarr_db::pool::create_sqlite_pool(directory)
        .await
        .expect("private file-backed SQLite pool");
    livrarr_db::pool::run_migrations(&pool)
        .await
        .expect("normal production migrations");
    let db = SqliteDb::new(pool);
    db.ensure_identity_authority_ready()
        .await
        .expect("activate normal identity authority");
    configure_discovery(&db).await;
    let api_key = "amp-save-tests-api-key".to_string();
    let existing: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE username='amp-save-tests'")
            .fetch_optional(db.pool())
            .await
            .unwrap();
    let user_id = match existing {
        Some(id) => id,
        None => {
            db.create_user(CreateUserDbRequest {
                username: "amp-save-tests".into(),
                password_hash: "unused-password-hash".into(),
                role: UserRole::Admin,
                api_key_hash: RealAuthCrypto.hash_token(&api_key).await.unwrap(),
            })
            .await
            .unwrap()
            .id
        }
    };
    let (transport, requests, gate) = captured_transport(provider, available);
    let harness = build_route_harness_from_parts(
        db,
        tempfile::tempdir().expect("router scratch owner"),
        directory.to_path_buf(),
        user_id,
        api_key,
        None,
        Vec::new(),
        Some(transport),
        None,
        true,
    )
    .await;
    (harness, requests, gate)
}

// Only mapped metadata, not cache/identity/status bookkeeping.
const MAPPED_FIELDS: &[(&str, &str, &str)] = &[
    ("language", "language", "language"),
    ("originalYear", "year", "year"),
    (
        "originalPublishDate",
        "originalPublishDate",
        "original_publish_date",
    ),
    ("editionPublishDate", "publishDate", "publish_date"),
    ("description", "description", "description"),
    ("publisher", "publisher", "publisher"),
    ("pageCount", "pageCount", "page_count"),
    ("seriesName", "seriesName", "series_name"),
    ("seriesPosition", "seriesPosition", "series_position"),
    ("rating", "rating", "rating"),
    ("ratingCount", "ratingCount", "rating_count"),
    ("genres", "genres", "genres"),
    ("coverUrl", "coverUrl", "cover_url"),
];

pub(super) fn assert_source(work: &Value, field: &str, source: Value, setter: &str) {
    let rows: Vec<_> = work["fieldSources"]
        .as_array()
        .expect("fieldSources must be readable")
        .iter()
        .filter(|row| row["field"] == field)
        .collect();
    assert_eq!(rows.len(), 1, "one source for {field}: {work}");
    assert_eq!(rows[0]["source"], source, "{field} source");
    assert_eq!(rows[0]["setter"], setter, "{field} ownership");
}

pub(super) fn assert_no_source(work: &Value, field: &str) {
    assert!(
        !work["fieldSources"]
            .as_array()
            .expect("fieldSources")
            .iter()
            .any(|row| row["field"] == field),
        "absent {field} must not acquire provenance: {work}"
    );
}

pub(super) fn assert_reference(work: &Value, provider: &str, kind: &str, value: &str) {
    assert!(
        work["sourceReferences"]
            .as_array()
            .expect("sourceReferences must be readable")
            .iter()
            .any(|row| row["provider"] == provider && row["kind"] == kind && row["value"] == value),
        "missing {provider}/{kind}/{value} reference: {work}"
    );
}

pub(super) fn assert_contributors(work: &Value, facts: &Value) {
    let provider = facts["provider"].as_str().unwrap();
    let author_kind = match provider {
        "open_library" => Some("open_library_author"),
        "hardcover" => Some("hardcover_author"),
        "goodreads" => Some("goodreads_author"),
        "google_books" => None,
        _ => panic!("unknown fixture provider"),
    };
    let rows = work["sourceReferences"]
        .as_array()
        .expect("sourceReferences");
    let mut expected = Vec::new();
    for (ordinal, contributor) in facts["contributors"].as_array().unwrap().iter().enumerate() {
        expected.push(json!(["contributor_name", contributor["name"], ordinal]));
        if let Some(id) = contributor
            .get("providerAuthorId")
            .filter(|id| !id.is_null())
        {
            expected.push(json!([
                author_kind.expect("this provider supplies an author namespace"),
                id,
                ordinal
            ]));
        }
    }
    let mut actual: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["provider"] == provider
                && (row["kind"] == "contributor_name"
                    || author_kind.is_some_and(|kind| row["kind"] == kind))
        })
        .map(|row| json!([row["kind"], row["value"], row["ordinal"]]))
        .collect();
    expected.sort_by_key(Value::to_string);
    actual.sort_by_key(Value::to_string);
    assert_eq!(
        actual, expected,
        "names and author ids stay associated, including repeated names"
    );
}

pub(super) fn assert_saved_view(work: &Value, facts: &Value) {
    assert_eq!(work["title"], "Dune");
    assert_eq!(work["authorName"], "Frank Herbert");
    assert!(
        work["subtitle"].is_null(),
        "capture subtitle must not become identity"
    );
    assert_eq!(work["descriptionTruncated"], facts["descriptionTruncated"]);
    for &(fact, field, provenance) in MAPPED_FIELDS {
        let expected = &facts[fact];
        let actual = work
            .get(field)
            .unwrap_or_else(|| panic!("missing read field {field}: {work}"));
        let empty_genres = fact == "genres" && expected.as_array().is_some_and(Vec::is_empty);
        if empty_genres {
            assert!(
                actual.is_null() || actual == &json!([]),
                "no invented genres"
            );
        } else {
            assert_eq!(actual, expected, "saved {field}");
        }
        if expected.is_null() || empty_genres {
            assert_no_source(work, provenance);
        } else {
            assert_source(work, provenance, facts["provider"].clone(), "provider");
        }
    }
    assert_contributors(work, facts);
    let provider = facts["provider"].as_str().unwrap();
    let (response_field, reference_kind, primary_id) = match provider {
        "google_books" => ("isbn13", "isbn_13", "9780441013593"),
        "open_library" => ("olKey", "open_library_work", "OL893414W"),
        "hardcover" => ("hcKey", "hardcover_work", "312460"),
        "goodreads" => ("grKey", "goodreads_book", "44767458"),
        _ => panic!("unknown captured provider"),
    };
    let rows = work["sourceReferences"]
        .as_array()
        .expect("sourceReferences");
    let matches_primary_id = |value: &Value| {
        value.as_str().is_some_and(|value| {
            let normalized = if provider == "open_library" {
                value.strip_prefix("/works/").unwrap_or(value)
            } else {
                value
            };
            normalized == primary_id
        })
    };
    // A source fact can remain readable without populating a legacy identity
    // scalar. A present scalar must still be correct; a reference cannot hide it.
    if let Some(value) = work.get(response_field).filter(|value| !value.is_null()) {
        assert!(
            matches_primary_id(value),
            "incorrect {response_field}: expected {primary_id}, got {value}"
        );
    } else {
        assert!(
            rows.iter().any(|row| row["provider"] == provider
                && row["kind"] == reference_kind
                && matches_primary_id(&row["value"])),
            "missing readable {provider}/{reference_kind}/{primary_id} in {response_field} or sourceReferences"
        );
    }
    let mut actual: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["provider"] == provider
                && matches!(
                    row["kind"].as_str(),
                    Some("google_volume" | "isbn_10" | "isbn_13" | "amazon" | "goodreads_work")
                )
        })
        .map(|row| json!({"kind":row["kind"], "value":row["value"]}))
        .collect();
    let mut expected = facts["references"].as_array().unwrap().clone();
    expected.sort_by_key(Value::to_string);
    actual.sort_by_key(Value::to_string);
    assert_eq!(
        actual, expected,
        "all typed references survive without family/order conflation"
    );
    for (fact, kind) in [
        ("subtitle", "subtitle"),
        ("bareTitle", "bare_title"),
        ("decoratedTitle", "decorated_title"),
        ("coverUrl", "cover_url"),
    ] {
        if let Some(value) = facts[fact].as_str() {
            assert_reference(work, provider, kind, value);
        } else {
            assert!(
                !rows
                    .iter()
                    .any(|row| row["provider"] == provider && row["kind"] == kind),
                "missing {kind} stays missing"
            );
        }
    }
}

pub(super) async fn assert_no_image(h: &RouteHarness, id: i64, work: &Value) {
    assert!(
        work["coverSource"].is_null(),
        "a URL is not saved image provenance"
    );
    assert_eq!(work["coverManual"], false);
    assert_eq!(work["coverWidth"], 0);
    assert_eq!(work["coverHeight"], 0);
    assert!(work.get("coverMtime").is_none_or(Value::is_null));
    let row: (Option<String>, bool, i32, i32) = sqlx::query_as(
        "SELECT cover_source,cover_manual,cover_width,cover_height FROM works WHERE user_id=?1 AND id=?2",
    ).bind(h.user_id).bind(id).fetch_one(h.db.pool()).await.unwrap();
    assert_eq!(row, (None, false, 0, 0));
    assert!(!h
        .state
        .data_dir
        .join("covers")
        .join(h.user_id.to_string())
        .join(format!("{id}.jpg"))
        .exists());
}

pub(super) async fn assert_primary_author_only(h: &RouteHarness, id: i64) {
    let authors: i64 = sqlx::query_scalar("SELECT count(*) FROM authors WHERE user_id=?1")
        .bind(h.user_id)
        .fetch_one(h.db.pool())
        .await
        .unwrap();
    let contributors: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM work_contributors WHERE user_id=?1 AND work_id=?2",
    )
    .bind(h.user_id)
    .bind(id)
    .fetch_one(h.db.pool())
    .await
    .unwrap();
    assert_eq!(
        authors, 1,
        "source contributor references do not create extra Authors"
    );
    assert_eq!(
        contributors, 1,
        "identity authorship stays with the primary Author"
    );
}
