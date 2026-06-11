use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

// =============================================================================
// Types — mirrors IR contracts for TagWriter
// =============================================================================

#[derive(Clone, Debug, PartialEq)]
pub struct TagMetadata {
    pub title: String,
    pub subtitle: Option<String>,
    pub author: String,
    pub narrator: Option<Vec<String>>,
    pub year: Option<i32>,
    pub genre: Option<Vec<String>>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub isbn: Option<String>,
    pub language: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TagWriteStatus {
    Written,
    Unsupported,
    NoData,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TagWriteError {
    FileNotFound { path: String },
    EpubFailed(String),
    M4bFailed(String),
    Mp3Failed(String),
    TempFileFailed(String),
    RenameFailed(String),
    BatchAborted(String),
    Io(String),
}

pub trait TagWriter: Send + Sync {
    fn write_tags<'a>(
        &'a self,
        file_path: &'a str,
        metadata: &'a TagMetadata,
        cover: Option<&'a [u8]>,
    ) -> impl std::future::Future<Output = Result<TagWriteStatus, TagWriteError>> + Send + 'a;
    fn write_tags_batch<'a>(
        &'a self,
        files: &'a [(&'a str, &'a TagMetadata, Option<&'a [u8]>)],
    ) -> impl std::future::Future<Output = Result<Vec<TagWriteStatus>, TagWriteError>> + Send + 'a;
}

// =============================================================================
// Mock infrastructure
// =============================================================================

#[derive(Clone, Debug, PartialEq)]
struct StoredTags {
    metadata: TagMetadata,
    cover: Option<Vec<u8>>,
    format: String,
    chapters_preserved: bool,
}

#[derive(Default)]
struct MockFs {
    files: HashSet<String>,
    temp_files: HashSet<String>,
    tags: HashMap<String, StoredTags>,
    temp_tags: HashMap<String, StoredTags>,
    fail_temp: HashSet<String>,
    fail_rename: HashSet<String>,
}

#[derive(Clone, Default)]
struct MockTagWriter {
    fs: Arc<Mutex<MockFs>>,
}

impl MockTagWriter {
    fn new() -> Self {
        Self::default()
    }
    fn add_file(&self, p: &str) {
        self.fs.lock().unwrap().files.insert(p.into());
    }
    fn set_fail_temp(&self, p: &str) {
        self.fs.lock().unwrap().fail_temp.insert(p.into());
    }
    fn set_fail_rename(&self, p: &str) {
        self.fs.lock().unwrap().fail_rename.insert(p.into());
    }
    fn has_file(&self, p: &str) -> bool {
        self.fs.lock().unwrap().files.contains(p)
    }
    fn has_temp(&self, p: &str) -> bool {
        self.fs.lock().unwrap().temp_files.contains(p)
    }
    fn get_tags(&self, p: &str) -> Option<StoredTags> {
        self.fs.lock().unwrap().tags.get(p).cloned()
    }

