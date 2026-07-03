//! livrarr-materialize — the single save step (REQ-012): cover download + tag
//! write, change-gated. DB-FREE leaf: depends only on domain + http + tagwrite.
//! Holds the relocated `download_cover_to_disk` (the cycle break, D-002). MUST
//! NOT depend on enrichment (failure isolation + one-home reuse).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use livrarr_domain::services::{
    FetchRequest, HttpFetcher, HttpMethod, MaterializeError, MaterializeOutcome,
    MaterializeRequest, MaterializeService, MaterializeTags, RateBucket, SavedCover,
    UserAgentProfile,
};
use livrarr_domain::RequestPriority;
use livrarr_tagwrite::{write_tags_batch, TagMetadata};

/// The on-disk path for a work's cover in a slot (`suffix` distinguishes slots:
/// "" = ebook, "_audiobook" = audiobook). Shared by the downloader and the
/// outcome so the reported path always matches the file written.
fn cover_file_path(covers_dir: &Path, work_id: i64, suffix: &str) -> PathBuf {
    covers_dir.join(format!("{work_id}{suffix}.jpg"))
}

/// The pacing bucket for a cover download URL: `OpenLibraryCovers` for
/// `covers.openlibrary.org` (case-insensitive), `None` for every other host —
/// including a URL that fails to parse (unpaced, matching prior behavior).
fn cover_bucket_for_url(url: &str) -> RateBucket {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(livrarr_domain::services::cover_bucket_for_host)
        })
        .unwrap_or(RateBucket::None)
}

/// Download a cover to disk via the SSRF-safe fetcher (runtime URL, insight 37),
/// writing atomically (tmp + rename) and returning the image bytes so the caller
/// can embed them in the file tags (R-002). Relocated from livrarr-metadata to
/// break the cover<->work_service cycle (D-002).
///
/// Decodes the image exactly ONCE (in spawn_blocking per insight 10): checks for
/// grayscale placeholders and extracts dimensions in the same pass. Returns
/// `(bytes, dims)` so callers never need a second decode.
pub async fn download_cover_to_disk<H: HttpFetcher>(
    http: &H,
    url: &str,
    covers_dir: &Path,
    work_id: i64,
    suffix: &str,
    priority: RequestPriority,
) -> Result<(Vec<u8>, Option<(i32, i32)>), Box<dyn std::error::Error + Send + Sync>> {
    tokio::fs::create_dir_all(covers_dir).await?;

    let req = FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(30),
        rate_bucket: cover_bucket_for_url(url),
        max_body_bytes: 10 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority,
    };

    let resp = http
        .fetch_ssrf_safe(req)
        .await
        .map_err(|e| format!("fetch: {e}"))?;
    if resp.status >= 400 {
        return Err(format!("cover download returned {}", resp.status).into());
    }

    let cover_path = cover_file_path(covers_dir, work_id, suffix);
    let tmp_path = cover_path.with_extension("jpg.tmp");
    let tmp_clone = tmp_path.clone();
    let target = cover_path.clone();
    let raw_bytes = resp.body;

    // Decode, validate, extract dims, and write — all in one spawn_blocking so
    // the CPU-heavy JPEG decode never blocks the async executor (insight 10).
    let result = tokio::task::spawn_blocking(move || {
        // Single decode: grayscale check + dims. Pass through if the bytes
        // aren't a recognisable image format (non-fatal per existing policy).
        let dims = match image::load_from_memory(&raw_bytes) {
            Ok(img) => {
                if matches!(
                    img.color(),
                    image::ColorType::L8
                        | image::ColorType::L16
                        | image::ColorType::La8
                        | image::ColorType::La16
                ) {
                    return Err("grayscale cover rejected (likely placeholder)".into());
                }
                Some((img.width() as i32, img.height() as i32))
            }
            Err(_) => None,
        };

        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_clone)?;
        f.write_all(&raw_bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp_clone, &target)?;
        Ok((raw_bytes, dims))
    })
    .await;

    match result {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(e)) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            Err(e)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            Err(format!("spawn error: {e}").into())
        }
    }
}

