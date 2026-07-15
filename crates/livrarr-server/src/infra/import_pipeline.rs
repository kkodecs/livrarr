use std::path::Path;

use crate::ApiError;
use livrarr_domain::sanitize_path_component;
pub use livrarr_library::import_workflow::build_target_path;

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
        .header("Cookie", sid)
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

    // Find longest matching remote_path prefix for this host. A trailing
    // slash on remote_path is a normal way to type a directory and must not
    // change matching — compare with it stripped. Enforce directory boundary:
    // remote_path must match at a `/` boundary to prevent partial matches
    // (e.g., /data/downloads matching /data/downloads_new).
    let best_match = host_matches
        .iter()
        .filter(|m| {
            let rp = m.remote_path.replace('\\', "/");
            let rp = rp.trim_end_matches('/');
            content_path == rp
                || content_path
                    .strip_prefix(rp)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|m| m.remote_path.trim_end_matches('/').len());

    match best_match {
        Some(mapping) => {
            let rp = mapping.remote_path.replace('\\', "/");
            let rp = rp.trim_end_matches('/');
            // Both roots are compared/joined with trailing slashes stripped,
            // so the remainder (empty, or always "/..." per the boundary
            // check above) supplies the one separator either root's own
            // trailing slash used to. Whatever combination of trailing
            // slashes remote_path/local_path were configured with, the
            // join is always exactly one separator — never zero (mapping
            // /a/b/ -> /local with no local trailing slash used to glue
            // the next path segment straight onto "local") or two.
            let local_root = mapping.local_path.trim_end_matches('/');
            let remainder = content_path.strip_prefix(rp).unwrap_or("");
            Ok(PathMappingResult {
                local_path: format!("{local_root}{remainder}"),
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

#[cfg(test)]
mod apply_remote_path_mapping_tests {
    use super::*;

    fn mapping(remote_path: &str, local_path: &str) -> livrarr_domain::RemotePathMapping {
        livrarr_domain::RemotePathMapping {
            id: 1,
            host: "sab.example.com".to_string(),
            remote_path: remote_path.to_string(),
            local_path: local_path.to_string(),
        }
    }

    #[test]
    fn remote_trailing_slash_local_no_trailing_slash_still_joins_with_one_separator() {
        // The live bug (2026-07-14): a trailing slash on remote_path only
        // used to glue the local root straight onto the next path segment
        // with zero separators.
        let m = mapping("/home/user/downloads/sabnzbd/complete/", "/mnt/incoming");
        let result = apply_remote_path_mapping(
            &[m],
            "sab.example.com",
            "/home/user/downloads/sabnzbd/complete/Book Title",
        )
        .unwrap();
        assert_eq!(result.local_path, "/mnt/incoming/Book Title");
    }

    #[test]
    fn neither_side_has_trailing_slash() {
        let m = mapping("/home/user/downloads/complete", "/mnt/incoming");
        let result = apply_remote_path_mapping(
            &[m],
            "sab.example.com",
            "/home/user/downloads/complete/Book",
        )
        .unwrap();
        assert_eq!(result.local_path, "/mnt/incoming/Book");
    }

    #[test]
    fn both_sides_have_trailing_slash() {
        let m = mapping("/home/user/downloads/complete/", "/mnt/incoming/");
        let result = apply_remote_path_mapping(
            &[m],
            "sab.example.com",
            "/home/user/downloads/complete/Book",
        )
        .unwrap();
        assert_eq!(result.local_path, "/mnt/incoming/Book");
    }

    #[test]
    fn only_local_side_has_trailing_slash() {
        let m = mapping("/home/user/downloads/complete", "/mnt/incoming/");
        let result = apply_remote_path_mapping(
            &[m],
            "sab.example.com",
            "/home/user/downloads/complete/Book",
        )
        .unwrap();
        assert_eq!(result.local_path, "/mnt/incoming/Book");
    }

    #[test]
    fn exact_match_with_no_remainder() {
        let m = mapping("/home/user/downloads/complete/", "/mnt/incoming");
        let result =
            apply_remote_path_mapping(&[m], "sab.example.com", "/home/user/downloads/complete")
                .unwrap();
        assert_eq!(result.local_path, "/mnt/incoming");
    }

    #[test]
    fn prefix_must_match_at_a_directory_boundary() {
        // /data/downloads must not match /data/downloads_extra/... — same
        // shape as the qBittorrent path_starts_with boundary bug.
        let m = mapping("/data/downloads", "/mnt/incoming");
        let result =
            apply_remote_path_mapping(&[m], "sab.example.com", "/data/downloads_extra/Book")
                .unwrap();
        assert_eq!(
            result.local_path, "/data/downloads_extra/Book",
            "unmapped — returned unchanged"
        );
    }
}