    fn ext(p: &str) -> Option<&str> {
        p.rsplit('.')
            .next()
            .and_then(|e| if p.contains('.') { Some(e) } else { None })
    }
    fn is_supported(p: &str) -> bool {
        matches!(Self::ext(p), Some("epub" | "m4b" | "mp3"))
    }
    fn tmp(p: &str) -> String {
        format!("{p}.tmp")
    }
    fn fmt(p: &str) -> String {
        match Self::ext(p) {
            Some("epub") => "epub",
            Some("m4b") => "m4b",
            Some("mp3") => "mp3",
            _ => "unsupported",
        }
        .into()
    }
    fn has_enrichment(m: &TagMetadata) -> bool {
        !m.title.is_empty()
            || !m.author.is_empty()
            || m.subtitle.is_some()
            || m.narrator.is_some()
            || m.year.is_some()
            || m.genre.is_some()
            || m.description.is_some()
            || m.publisher.is_some()
            || m.isbn.is_some()
            || m.language.is_some()
            || m.series_name.is_some()
            || m.series_position.is_some()
    }
    fn write_temp(&self, p: &str, m: &TagMetadata, c: Option<&[u8]>) -> Result<(), TagWriteError> {
        let mut fs = self.fs.lock().unwrap();
        if !fs.files.contains(p) {
            return Err(TagWriteError::FileNotFound { path: p.into() });
        }
        if fs.fail_temp.contains(p) {
            let t = Self::tmp(p);
            fs.temp_files.remove(&t);
            fs.temp_tags.remove(&t);
            return Err(TagWriteError::TempFileFailed(p.into()));
        }
        let t = Self::tmp(p);
        fs.temp_files.insert(t.clone());
        fs.temp_tags.insert(
            t,
            StoredTags {
                metadata: m.clone(),
                cover: c.map(|c| c.to_vec()),
                format: Self::fmt(p),
                chapters_preserved: Self::ext(p) == Some("m4b"),
            },
        );
        Ok(())
    }
    fn rename_temp(&self, p: &str) -> Result<(), TagWriteError> {
        let mut fs = self.fs.lock().unwrap();
        let t = Self::tmp(p);
        if fs.fail_rename.contains(p) {
            fs.temp_files.remove(&t);
            fs.temp_tags.remove(&t);
            return Err(TagWriteError::RenameFailed(p.into()));
        }
        let tags = fs
            .temp_tags
            .remove(&t)
            .ok_or_else(|| TagWriteError::Io("missing temp".into()))?;
        fs.temp_files.remove(&t);
        fs.tags.insert(p.into(), tags);
        Ok(())
    }
    fn cleanup_temp(&self, p: &str) {
        let mut fs = self.fs.lock().unwrap();
        let t = Self::tmp(p);
        fs.temp_files.remove(&t);
        fs.temp_tags.remove(&t);
    }
}

impl TagWriter for MockTagWriter {
    async fn write_tags(
        &self,
        path: &str,
        meta: &TagMetadata,
        cover: Option<&[u8]>,
    ) -> Result<TagWriteStatus, TagWriteError> {
        if !Self::is_supported(path) {
            return Ok(TagWriteStatus::Unsupported);
        }
        if !Self::has_enrichment(meta) {
            return Ok(TagWriteStatus::NoData);
        }
        if let Err(e) = self.write_temp(path, meta, cover) {
            self.cleanup_temp(path);
            return Err(e);
        }
        if let Err(e) = self.rename_temp(path) {
            self.cleanup_temp(path);
            return Err(e);
        }
        Ok(TagWriteStatus::Written)
    }
    async fn write_tags_batch(
        &self,
        files: &[(&str, &TagMetadata, Option<&[u8]>)],
    ) -> Result<Vec<TagWriteStatus>, TagWriteError> {
        if files.is_empty() {
            return Ok(vec![]);
        }
        let mut statuses = Vec::with_capacity(files.len());
        let mut to_commit: Vec<&str> = Vec::new();
        // Pass 1: write temps
        for (p, m, c) in files {
            if !Self::is_supported(p) {
                statuses.push(TagWriteStatus::Unsupported);
                continue;
            }
            if !Self::has_enrichment(m) {
                statuses.push(TagWriteStatus::NoData);
                continue;
            }
            if let Err(e) = self.write_temp(p, m, *c) {
                for done in &to_commit {
                    self.cleanup_temp(done);
                }
                return Err(TagWriteError::BatchAborted(format!("{e:?}")));
            }
            to_commit.push(p);
            statuses.push(TagWriteStatus::Written);
        }
        // Pass 2: rename all-or-nothing
        for p in &to_commit {
            if let Err(e) = self.rename_temp(p) {
                for r in &to_commit {
                    self.cleanup_temp(r);
                }
                return Err(e);
            }
        }
        Ok(statuses)
    }
}

// =============================================================================
// Helpers
// =============================================================================

