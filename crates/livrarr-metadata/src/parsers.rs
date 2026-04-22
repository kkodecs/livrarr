//! CSV parsers for Goodreads and Hardcover book list exports.
//!
//! Both parsers produce `Vec<ImportRow>` from raw CSV bytes. They handle:
//! - BOM stripping (common in Windows CSV exports)
//! - Goodreads `="..."` ISBN wrapping
//! - Case-insensitive header matching
//! - Missing optional columns
//!
//! Moved from livrarr-server to livrarr-metadata: colocated with
//! ListServiceImpl since it has no server deps and is only consumed
//! by the list import service.

use std::collections::HashMap;

/// Detected CSV source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsvSource {
    Goodreads,
    Hardcover,
}

/// Reading status from the source platform (display-only for alpha3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportStatus {
    WantToRead,
    Reading,
    Read,
    Paused,
    #[serde(rename = "dnf")]
    DNF,
}

/// A single parsed row from a CSV import.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRow {
    pub row_index: usize,
    pub title: String,
    pub author: String,
    pub isbn_13: Option<String>,
    pub isbn_10: Option<String>,
    pub year: Option<i32>,
    pub status: Option<ImportStatus>,
    pub rating: Option<f32>,
}

/// Auto-detect CSV source from headers.
pub fn detect_csv_source(headers: &csv::StringRecord) -> Result<CsvSource, ParseError> {
    let lower: Vec<String> = headers.iter().map(|h| h.trim().to_lowercase()).collect();

    // Goodreads: has "exclusive shelf" column (unique to Goodreads)
    if lower.contains(&"exclusive shelf".to_string()) {
        return Ok(CsvSource::Goodreads);
    }

    // Goodreads: has "book id" column (also unique)
    if lower.contains(&"book id".to_string()) {
        return Ok(CsvSource::Goodreads);
    }

    // Hardcover: has "status" column and no "exclusive shelf"
    // Also check for hardcover-specific patterns
    if lower.contains(&"status".to_string())
        && (lower.contains(&"isbn_13".to_string()) || lower.contains(&"isbn 13".to_string()))
    {
        return Ok(CsvSource::Hardcover);
    }

    // Hardcover: check for "date started" / "date finished" (Hardcover-specific)
    if lower.contains(&"date started".to_string()) || lower.contains(&"date finished".to_string()) {
        return Ok(CsvSource::Hardcover);
    }

    Err(ParseError::UnknownFormat {
        detected_headers: headers.iter().map(|h| h.to_string()).collect(),
    })
}

/// Parse a Goodreads CSV export.
pub fn parse_goodreads_csv(bytes: &[u8]) -> Result<Vec<ImportRow>, ParseError> {
    let bytes = strip_bom(bytes);
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);

    let headers = rdr
        .headers()
        .map_err(|e| ParseError::CsvError(e.to_string()))?
        .clone();
    let col = build_column_map(&headers);

    let title_idx = col
        .get("title")
        .ok_or(ParseError::MissingColumn("Title".into()))?;
    let author_idx = col
        .get("author")
        .ok_or(ParseError::MissingColumn("Author".into()))?;
    let isbn13_idx = col.get("isbn13");
    let isbn_idx = col.get("isbn");
    let year_idx = col.get("original publication year");
    let shelf_idx = col.get("exclusive shelf");
    let rating_idx = col.get("my rating");

    let mut rows = Vec::new();
    for (i, result) in rdr.records().enumerate() {
        let record = match result {
            Ok(r) => r,
            Err(_) => {
                rows.push(ImportRow {
                    row_index: i,
                    title: String::new(),
                    author: String::new(),
                    isbn_13: None,
                    isbn_10: None,
                    year: None,
                    status: None,
                    rating: None,
                });
                continue;
            }
        };

        let title = get_field(&record, *title_idx).unwrap_or_default();
        let author = get_field(&record, *author_idx).unwrap_or_default();

        // Goodreads wraps ISBNs in ="..." for Excel safety
        let isbn_13 = isbn13_idx
            .and_then(|idx| get_field(&record, *idx))
            .map(|v| strip_excel_wrapper(&v))
            .filter(|v| !v.is_empty());

        let isbn_10 = isbn_idx
            .and_then(|idx| get_field(&record, *idx))
            .map(|v| strip_excel_wrapper(&v))
            .filter(|v| !v.is_empty());

        let year = year_idx
            .and_then(|idx| get_field(&record, *idx))
            .and_then(|v| v.parse::<i32>().ok());

        let status = shelf_idx
            .and_then(|idx| get_field(&record, *idx))
            .and_then(|v| match v.to_lowercase().as_str() {
                "to-read" => Some(ImportStatus::WantToRead),
                "currently-reading" => Some(ImportStatus::Reading),
                "read" => Some(ImportStatus::Read),
                _ => None,
            });

        let rating = rating_idx
            .and_then(|idx| get_field(&record, *idx))
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|&r| r > 0.0);

        rows.push(ImportRow {
            row_index: i,
            title,
            author,
            isbn_13,
            isbn_10,
            year,
            status,
            rating,
        });
    }

    Ok(rows)
}

