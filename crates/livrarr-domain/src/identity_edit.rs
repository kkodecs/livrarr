//! Identity-edit input classification — the ONE place paste parsing lives.
//!
//! `classify_identifier_input` turns a pasted identifier or provider URL into a
//! `(slot, canonical value)` pair. The slot-free precedence is ordered so the
//! normalizer overlaps cannot misroute: a 10-digit checksum-invalid value is a
//! GR key, never an ASIN (bare ASIN classification requires a letter), and an
//! `OL…W` key is checked before the ASIN shape so it is never swallowed by it.

use crate::identity::AnchorType;
use crate::normalization::{normalize_asin, normalize_gr_key, normalize_isbn13, AsinNorm};
use crate::WorkId;

/// Typed failure of [`classify_identifier_input`]. All variants map to 422.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClassifyError {
    /// An OpenLibrary `OL…M` edition key — editions are not work identities.
    #[error("that's an edition id (OL…M) — paste the work id ending in W")]
    EditionKey,
    /// Slot-hinted paste that classifies to a different slot. Never a silent
    /// slot switch.
    #[error("that looks like a {} identifier — use Fix match", classified.as_str())]
    WrongSlot {
        hinted: AnchorType,
        classified: AnchorType,
    },
    /// Nothing recognizable.
    #[error("not a recognized identifier or provider URL")]
    Unrecognized,
}

/// Typed identity-edit failure surfaced by the preview/commit/clear surfaces.
/// The HTTP mapping is fixed by the API error contract: `InvalidValue` → 422,
/// `StalePreview` → 409 `preview_required`, `Collision` → 409
/// `anchor_collision`, `NotFound`/`EmptySlot` → 404, `Capacity` → 503
/// `preview_capacity` + Retry-After, `Unavailable` → 503, `Db` → 500.
#[derive(Debug, thiserror::Error)]
pub enum IdentityEditError {
    #[error("{0}")]
    InvalidValue(String),
    #[error("preview required — the snapshot is missing, used, or stale")]
    StalePreview,
    #[error("identifier already belongs to another work")]
    Collision {
        owning_work_id: WorkId,
        owning_work_title: String,
    },
    #[error("work not found")]
    NotFound,
    #[error("identity slot is already empty")]
    EmptySlot,
    #[error("preview capacity exhausted — retry shortly")]
    Capacity { retry_after_secs: u64 },
    #[error("storage temporarily unavailable")]
    Unavailable,
    #[error("database error: {0}")]
    Db(String),
}

/// Classify a pasted identifier or provider URL into `(slot, canonical value)`.
///
/// `hint = None` is the Fix-match road (full precedence table). `hint =
/// Some(slot)` is the row-pencil road: only that slot's forms are accepted; a
/// value that classifies to a different slot is [`ClassifyError::WrongSlot`].
pub fn classify_identifier_input(
    input: &str,
    hint: Option<AnchorType>,
) -> Result<(AnchorType, String), ClassifyError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ClassifyError::Unrecognized);
    }
    match hint {
        None => classify_free(trimmed),
        Some(hinted) => classify_hinted(trimmed, hinted),
    }
}

fn classify_free(input: &str) -> Result<(AnchorType, String), ClassifyError> {
    // 1. Provider URL forms.
    if let Some(result) = classify_url(input)? {
        return Ok(result);
    }
    // 2. ISBN-13 / ISBN-10 (separator-tolerant, checksum-validated).
    if let Some(isbn) = normalize_isbn13(input) {
        return Ok((AnchorType::new(AnchorType::ISBN_13), isbn));
    }
    // 3. All-digits (checksum failed or non-10/13 length) → GR key, never ASIN.
    if input.chars().all(|c| c.is_ascii_digit()) {
        if let Some(key) = normalize_gr_key(input) {
            return Ok((AnchorType::new(AnchorType::GR_WORK), key));
        }
    }
    // 4. Bare OL key — before the ASIN shape (a 10-char OL key is
    // alphanumeric-with-letters and would otherwise be swallowed by it).
    match ol_key_kind(input) {
        OlKeyKind::Work(key) => return Ok((AnchorType::new(AnchorType::OL_WORK), key)),
        OlKeyKind::Edition => return Err(ClassifyError::EditionKey),
        OlKeyKind::NotOl => {}
    }
    // 5. 10-char alphanumeric with at least one letter → ASIN (an
    // ISBN-10-shaped value with a trailing X folds to isbn_13 per AsinNorm).
    if input.len() == 10
        && input.chars().all(|c| c.is_ascii_alphanumeric())
        && input.chars().any(|c| c.is_ascii_alphabetic())
    {
        match normalize_asin(input) {
            AsinNorm::Asin(asin) => return Ok((AnchorType::new(AnchorType::ASIN), asin)),
            AsinNorm::Isbn13(isbn) => return Ok((AnchorType::new(AnchorType::ISBN_13), isbn)),
            AsinNorm::Invalid => {}
        }
    }
    Err(ClassifyError::Unrecognized)
}

