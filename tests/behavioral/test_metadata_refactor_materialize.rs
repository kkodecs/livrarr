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