fn make_full_metadata() -> TagMetadata {
    TagMetadata {
        title: "Full Title".into(),
        subtitle: Some("Subtitle".into()),
        author: "Author Name".into(),
        narrator: Some(vec!["Narrator One".into(), "Narrator Two".into()]),
        year: Some(2024),
        genre: Some(vec!["Fantasy".into(), "Adventure".into()]),
        description: Some("Description".into()),
        publisher: Some("Publisher".into()),
        isbn: Some("9781234567890".into()),
        language: Some("en".into()),
        series_name: Some("Series".into()),
        series_position: Some(2.0),
    }
}

fn make_empty_metadata() -> TagMetadata {
    TagMetadata {
        title: "".into(),
        subtitle: None,
        author: "".into(),
        narrator: None,
        year: None,
        genre: None,
        description: None,
        publisher: None,
        isbn: None,
        language: None,
        series_name: None,
        series_position: None,
    }
}

fn make_partial_metadata() -> TagMetadata {
    TagMetadata {
        title: "Partial Title".into(),
        subtitle: None,
        author: "Partial Author".into(),
        narrator: None,
        year: None,
        genre: None,
        description: None,
        publisher: None,
        isbn: None,
        language: None,
        series_name: None,
        series_position: None,
    }
}

// =============================================================================
// TAG-001 — Partial data / no data
// =============================================================================

#[tokio::test]
async fn test_tagwrite_no_enrichment_data_returns_nodata() {
    // Satisfies: TAG-001 — No enrichment data at all returns NoData
    let w = MockTagWriter::new();
    w.add_file("book.mp3");
    let result = w.write_tags("book.mp3", &make_empty_metadata(), None).await;
    assert_eq!(result, Ok(TagWriteStatus::NoData));
    assert!(w.get_tags("book.mp3").is_none());
    assert!(!w.has_temp("book.mp3.tmp"));
}

#[tokio::test]
async fn test_tagwrite_partial_metadata_writes_better_than_none() {
    // Satisfies: TAG-001 — Partial metadata writes available fields and returns Written
    let w = MockTagWriter::new();
    w.add_file("book.mp3");
    let result = w
        .write_tags("book.mp3", &make_partial_metadata(), None)
        .await;
    assert_eq!(result, Ok(TagWriteStatus::Written));
    let s = w.get_tags("book.mp3").unwrap();
    assert_eq!(s.metadata.title, "Partial Title");
    assert_eq!(s.metadata.author, "Partial Author");
    assert_eq!(s.metadata.subtitle, None);
}

// =============================================================================
// TAG-002 — Temp-file-then-rename, failure cleanup
// =============================================================================

#[tokio::test]
async fn test_tagwrite_temp_creation_failure_leaves_original_untouched() {
    // Satisfies: TAG-002 — Temp-file write failure cleans temp and leaves original untouched
    let w = MockTagWriter::new();
    w.add_file("book.epub");
    let orig = make_partial_metadata();
    w.write_tags("book.epub", &orig, None).await.unwrap();
    w.set_fail_temp("book.epub");
    let result = w
        .write_tags("book.epub", &make_full_metadata(), Some(&[1, 2, 3]))
        .await;
    assert_eq!(
        result,
        Err(TagWriteError::TempFileFailed("book.epub".into()))
    );
    assert!(!w.has_temp("book.epub.tmp"));
    assert_eq!(w.get_tags("book.epub").unwrap().metadata.title, orig.title);
}

#[tokio::test]
async fn test_tagwrite_rename_failure_cleans_temp_and_preserves_original() {
    // Satisfies: TAG-002 — Rename failure cleans temp and leaves original untouched
    let w = MockTagWriter::new();
    w.add_file("book.m4b");
    let orig = make_partial_metadata();
    w.write_tags("book.m4b", &orig, None).await.unwrap();
    w.set_fail_rename("book.m4b");
    let result = w
        .write_tags("book.m4b", &make_full_metadata(), Some(&[9, 9]))
        .await;
    assert_eq!(result, Err(TagWriteError::RenameFailed("book.m4b".into())));
    assert!(!w.has_temp("book.m4b.tmp"));
    assert_eq!(w.get_tags("book.m4b").unwrap().metadata.title, orig.title);
}

