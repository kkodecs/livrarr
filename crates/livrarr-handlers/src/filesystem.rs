use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::middleware::RequireAdmin;
use crate::types::api_error::ApiError;

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

pub async fn browse<S: Clone + Send + Sync + 'static>(
    State(_state): State<S>,
    RequireAdmin(_auth): RequireAdmin,
    Query(query): Query<BrowseQuery>,
) -> Result<Json<BrowseResponse>, ApiError> {
    let path = query
        .path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));

    let result: Result<BrowseResponse, String> = tokio::task::spawn_blocking(move || {
        let canonical = path
            .canonicalize()
            .map_err(|_| "path not found or not accessible".to_string())?;

        if !canonical.is_dir() {
            return Err("path is not a directory".to_string());
        }

        let parent = canonical.parent().map(|p| p.display().to_string());

        let mut directories = Vec::new();
        let entries = match std::fs::read_dir(&canonical) {
            Ok(e) => e,
            Err(_) => {
                return Ok(BrowseResponse {
                    parent,
                    directories,
                });
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

        Ok(BrowseResponse {
            parent,
            directories,
        })
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking join error: {e}")))?;

    let response = result.map_err(ApiError::BadRequest)?;
    Ok(Json(response))
}