/// Convert the domain tag mirror to the tagwrite type at the crate boundary
/// (insight 9e: no tagwrite types in domain signatures). Fields are 1:1.
fn to_tag_metadata(tags: MaterializeTags) -> TagMetadata {
    TagMetadata {
        title: tags.title,
        subtitle: tags.subtitle,
        author: tags.author,
        narrator: tags.narrator,
        year: tags.year,
        genre: tags.genre,
        description: tags.description,
        publisher: tags.publisher,
        isbn: tags.isbn,
        language: tags.language,
        series_name: tags.series_name,
        series_position: tags.series_position,
    }
}

/// The live save service (REQ-012). Downloads covers via `fetch_ssrf_safe`
/// (runtime URLs, insight 37) and writes file tags via livrarr-tagwrite.
pub struct LiveMaterializeService<H> {
    http: Arc<H>,
}

impl<H> LiveMaterializeService<H> {
    pub fn new(http: Arc<H>) -> Self {
        Self { http }
    }
}

impl<H> MaterializeService for LiveMaterializeService<H>
where
    H: HttpFetcher + Send + Sync + 'static,
{
    async fn materialize(
        &self,
        request: MaterializeRequest,
    ) -> Result<MaterializeOutcome, MaterializeError> {
        // REQ-012/AC-010: a no-op when nothing changed — no downloads, no tag rewrite.
        if !request.changed {
            return Ok(MaterializeOutcome {
                skipped_unchanged: true,
                ..MaterializeOutcome::default()
            });
        }

        let mut outcome = MaterializeOutcome::default();
        let mut ebook_cover_bytes: Option<Vec<u8>> = None;

        // Ebook cover: download a NEW url, or a chosen url whose local file is
        // missing — never over a user lock (REQ-008), never blanking an
        // existing cover when no new url (REQ-006). The file check is what
        // makes first acquisition work on every door: the merge stamps the
        // chosen url onto the work BEFORE this request is built, so
        // chosen == current on the very first pass and url inequality alone
        // would skip the only download the work ever gets. URL equality with
        // the bytes already on disk is the true "nothing to do". The bytes
        // feed the tag write. A download failure propagates (the caller — the
        // pipeline — saves the work anyway per REQ-013).
        let ebook = &request.ebook_cover;
        if !ebook.user_locked {
            if let Some(url) = ebook.chosen_new_url.as_deref() {
                let file_present = tokio::fs::try_exists(cover_file_path(
                    &request.covers_dir,
                    request.work_id,
                    "",
                ))
                .await
                .unwrap_or(false);
                if ebook.current_url.as_deref() != Some(url) || !file_present {
                    let (bytes, dims) = download_cover_to_disk(
                        &*self.http,
                        url,
                        &request.covers_dir,
                        request.work_id,
                        "",
                        RequestPriority::Normal,
                    )
                    .await
                    .map_err(|e| MaterializeError::CoverDownload(e.to_string()))?;
                    let path = cover_file_path(&request.covers_dir, request.work_id, "");
                    outcome.ebook_cover_path = Some(path.to_string_lossy().into_owned());
                    // REQ-017: dims from the single decode in download_cover_to_disk.
                    outcome.saved_cover = dims.map(|(width, height)| SavedCover {
                        path,
                        width,
                        height,
                    });
                    ebook_cover_bytes = Some(bytes);
                }
            }
        }

        // Audiobook cover: same rules, separate slot. Identity-independent (REQ-015).
        let audiobook = &request.audiobook_cover;
        if !audiobook.user_locked {
            if let Some(url) = audiobook.chosen_new_url.as_deref() {
                let file_present = tokio::fs::try_exists(cover_file_path(
                    &request.covers_dir,
                    request.work_id,
                    "_audiobook",
                ))
                .await
                .unwrap_or(false);
                if audiobook.current_url.as_deref() != Some(url) || !file_present {
                    let (_, dims) = download_cover_to_disk(
                        &*self.http,
                        url,
                        &request.covers_dir,
                        request.work_id,
                        "_audiobook",
                        RequestPriority::Normal,
                    )
                    .await
                    .map_err(|e| MaterializeError::CoverDownload(e.to_string()))?;
                    let path = cover_file_path(&request.covers_dir, request.work_id, "_audiobook");
                    outcome.audiobook_cover_path = Some(path.to_string_lossy().into_owned());
                    outcome.saved_audiobook_cover = dims.map(|(width, height)| SavedCover {
                        path,
                        width,
                        height,
                    });
                }
            }
        }

        // Tag write: best-effort (REQ-013, spec §4). A missing/locked file or a
        // disabled audio format must NOT fail the save — the cover + DB metadata
        // are the load-bearing artifacts; `tags_written` reflects the outcome.
        if request.tag_fields_changed && !request.file_paths.is_empty() {
            let paths: Vec<String> = request
                .file_paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            if write_tags_batch(paths, to_tag_metadata(request.tags), ebook_cover_bytes)
                .await
                .is_ok()
            {
                outcome.tags_written = true;
            }
        }

        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use livrarr_domain::services::{FetchError, FetchResponse};
    use std::sync::Mutex;

    /// Records the `RateBucket` and `RequestPriority` of the last request it
    /// was handed and returns a canned 200 response. `download_cover_to_disk`
    /// tolerates a body that doesn't decode as an image (dims come back
    /// `None`, non-fatal per existing policy) — this fake never needs a real
    /// JPEG.
    struct RecordingFetcher {
        last_bucket: Mutex<Option<RateBucket>>,
        last_priority: Mutex<Option<RequestPriority>>,
    }

    impl RecordingFetcher {
        fn new() -> Self {
            Self {
                last_bucket: Mutex::new(None),
                last_priority: Mutex::new(None),
            }
        }
    }

    impl HttpFetcher for RecordingFetcher {
        async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
            self.fetch_ssrf_safe(req).await
        }

        async fn fetch_ssrf_safe(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
            *self.last_bucket.lock().unwrap() = Some(req.rate_bucket.clone());
            *self.last_priority.lock().unwrap() = Some(req.priority);
            Ok(FetchResponse {
                status: 200,
                headers: vec![],
                body: b"not-a-real-image".to_vec(),
            })
        }
    }

    #[tokio::test]
    async fn download_cover_to_disk_routes_ol_covers_host_and_passes_priority_through() {
        let fetcher = RecordingFetcher::new();
        let dir = tempfile::tempdir().expect("tempdir");

        download_cover_to_disk(
            &fetcher,
            "https://covers.openlibrary.org/b/isbn/9780306406157-L.jpg",
            dir.path(),
            1,
            "",
            RequestPriority::Interactive,
        )
        .await
        .expect("download should succeed against the canned 200 response");

        assert_eq!(
            *fetcher.last_bucket.lock().unwrap(),
            Some(RateBucket::OpenLibraryCovers)
        );
        assert_eq!(
            *fetcher.last_priority.lock().unwrap(),
            Some(RequestPriority::Interactive)
        );
    }

    #[tokio::test]
    async fn download_cover_to_disk_non_ol_host_uses_none_bucket() {
        let fetcher = RecordingFetcher::new();
        let dir = tempfile::tempdir().expect("tempdir");

        download_cover_to_disk(
            &fetcher,
            "https://i.gr-assets.com/books/12345/cover.jpg",
            dir.path(),
            2,
            "",
            RequestPriority::Normal,
        )
        .await
        .expect("download should succeed against the canned 200 response");

        assert_eq!(*fetcher.last_bucket.lock().unwrap(), Some(RateBucket::None));
        assert_eq!(
            *fetcher.last_priority.lock().unwrap(),
            Some(RequestPriority::Normal)
        );
    }

    /// B4: the cover backfill job (`livrarr-server/src/jobs/cover_backfill.rs`)
    /// passes `RequestPriority::Low` — a background one-shot pass over every
    /// work's cover, never ahead of a foreground door.
    #[tokio::test]
    async fn download_cover_to_disk_passes_backfill_low_priority_through() {
        let fetcher = RecordingFetcher::new();
        let dir = tempfile::tempdir().expect("tempdir");

        download_cover_to_disk(
            &fetcher,
            "https://i.gr-assets.com/books/12345/cover.jpg",
            dir.path(),
            3,
            "",
            RequestPriority::Low,
        )
        .await
        .expect("download should succeed against the canned 200 response");

        assert_eq!(
            *fetcher.last_priority.lock().unwrap(),
            Some(RequestPriority::Low)
        );
    }
}