#[tokio::test]
async fn test_tagwrite_nonexistent_file_returns_filenotfound() {
    // Satisfies: TAG-002 — Nonexistent supported file returns FileNotFound
    let w = MockTagWriter::new();
    let result = w
        .write_tags("missing.mp3", &make_full_metadata(), None)
        .await;
    assert_eq!(
        result,
        Err(TagWriteError::FileNotFound {
            path: "missing.mp3".into()
        })
    );
}

#[tokio::test]
async fn test_tagwrite_original_file_preserved_after_successful_write() {
    // Satisfies: TAG-002 — Temp-file-then-rename leaves original file present, no leftover temp
    let w = MockTagWriter::new();
    w.add_file("stable.epub");
    let result = w
        .write_tags("stable.epub", &make_full_metadata(), None)
        .await;
    assert_eq!(result, Ok(TagWriteStatus::Written));
    assert!(w.has_file("stable.epub"));
    assert!(!w.has_temp("stable.epub.tmp"));
}

// =============================================================================
// TAG-003 — EPUB
// =============================================================================

#[tokio::test]
async fn test_tagwrite_epub_full_metadata_writes_successfully() {
    // Satisfies: TAG-003 — EPUB with full metadata + cover returns Written
    let w = MockTagWriter::new();
    w.add_file("novel.epub");
    let cover = [1_u8, 2, 3, 4];
    let result = w
        .write_tags("novel.epub", &make_full_metadata(), Some(&cover))
        .await;
    assert_eq!(result, Ok(TagWriteStatus::Written));
    let s = w.get_tags("novel.epub").unwrap();
    assert_eq!(s.format, "epub");
    assert_eq!(s.metadata.series_name, Some("Series".into()));
    assert_eq!(s.cover, Some(cover.to_vec()));
}

#[tokio::test]
async fn test_tagwrite_epub_partial_metadata_writes_successfully() {
    // Satisfies: TAG-003 — EPUB with partial metadata returns Written
    let w = MockTagWriter::new();
    w.add_file("partial.epub");
    let result = w
        .write_tags("partial.epub", &make_partial_metadata(), None)
        .await;
    assert_eq!(result, Ok(TagWriteStatus::Written));
    let s = w.get_tags("partial.epub").unwrap();
    assert_eq!(s.format, "epub");
    assert_eq!(s.metadata.title, "Partial Title");
}

// =============================================================================
// TAG-004 — M4B
// =============================================================================

#[tokio::test]
async fn test_tagwrite_m4b_full_metadata_writes_successfully() {
    // Satisfies: TAG-004 — M4B with full metadata + cover returns Written, chapters preserved
    let w = MockTagWriter::new();
    w.add_file("audio.m4b");
    let cover = [5_u8, 6, 7];
    let result = w
        .write_tags("audio.m4b", &make_full_metadata(), Some(&cover))
        .await;
    assert_eq!(result, Ok(TagWriteStatus::Written));
    let s = w.get_tags("audio.m4b").unwrap();
    assert_eq!(s.format, "m4b");
    assert!(s.chapters_preserved);
    assert_eq!(s.cover, Some(cover.to_vec()));
}

#[tokio::test]
async fn test_tagwrite_m4b_partial_metadata_writes_successfully() {
    // Satisfies: TAG-004 — M4B with partial metadata returns Written, chapters preserved
    let w = MockTagWriter::new();
    w.add_file("partial.m4b");
    let result = w
        .write_tags("partial.m4b", &make_partial_metadata(), None)
        .await;
    assert_eq!(result, Ok(TagWriteStatus::Written));
    let s = w.get_tags("partial.m4b").unwrap();
    assert_eq!(s.format, "m4b");
    assert!(s.chapters_preserved);
}