fn classify_hinted(input: &str, hinted: AnchorType) -> Result<(AnchorType, String), ClassifyError> {
    let accepted = match hinted.as_str() {
        AnchorType::GR_WORK => {
            let from_url =
                url_segment(input, "goodreads.com", &["/book/show/"]).and_then(normalize_gr_key);
            from_url.or_else(|| {
                input
                    .chars()
                    .all(|c| c.is_ascii_digit())
                    .then(|| normalize_gr_key(input))
                    .flatten()
            })
        }
        AnchorType::OL_WORK => {
            let candidate = url_segment(input, "openlibrary.org", &["/works/"]).unwrap_or(input);
            match ol_key_kind(candidate) {
                OlKeyKind::Work(key) => Some(key),
                OlKeyKind::Edition => return Err(ClassifyError::EditionKey),
                OlKeyKind::NotOl => None,
            }
        }
        AnchorType::ASIN => {
            let candidate =
                url_segment(input, "amazon.", &["/dp/", "/gp/product/"]).unwrap_or(input);
            match normalize_asin(candidate) {
                // The ISBN-10 → isbn_13 fold happens only on the slot-free
                // road; on the ASIN row it is a wrong-slot paste.
                AsinNorm::Asin(asin)
                    if candidate.len() == 10
                        && candidate.chars().any(|c| c.is_ascii_alphabetic()) =>
                {
                    Some(asin)
                }
                _ => None,
            }
        }
        AnchorType::ISBN_13 => normalize_isbn13(input),
        _ => None,
    };
    if let Some(value) = accepted {
        return Ok((hinted, value));
    }
    // Not this slot's form — name the slot it does classify to (422, never a
    // silent slot switch), or propagate the slot-free error.
    match classify_free(input) {
        Ok((classified, value)) if classified == hinted => Ok((classified, value)),
        Ok((classified, _)) => Err(ClassifyError::WrongSlot { hinted, classified }),
        Err(e) => Err(e),
    }
}

/// Recognized provider URL → `(slot, canonical value)`. `Ok(None)` when the
/// input is not a recognized provider URL at all.
fn classify_url(input: &str) -> Result<Option<(AnchorType, String)>, ClassifyError> {
    if let Some(seg) = url_segment(input, "goodreads.com", &["/book/show/"]) {
        return match normalize_gr_key(seg) {
            Some(key) => Ok(Some((AnchorType::new(AnchorType::GR_WORK), key))),
            None => Err(ClassifyError::Unrecognized),
        };
    }
    if let Some(seg) = url_segment(input, "openlibrary.org", &["/works/", "/books/"]) {
        return match ol_key_kind(seg) {
            OlKeyKind::Work(key) => Ok(Some((AnchorType::new(AnchorType::OL_WORK), key))),
            OlKeyKind::Edition => Err(ClassifyError::EditionKey),
            OlKeyKind::NotOl => Err(ClassifyError::Unrecognized),
        };
    }
    if let Some(seg) = url_segment(input, "amazon.", &["/dp/", "/gp/product/"]) {
        return match normalize_asin(seg) {
            AsinNorm::Asin(asin) => Ok(Some((AnchorType::new(AnchorType::ASIN), asin))),
            AsinNorm::Isbn13(isbn) => Ok(Some((AnchorType::new(AnchorType::ISBN_13), isbn))),
            AsinNorm::Invalid => Err(ClassifyError::Unrecognized),
        };
    }
    Ok(None)
}

/// The path segment following the first `marker` in a URL whose host contains
/// `host` (case-insensitive host/path match; the segment keeps its case).
fn url_segment<'a>(input: &'a str, host: &str, markers: &[&str]) -> Option<&'a str> {
    let lower = input.to_ascii_lowercase();
    if !lower.contains(host) {
        return None;
    }
    for marker in markers {
        if let Some(pos) = lower.find(marker) {
            let rest = &input[pos + marker.len()..];
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            let seg = &rest[..end];
            if !seg.is_empty() {
                return Some(seg);
            }
        }
    }
    None
}

enum OlKeyKind {
    Work(String),
    Edition,
    NotOl,
}

/// `OL<digits>W` → work key (canonicalized uppercase); `OL<digits>M` →
/// edition; anything else is not an OL key.
fn ol_key_kind(candidate: &str) -> OlKeyKind {
    let upper = candidate.to_ascii_uppercase();
    let Some(body) = upper.strip_prefix("OL") else {
        return OlKeyKind::NotOl;
    };
    if body.len() < 2 {
        return OlKeyKind::NotOl;
    }
    let (digits, tail) = body.split_at(body.len() - 1);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return OlKeyKind::NotOl;
    }
    match tail {
        "W" => OlKeyKind::Work(upper),
        "M" => OlKeyKind::Edition,
        _ => OlKeyKind::NotOl,
    }
}
