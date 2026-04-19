use std::path::{Path, PathBuf};
use std::sync::Arc;

use livrarr_domain::services::{ImportIoService, TagService};
use livrarr_domain::{LibraryItem, Work};

pub struct LiveTagService<I> {
    import_io: Arc<I>,
    data_dir: Arc<PathBuf>,
}

impl<I> LiveTagService<I> {
    pub fn new(import_io: Arc<I>, data_dir: Arc<PathBuf>) -> Self {
        Self {
            import_io,
            data_dir,
        }
    }
}

impl<I: ImportIoService + Send + Sync> TagService for LiveTagService<I> {
    async fn retag_library_items(&self, work: &Work, items: &[LibraryItem]) -> Vec<String> {
        let tag_metadata = build_tag_metadata(work);
        let cover_data = read_cover_bytes(&self.data_dir, work.id).await;

        let mut warnings = Vec::new();

        let mut mp3_items = Vec::new();
        let mut other_items = Vec::new();
        for item in items {
            let ext = Path::new(&item.path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext == "mp3" {
                mp3_items.push(item);
            } else {
                other_items.push(item);
            }
        }

        for item in &other_items {
            let root = match self.import_io.get_root_folder(item.root_folder_id).await {
                Ok(rf) => rf,
                Err(e) => {
                    warnings.push(format!("root folder lookup failed: {e}"));
                    continue;
                }
            };

            let abs = format!("{}/{}", root.path, item.path);
            let tmp = format!("{abs}.tmp");

            let src = abs.clone();
            let dst = tmp.clone();
            let copy_result = tokio::task::spawn_blocking(move || std::fs::copy(&src, &dst)).await;

            if copy_result.is_err() || copy_result.as_ref().unwrap().is_err() {
                warnings.push(format!("retag: copy to .tmp failed for {abs}"));
                continue;
            }

            match livrarr_tagwrite::write_tags(
                tmp.clone(),
                tag_metadata.clone(),
                cover_data.clone(),
            )
            .await
            {
                Ok(livrarr_tagwrite::TagWriteStatus::Written) => {
                    let tmp2 = tmp.clone();
                    let abs2 = abs.clone();
                    let rename_result = tokio::task::spawn_blocking(move || {
                        if let Ok(f) = std::fs::File::open(&tmp2) {
                            let _ = f.sync_all();
                        }
                        std::fs::rename(&tmp2, &abs2)
                    })
                    .await;

                    match rename_result {
                        Ok(Ok(())) => {
                            let new_size = Path::new(&abs)
                                .metadata()
                                .map(|m| m.len() as i64)
                                .unwrap_or(0);
                            if let Err(e) = self
                                .import_io
                                .update_library_item_size(item.user_id, item.id, new_size)
                                .await
                            {
                                tracing::warn!("update_library_item_size failed: {e}");
                            }
                        }
                        _ => {
                            warnings.push(format!("retag: rename failed for {abs}"));
                            let _ = std::fs::remove_file(&tmp);
                        }
                    }
                }
                Ok(livrarr_tagwrite::TagWriteStatus::Unsupported)
                | Ok(livrarr_tagwrite::TagWriteStatus::NoData) => {
                    let _ = std::fs::remove_file(&tmp);
                }
                Err(e) => {
                    warnings.push(format!("retag: tag write failed for {abs}: {e}"));
                    let _ = std::fs::remove_file(&tmp);
                }
            }
        }

        if !mp3_items.is_empty() {
            let first = mp3_items[0];
            let root = match self.import_io.get_root_folder(first.root_folder_id).await {
                Ok(rf) => rf,
                Err(e) => {
                    warnings.push(format!("root folder lookup failed: {e}"));
                    return warnings;
                }
            };

            let mut abs_paths = Vec::new();
            let mut tmp_paths = Vec::new();
            for item in &mp3_items {
                let abs = format!("{}/{}", root.path, item.path);
                let tmp = format!("{abs}.tmp");
                abs_paths.push(abs);
                tmp_paths.push(tmp);
            }

            let mut copy_ok = true;
            for (abs, tmp) in abs_paths.iter().zip(tmp_paths.iter()) {
                let src = abs.clone();
                let dst = tmp.clone();
                let result = tokio::task::spawn_blocking(move || std::fs::copy(&src, &dst)).await;
                if result.is_err() || result.unwrap().is_err() {
                    warnings.push(format!("MP3 batch: copy to .tmp failed for {abs}"));
                    copy_ok = false;
                    break;
                }
            }

            if !copy_ok {
                for tmp in &tmp_paths {
                    let _ = std::fs::remove_file(tmp);
                }
            } else {
                match livrarr_tagwrite::write_tags_batch(
                    tmp_paths.clone(),
                    tag_metadata.clone(),
                    cover_data.clone(),
                )
                .await
                {
                    Ok(_) => {
                        for (i, (tmp, abs)) in tmp_paths.iter().zip(abs_paths.iter()).enumerate() {
                            if let Ok(f) = std::fs::File::open(tmp) {
                                let _ = f.sync_all();
                            }
                            if let Err(e) = std::fs::rename(tmp, abs) {
                                warnings.push(format!("MP3 batch rename failed for {abs}: {e}"));
                                let _ = std::fs::remove_file(tmp);
                            } else {
                                let new_size = Path::new(abs)
                                    .metadata()
                                    .map(|m| m.len() as i64)
                                    .unwrap_or(0);
                                if let Err(e) = self
                                    .import_io
                                    .update_library_item_size(
                                        mp3_items[i].user_id,
                                        mp3_items[i].id,
                                        new_size,
                                    )
                                    .await
                                {
                                    tracing::warn!("update_library_item_size failed: {e}");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warnings.push(format!("MP3 batch tag write failed: {e}"));
                        for tmp in &tmp_paths {
                            let _ = std::fs::remove_file(tmp);
                        }
                    }
                }
            }
        }

        warnings
    }
}

fn build_tag_metadata(work: &Work) -> livrarr_tagwrite::TagMetadata {
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

async fn read_cover_bytes(data_dir: &Path, work_id: i64) -> Option<Vec<u8>> {
    let cover_path = data_dir.join("covers").join(format!("{work_id}.jpg"));
    tokio::fs::read(&cover_path).await.ok()
}
