//! livrarr-materialize — the single save step (REQ-012): cover download + tag
//! write, change-gated. DB-FREE leaf: depends only on domain + http + tagwrite.
//! Holds the relocated `download_cover_to_disk` (the cycle break, D-002). MUST
//! NOT depend on enrichment (failure isolation + one-home reuse).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use livrarr_domain::services::{
    FetchRequest, HttpFetcher, HttpMethod, MaterializeError, MaterializeOutcome,
    MaterializeRequest, MaterializeService, MaterializeTags, RateBucket, UserAgentProfile,
};
use livrarr_tagwrite::{write_tags_batch, TagMetadata};

/// The on-disk path for a work's cover in a slot (`suffix` distinguishes slots:
/// "" = ebook, "_audiobook" = audiobook). Shared by the downloader and the
/// outcome so the reported path always matches the file written.
fn cover_file_path(covers_dir: &Path, work_id: i64, suffix: &str) -> PathBuf {
    covers_dir.join(format!("{work_id}{suffix}.jpg"))
}

/// Download a cover to disk via the SSRF-safe fetcher (runtime URL, insight 37),
/// writing atomically (tmp + rename) and returning the image bytes so the caller
/// can embed them in the file tags (R-002). Relocated from livrarr-metadata to
/// break the cover<->work_service cycle (D-002).
pub async fn download_cover_to_disk<H: HttpFetcher>(
    http: &H,
    url: &str,
    covers_dir: &Path,
    work_id: i64,
    suffix: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    tokio::fs::create_dir_all(covers_dir).await?;

    let req = FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(30),
        rate_bucket: RateBucket::None,
        max_body_bytes: 10 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
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
    let bytes = resp.body;
    let bytes_for_write = bytes.clone();
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_clone)?;
        f.write_all(&bytes_for_write)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp_clone, &target)
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(bytes),
        Ok(Err(e)) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            Err(Box::new(e))
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

        // Ebook cover: download only a NEW url, never over a user lock (REQ-008),
        // never blanking an existing cover when no new url (REQ-006). The bytes
        // feed the tag write. A download failure propagates (the caller — the
        // pipeline — saves the work anyway per REQ-013).
        let ebook = &request.ebook_cover;
        if !ebook.user_locked {
            if let Some(url) = ebook.chosen_new_url.as_deref() {
                if ebook.current_url.as_deref() != Some(url) {
                    let bytes = download_cover_to_disk(
                        &*self.http,
                        url,
                        &request.covers_dir,
                        request.work_id,
                        "",
                    )
                    .await
                    .map_err(|e| MaterializeError::CoverDownload(e.to_string()))?;
                    ebook_cover_bytes = Some(bytes);
                    outcome.ebook_cover_path = Some(
                        cover_file_path(&request.covers_dir, request.work_id, "")
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }

        // Audiobook cover: same rules, separate slot. Identity-independent (REQ-015).
        let audiobook = &request.audiobook_cover;
        if !audiobook.user_locked {
            if let Some(url) = audiobook.chosen_new_url.as_deref() {
                if audiobook.current_url.as_deref() != Some(url) {
                    download_cover_to_disk(
                        &*self.http,
                        url,
                        &request.covers_dir,
                        request.work_id,
                        "_audiobook",
                    )
                    .await
                    .map_err(|e| MaterializeError::CoverDownload(e.to_string()))?;
                    outcome.audiobook_cover_path = Some(
                        cover_file_path(&request.covers_dir, request.work_id, "_audiobook")
                            .to_string_lossy()
                            .into_owned(),
                    );
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
