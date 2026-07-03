use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use livrarr_domain::services::{
    CoverSlotState, FetchError, FetchRequest, FetchResponse, HttpFetcher, MaterializeRequest,
    MaterializeService, MaterializeTags,
};
use livrarr_materialize::LiveMaterializeService;

#[derive(Default)]
struct SpyHttp {
    fetch_calls: AtomicUsize,
    safe_fetch_calls: AtomicUsize,
}

impl SpyHttp {
    fn safe_fetch_count(&self) -> usize {
        self.safe_fetch_calls.load(Ordering::SeqCst)
    }
}

impl HttpFetcher for SpyHttp {
    async fn fetch(&self, _req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.fetch_calls.fetch_add(1, Ordering::SeqCst);
        Ok(FetchResponse {
            status: 200,
            headers: vec![],
            body: b"cover-bytes".to_vec(),
        })
    }

    async fn fetch_ssrf_safe(&self, _req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.safe_fetch_calls.fetch_add(1, Ordering::SeqCst);
        Ok(FetchResponse {
            status: 200,
            headers: vec![],
            body: b"cover-bytes".to_vec(),
        })
    }
}

fn request(changed: bool) -> MaterializeRequest {
    MaterializeRequest {
        work_id: 42,
        changed,
        tag_fields_changed: changed,
        ebook_cover: CoverSlotState {
            chosen_new_url: Some("https://covers.example.test/ebook.jpg".to_string()),
            current_url: None,
            current_path: None,
            user_locked: false,
        },
        audiobook_cover: CoverSlotState {
            chosen_new_url: Some("https://covers.example.test/audio.jpg".to_string()),
            current_url: None,
            current_path: None,
            user_locked: false,
        },
        file_paths: vec![PathBuf::from("/tmp/livrarr-metadata-refactor.epub")],
        tags: MaterializeTags {
            title: "Contract Book".to_string(),
            author: "Contract Author".to_string(),
            ..Default::default()
        },
        covers_dir: std::env::temp_dir().join("livrarr-materialize-behavioral"),
    }
}

#[tokio::test]
async fn author_page_cached_cover_is_materialized_for_the_work() {
    // AC-002
    let http = Arc::new(SpyHttp::default());
    let service = LiveMaterializeService::new(http.clone());

    let outcome = service
        .materialize(request(true))
        .await
        .expect("changed materialization should succeed");

    assert!(
        outcome.ebook_cover_path.is_some(),
        "the ebook cover slot must be written when cached provider payloads carry a cover"
    );
    assert!(
        outcome.audiobook_cover_path.is_some(),
        "the audiobook cover slot must be written when cached provider payloads carry a cover"
    );
    assert!(
        http.safe_fetch_count() >= 2,
        "cover downloads must go through the SSRF-safe fetcher"
    );
}

/// The live pipeline stamps the chosen cover URL onto the work BEFORE the
/// materialize request is built, so on first acquisition chosen == current.
/// The URL-inequality gate alone would skip the only download the work ever
/// gets (every non-search door shipped cover-less until a restart); the gate
/// must also ask whether the bytes are actually on disk.
#[tokio::test]
async fn first_acquisition_downloads_when_chosen_equals_current_and_file_missing() {
    let http = Arc::new(SpyHttp::default());
    let service = LiveMaterializeService::new(http.clone());

    let covers_dir =
        std::env::temp_dir().join(format!("livrarr-mz-first-acq-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&covers_dir);

    let url = "https://covers.example.test/ebook.jpg".to_string();
    let mut req = request(true);
    req.covers_dir = covers_dir.clone();
    // Post-merge shape: the row already carries the chosen URL.
    req.ebook_cover.current_url = Some(url.clone());
    req.ebook_cover.chosen_new_url = Some(url);
    req.audiobook_cover.chosen_new_url = None;

    let outcome = service
        .materialize(req)
        .await
        .expect("first acquisition should succeed");

    assert_eq!(
        http.safe_fetch_count(),
        1,
        "chosen == current with NO file on disk is first acquisition, not a no-op"
    );
    assert!(outcome.ebook_cover_path.is_some());
    assert!(
        covers_dir.join("42.jpg").exists(),
        "the cover file must exist after first acquisition"
    );
}

/// The gate's original purpose stays intact: same URL with the bytes already
/// on disk is the true "nothing to do" — refresh never re-downloads.
#[tokio::test]
async fn url_equality_with_file_present_skips_redownload() {
    let http = Arc::new(SpyHttp::default());
    let service = LiveMaterializeService::new(http.clone());

    let covers_dir =
        std::env::temp_dir().join(format!("livrarr-mz-idempotent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&covers_dir);

    let url = "https://covers.example.test/ebook.jpg".to_string();
    let mut req = request(true);
    req.covers_dir = covers_dir.clone();
    req.ebook_cover.current_url = Some(url.clone());
    req.ebook_cover.chosen_new_url = Some(url);
    req.audiobook_cover.chosen_new_url = None;

    let first = service
        .materialize(req.clone())
        .await
        .expect("first pass downloads");
    assert_eq!(http.safe_fetch_count(), 1);
    assert!(first.ebook_cover_path.is_some());

    let second = service
        .materialize(req)
        .await
        .expect("second pass is a cover no-op");
    assert_eq!(
        http.safe_fetch_count(),
        1,
        "same URL with the file already on disk must not re-download"
    );
    assert_eq!(second.ebook_cover_path, None);
}

#[tokio::test]
async fn unchanged_materialize_request_skips_cover_downloads_and_tag_rewrites() {
    // AC-010
    let http = Arc::new(SpyHttp::default());
    let service = LiveMaterializeService::new(http.clone());

    let outcome = service
        .materialize(request(false))
        .await
        .expect("unchanged materialization should be a successful no-op");

    assert!(outcome.skipped_unchanged);
    assert_eq!(outcome.ebook_cover_path, None);
    assert_eq!(outcome.audiobook_cover_path, None);
    assert!(!outcome.tags_written);
    assert_eq!(
        http.safe_fetch_count(),
        0,
        "unchanged metadata must not download covers"
    );

    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(
        http.safe_fetch_count(),
        0,
        "unchanged metadata must not schedule delayed cover work"
    );
}
