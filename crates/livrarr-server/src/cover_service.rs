use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::WorkDb;
use livrarr_domain::services::CoverServiceError;
use livrarr_domain::{
    CoverCandidate, CoverCandidateSource, CoverMediaType, CoverTrust, InternalCoverCandidate,
    MetadataProvider, UserId, Work, WorkId,
};
use livrarr_http::fetcher::HttpFetcherImpl;
use livrarr_http::HttpClient;
use livrarr_metadata::cover_resolution::measure_dimensions;
use livrarr_metadata::provider_client::ProviderClient;

type HmacSha256 = Hmac<Sha256>;

pub struct LiveCoverService {
    db: SqliteDb,
    http: HttpClient,
    http_fetcher: HttpFetcherImpl,
    clients: HashMap<MetadataProvider, ProviderClient>,
    hmac_key: Vec<u8>,
    data_dir: Arc<PathBuf>,
}

impl LiveCoverService {
    pub fn new(
        db: SqliteDb,
        http: HttpClient,
        http_fetcher: HttpFetcherImpl,
        clients: HashMap<MetadataProvider, ProviderClient>,
        hmac_key: Vec<u8>,
        data_dir: Arc<PathBuf>,
    ) -> Self {
        Self {
            db,
            http,
            http_fetcher,
            clients,
            hmac_key,
            data_dir,
        }
    }
}

fn sign_url(url: &str, key: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(url.as_bytes());
    let result = mac.finalize();
    data_encoding::HEXLOWER.encode(&result.into_bytes())
}

fn source_display_name(source: &CoverCandidateSource) -> String {
    match source {
        CoverCandidateSource::Provider(p) => format!("{p:?}").to_lowercase(),
        CoverCandidateSource::Epub => "epub".to_string(),
        CoverCandidateSource::IsbnOl => "isbn_ol".to_string(),
        CoverCandidateSource::IsbnAmazon => "isbn_amazon".to_string(),
    }
}

fn candidate_id(source: &CoverCandidateSource, media_type: CoverMediaType) -> String {
    let media = match media_type {
        CoverMediaType::Ebook => "ebook",
        CoverMediaType::Audiobook => "audiobook",
    };
    format!("{}:{media}", source_display_name(source))
}

fn to_cover_candidate(
    internal: &InternalCoverCandidate,
    work_id: WorkId,
    hmac_key: &[u8],
) -> CoverCandidate {
    let cid = candidate_id(&internal.source, internal.media_type);

    let proxy_url = match internal.source {
        CoverCandidateSource::Epub => {
            format!("/api/v1/work/{work_id}/cover/epub-preview")
        }
        _ => {
            let sig = sign_url(&internal.url, hmac_key);
            let encoded = urlencoding::encode(&internal.url);
            format!("/api/v1/coverproxy?url={encoded}&sig={sig}")
        }
    };

    CoverCandidate {
        candidate_id: cid,
        proxy_url,
        source: source_display_name(&internal.source),
        media_type: internal.media_type,
        width: 0,
        height: 0,
        passes_quality_gate: false,
    }
}

fn parse_candidate_id(
    cid: &str,
) -> Result<(CoverCandidateSource, CoverMediaType), CoverServiceError> {
    let (source_str, media_str) = cid
        .rsplit_once(':')
        .ok_or_else(|| CoverServiceError::InvalidCandidate("missing ':' separator".into()))?;

    let media_type = match media_str {
        "ebook" => CoverMediaType::Ebook,
        "audiobook" => CoverMediaType::Audiobook,
        _ => {
            return Err(CoverServiceError::InvalidCandidate(format!(
                "unknown media type: {media_str}"
            )))
        }
    };

    let source = match source_str {
        "hardcover" => CoverCandidateSource::Provider(MetadataProvider::Hardcover),
        "goodreads" => CoverCandidateSource::Provider(MetadataProvider::Goodreads),
        "openlibrary" | "open_library" => {
            CoverCandidateSource::Provider(MetadataProvider::OpenLibrary)
        }
        "audnexus" => CoverCandidateSource::Provider(MetadataProvider::Audnexus),
        "epub" => CoverCandidateSource::Epub,
        "isbn_ol" => CoverCandidateSource::IsbnOl,
        "isbn_amazon" => CoverCandidateSource::IsbnAmazon,
        _ => {
            return Err(CoverServiceError::InvalidCandidate(format!(
                "unknown source: {source_str}"
            )))
        }
    };

    Ok((source, media_type))
}

