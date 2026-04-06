use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::middleware::RequireAdmin;
use crate::state::AppState;
use crate::ApiError;

#[derive(Debug, Deserialize)]
pub struct BrowseQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseResponse {
    pub parent: Option<String>,
    pub directories: Vec<DirEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub path: String,
}

/// GET /api/v1/filesystem
pub async fn browse(
    State(_state): State<AppState>,
    RequireAdmin(_auth): RequireAdmin,
    Query(query): Query<BrowseQuery>,
) -> Result<Json<BrowseResponse>, ApiError> {
    let path = query
        .path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));

    // Canonicalize to prevent traversal via `..`.
    let canonical = path
        .canonicalize()
        .map_err(|_| ApiError::BadRequest("path not found or not accessible".into()))?;

    if !canonical.is_dir() {
        return Err(ApiError::BadRequest("path is not a directory".into()));
    }

    let parent = canonical.parent().map(|p| p.display().to_string());

    let mut directories = Vec::new();
    let entries = match std::fs::read_dir(&canonical) {
        Ok(e) => e,
        Err(_) => {
            // Permission denied or other read error — return empty listing per spec.
            return Ok(Json(BrowseResponse {
                parent,
                directories,
            }));
        }
    };

    for entry in entries.flatten() {
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip hidden directories.
        if name.starts_with('.') {
            continue;
        }
        let entry_path = entry.path().display().to_string();
        directories.push(DirEntry {
            name,
            path: entry_path,
        });
    }

    directories.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    Ok(Json(BrowseResponse {
        parent,
        directories,
    }))
}
