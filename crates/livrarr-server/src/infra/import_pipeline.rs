use std::path::Path;

use crate::{ApiError, MediaType};
use livrarr_domain::sanitize_path_component;

/// Build target path: `{root}/{user_id}/{author}/{title}.{ext}` (ebook)
/// or `{root}/{user_id}/{author}/{title}/{relative}` (audiobook).
pub fn build_target_path(
    root: &str,
    user_id: i64,
    author: &str,
    title: &str,
    media_type: MediaType,
    source_file: &Path,
    source_root: &Path,
) -> String {
    let author_san = sanitize_path_component(author, "Unknown Author");
    let title_san = sanitize_path_component(title, "Unknown Title");
    let root = root.trim_end_matches('/');

    match media_type {
        MediaType::Ebook => {
            let ext = source_file
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("epub");
            format!("{root}/{user_id}/{author_san}/{title_san}.{ext}")
        }
        MediaType::Audiobook => {
            // Preserve subdirectory structure from source.
            let relative = if source_file == source_root {
                Path::new(
                    source_file
                        .file_name()
                        .unwrap_or(std::ffi::OsStr::new("unknown")),
                )
            } else {
                source_file.strip_prefix(source_root).unwrap_or(source_file)
            };
            let relative_str = relative.to_string_lossy();
            format!("{root}/{user_id}/{author_san}/{title_san}/{relative_str}")
        }
    }
}

/// Fetch torrent content_path from qBittorrent by hash.
pub async fn fetch_qbit_content_path(
    http_client: &livrarr_http::HttpClient,
    client: &livrarr_domain::DownloadClient,
    hash: &str,
) -> Result<String, ApiError> {
    let base_url = crate::infra::release_helpers::qbit_base_url(client);
    let sid = crate::infra::release_helpers::qbit_login(http_client, &base_url, client).await?;

    let info_url = format!("{base_url}/api/v2/torrents/info");
    // Admin-configured endpoint — use SSRF-safe client for redirect protection.
    let resp = http_client
        .get(&info_url)
        .query(&[("hashes", hash)])
        .header("Cookie", format!("SID={sid}"))
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("qBittorrent request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "qBittorrent returned {}",
            resp.status()
        )));
    }

    let torrents: Vec<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| ApiError::BadGateway(format!("qBittorrent parse error: {e}")))?;

    let torrent = torrents.first().ok_or(ApiError::NotFound)?;

    torrent
        .get("content_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ApiError::BadGateway("qBittorrent torrent missing content_path".to_string()))
}

/// Fetch SABnzbd storage path from history by nzo_id.
pub(crate) async fn fetch_sabnzbd_storage_path(
    http_client: &livrarr_http::HttpClient,
    client: &livrarr_domain::DownloadClient,
    nzo_id: &str,
) -> Result<String, ApiError> {
    let base_url = livrarr_handlers::download_client::client_base_url(client);
    let api_key = client.api_key.as_deref().unwrap_or("");

    // SABnzbd search param searches by name, not nzo_id. Fetch recent history and match client-side.
    let url = format!("{base_url}/api?mode=history&apikey={api_key}&output=json&limit=200");
    // Admin-configured endpoint — use SSRF-safe client so a redirect to an
    // internal address is blocked.
    let resp = http_client.get(&url).send().await.map_err(|e| {
        ApiError::BadGateway(format!(
            "SABnzbd history request failed: {}",
            e.without_url()
        ))
    })?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "SABnzbd history returned {}",
            resp.status()
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ApiError::BadGateway(format!("SABnzbd history parse error: {e}")))?;

    let entry = body
        .get("history")
        .and_then(|h| h.get("slots"))
        .and_then(|s| s.as_array())
        .and_then(|slots| {
            slots.iter().find(|e| {
                e.get("nzo_id")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| n == nzo_id)
            })
        })
        .ok_or_else(|| {
            ApiError::BadGateway(format!(
                "SABnzbd history entry not found for nzo_id={nzo_id}"
            ))
        })?;

    entry
        .get("storage")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            ApiError::BadGateway("SABnzbd history entry missing storage path".to_string())
        })
}

#[derive(Clone)]
pub struct PathMappingResult {
    pub local_path: String,
    pub configured_remote_path: Option<String>,
    pub configured_local_path: Option<String>,
}

