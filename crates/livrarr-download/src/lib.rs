use livrarr_domain::{
    DbError, DownloadClient, DownloadClientId, Grab, GrabId, GrabStatus, QueueStatus,
    RemotePathMapping, UserId, WorkId,
};
use serde::{Deserialize, Serialize};

// Re-export ProwlarrConfig from livrarr-db for use in trait signatures.
pub use livrarr_db::ProwlarrConfig;

// =============================================================================
// CRATE: livrarr-download
// =============================================================================
// Prowlarr search + qBit client.

// ---------------------------------------------------------------------------
// Prowlarr Client
// ---------------------------------------------------------------------------

/// Prowlarr Torznab search.
#[trait_variant::make(Send)]
pub trait ProwlarrClient: Send + Sync {
    /// Search Prowlarr for releases. Categories 7020 (ebooks) + 3030 (audiobooks).
    async fn search_releases(
        &self,
        query: &str,
        config: &ProwlarrConfig,
    ) -> Result<Vec<ProwlarrRelease>, DownloadError>;

    /// Test Prowlarr connection.
    async fn test_connection(&self, config: &ProwlarrConfig) -> Result<(), DownloadError>;
}

/// Release from Prowlarr (pass-through, not persisted).
#[derive(Debug, Clone)]
pub struct ProwlarrRelease {
    pub title: String,
    pub indexer: String,
    pub size: i64,
    pub guid: String,
    pub download_url: String,
    pub seeders: Option<i32>,
    pub leechers: Option<i32>,
    pub publish_date: Option<String>,
    pub categories: Vec<i32>,
}

// ---------------------------------------------------------------------------
// qBittorrent Client
// ---------------------------------------------------------------------------

/// qBittorrent API v2 client.
#[trait_variant::make(Send)]
pub trait QBitClient: Send + Sync {
    /// Authenticate to qBit. Caches session cookie.
    async fn authenticate(&self, config: &DownloadClient) -> Result<(), DownloadError>;

    /// Add torrent via magnet URL.
    async fn add_torrent_magnet(
        &self,
        config: &DownloadClient,
        magnet: &str,
        category: &str,
    ) -> Result<(), DownloadError>;

    /// Add torrent via .torrent file upload (multipart).
    async fn add_torrent_file(
        &self,
        config: &DownloadClient,
        filename: &str,
        data: &[u8],
        category: &str,
    ) -> Result<(), DownloadError>;

    /// List torrents in a category.
    async fn list_torrents(
        &self,
        config: &DownloadClient,
        category: &str,
    ) -> Result<Vec<QBitTorrent>, DownloadError>;

    /// Get a specific torrent by hash.
    async fn get_torrent(
        &self,
        config: &DownloadClient,
        hash: &str,
    ) -> Result<Option<QBitTorrent>, DownloadError>;

    /// Test connection: auth + API version + category check + torrent list access.
    async fn test_connection(&self, config: &DownloadClient) -> Result<(), DownloadError>;
}

/// Torrent info from qBit API.
#[derive(Debug, Clone, Default)]
pub struct QBitTorrent {
    pub hash: String,
    pub name: String,
    pub state: String,
    pub size: i64,
    pub downloaded: i64,
    pub progress: f64,
    pub eta: Option<i64>,
    pub content_path: String,
    pub category: String,
}

// ---------------------------------------------------------------------------
// Download Service (orchestrator)
// ---------------------------------------------------------------------------

