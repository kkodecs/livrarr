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

/// Build a Google Books `volumes` body from (title, author, isbn13, language)
/// tuples — like `gb_volumes_body` but with an explicit per-volume `language`,
/// so the HARD language filter (#8) can be exercised at selection time.
fn gb_volumes_body_lang(works: &[(&str, &str, &str, &str)]) -> Vec<u8> {
    let items: Vec<serde_json::Value> = works
        .iter()
        .map(|(title, author, isbn, lang)| {
            serde_json::json!({
                "id": format!("gb-{isbn}"),
                "volumeInfo": {
                    "title": title,
                    "authors": [author],
                    "publishedDate": "1927",
                    "language": lang,
                    "industryIdentifiers": [{"type": "ISBN_13", "identifier": isbn}],
                }
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({ "items": items })).unwrap()
}

fn query_lang(id: usize, title: &str, author: &str, language: Option<&str>) -> EagerQuery {
    EagerQuery {
        id,
        title: title.into(),
        author: author.into(),
        language: language.map(|s| s.to_string()),
        isbn: None,
    }
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

// =============================================================================
// HARD language filter (#8): foreign file must not match the English edition
// =============================================================================

#[tokio::test]
async fn test_eager_match_german_file_picks_german_over_english() {
    // A German file, with both a German and an English same-title candidate in
    // the corpus, must pick the German one.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    set_gb_key(&db).await;

    let body = gb_volumes_body_lang(&[
        ("Der Steppenwolf", "Hermann Hesse", "9780000000001", "en"),
        ("Der Steppenwolf", "Hermann Hesse", "9780000000002", "de"),
    ]);
    let http = StubHttpFetcher::with_ok(200, body);
    let svc = WorkServiceImpl::without_enrichment(db, http, test_data_dir());

    let queries = vec![query_lang(
        0,
        "Der Steppenwolf",
        "Hermann Hesse",
        Some("de"),
    )];
    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    assert_eq!(matches.len(), 1);
    let (_, r) = &matches[0];
    assert_eq!(
        r.isbn_13.as_deref(),
        Some("9780000000002"),
        "German file must pick the German edition, not the English one"
    );
    assert_eq!(r.language.as_deref(), Some("de"));
}

#[tokio::test]
async fn test_eager_match_german_file_abstains_when_only_english() {
    // A German file with ONLY an English candidate must abstain (fall to manual
    // search), never silently take the English edition.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    set_gb_key(&db).await;

    let body = gb_volumes_body_lang(&[("Der Steppenwolf", "Hermann Hesse", "9780000000001", "en")]);
    let http = StubHttpFetcher::with_ok(200, body);
    let svc = WorkServiceImpl::without_enrichment(db, http, test_data_dir());

    let queries = vec![query_lang(
        0,
        "Der Steppenwolf",
        "Hermann Hesse",
        Some("de"),
    )];
    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    assert!(
        matches.is_empty(),
        "German file with only an English candidate must abstain"
    );
}

#[tokio::test]
async fn test_eager_match_unknown_language_still_matches() {
    // A file with unknown language (None) must NOT be language-filtered: it ranks
    // on title+author as before and matches the (English) candidate.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    set_gb_key(&db).await;

    let body = gb_volumes_body_lang(&[(
        "The Great Gatsby",
        "F. Scott Fitzgerald",
        "9780000000003",
        "en",
    )]);
    let http = StubHttpFetcher::with_ok(200, body);
    let svc = WorkServiceImpl::without_enrichment(db, http, test_data_dir());

    let queries = vec![query_lang(
        0,
        "The Great Gatsby",
        "F. Scott Fitzgerald",
        None,
    )];
    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    assert_eq!(
        matches.len(),
        1,
        "unknown-language file must still match on title+author"
    );
    assert_eq!(matches[0].1.isbn_13.as_deref(), Some("9780000000003"));
}

#[tokio::test]
async fn test_eager_match_anchor_graft_respects_language() {
    // A German file pins (by ISBN) an anchorless German Google Books volume. The
    // corpus also has a same-title English OpenLibrary doc carrying an anchor.
    // The anchor must NOT be grafted — it is the wrong-language work.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    set_gb_key(&db).await;

    // GB volume: German, anchorless, the ISBN pick. OL doc: English, same title +
    // author, carries an ol_key. (OL's LookupResult.language echoes the query
    // lang; the group lang here is "de", so the OL doc would be labeled "de" — to
    // make the English-anchor case real we instead rely on GB carrying the only
    // anchor-eligible English candidate.)
    let body = serde_json::to_vec(&serde_json::json!({
        "items": [
            {
                "id": "gb-de",
                "volumeInfo": {
                    "title": "Der Steppenwolf",
                    "authors": ["Hermann Hesse"],
                    "publishedDate": "1927",
                    "language": "de",
                    "industryIdentifiers": [{"type": "ISBN_13", "identifier": "9780000000010"}],
                }
            },
            {
                "id": "gb-en",
                "volumeInfo": {
                    "title": "Der Steppenwolf",
                    "authors": ["Hermann Hesse"],
                    "publishedDate": "1927",
                    "language": "en",
                    "industryIdentifiers": [{"type": "ISBN_13", "identifier": "9780000000011"}],
                }
            }
        ],
        "docs": [{
            "key": "/works/OL_EN",
            "title": "Der Steppenwolf",
            "author_name": ["Hermann Hesse"],
            "first_publish_year": 1927,
            "cover_i": 333,
        }],
    }))
    .unwrap();
    let http = StubHttpFetcher::with_ok(200, body);
    let svc = WorkServiceImpl::without_enrichment(db, http, test_data_dir());

    let queries = vec![EagerQuery {
        id: 0,
        title: "Der Steppenwolf".into(),
        author: "Hermann Hesse".into(),
        language: Some("de".into()),
        isbn: Some("9780000000010".into()),
    }];
    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    assert_eq!(matches.len(), 1);
    let (_, r) = &matches[0];
    // Picked the German edition by ISBN.
    assert_eq!(r.isbn_13.as_deref(), Some("9780000000010"));
    assert_eq!(r.language.as_deref(), Some("de"));
}

// =============================================================================
// Per-file 4-way fallback (#6): author-batch abstains, title+author finds it
// =============================================================================

#[tokio::test]
async fn test_eager_match_fallback_finds_title_when_author_batch_abstains() {
    // The author-scoped batch corpus does NOT contain the queried title (an
    // incomplete author facet), so the batch abstains. The per-file fallback then
    // runs the full 4-way `"<title> <author>"` discovery, whose corpus DOES
    // contain the title, and confidently matches it.
    //
    // Stub call sequence (GB unconfigured, Hardcover disabled by default):
    //   call #1  -> author-batch OpenLibrary query  (gets `batch_body`)
    //   call #2+ -> per-file fallback (OL + Goodreads)  (replays `fallback_body`)
    // The stub pops queued responses FIFO while >1 remain, then replays the last
    // one — so the batch consumes `batch_body` and every fallback call sees
    // `fallback_body`. Goodreads parses the OL-shaped body to empty (non-array),
    // leaving OpenLibrary to carry the fallback corpus.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    // Batch corpus: a DIFFERENT book by the same author — the queried title is
    // absent, so the batch cannot match it.
    let batch_body = ol_search_body(&[("OL_OTHER", "Some Other Book", "James S. A. Corey")]);
    // Fallback corpus: the queried title, found by the title+author search.
    let fallback_body = ol_search_body(&[("OL_LEV", "Leviathan Wakes", "James S. A. Corey")]);

    let http = StubHttpFetcher::new();
    http.push_response(Ok(livrarr_domain::services::FetchResponse {
        status: 200,
        headers: vec![],
        body: batch_body,
    }));
    http.push_response(Ok(livrarr_domain::services::FetchResponse {
        status: 200,
        headers: vec![],
        body: fallback_body,
    }));

    let svc = WorkServiceImpl::without_enrichment(db, http, test_data_dir());

    let queries = vec![query(0, "Leviathan Wakes", "James S. A. Corey")];
    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    assert_eq!(
        matches.len(),
        1,
        "fallback must confidently match a title the author batch missed"
    );
    let (id, r) = &matches[0];
    assert_eq!(*id, 0);
    assert_eq!(r.title, "Leviathan Wakes");
    assert_eq!(
        r.ol_key.as_deref(),
        Some("OL_LEV"),
        "fallback hit must carry the work anchor from the title+author corpus"
    );
}

#[tokio::test]
async fn test_eager_match_fallback_guard_holds_on_wrong_book() {
    // The author batch abstains AND the per-file fallback corpus contains only a
    // wrong-title book — the confident-match guard (best_candidate_index_lang)
    // must still abstain, never auto-picking a wrong book.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    let batch_body = ol_search_body(&[("OL_OTHER", "Some Other Book", "James S. A. Corey")]);
    // Fallback corpus has a same-author but title-mismatched book only.
    let fallback_body = ol_search_body(&[("OL_WRONG", "An Unrelated Title", "James S. A. Corey")]);

    let http = StubHttpFetcher::new();
    http.push_response(Ok(livrarr_domain::services::FetchResponse {
        status: 200,
        headers: vec![],
        body: batch_body,
    }));
    http.push_response(Ok(livrarr_domain::services::FetchResponse {
        status: 200,
        headers: vec![],
        body: fallback_body,
    }));

    let svc = WorkServiceImpl::without_enrichment(db, http, test_data_dir());

    let queries = vec![query(0, "Leviathan Wakes", "James S. A. Corey")];
    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    assert!(
        matches.is_empty(),
        "fallback must abstain when only a wrong-title candidate is found (no wrong-book pick)"
    );
}

#[tokio::test]
async fn test_eager_match_fallback_guard_holds_on_wrong_language() {
    // The author batch abstains; the fallback corpus contains the right title but
    // only in the wrong language. The HARD language guard must abstain rather than
    // take the wrong-language edition.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    set_gb_key(&db).await; // Google Books carries per-volume language for the guard

    // With a GB key, the author batch fires TWO providers (Google Books + Open-
    // Library), so both batch calls must see a non-matching corpus. Queue the
    // batch body twice, then the fallback body — the stub replays the last entry
    // for every subsequent (fallback) call. Google Books is the only provider
    // carrying true per-volume language, so the guard is exercised on GB results.
    let batch_body =
        gb_volumes_body_lang(&[("Ein Anderes Buch", "Hermann Hesse", "9780000000099", "de")]);
    // Fallback corpus: the queried title, but English-only.
    let fallback_body =
        gb_volumes_body_lang(&[("Der Steppenwolf", "Hermann Hesse", "9780000000098", "en")]);

    let http = StubHttpFetcher::new();
    http.push_response(Ok(livrarr_domain::services::FetchResponse {
        status: 200,
        headers: vec![],
        body: batch_body.clone(),
    }));
    http.push_response(Ok(livrarr_domain::services::FetchResponse {
        status: 200,
        headers: vec![],
        body: batch_body,
    }));
    http.push_response(Ok(livrarr_domain::services::FetchResponse {
        status: 200,
        headers: vec![],
        body: fallback_body,
    }));

    let svc = WorkServiceImpl::without_enrichment(db, http, test_data_dir());

    let queries = vec![query_lang(
        0,
        "Der Steppenwolf",
        "Hermann Hesse",
        Some("de"),
    )];
    let matches = svc.eager_match_by_author(user_id, queries).await.unwrap();

    assert!(
        matches.is_empty(),
        "German file must abstain when the fallback finds only an English edition"
    );
}