// =============================================================================
// TAG-005 — MP3
// =============================================================================

#[tokio::test]
async fn test_tagwrite_mp3_full_metadata_writes_successfully() {
    // Satisfies: TAG-005 — MP3 with full metadata + cover returns Written
    let w = MockTagWriter::new();
    w.add_file("track.mp3");
    let cover = [8_u8, 8, 8];
    let result = w
        .write_tags("track.mp3", &make_full_metadata(), Some(&cover))
        .await;
    assert_eq!(result, Ok(TagWriteStatus::Written));
    let s = w.get_tags("track.mp3").unwrap();
    assert_eq!(s.format, "mp3");
    assert_eq!(s.cover, Some(cover.to_vec()));
}

#[tokio::test]
async fn test_tagwrite_mp3_partial_metadata_writes_successfully() {
    // Satisfies: TAG-005 — MP3 with partial metadata returns Written
    let w = MockTagWriter::new();
    w.add_file("partial.mp3");
    let result = w
        .write_tags("partial.mp3", &make_partial_metadata(), None)
        .await;
    assert_eq!(result, Ok(TagWriteStatus::Written));
    let s = w.get_tags("partial.mp3").unwrap();
    assert_eq!(s.format, "mp3");
    assert_eq!(s.metadata.title, "Partial Title");
}

#[tokio::test]
async fn test_tagwrite_none_cover_still_writes_tags() {
    // Satisfies: TAG-005 — None cover still writes tags, cover stored as None
    let w = MockTagWriter::new();
    w.add_file("nocover.mp3");
    let result = w
        .write_tags("nocover.mp3", &make_full_metadata(), None)
        .await;
    assert_eq!(result, Ok(TagWriteStatus::Written));
    assert_eq!(w.get_tags("nocover.mp3").unwrap().cover, None);
}

// =============================================================================
// TAG-006 — Batch write (multi-file MP3)
// =============================================================================

#[tokio::test]
async fn test_tagwrite_batch_multiple_mp3_all_written() {
    // Satisfies: TAG-006 — Batch write across multiple MP3 files completes atomically
    let w = MockTagWriter::new();
    w.add_file("a.mp3");
    w.add_file("b.mp3");
    let (m1, m2) = (make_full_metadata(), make_partial_metadata());
    let result = w
        .write_tags_batch(&[("a.mp3", &m1, Some(&[1, 2][..])), ("b.mp3", &m2, None)])
        .await;
    assert_eq!(
        result,
        Ok(vec![TagWriteStatus::Written, TagWriteStatus::Written])
    );
    assert!(w.get_tags("a.mp3").is_some());
    assert!(w.get_tags("b.mp3").is_some());
    assert!(!w.has_temp("a.mp3.tmp"));
    assert!(!w.has_temp("b.mp3.tmp"));
}

#[tokio::test]
async fn test_tagwrite_batch_pass1_failure_aborts_and_cleans_all_temps() {
    // Satisfies: TAG-006 — Any pass-1 temp write failure aborts batch and cleans all temps
    let w = MockTagWriter::new();
    w.add_file("a.mp3");
    w.add_file("b.mp3");
    w.set_fail_temp("b.mp3");
    let meta = make_full_metadata();
    let result = w
        .write_tags_batch(&[("a.mp3", &meta, None), ("b.mp3", &meta, None)])
        .await;
    assert!(matches!(result, Err(TagWriteError::BatchAborted(_))));
    assert!(!w.has_temp("a.mp3.tmp"));
    assert!(!w.has_temp("b.mp3.tmp"));
    assert!(w.get_tags("a.mp3").is_none());
    assert!(w.get_tags("b.mp3").is_none());
}