/// Download operations -- grab, queue, release search.
#[trait_variant::make(Send)]
pub trait DownloadService: Send + Sync {
    /// Search for releases via Prowlarr.
    async fn search_releases(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<ReleaseSearchResult>, DownloadError>;

    /// Grab a release. Validates, selects client, adds torrent, polls for confirmation.
    async fn grab(&self, user_id: UserId, req: GrabRequest) -> Result<GrabResult, DownloadError>;

    /// Get live queue from all enabled qBit clients.
    async fn get_queue(&self, user_id: UserId) -> Result<QueueResponse, DownloadError>;

    /// Remove grab from Livrarr tracking. Torrent stays in qBit.
    async fn remove_from_queue(
        &self,
        user_id: UserId,
        grab_id: GrabId,
    ) -> Result<(), DownloadError>;
}

#[derive(Debug)]
pub struct GrabRequest {
    pub work_id: WorkId,
    pub download_url: String,
    pub title: String,
    pub indexer: String,
    pub guid: String,
    pub size: Option<i64>,
    pub download_client_id: Option<DownloadClientId>,
    pub source: TorrentSource,
}

impl Default for GrabRequest {
    fn default() -> Self {
        Self {
            work_id: WorkId::default(),
            download_url: String::new(),
            title: String::new(),
            indexer: String::new(),
            guid: String::new(),
            size: None,
            download_client_id: None,
            source: TorrentSource::Magnet(String::new()),
        }
    }
}

pub struct GrabResult {
    pub grab: Grab,
    pub status: GrabStatus,
    pub warning: Option<String>,
}

impl Default for GrabResult {
    fn default() -> Self {
        Self {
            grab: Grab {
                id: 0,
                user_id: 0,
                work_id: 0,
                download_client_id: 0,
                title: String::new(),
                indexer: String::new(),
                guid: String::new(),
                size: None,
                download_url: String::new(),
                download_id: None,
                status: GrabStatus::Sent,
                import_error: None,
                media_type: None,
                grabbed_at: chrono::Utc::now(),
            },
            status: GrabStatus::Sent,
            warning: None,
        }
    }
}

#[derive(Default)]
pub struct ReleaseSearchResult {
    pub title: String,
    pub indexer: String,
    pub size: i64,
    pub guid: String,
    pub download_url: String,
    pub seeders: Option<i32>,
    pub leechers: Option<i32>,
    pub publish_date: Option<String>,
    pub categories: Vec<i32>,
}

pub struct QueueResponse {
    pub items: Vec<QBitTorrent>,
    pub warnings: Vec<String>,
}

/// Queue item -- joined from qBit torrent + Livrarr grab.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub id: GrabId,
    pub download_id: String,
    pub title: String,
    pub status: QueueStatus,
    pub size: i64,
    pub sizeleft: i64,
    pub eta: Option<i64>,
    pub indexer: String,
    pub download_client: String,
    pub work_id: WorkId,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItemResponse {
    pub id: GrabId,
    pub download_id: String,
    pub title: String,
    pub status: QueueStatus,
    pub size: i64,
    pub sizeleft: i64,
    pub eta: Option<i64>,
    pub indexer: String,
    pub download_client: String,
    pub work_id: WorkId,
}

// ---------------------------------------------------------------------------
// Torrent Source / Hash Extraction
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum TorrentSource {
    /// Magnet URI.
    Magnet(String),
    /// Download URL for .torrent file.
    Url(String),
    /// .torrent file with filename and raw bytes.
    TorrentFile { filename: String, data: Vec<u8> },
}

/// Extract torrent hash from magnet link or .torrent file.
///
/// Satisfies: DLC-007
pub fn extract_torrent_hash(source: &TorrentSource) -> Result<String, DownloadError> {
    match source {
        TorrentSource::Magnet(uri) => extract_hash_from_magnet(uri),
        TorrentSource::Url(_) => {
            // URL sources need to be fetched first, then parsed as .torrent
            // For hash extraction, we need the data — this shouldn't be called directly on URLs
            // without first fetching. Return a placeholder that the service layer handles.
            Err(DownloadError::InvalidMagnet {
                reason: "cannot extract hash from URL without fetching".to_string(),
            })
        }
        TorrentSource::TorrentFile { data, .. } => extract_hash_from_torrent_file(data),
    }
}

/// Simple percent-decode for magnet URI components.
fn percent_decode(input: &str) -> String {
    let mut result = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                result.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn extract_hash_from_magnet(uri: &str) -> Result<String, DownloadError> {
    // Parse xt= parameters
    let xt = uri
        .split('&')
        .chain(uri.split('?').skip(1).take(1))
        .flat_map(|s| s.split('&'))
        .find(|p| p.starts_with("xt="))
        .ok_or_else(|| DownloadError::InvalidMagnet {
            reason: "no xt= parameter found".to_string(),
        })?;

    let xt_raw = &xt[3..]; // skip "xt="
                           // URL-decode the xt value — magnet URIs may percent-encode colons and other chars.
    let xt_value = percent_decode(xt_raw);

    if let Some(hash) = xt_value.strip_prefix("urn:btih:") {
        // BitTorrent v1: SHA-1
        let trimmed = hash.trim_end_matches('=');
        if trimmed.len() == 40 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(trimmed.to_lowercase())
        } else {
            // Base32 encoded — decode to bytes then hex
            let upper = hash.trim_end_matches('=').to_uppercase();
            let mut decoded = data_encoding::BASE32_NOPAD
                .decode(upper.as_bytes())
                .map_err(|e| DownloadError::InvalidMagnet {
                    reason: format!("invalid base32: {e}"),
                })?;
            // SHA-1 is always 20 bytes; zero-pad if input was short
            while decoded.len() < 20 {
                decoded.push(0);
            }
            decoded.truncate(20);
            Ok(data_encoding::HEXLOWER.encode(&decoded))
        }
    } else if let Some(hash) = xt_value.strip_prefix("urn:btmh:") {
        // BitTorrent v2: SHA-256 with multihash prefix "1220"
        // (varint function code 0x12 = SHA-256, varint digest length 0x20 = 32 bytes)
        let stripped = hash
            .strip_prefix("1220")
            .ok_or_else(|| DownloadError::InvalidMagnet {
                reason: "unrecognized multihash prefix in btmh hash".to_string(),
            })?;
        Ok(stripped.to_lowercase())
    } else {
        Err(DownloadError::InvalidMagnet {
            reason: "no btih or btmh hash in xt parameter".to_string(),
        })
    }
}

fn extract_hash_from_torrent_file(data: &[u8]) -> Result<String, DownloadError> {
    // Verify the top-level bencode object is well-formed and there's no trailing data.
    {
        let mut top_decoder = bendy::decoding::Decoder::new(data);
        let top_obj = top_decoder
            .next_object()
            .map_err(|e| DownloadError::InvalidTorrentFile {
                reason: format!("malformed torrent file: {e}"),
            })?
            .ok_or_else(|| DownloadError::InvalidTorrentFile {
                reason: "empty torrent file".to_string(),
            })?;
        // Consume the object fully so the decoder advances past it.
        let _raw = top_obj
            .try_into_dictionary()
            .map_err(|e| DownloadError::InvalidTorrentFile {
                reason: format!("torrent root is not a dictionary: {e}"),
            })?
            .into_raw()
            .map_err(|e| DownloadError::InvalidTorrentFile {
                reason: format!("failed to read torrent root: {e}"),
            })?;
        // After consuming the top-level object, the decoder should be at EOF.
        // Propagate decoder errors — malformed trailing bytes are invalid, not ignorable.
        let trailing = top_decoder.next_object().map(|o| o.is_some());
        match trailing {
            Ok(false) => {} // EOF — correct
            Ok(true) => {
                return Err(DownloadError::InvalidTorrentFile {
                    reason: "trailing data after root bencode object".to_string(),
                });
            }
            Err(e) => {
                return Err(DownloadError::InvalidTorrentFile {
                    reason: format!("malformed trailing data: {e}"),
                });
            }
        }
    }

    // Structurally parse the top-level dictionary to find the "info" key and extract
    // its raw bencode bytes. This avoids the naive byte-pattern search for "4:infod"
    // which can match inside nested values.
    let info_bytes = find_info_dict_bytes(data)?;

    use sha1::Digest;
    let hash = sha1::Sha1::digest(info_bytes);
    Ok(data_encoding::HEXLOWER.encode(&hash))
}

/// Structurally walk the top-level bencode dictionary to extract the raw bytes of
/// the `info` value. Uses `bendy` so we only match the actual top-level key, not
/// a substring buried inside a nested value.
fn find_info_dict_bytes(data: &[u8]) -> Result<&[u8], DownloadError> {
    use bendy::decoding::{Decoder, Object};

    let mut decoder = Decoder::new(data);
    let top = decoder
        .next_object()
        .map_err(|e| DownloadError::InvalidTorrentFile {
            reason: format!("malformed torrent file: {e}"),
        })?
        .ok_or_else(|| DownloadError::InvalidTorrentFile {
            reason: "empty torrent file".to_string(),
        })?;

    let mut dict = top
        .try_into_dictionary()
        .map_err(|e| DownloadError::InvalidTorrentFile {
            reason: format!("torrent root is not a dictionary: {e}"),
        })?;

    while let Some(pair) = dict
        .next_pair()
        .map_err(|e| DownloadError::InvalidTorrentFile {
            reason: format!("error reading torrent dictionary: {e}"),
        })?
    {
        let (key, value) = pair;
        if key == b"info" {
            // `value` is an Object — convert to raw bytes to get the exact bencode span.
            match value {
                Object::Dict(d) => {
                    return d.into_raw().map_err(|e| DownloadError::InvalidTorrentFile {
                        reason: format!("failed to read raw info bytes: {e}"),
                    });
                }
                _ => {
                    return Err(DownloadError::InvalidTorrentFile {
                        reason: "info key is not a dictionary".to_string(),
                    });
                }
            }
        }
    }

    Err(DownloadError::InvalidTorrentFile {
        reason: "no info dictionary found".to_string(),
    })
}

/// Resolve download client paths to local paths.
///
/// Satisfies: DLC-013
pub fn resolve_remote_path(
    path: &str,
    client_host: &str,
    mappings: &[RemotePathMapping],
) -> String {
    let host_lower = client_host.to_lowercase();
    let mut best_match: Option<&RemotePathMapping> = None;
    let mut best_len = 0;

    for m in mappings {
        if m.host.to_lowercase() != host_lower {
            continue;
        }
        if path.starts_with(&m.remote_path) && m.remote_path.len() > best_len {
            // Verify the match is at a path boundary: the path must be exactly
            // the remote_path, or the next character must be '/'.
            // This prevents /data matching /database.
            let remainder = &path[m.remote_path.len()..];
            if !remainder.is_empty() && !remainder.starts_with('/') {
                continue;
            }
            best_match = Some(m);
            best_len = m.remote_path.len();
        }
    }

    match best_match {
        Some(m) => {
            let suffix = &path[m.remote_path.len()..];
            format!("{}{}", m.local_path, suffix)
        }
        None => path.to_string(),
    }
}

/// Map qBit state string to QueueStatus.
///
/// Satisfies: DLC-011
pub fn map_qbit_state(state: &str) -> QueueStatus {
    match state {
        "downloading" | "stalledDL" | "forcedDL" => QueueStatus::Downloading,
        "metaDL" | "allocating" | "queuedDL" | "checkingDL" | "checkingResumeData" => {
            QueueStatus::Queued
        }
        "pausedDL" => QueueStatus::Paused,
        "pausedUP" | "uploading" | "stalledUP" | "forcedUP" | "queuedUP" | "checkingUP" => {
            QueueStatus::Completed
        }
        "missingFiles" | "moving" | "unknown" => QueueStatus::Warning,
        "error" => QueueStatus::Error,
        _ => QueueStatus::Warning,
    }
}

/// Normalize eta value from qBit (handle sentinel values).
///
/// Satisfies: DLC-011
pub fn normalize_eta(eta: Option<i64>) -> Option<i64> {
    match eta {
        Some(v) if v < 0 => None,
        Some(v) if v >= 8640000 => None,
        Some(v) if v > 365 * 86400 => None,
        other => other,
    }
}

// ---------------------------------------------------------------------------
// DownloadError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("no download client configured")]
    NoClient,
    #[error("no enabled download client")]
    NoEnabledClient,
    #[error("download client connection failed: {0}")]
    ConnectionFailed(String),
    #[error("download client auth failed")]
    AuthFailed,
    #[error("download client rejected torrent: {reason}")]
    Rejected { reason: String },
    #[error("SSL certificate validation failed")]
    SslValidationFailed,
    #[error("qBittorrent API version unsupported")]
    ApiVersionUnsupported,
    #[error("category access/creation failed: {0}")]
    CategoryFailed(String),
    #[error("prowlarr not configured")]
    ProwlarrNotConfigured,
    #[error("prowlarr unreachable: {0}")]
    ProwlarrUnreachable(String),
    #[error("duplicate grab")]
    Duplicate,
    #[error("invalid download URL")]
    InvalidUrl,
    #[error("invalid magnet link: {reason}")]
    InvalidMagnet { reason: String },
    #[error("invalid .torrent file: {reason}")]
    InvalidTorrentFile { reason: String },
    #[error(".torrent fetch failed: {0}")]
    TorrentFetchFailed(String),
    #[error("torrent not found in client after initial polling")]
    Unconfirmed,
    #[error("torrent missing from client after prior confirmation")]
    MissingAfterConfirmation,
    #[error("work not found")]
    WorkNotFound,
    #[error("grab not found")]
    GrabNotFound,
    #[error("database error: {0}")]
    Db(#[from] DbError),
    #[error("HTTP error: {0}")]
    Http(String),
}