impl livrarr_domain::services::CoverService for LiveCoverService {
    async fn fetch_alternatives(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<CoverCandidate>, CoverServiceError> {
        let work = self
            .db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| CoverServiceError::Internal(e.to_string()))?;

        let internals = livrarr_metadata::cover_alternatives::fetch_internal_alternatives(
            &work,
            &self.clients,
            &self.http,
        )
        .await;

        let candidates: Vec<CoverCandidate> = internals
            .iter()
            .map(|ic| to_cover_candidate(ic, work_id, &self.hmac_key))
            .collect();

        Ok(candidates)
    }

    async fn select_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
        candidate_id: &str,
        media_type: CoverMediaType,
    ) -> Result<(), CoverServiceError> {
        let (source, embedded_media) = parse_candidate_id(candidate_id)?;

        if embedded_media != media_type {
            return Err(CoverServiceError::InvalidCandidate(
                "media_type mismatch between candidate_id and request".into(),
            ));
        }

        let work = self
            .db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| CoverServiceError::Internal(e.to_string()))?;

        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        let suffix = media_type.suffix();

        // Resolve the cover URL based on source type
        let url = match &source {
            CoverCandidateSource::Provider(provider) => self
                .resolve_provider_url(*provider, &work)
                .await?
                .ok_or_else(|| {
                    CoverServiceError::Internal("provider returned no cover URL".into())
                })?,
            CoverCandidateSource::IsbnOl => {
                livrarr_metadata::cover::resolve_cover_english(&self.http, work.isbn_13.as_deref())
                    .await
                    .ok_or_else(|| CoverServiceError::Internal("ISBN cover not found".into()))?
            }
            CoverCandidateSource::IsbnAmazon => {
                livrarr_metadata::cover::resolve_cover_foreign(&self.http, work.isbn_13.as_deref())
                    .await
                    .ok_or_else(|| CoverServiceError::Internal("ISBN cover not found".into()))?
            }
            CoverCandidateSource::Epub => {
                // EPUB extraction — TODO: wire CoverIoService when available
                return Err(CoverServiceError::Internal(
                    "EPUB cover selection not yet implemented".into(),
                ));
            }
        };

        // Download the cover
        livrarr_metadata::work_service::download_cover_to_disk(
            &self.http_fetcher,
            &url,
            &covers_dir,
            work_id,
            suffix,
        )
        .await
        .map_err(|e| CoverServiceError::Internal(e.to_string()))?;

        // Measure dimensions
        let cover_path = covers_dir.join(format!("{work_id}{suffix}.jpg"));
        let (w, h) = measure_dimensions(&cover_path).unwrap_or((0, 0));

        // Invalidate thumbnails
        let thumb = covers_dir.join(format!("{work_id}{suffix}_thumb.jpg"));
        let _ = tokio::fs::remove_file(&thumb).await;
        if media_type == CoverMediaType::Ebook && work.audiobook_cover_url.is_none() {
            let audio_thumb = covers_dir.join(format!("{work_id}_audio_thumb.jpg"));
            let _ = tokio::fs::remove_file(&audio_thumb).await;
        }

        // Update DB with User trust — branch on media type
        let source_name = source_display_name(&source);
        match media_type {
            CoverMediaType::Ebook => {
                self.db
                    .update_cover_metadata(
                        user_id,
                        work_id,
                        Some(&url),
                        &source_name,
                        CoverTrust::User,
                        w as i32,
                        h as i32,
                    )
                    .await
            }
            CoverMediaType::Audiobook => {
                self.db
                    .update_audiobook_cover_metadata(
                        user_id,
                        work_id,
                        Some(&url),
                        &source_name,
                        CoverTrust::User,
                        w as i32,
                        h as i32,
                    )
                    .await
            }
        }
        .map_err(|e| CoverServiceError::Internal(e.to_string()))?;

        Ok(())
    }

    async fn upload_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
        data: &[u8],
        media_type: CoverMediaType,
    ) -> Result<(), CoverServiceError> {
        const MAX_UPLOAD_SIZE: usize = 5 * 1024 * 1024;

        if data.len() > MAX_UPLOAD_SIZE {
            return Err(CoverServiceError::UploadValidation(format!(
                "file too large: {} bytes (max {})",
                data.len(),
                MAX_UPLOAD_SIZE
            )));
        }

        // Validate magic bytes
        if !(data.starts_with(&[0xFF, 0xD8]) // JPEG
            || data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) // PNG
            || (data.len() >= 12 && &data[8..12] == b"WEBP"))
        // WebP
        {
            return Err(CoverServiceError::UploadValidation(
                "unsupported format: must be JPEG, PNG, or WebP".into(),
            ));
        }

        // Decode + re-encode as JPEG in blocking thread
        let data_owned = data.to_vec();
        let jpeg_bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
            let img = image::load_from_memory(&data_owned)
                .map_err(|e| format!("image decode failed: {e}"))?;

            if img.width() > 8000 || img.height() > 8000 {
                return Err(format!(
                    "image too large: {}x{} (max 8000x8000)",
                    img.width(),
                    img.height()
                ));
            }

            let mut buf = std::io::Cursor::new(Vec::new());
            img.write_to(&mut buf, image::ImageFormat::Jpeg)
                .map_err(|e| format!("JPEG encode failed: {e}"))?;
            Ok(buf.into_inner())
        })
        .await
        .map_err(|e| CoverServiceError::Internal(e.to_string()))?
        .map_err(CoverServiceError::UploadValidation)?;

        // Atomic write to disk
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());
        tokio::fs::create_dir_all(&covers_dir)
            .await
            .map_err(|e| CoverServiceError::Internal(e.to_string()))?;

        let suffix = media_type.suffix();
        let cover_path = covers_dir.join(format!("{work_id}{suffix}.jpg"));
        let tmp_path = cover_path.with_extension("jpg.upload.tmp");

        let tmp_clone = tmp_path.clone();
        let bytes = jpeg_bytes;
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp_clone)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            Ok(())
        })
        .await
        .map_err(|e| CoverServiceError::Internal(e.to_string()))?
        .map_err(|e| CoverServiceError::Internal(e.to_string()))?;

        tokio::fs::rename(&tmp_path, &cover_path)
            .await
            .map_err(|e| CoverServiceError::Internal(e.to_string()))?;

        // Measure dimensions
        let (w, h) = measure_dimensions(&cover_path).unwrap_or((0, 0));

        // Invalidate thumbnails
        let thumb = covers_dir.join(format!("{work_id}{suffix}_thumb.jpg"));
        let _ = tokio::fs::remove_file(&thumb).await;

        let work = self
            .db
            .get_work(user_id, work_id)
            .await
            .map_err(|e| CoverServiceError::Internal(e.to_string()))?;
        if media_type == CoverMediaType::Ebook && work.audiobook_cover_url.is_none() {
            let audio_thumb = covers_dir.join(format!("{work_id}_audio_thumb.jpg"));
            let _ = tokio::fs::remove_file(&audio_thumb).await;
        }

        // Update DB — branch on media type
        match media_type {
            CoverMediaType::Ebook => {
                self.db
                    .update_cover_metadata(
                        user_id,
                        work_id,
                        None,
                        "user_upload",
                        CoverTrust::User,
                        w as i32,
                        h as i32,
                    )
                    .await
            }
            CoverMediaType::Audiobook => {
                self.db
                    .update_audiobook_cover_metadata(
                        user_id,
                        work_id,
                        None,
                        "user_upload",
                        CoverTrust::User,
                        w as i32,
                        h as i32,
                    )
                    .await
            }
        }
        .map_err(|e| CoverServiceError::Internal(e.to_string()))?;

        Ok(())
    }
}

