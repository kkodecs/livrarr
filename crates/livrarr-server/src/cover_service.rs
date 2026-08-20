use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::WorkDb;
use livrarr_domain::identity_layer::{IdentityRepositoryError, WorkIdentityRepository, WorkRoute};
use livrarr_domain::services::CoverServiceError;
use livrarr_domain::{
    CoverCandidate, CoverCandidateSource, CoverMediaType, InternalCoverCandidate, MetadataProvider,
    UserId, Work, WorkId,
};
use livrarr_external_data::provider_client::ProviderClient;
use livrarr_http::fetcher::HttpFetcherImpl;
use livrarr_metadata::cover_write_gate::{
    run_user_cover_write, GateOutcome, UserCoverError, UserCoverInput, UserCoverPayload,
};

type HmacSha256 = Hmac<Sha256>;

pub struct LiveCoverService {
    db: SqliteDb,
    http_fetcher: HttpFetcherImpl,
    clients: HashMap<MetadataProvider, ProviderClient>,
    hmac_key: Vec<u8>,
    data_dir: Arc<PathBuf>,
}

impl LiveCoverService {
    pub fn new(
        db: SqliteDb,
        http_fetcher: HttpFetcherImpl,
        clients: HashMap<MetadataProvider, ProviderClient>,
        hmac_key: Vec<u8>,
        data_dir: Arc<PathBuf>,
    ) -> Self {
        Self {
            db,
            http_fetcher,
            clients,
            hmac_key,
            data_dir,
        }
    }

    async fn active_routes_for_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<WorkRoute>, CoverServiceError> {
        match self.db.read_captured_identity(user_id, work_id).await {
            Ok(identity) => Ok(identity.active_routes),
            // Presentation remains available when a related Author row is
            // missing; without a readable graph, provider clients use their
            // candidate-text path rather than a frozen scalar fallback.
            Err(IdentityRepositoryError::NotFound) => Ok(Vec::new()),
            Err(error) => Err(CoverServiceError::Internal(error.to_string())),
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
        "audible" => CoverCandidateSource::Provider(MetadataProvider::Audible),
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
        let routes = self.active_routes_for_cover(user_id, work_id).await?;

        let internals = livrarr_metadata::cover_alternatives::fetch_internal_alternatives(
            &work,
            &routes,
            &self.clients,
            &self.http_fetcher,
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

        // Resolve the cover URL based on source type
        let url = match &source {
            CoverCandidateSource::Provider(provider) => {
                let routes = self.active_routes_for_cover(user_id, work_id).await?;
                self.resolve_provider_url(*provider, &work, &routes)
                    .await?
                    .ok_or_else(|| {
                        CoverServiceError::Internal("provider returned no cover URL".into())
                    })?
            }
            CoverCandidateSource::IsbnOl => livrarr_metadata::cover::resolve_cover_english(
                &self.http_fetcher,
                work.isbn_13.as_deref(),
                livrarr_domain::RequestPriority::Normal,
            )
            .await
            .ok_or_else(|| CoverServiceError::Internal("ISBN cover not found".into()))?,
            CoverCandidateSource::IsbnAmazon => livrarr_metadata::cover::resolve_cover_foreign(
                &self.http_fetcher,
                work.isbn_13.as_deref(),
                livrarr_domain::RequestPriority::Normal,
            )
            .await
            .ok_or_else(|| CoverServiceError::Internal("ISBN cover not found".into()))?,
            CoverCandidateSource::Epub => {
                // EPUB extraction — TODO: wire CoverIoService when available
                return Err(CoverServiceError::Internal(
                    "EPUB cover selection not yet implemented".into(),
                ));
            }
        };

        let source_name = source_display_name(&source);

        let outcome = run_user_cover_write(
            &self.db,
            &self.http_fetcher,
            user_id,
            UserCoverInput {
                covers_dir,
                work_id,
                media_type,
                payload: UserCoverPayload::Url {
                    url,
                    source: source_name,
                },
            },
        )
        .await;

        map_user_cover_outcome(outcome)
    }

    async fn upload_cover(
        &self,
        user_id: UserId,
        work_id: WorkId,
        data: &[u8],
        media_type: CoverMediaType,
    ) -> Result<(), CoverServiceError> {
        let covers_dir = self.data_dir.join("covers").join(user_id.to_string());

        let outcome = run_user_cover_write(
            &self.db,
            &self.http_fetcher,
            user_id,
            UserCoverInput {
                covers_dir,
                work_id,
                media_type,
                payload: UserCoverPayload::Bytes {
                    data: data.to_vec(),
                },
            },
        )
        .await;

        map_user_cover_outcome(outcome)
    }
}

/// Both `select_cover` and `upload_cover` end the same way: offer the
/// resolved candidate to the write gate and translate its outcome. A user
/// write never legitimately produces `NoOp`/`AlreadyCurrent`/`Rejected` (the
/// gate skips those guards for this entry point) — that path is treated as
/// an internal error rather than silently swallowed.
fn map_user_cover_outcome(
    outcome: Result<GateOutcome, UserCoverError>,
) -> Result<(), CoverServiceError> {
    match outcome {
        Ok(GateOutcome::Accepted { .. }) => Ok(()),
        Ok(other) => Err(CoverServiceError::Internal(format!(
            "cover write gate: unexpected outcome for a user cover write: {other:?}"
        ))),
        Err(UserCoverError::Validation(m)) => Err(CoverServiceError::UploadValidation(m)),
    }
}

impl LiveCoverService {
    async fn resolve_provider_url(
        &self,
        provider: MetadataProvider,
        work: &Work,
        routes: &[WorkRoute],
    ) -> Result<Option<String>, CoverServiceError> {
        let client = self.clients.get(&provider).ok_or_else(|| {
            CoverServiceError::Internal(format!("no client for provider {provider:?}"))
        })?;

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            client.fetch_for_cover(work, routes, livrarr_domain::RequestPriority::Normal),
        )
        .await
        .map_err(|_| CoverServiceError::Internal("provider timeout".into()))?;

        match outcome {
            livrarr_external_data::ProviderOutcome::Success(detail) => Ok(detail.cover_url.clone()),
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