/// Parse a Hardcover CSV export.
pub fn parse_hardcover_csv(bytes: &[u8]) -> Result<Vec<ImportRow>, ParseError> {
    let bytes = strip_bom(bytes);
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);

    let headers = rdr
        .headers()
        .map_err(|e| ParseError::CsvError(e.to_string()))?
        .clone();
    let col = build_column_map(&headers);

    let title_idx = col
        .get("title")
        .ok_or(ParseError::MissingColumn("Title".into()))?;
    let author_idx = col
        .get("author")
        .ok_or(ParseError::MissingColumn("Author".into()))?;
    let isbn13_idx = col.get("isbn_13").or_else(|| col.get("isbn 13"));
    let isbn10_idx = col.get("isbn_10").or_else(|| col.get("isbn 10"));
    let status_idx = col.get("status");
    let rating_idx = col.get("rating");

    let mut rows = Vec::new();
    for (i, result) in rdr.records().enumerate() {
        let record = match result {
            Ok(r) => r,
            Err(_) => {
                rows.push(ImportRow {
                    row_index: i,
                    title: String::new(),
                    author: String::new(),
                    isbn_13: None,
                    isbn_10: None,
                    year: None,
                    status: None,
                    rating: None,
                });
                continue;
            }
        };

        let title = get_field(&record, *title_idx).unwrap_or_default();
        let author = get_field(&record, *author_idx).unwrap_or_default();

        let isbn_13 = isbn13_idx
            .and_then(|idx| get_field(&record, *idx))
            .filter(|v| !v.is_empty());

        let isbn_10 = isbn10_idx
            .and_then(|idx| get_field(&record, *idx))
            .filter(|v| !v.is_empty());

        let status = status_idx
            .and_then(|idx| get_field(&record, *idx))
            .and_then(|v| match v.to_lowercase().as_str() {
                "want to read" => Some(ImportStatus::WantToRead),
                "currently reading" => Some(ImportStatus::Reading),
                "read" => Some(ImportStatus::Read),
                "paused" => Some(ImportStatus::Paused),
                "did not finish" | "dnf" => Some(ImportStatus::DNF),
                _ => None,
            });

        let rating = rating_idx
            .and_then(|idx| get_field(&record, *idx))
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|&r| r > 0.0);

        rows.push(ImportRow {
            row_index: i,
            title,
            author,
            isbn_13,
            isbn_10,
            year: None, // Hardcover CSV doesn't include year
            status,
            rating,
        });
    }

    Ok(rows)
}

/// Errors from CSV parsing.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("CSV parse error: {0}")]
    CsvError(String),

    #[error("missing required column: {0}")]
    MissingColumn(String),

    #[error("unknown CSV format — detected headers: {detected_headers:?}")]
    UnknownFormat { detected_headers: Vec<String> },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Strip UTF-8 BOM if present (public for handler use).
pub fn strip_bom_pub(bytes: &[u8]) -> &[u8] {
    strip_bom(bytes)
}

/// Strip UTF-8 BOM if present.
fn strip_bom(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    }
}

/// Build a case-insensitive column name -> index map.
fn build_column_map(headers: &csv::StringRecord) -> HashMap<String, usize> {
    headers
        .iter()
        .enumerate()
        .map(|(i, h)| (h.trim().to_lowercase(), i))
        .collect()
}

/// Get a trimmed field value from a record, returning None if out of bounds or empty.
fn get_field(record: &csv::StringRecord, idx: usize) -> Option<String> {
    record
        .get(idx)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Strip Goodreads Excel-safe ISBN wrapping: `="0060590297"` -> `0060590297`.
fn strip_excel_wrapper(val: &str) -> String {
    let trimmed = val.trim();
    if trimmed.len() > 3 && trimmed.starts_with("=\"") && trimmed.ends_with('"') {
        trimmed[2..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}