pub fn apply_remote_path_mapping(
    mappings: &[livrarr_domain::RemotePathMapping],
    client_host: &str,
    content_path: &str,
) -> Result<PathMappingResult, ApiError> {
    // Normalize Windows backslashes — download clients on Windows report paths
    // like C:\Downloads\book.epub that need to match Linux forward-slash mappings.
    let content_path = &content_path.replace('\\', "/");

    // Extract hostname from client_host URL (strip scheme, port, path).
    let client_hostname = client_host
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(':')
        .next()
        .unwrap_or(client_host);

    // Filter to mappings that match this host.
    let host_matches: Vec<_> = mappings
        .iter()
        .filter(|m| {
            let mh = m.host.to_ascii_lowercase();
            let ch = client_hostname.to_ascii_lowercase();
            ch == mh || ch.ends_with(&format!(".{mh}"))
        })
        .collect();

    // Find longest matching remote_path prefix for this host.
    // Enforce directory boundary: remote_path must match at a `/` boundary
    // to prevent partial matches (e.g., /data/downloads matching /data/downloads_new).
    let best_match = host_matches
        .iter()
        .filter(|m| {
            let rp = m.remote_path.replace('\\', "/");
            if content_path.starts_with(&rp) {
                // Exact match or next char is '/' (directory boundary).
                content_path.len() == rp.len()
                    || content_path.as_bytes().get(rp.len()) == Some(&b'/')
                    || rp.ends_with('/')
            } else {
                false
            }
        })
        .max_by_key(|m| m.remote_path.len());

    match best_match {
        Some(mapping) => {
            let rp = mapping.remote_path.replace('\\', "/");
            let local = content_path.replacen(&rp, &mapping.local_path, 1);
            // Normalize double slashes from trailing/leading slash mismatches.
            Ok(PathMappingResult {
                local_path: local.replace("//", "/"),
                configured_remote_path: Some(mapping.remote_path.clone()),
                configured_local_path: Some(mapping.local_path.clone()),
            })
        }
        None => {
            // No path-prefix match, but include host-matched mapping config
            // for diagnostics (so the user/AI can see what's configured).
            let (cfg_remote, cfg_local) = host_matches
                .first()
                .map(|m| (Some(m.remote_path.clone()), Some(m.local_path.clone())))
                .unwrap_or((None, None));
            Ok(PathMappingResult {
                local_path: content_path.to_string(),
                configured_remote_path: cfg_remote,
                configured_local_path: cfg_local,
            })
        }
    }
}

/// CWA downstream integration: hardlink first, copy fallback, then touch to trigger inotify.
/// CWA expects flat files in the ingest root, no subdirectories.
/// Returns Some(warning) on failure, None on success.
pub(crate) fn cwa_copy(
    source_path: &str,
    cwa_ingest_path: &str,
    _user_id: i64,
    author: &str,
    title: &str,
    extension: &str,
) -> Option<String> {
    let author_san = sanitize_path_component(author, "Unknown Author");
    let title_san = sanitize_path_component(title, "Unknown Title");
    let dst_dir = Path::new(cwa_ingest_path);
    let dst = dst_dir.join(format!("{author_san} - {title_san}.{extension}"));

    if dst.exists() {
        return Some(format!("CWA destination already exists: {}", dst.display()));
    }

    if let Err(e) = std::fs::create_dir_all(dst_dir) {
        return Some(format!("CWA create_dir_all failed: {e}"));
    }

    // Hardlink first (zero extra disk space on same filesystem).
    let result = match std::fs::hard_link(source_path, &dst) {
        Ok(()) => None,
        Err(e) if e.raw_os_error() == Some(18) => {
            // EXDEV — cross-filesystem, fallback to copy.
            match std::fs::copy(source_path, &dst) {
                Ok(_) => None,
                Err(e) => Some(format!("CWA copy failed: {e}")),
            }
        }
        Err(e) => Some(format!("CWA hardlink failed: {e}")),
    };

    // Touch the file to trigger inotify (hardlinks don't fire IN_CREATE).
    // Open for writing and close — triggers IN_CLOSE_WRITE which CWA watches.
    if result.is_none() {
        let _ = std::fs::OpenOptions::new().append(true).open(&dst);
    }

    result
}

/// Build TagMetadata from a Work record for tag writing.
pub fn build_tag_metadata(work: &livrarr_domain::Work) -> livrarr_tagwrite::TagMetadata {
    livrarr_tagwrite::TagMetadata {
        title: work.title.clone(),
        subtitle: work.subtitle.clone(),
        author: work.author_name.clone(),
        narrator: work.narrator.clone(),
        year: work.year,
        genre: work.genres.clone(),
        description: work.description.clone(),
        publisher: work.publisher.clone(),
        isbn: work.isbn_13.clone(),
        language: work.language.clone(),
        series_name: work.series_name.clone(),
        series_position: work.series_position,
    }
}

/// Read cover image bytes for tag embedding.
/// Checks new tenant-aware path first, falls back to old flat layout.
/// Returns None if the file doesn't exist (not an error per TAG-V21-003).
pub async fn read_cover_bytes(
    data_dir: &std::path::Path,
    user_id: i64,
    work_id: i64,
) -> Option<Vec<u8>> {
    // Try new tenant-aware path: covers/{user_id}/{work_id}.jpg
    let new_path = data_dir
        .join("covers")
        .join(user_id.to_string())
        .join(format!("{work_id}.jpg"));
    if let Ok(bytes) = tokio::fs::read(&new_path).await {
        return Some(bytes);
    }
    // Fallback to old flat layout: covers/{work_id}.jpg
    let old_path = data_dir.join("covers").join(format!("{work_id}.jpg"));
    tokio::fs::read(&old_path).await.ok()
}
