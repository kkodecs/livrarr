//! KASH sidecar parsing and timestamp-space lookups.
//!
//! Pure module: no file IO, no CFI structural parsing. CFI strings are opaque
//! payloads carried alongside audio timestamps; all ordering decisions happen
//! in `ts` (audio seconds) space. The cross-format coordinate for the whole
//! feature is the audio timestamp — ebook percentage and audio percentage are
//! not comparable.

use serde::{Deserialize, Serialize};

/// Audio identity/drift tolerance when comparing an m4b container duration
/// against a `.kash` `duration_seconds`. A sidecar generated from the same
/// file agrees sub-second; a different cut differs by far more.
pub const DURATION_TOLERANCE_SECS: f64 = 2.0;

/// A parsed `.kash` alignment sidecar.
#[derive(Debug, Clone, Deserialize)]
pub struct Kash {
    pub version: u32,
    pub epub_hash: String,
    /// Provenance only — never recomputed or verified against the m4b in v1.
    pub audio_hash: String,
    pub duration_seconds: f64,
    pub chapters: Vec<KashChapter>,
    /// Strictly increasing in `ts` after parse normalization: generators emit
    /// occasional same-second ties, which collapse to the final anchor of
    /// each tied run; a ts decrease is corrupt and rejected.
    pub alignment: Vec<AlignmentEntry>,
}

/// One CFI ↔ timestamp anchor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignmentEntry {
    pub cfi: String,
    pub ts: f64,
}

/// Chapter span in timestamp space (for human-readable jump labels).
#[derive(Debug, Clone, Deserialize)]
pub struct KashChapter {
    pub title: String,
    pub start: f64,
    pub end: f64,
}

/// A resolved jump target: the anchor's CFI (ebook side) and timestamp
/// (audio side). `ts` 0.0 with the first anchor's CFI represents "the start"
/// for a furthest position before the first anchor.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTarget {
    pub cfi: String,
    pub ts: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum KashError {
    #[error("kash sidecar unreadable")]
    Unreadable,
    #[error("kash sidecar malformed: {0}")]
    Malformed(String),
}

/// Parse, validate, and normalize a `.kash` sidecar.
///
/// Rejects: unsupported version, non-finite/non-positive duration, empty
/// `epub_hash`, empty alignment, non-finite anchor `ts`, and any `ts`
/// DECREASE. Equal-`ts` runs are normalized by keeping the final anchor of
/// the run — generators sample anchors at coarse boundaries and emit
/// occasional ties — so the returned alignment is strictly increasing and
/// lookups binary-search without re-checking.
pub fn parse_kash(bytes: &[u8]) -> Result<Kash, KashError> {
    let mut kash: Kash =
        serde_json::from_slice(bytes).map_err(|e| KashError::Malformed(e.to_string()))?;

    if kash.version != 1 {
        return Err(KashError::Malformed("unsupported version".to_string()));
    }
    if !kash.duration_seconds.is_finite() || kash.duration_seconds <= 0.0 {
        return Err(KashError::Malformed(
            "duration_seconds must be finite and positive".to_string(),
        ));
    }
    if kash.epub_hash.is_empty() {
        return Err(KashError::Malformed("epub_hash is empty".to_string()));
    }
    if kash.alignment.is_empty() {
        return Err(KashError::Malformed("alignment is empty".to_string()));
    }

    let mut normalized: Vec<AlignmentEntry> = Vec::with_capacity(kash.alignment.len());
    for entry in kash.alignment.drain(..) {
        if !entry.ts.is_finite() {
            return Err(KashError::Malformed(
                "alignment ts is not finite".to_string(),
            ));
        }
        match normalized.last_mut() {
            Some(prev) if entry.ts < prev.ts => {
                return Err(KashError::Malformed("alignment ts decreases".to_string()));
            }
            Some(prev) if entry.ts == prev.ts => *prev = entry,
            _ => normalized.push(entry),
        }
    }
    kash.alignment = normalized;

    Ok(kash)
}

/// Nearest anchor at or before `ts`. `None` when `ts` is before the first
/// anchor. At or beyond the last anchor returns the LAST anchor (never skips
/// unanchored tail content — REQ-015).
pub fn anchor_at_or_before(kash: &Kash, ts: f64) -> Option<&AlignmentEntry> {
    let i = kash.alignment.partition_point(|entry| entry.ts <= ts);
    if i == 0 {
        None
    } else {
        Some(&kash.alignment[i - 1])
    }
}

/// Resolve the jump target for a furthest position, suppressing any target
/// not strictly ahead of `current_ts` (never-backward — REQ-006/REQ-015).
pub fn resolve_target(kash: &Kash, furthest_ts: f64, current_ts: f64) -> Option<ResolvedTarget> {
    let target = match anchor_at_or_before(kash, furthest_ts) {
        Some(anchor) => ResolvedTarget {
            cfi: anchor.cfi.clone(),
            ts: anchor.ts,
        },
        None => ResolvedTarget {
            cfi: kash.alignment[0].cfi.clone(),
            ts: 0.0,
        },
    };
    if target.ts <= current_ts {
        None
    } else {
        Some(target)
    }
}

/// Human-readable ebook-direction label for a timestamp: chapter title +
/// percent of book when a chapter covers `ts`, percent only otherwise.
pub fn chapter_label(kash: &Kash, ts: f64) -> String {
    let pct = ((ts / kash.duration_seconds).clamp(0.0, 1.0) * 100.0).round() as u32;
    let chapter = kash
        .chapters
        .iter()
        .find(|ch| ch.start <= ts && ts < ch.end);
    match chapter {
        Some(ch) => format!("{} \u{2014} {}%", ch.title, pct),
        None => format!("{}%", pct),
    }
}