impl LiveCoverService {
    async fn resolve_provider_url(
        &self,
        provider: MetadataProvider,
        work: &Work,
    ) -> Result<Option<String>, CoverServiceError> {
        let client = self.clients.get(&provider).ok_or_else(|| {
            CoverServiceError::Internal(format!("no client for provider {provider:?}"))
        })?;

        let ctx = livrarr_metadata::EnrichmentContext {
            priority: livrarr_domain::RequestPriority::Normal,
            mode: livrarr_metadata::EnrichmentMode::Manual,
        };

        let outcome =
            tokio::time::timeout(std::time::Duration::from_secs(10), client.fetch(work, &ctx))
                .await
                .map_err(|_| CoverServiceError::Internal("provider timeout".into()))?;

        match outcome {
            livrarr_metadata::ProviderOutcome::Success(detail) => Ok(detail.cover_url.clone()),
            _ => Ok(None),
        }
    }
}

pub fn generate_hmac_key() -> Vec<u8> {
    let mut key = vec![0u8; 32];
    getrandom::getrandom(&mut key).expect("getrandom failed");
    key
}

pub fn verify_hmac_signature(url: &str, sig: &str, key: &[u8]) -> bool {
    let expected = sign_url(url, key);
    subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), sig.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let key = b"test-key-32-bytes-long-enough!!!";
        let url = "https://example.test/cover.jpg";
        let sig = sign_url(url, key);
        assert!(verify_hmac_signature(url, &sig, key));
    }

    #[test]
    fn verify_rejects_wrong_sig() {
        let key = b"test-key-32-bytes-long-enough!!!";
        let url = "https://example.test/cover.jpg";
        assert!(!verify_hmac_signature(url, "wrong", key));
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let key1 = b"key-one-xxxxxxxxxxxxxxxxxxxxxxx";
        let key2 = b"key-two-xxxxxxxxxxxxxxxxxxxxxxx";
        let url = "https://example.test/cover.jpg";
        let sig = sign_url(url, key1);
        assert!(!verify_hmac_signature(url, &sig, key2));
    }

    #[test]
    fn candidate_id_roundtrip() {
        let cid = candidate_id(
            &CoverCandidateSource::Provider(MetadataProvider::Goodreads),
            CoverMediaType::Ebook,
        );
        assert_eq!(cid, "goodreads:ebook");
        let (source, media) = parse_candidate_id(&cid).unwrap();
        assert_eq!(
            source,
            CoverCandidateSource::Provider(MetadataProvider::Goodreads)
        );
        assert_eq!(media, CoverMediaType::Ebook);
    }

    #[test]
    fn candidate_id_epub_audiobook() {
        let cid = candidate_id(&CoverCandidateSource::Epub, CoverMediaType::Audiobook);
        assert_eq!(cid, "epub:audiobook");
    }

    #[test]
    fn candidate_id_isbn_sources() {
        let cid_ol = candidate_id(&CoverCandidateSource::IsbnOl, CoverMediaType::Ebook);
        assert_eq!(cid_ol, "isbn_ol:ebook");
        let cid_amz = candidate_id(&CoverCandidateSource::IsbnAmazon, CoverMediaType::Ebook);
        assert_eq!(cid_amz, "isbn_amazon:ebook");
    }

    #[test]
    fn parse_invalid_candidate_id() {
        assert!(parse_candidate_id("no-colon").is_err());
        assert!(parse_candidate_id("goodreads:unknown_media").is_err());
        assert!(parse_candidate_id("unknown_provider:ebook").is_err());
    }

    #[test]
    fn to_cover_candidate_uses_proxy_url() {
        let key = b"test-key-for-hmac-signing!!!!!!";
        let internal = InternalCoverCandidate {
            source: CoverCandidateSource::Provider(MetadataProvider::Goodreads),
            url: "https://raw-provider.test/cover.jpg".into(),
            media_type: CoverMediaType::Ebook,
            edition_title: None,
        };
        let candidate = to_cover_candidate(&internal, 42, key);
        assert_eq!(candidate.candidate_id, "goodreads:ebook");
        assert!(candidate.proxy_url.starts_with("/api/v1/coverproxy?url="));
        assert!(candidate.proxy_url.contains("&sig="));
        assert!(
            !candidate.proxy_url.contains("https://raw-provider.test"),
            "raw URL must not appear in proxy_url path"
        );
    }

    #[test]
    fn to_cover_candidate_epub_uses_preview_url() {
        let key = b"test-key-for-hmac-signing!!!!!!";
        let internal = InternalCoverCandidate {
            source: CoverCandidateSource::Epub,
            url: "internal://epub-cover".into(),
            media_type: CoverMediaType::Ebook,
            edition_title: None,
        };
        let candidate = to_cover_candidate(&internal, 99, key);
        assert_eq!(candidate.proxy_url, "/api/v1/work/99/cover/epub-preview");
    }
}
