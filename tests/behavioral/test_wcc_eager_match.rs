// tests/behavioral/test_wcc_eager_match.rs
#![allow(dead_code, unused_imports)]

//! Behavioral tests for WorkService::eager_match_by_author (#97).
//!
//! The eager pass is the cheap best-guess discovery for manual import: it
//! groups parsed files by author and issues ONE author-scoped query per
//! provider instead of one search per title, then locally matches each file's
//! title against the author's returned corpus. These tests lock the genuinely
//! new behavior — by-author batching (call count), author-scoped local match,
//! and omission of unmatched queries — not the provider JSON parsing (already
//! covered by the lookup_* tests).
//!
//! Google Books is left unconfigured (no API key) so it short-circuits before
//! any HTTP fetch; that makes the OpenLibrary call path deterministic and lets
//! `call_count()` directly measure how many author-scoped queries were issued.

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::*;
use livrarr_domain::services::*;
use livrarr_domain::UserRole;
use livrarr_metadata::work_service::WorkServiceImpl;

fn test_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("livrarr-test-eager-{}", std::process::id()))
}

async fn setup_user(db: &SqliteDb) -> i64 {
    db.create_user(CreateUserDbRequest {
        username: "eageruser".into(),
        password_hash: "hash".into(),
        role: UserRole::Admin,
        api_key_hash: "eagerhash".into(),
    })
    .await
    .unwrap()
    .id
}