#[tokio::test]
async fn test_tagwrite_batch_rename_failure_cleans_remaining_temps() {
    // Satisfies: TAG-006 — Pass-2 rename failure aborts all-or-nothing and cleans temps
    let w = MockTagWriter::new();
    w.add_file("a.mp3");
    w.add_file("b.mp3");
    let orig = make_partial_metadata();
    w.write_tags("a.mp3", &orig, None).await.unwrap();
    w.write_tags("b.mp3", &orig, None).await.unwrap();
    w.set_fail_rename("b.mp3");
    let updated = make_full_metadata();
    let result = w
        .write_tags_batch(&[("a.mp3", &updated, None), ("b.mp3", &updated, None)])
        .await;
    assert_eq!(result, Err(TagWriteError::RenameFailed("b.mp3".into())));
    assert!(!w.has_temp("a.mp3.tmp"));
    assert!(!w.has_temp("b.mp3.tmp"));
    assert_eq!(w.get_tags("b.mp3").unwrap().metadata.title, orig.title);
}

#[tokio::test]
async fn test_tagwrite_batch_single_file_behaves_like_write_tags() {
    // Satisfies: TAG-006 — Batch with single file behaves like single-file write
    let w = MockTagWriter::new();
    w.add_file("single.mp3");
    let meta = make_full_metadata();
    let result = w.write_tags_batch(&[("single.mp3", &meta, None)]).await;
    assert_eq!(result, Ok(vec![TagWriteStatus::Written]));
    assert!(w.get_tags("single.mp3").is_some());
}

#[tokio::test]
async fn test_tagwrite_batch_empty_list_returns_empty_vec() {
    // Satisfies: TAG-006 — Batch with empty file list returns empty vector
    let w = MockTagWriter::new();
    assert_eq!(
        w.write_tags_batch(&[]).await,
        Ok(Vec::<TagWriteStatus>::new())
    );
}

// =============================================================================
// TAG-007 — Re-enrichment
// =============================================================================

#[tokio::test]
async fn test_tagwrite_reenrichment_overwrites_existing_tags() {
    // Satisfies: TAG-007 — Re-enrichment overwrites tags; repeated writes are idempotent
    let w = MockTagWriter::new();
    w.add_file("rewrite.mp3");
    let (first, second) = (make_partial_metadata(), make_full_metadata());
    assert_eq!(
        w.write_tags("rewrite.mp3", &first, None).await,
        Ok(TagWriteStatus::Written)
    );
    assert_eq!(
        w.write_tags("rewrite.mp3", &second, Some(&[7, 7, 7])).await,
        Ok(TagWriteStatus::Written)
    );
    let after2 = w.get_tags("rewrite.mp3").unwrap();
    assert_eq!(
        w.write_tags("rewrite.mp3", &second, Some(&[7, 7, 7])).await,
        Ok(TagWriteStatus::Written)
    );
    let after3 = w.get_tags("rewrite.mp3").unwrap();
    assert_eq!(after2, after3);
    assert_eq!(after3.metadata.title, "Full Title");
    assert_eq!(after3.cover, Some(vec![7, 7, 7]));
}

// =============================================================================
// TAG-008 — Unsupported formats
// =============================================================================

#[tokio::test]
async fn test_tagwrite_unsupported_pdf_returns_unsupported() {
    // Satisfies: TAG-008 — Unsupported .pdf returns Unsupported without error
    let w = MockTagWriter::new();
    w.add_file("doc.pdf");
    let result = w.write_tags("doc.pdf", &make_full_metadata(), None).await;
    assert_eq!(result, Ok(TagWriteStatus::Unsupported));
    assert!(w.get_tags("doc.pdf").is_none());
}

#[tokio::test]
async fn test_tagwrite_unsupported_mobi_returns_unsupported() {
    // Satisfies: TAG-008 — Unsupported .mobi returns Unsupported without error
    let w = MockTagWriter::new();
    w.add_file("book.mobi");
    let result = w.write_tags("book.mobi", &make_full_metadata(), None).await;
    assert_eq!(result, Ok(TagWriteStatus::Unsupported));
    assert!(w.get_tags("book.mobi").is_none());
}