/// Build an OpenLibrary `search.json`-shaped body from (ol_key, title, author)
/// triples — the exact shape `lookup_openlibrary` parses.
fn ol_search_body(works: &[(&str, &str, &str)]) -> Vec<u8> {
    let docs: Vec<serde_json::Value> = works
        .iter()
        .map(|(key, title, author)| {
            serde_json::json!({
                "key": format!("/works/{key}"),
                "title": title,
                "author_name": [author],
                "first_publish_year": 1965,
                "cover_i": 111,
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({ "docs": docs })).unwrap()
}

fn query(id: usize, title: &str, author: &str) -> EagerQuery {
    EagerQuery {
        id,
        title: title.into(),
        author: author.into(),
        language: Some("en".into()),
        isbn: None,
    }
}

/// Build a Google Books `volumes` body from (title, author, isbn13) triples —
/// the shape `lookup_google_books`/`fetch_gb_volumes` parses (GB is the only
/// provider that emits `isbn_13` on a LookupResult).
fn gb_volumes_body(works: &[(&str, &str, &str)]) -> Vec<u8> {
    let items: Vec<serde_json::Value> = works
        .iter()
        .map(|(title, author, isbn)| {
            serde_json::json!({
                "id": format!("gb-{isbn}"),
                "volumeInfo": {
                    "title": title,
                    "authors": [author],
                    "publishedDate": "1965",
                    "industryIdentifiers": [{"type": "ISBN_13", "identifier": isbn}],
                }
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({ "items": items })).unwrap()
}

/// Set a Google Books API key on the test DB so `lookup_google_books` actually
/// fetches (it short-circuits to empty without a key).
async fn set_gb_key(db: &SqliteDb) {
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
    .unwrap();
}

// =============================================================================
// Grouping + local match, single author (also proves GB-absent -> OL-only)
// =============================================================================

#[tokio::test]
async fn test_eager_match_groups_one_author_into_single_call() {
    // Three files by one author must issue ONE author-scoped OL query (not three
    // per-title searches), and each title must match its corpus entry.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let body = ol_search_body(&[
        ("OL1W", "Dune", "Frank Herbert"),
        ("OL2W", "Dune Messiah", "Frank Herbert"),
        ("OL3W", "Children of Dune", "Frank Herbert"),
    ]);
    let http = StubHttpFetcher::with_ok(200, body);
    let svc = WorkServiceImpl::without_enrichment(db, http.clone(), test_data_dir());

    let queries = vec![
        query(0, "Dune", "Frank Herbert"),
        query(1, "Dune Messiah", "Frank Herbert"),
        query(2, "Children of Dune", "Frank Herbert"),
    ];

    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    // One author => one OpenLibrary call (GB unconfigured => no GB fetch).
    assert_eq!(
        http.call_count(),
        1,
        "expected one author-scoped query, not one per title"
    );

    // All three titles matched their corpus entries.
    assert_eq!(matches.len(), 3);
    let by_id: std::collections::HashMap<usize, &LookupResult> =
        matches.iter().map(|(id, r)| (*id, r)).collect();
    assert_eq!(by_id[&0].ol_key.as_deref(), Some("OL1W"));
    assert_eq!(by_id[&1].ol_key.as_deref(), Some("OL2W"));
    assert_eq!(by_id[&2].ol_key.as_deref(), Some("OL3W"));

    // Eager pass is suggestion-only: no resolver, so no candidate_id.
    assert!(
        by_id[&0].candidate_id.is_none(),
        "eager match must not assign a candidate_id (identity locks at create)"
    );
}

// =============================================================================
// Two authors => two calls, author-scoped matching (no cross-author bleed)
// =============================================================================

#[tokio::test]
async fn test_eager_match_two_authors_two_calls_no_cross_bleed() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    // Single replayed corpus containing BOTH authors' works. The matcher must
    // disambiguate by author so each query picks its own author's work.
    let body = ol_search_body(&[
        ("OL_DUNE", "Dune", "Frank Herbert"),
        ("OL_FOUND", "Foundation", "Isaac Asimov"),
    ]);
    let http = StubHttpFetcher::with_ok(200, body);
    let svc = WorkServiceImpl::without_enrichment(db, http.clone(), test_data_dir());

    let queries = vec![
        query(0, "Dune", "Frank Herbert"),
        query(1, "Foundation", "Isaac Asimov"),
    ];

    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    // Two distinct authors => two author-scoped OL queries (not two per-title).
    assert_eq!(http.call_count(), 2, "expected one query per author");

    let by_id: std::collections::HashMap<usize, &LookupResult> =
        matches.iter().map(|(id, r)| (*id, r)).collect();
    assert_eq!(by_id[&0].ol_key.as_deref(), Some("OL_DUNE"));
    assert_eq!(by_id[&0].author_name, "Frank Herbert");
    assert_eq!(by_id[&1].ol_key.as_deref(), Some("OL_FOUND"));
    assert_eq!(by_id[&1].author_name, "Isaac Asimov");
}

// =============================================================================
// Unmatched titles are omitted (no fuzzy false positives)
// =============================================================================

#[tokio::test]
async fn test_eager_match_omits_unmatched_title() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    // Corpus has only "Dune"; the queried title is absent from it.
    let body = ol_search_body(&[("OL_DUNE", "Dune", "Frank Herbert")]);
    let http = StubHttpFetcher::with_ok(200, body);
    let svc = WorkServiceImpl::without_enrichment(db, http.clone(), test_data_dir());

    let queries = vec![query(0, "A Totally Different Book", "Frank Herbert")];

    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    assert!(
        matches.is_empty(),
        "an unmatched title must be omitted, not fuzzily matched to a wrong work"
    );
}

// =============================================================================
// Embedded ISBN pins the exact edition, beating a divergent parsed title
// =============================================================================

#[tokio::test]
async fn test_eager_match_isbn_beats_title() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    set_gb_key(&db).await; // so Google Books (the only isbn_13-bearing provider) fetches

    // Author corpus from Google Books; OpenLibrary also fires (key present) but
    // parses the GB-shaped body to nothing, so the corpus is the GB volumes.
    let body = gb_volumes_body(&[
        ("Dune", "Frank Herbert", "9780441013593"),
        ("Dune Messiah", "Frank Herbert", "9780593098233"),
    ]);
    let http = StubHttpFetcher::with_ok(200, body);
    let svc = WorkServiceImpl::without_enrichment(db, http.clone(), test_data_dir());

    // The parsed title is garbage, but the file carries Dune's ISBN.
    let queries = vec![EagerQuery {
        id: 0,
        title: "garbled scan title".into(),
        author: "Frank Herbert".into(),
        language: Some("en".into()),
        isbn: Some("9780441013593".into()),
    }];

    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    // ISBN pinned the exact edition despite the non-matching title: we got Dune,
    // not nothing (title would have matched neither) and not Dune Messiah.
    assert_eq!(matches.len(), 1);
    let (_, r) = &matches[0];
    assert_eq!(r.title, "Dune");
    assert_eq!(r.isbn_13.as_deref(), Some("9780441013593"));
}

// =============================================================================
// Anchor-graft must not borrow a different author's work anchor
// =============================================================================

#[tokio::test]
async fn test_eager_match_no_cross_author_anchor_graft() {
    // One body serves BOTH providers (with_ok repeats it): a Google Books volume
    // — the ISBN hit for the queried author, carrying NO work anchor — and an
    // OpenLibrary doc with the SAME title but a DIFFERENT author, carrying an
    // ol_key. GB reads `items`, OL reads `docs`, so the corpus is order-
    // independent. The anchorless GB pick must NOT graft the wrong author's anchor.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    set_gb_key(&db).await;

    let body = serde_json::to_vec(&serde_json::json!({
        "items": [{
            "id": "gb-dune",
            "volumeInfo": {
                "title": "Dune",
                "authors": ["Frank Herbert"],
                "publishedDate": "1965",
                "industryIdentifiers": [{"type": "ISBN_13", "identifier": "9780441013593"}],
            }
        }],
        "docs": [{
            "key": "/works/OL_WRONG",
            "title": "Dune",
            "author_name": ["Brian Herbert"],
            "first_publish_year": 1965,
            "cover_i": 222,
        }],
    }))
    .unwrap();
    let http = StubHttpFetcher::with_ok(200, body);
    let svc = WorkServiceImpl::without_enrichment(db, http, test_data_dir());

    // The file's ISBN pins the anchorless Google Books volume as the pick.
    let queries = vec![EagerQuery {
        id: 0,
        title: "Dune".into(),
        author: "Frank Herbert".into(),
        language: Some("en".into()),
        isbn: Some("9780441013593".into()),
    }];

    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    assert_eq!(matches.len(), 1);
    let (_, r) = &matches[0];
    assert_eq!(r.author_name, "Frank Herbert");
    assert_eq!(r.isbn_13.as_deref(), Some("9780441013593"));
    // The same-title OpenLibrary anchor belongs to Brian Herbert — it must NOT be
    // grafted onto Frank Herbert's work.
    assert!(
        r.ol_key.is_none(),
        "must not graft a different author's work anchor (OL_WRONG is Brian Herbert's)"
    );
}
