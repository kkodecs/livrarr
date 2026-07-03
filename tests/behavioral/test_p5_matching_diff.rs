//! Phase 5 Unit C — old-vs-new matching decision-diff harness (REQ-018/D8).
//!
//! Zero-write, offline comparison of what the OLD matching code decides vs
//! what the NEW `livrarr_domain::identity_matching` authority decides, over
//! every candidate pair of one user's works in a real library snapshot.
//! Writes nothing: it opens the snapshot file `SQLITE_OPEN_READONLY` (a
//! write attempt fails at the SQLite layer, not just "we only issued
//! SELECTs") and only ever runs `SELECT` statements against it.
//!
//! Not part of the normal suite — `#[ignore]`d, run on demand:
//!
//! ```text
//! sqlite3 /path/to/live.db ".backup '/path/to/snapshot.db'"
//! MATCHING_DIFF_DB_PATH=/path/to/snapshot.db \
//! MATCHING_DIFF_OUT_DIR=/path/to/reports \
//!   cargo test -p livrarr-behavioral --test test_p5_matching_diff \
//!     -- --ignored --nocapture
//! ```
//!
//! `MATCHING_DIFF_OUT_DIR` defaults to `<workspace root>/reports`. Emits
//! `matching-diff.json` (machine-readable) and `matching-diff.md` (the
//! PO-facing report) into that directory.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;

use livrarr_domain::identity_matching::{
    author_verdict, id_verdict, language_verdict, parse_title, title_verdict,
    title_verdict_with_positions, AuthorVerdict, IdEvidence, IdVerdict, LanguageVerdict,
    TitleVerdict,
};
use livrarr_domain::Work;
use livrarr_matching::work_dedup::{self, ProviderKeys};
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

// ===========================================================================
// Data loading — the ONLY code in this file that touches the database.
// ===========================================================================

/// One `works` row, projected to the fields every seat below needs. Loaded
/// once from the read-only snapshot; every seat function is a pure
/// computation over these structs plus the real matching-crate functions.
#[derive(Debug, Clone)]
struct WorkRow {
    id: i64,
    user_id: i64,
    title: String,
    author_name: String,
    normalized_title: String,
    normalized_author: String,
    language: Option<String>,
    series_position: Option<f64>,
    ol_key: Option<String>,
    gr_key: Option<String>,
    hc_key: Option<String>,
    isbn_13: Option<String>,
    asin: Option<String>,
}

/// Open the snapshot file `SQLITE_OPEN_READONLY`. Structural zero-write
/// guarantee: any write attempt against this pool fails at the SQLite
/// layer, regardless of what SQL this file happens to issue.
async fn open_snapshot_readonly(path: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::new().filename(path).read_only(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap_or_else(|e| panic!("failed to open read-only snapshot at {path:?}: {e}"))
}

async fn load_works(pool: &SqlitePool) -> Vec<WorkRow> {
    let rows = sqlx::query(
        "SELECT id, user_id, title, author_name, normalized_title, normalized_author, \
         language, series_position, ol_key, gr_key, hc_key, isbn_13, asin \
         FROM works ORDER BY user_id, id",
    )
    .fetch_all(pool)
    .await
    .expect("select works from snapshot");

    rows.into_iter()
        .map(|row| WorkRow {
            id: row.try_get("id").expect("id"),
            user_id: row.try_get("user_id").expect("user_id"),
            title: row.try_get("title").expect("title"),
            author_name: row.try_get("author_name").expect("author_name"),
            normalized_title: row.try_get("normalized_title").expect("normalized_title"),
            normalized_author: row.try_get("normalized_author").expect("normalized_author"),
            language: row.try_get("language").expect("language"),
            series_position: row.try_get("series_position").expect("series_position"),
            ol_key: row.try_get("ol_key").expect("ol_key"),
            gr_key: row.try_get("gr_key").expect("gr_key"),
            hc_key: row.try_get("hc_key").expect("hc_key"),
            isbn_13: row.try_get("isbn_13").expect("isbn_13"),
            asin: row.try_get("asin").expect("asin"),
        })
        .collect()
}

/// The install-wide default language (migration 067,
/// `metadata_config.default_language`; singleton row, `'en'` fallback if
/// the row is somehow absent).
async fn load_default_language(pool: &SqlitePool) -> String {
    sqlx::query_scalar::<_, String>("SELECT default_language FROM metadata_config LIMIT 1")
        .fetch_optional(pool)
        .await
        .expect("select default_language from snapshot")
        .unwrap_or_else(|| "en".to_string())
}

// ===========================================================================
// Shared helpers
// ===========================================================================

/// Build a domain `Work` carrying only the fields `find_matching_work`
/// reads (title, author_name, ol_key, gr_key, hc_key, isbn_13, asin).
/// Every other field takes its `Default` impl — the cascade never looks at
/// them, so this is a faithful, zero-risk bridge from the snapshot row to
/// the real production function's input type.
fn to_domain_work(row: &WorkRow) -> Work {
    Work {
        title: row.title.clone(),
        author_name: row.author_name.clone(),
        ol_key: row.ol_key.clone(),
        gr_key: row.gr_key.clone(),
        hc_key: row.hc_key.clone(),
        isbn_13: row.isbn_13.clone(),
        asin: row.asin.clone(),
        ..Default::default()
    }
}

fn to_id_evidence(row: &WorkRow) -> IdEvidence<'_> {
    IdEvidence {
        ol_key: row.ol_key.as_deref(),
        gr_key: row.gr_key.as_deref(),
        hc_key: row.hc_key.as_deref(),
        isbn_13: row.isbn_13.as_deref(),
        asin: row.asin.as_deref(),
    }
}

fn describe_id_verdict(v: IdVerdict) -> &'static str {
    match v {
        IdVerdict::WorkKeyEqual => "id_verdict=WorkKeyEqual",
        IdVerdict::WorkKeyContradiction => "id_verdict=WorkKeyContradiction",
        IdVerdict::EditionBridge => "id_verdict=EditionBridge",
        IdVerdict::NoEvidence => "id_verdict=NoEvidence",
    }
}

/// Two provider-key strings are equal evidence only when both are present
/// and non-blank (mirrors the `.filter(|k| !k.is_empty())` guard every
/// OLD-seat call site below already applies).
fn key_eq(a: &Option<String>, b: &Option<String>) -> bool {
    matches!((a.as_deref(), b.as_deref()), (Some(x), Some(y)) if !x.is_empty() && x == y)
}

/// A work's identity for report output: enough for the PO to recognize the
/// book without re-deriving anything from an id.
#[derive(Debug, Clone)]
struct PairRef {
    id: i64,
    title: String,
    author: String,
}

impl PairRef {
    fn from(row: &WorkRow) -> Self {
        PairRef {
            id: row.id,
            title: row.title.clone(),
            author: row.author_name.clone(),
        }
    }
}

fn pair_ref_json(p: &PairRef) -> serde_json::Value {
    json!({ "id": p.id, "title": p.title, "author": p.author })
}

/// Markdown table cells: escape the one character that breaks a table row.
fn md_escape(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ").replace('\r', "")
}

/// Group works by user — matching only ever compares within one user's
/// library (P4 user-scoping), never across users.
fn group_by_user(works: &[WorkRow]) -> BTreeMap<i64, Vec<&WorkRow>> {
    let mut map: BTreeMap<i64, Vec<&WorkRow>> = BTreeMap::new();
    for w in works {
        map.entry(w.user_id).or_default().push(w);
    }
    map
}

/// Every unordered pair of distinct works within one user's library, as
/// index pairs into the caller's slice.
fn pairs(works: &[&WorkRow]) -> Vec<(usize, usize)> {
    let n = works.len();
    let mut out = Vec::with_capacity(n * n.saturating_sub(1) / 2);
    for i in 0..n {
        for j in (i + 1)..n {
            out.push((i, j));
        }
    }
    out
}

// ===========================================================================
// Seat 1 — library dedup / absorb
// ===========================================================================
//
// OLD: `work_dedup::find_matching_work` (crates/livrarr-matching/src/
// work_dedup.rs:52-118) — a three-tier cascade: provider key equality
// (ol_key/gr_key/isbn_13/asin — NOT hc_key, `ProviderKeys` has no hc_key
// field), then exact normalized title+author, then base-title equality
// when EXACTLY one side's raw title carries a subtitle (a colon). Called
// here through the real function against a single-work "existing" slice,
// so the verdict is exactly what production computes for this pair — no
// reimplementation of the decision itself.
//
// NEW: identity_matching::title_verdict + author_verdict + id_verdict,
// D2/REQ-005/REQ-006 semantics:
//   - VetoVolume → veto, never merge.
//   - Different mains → no match.
//   - Same mains + a same-provider work-key contradiction → CONFLICT
//     (REQ-006: the contradiction outranks all text and edition-id
//     agreement — the ISBN-collision/AC-021 shape; never merge, surfaced
//     distinctly rather than parked grey).
//   - Same + Agree → match. Same + Abstain (no usable author on a side)
//     → match: REQ-005 keeps current semantics — authorless agreement
//     requires exact full-title equality, which Same satisfies.
//   - Same + Disagree (zero author overlap) → blocked; grey ONLY when
//     positive ID evidence carries the pair (WorkKeyEqual/EditionBridge),
//     else no match (REQ-005 rule c).
//   - Grey title, or Same + Grey author → grey (never absorb).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewBucket {
    Match,
    Grey,
    Conflict,
    VetoVolume,
    NoMatch,
}

impl NewBucket {
    fn label(self) -> &'static str {
        match self {
            NewBucket::Match => "match",
            NewBucket::Grey => "grey",
            NewBucket::Conflict => "conflict",
            NewBucket::VetoVolume => "veto_volume",
            NewBucket::NoMatch => "no_match",
        }
    }
}

fn seat1_new_bucket(tv: TitleVerdict, av: AuthorVerdict, idv: IdVerdict) -> NewBucket {
    match tv {
        TitleVerdict::VetoVolume => NewBucket::VetoVolume,
        TitleVerdict::Different => NewBucket::NoMatch,
        TitleVerdict::Same => {
            if idv == IdVerdict::WorkKeyContradiction {
                return NewBucket::Conflict;
            }
            match av {
                AuthorVerdict::Agree | AuthorVerdict::Abstain => NewBucket::Match,
                AuthorVerdict::Grey => NewBucket::Grey,
                AuthorVerdict::Disagree => {
                    if matches!(idv, IdVerdict::WorkKeyEqual | IdVerdict::EditionBridge) {
                        NewBucket::Grey
                    } else {
                        NewBucket::NoMatch
                    }
                }
            }
        }
        TitleVerdict::Grey { .. } => NewBucket::Grey,
    }
}

struct Seat1Row {
    a: PairRef,
    b: PairRef,
    old_absorb: bool,
    old_reason: String,
    new_bucket: NewBucket,
    new_reason: String,
    id_verdict: IdVerdict,
    changed: bool,
    class: &'static str,
}

fn seat1_pair(a: &WorkRow, b: &WorkRow) -> Seat1Row {
    let work_a = to_domain_work(a);
    let existing = std::slice::from_ref(&work_a);
    let keys_b = ProviderKeys {
        ol_key: b.ol_key.as_deref(),
        gr_key: b.gr_key.as_deref(),
        isbn_13: b.isbn_13.as_deref(),
        asin: b.asin.as_deref(),
    };
    let full_hit =
        work_dedup::find_matching_work(existing, &b.title, &b.author_name, &keys_b).is_some();
    let keyless_hit = work_dedup::find_matching_work(
        existing,
        &b.title,
        &b.author_name,
        &ProviderKeys::default(),
    )
    .is_some();

    // Tier attribution below is DESCRIPTIVE only (for the reason string) —
    // the match/no-match verdict itself always comes from the two real
    // `find_matching_work` calls above, never from this classification.
    let old_reason = if full_hit && !keyless_hit {
        let mut via = Vec::new();
        if key_eq(&a.ol_key, &b.ol_key) {
            via.push("ol_key");
        }
        if key_eq(&a.gr_key, &b.gr_key) {
            via.push("gr_key");
        }
        if key_eq(&a.isbn_13, &b.isbn_13) {
            via.push("isbn_13");
        }
        if key_eq(&a.asin, &b.asin) {
            via.push("asin");
        }
        format!("old matched via provider key ({})", via.join(", "))
    } else if keyless_hit {
        if a.title.contains(':') != b.title.contains(':') {
            "old matched via base-title (one-sided colon) tier".to_string()
        } else {
            "old matched via exact normalized title+author tier".to_string()
        }
    } else {
        "old: no match".to_string()
    };

    let pa = parse_title(&a.title);
    let pb = parse_title(&b.title);
    let tv = title_verdict(&pa, &pb);
    let av = author_verdict(
        std::slice::from_ref(&a.author_name),
        std::slice::from_ref(&b.author_name),
    );
    let idv = id_verdict(&to_id_evidence(a), &to_id_evidence(b));
    let new_bucket = seat1_new_bucket(tv, av, idv);
    let new_reason = format!("new: title={tv:?}, author={av:?}");

    let new_absorb = new_bucket == NewBucket::Match;
    let changed = full_hit != new_absorb;
    let class = if !changed {
        "agree"
    } else if full_hit {
        "old_absorbed_new_blocks"
    } else {
        "new_matches_old_didnt"
    };

    Seat1Row {
        a: PairRef::from(a),
        b: PairRef::from(b),
        old_absorb: full_hit,
        old_reason,
        new_bucket,
        new_reason,
        id_verdict: idv,
        changed,
        class,
    }
}

// ===========================================================================
// Seat 2 — DB identity key
// ===========================================================================
//
// OLD: equality of the STORED `works.normalized_title`/`normalized_author`
// columns (crates/livrarr-domain/src/lib.rs:885-917, `normalize_for_matching`
// — illegal-filesystem-char strip + lowercase + whitespace collapse; no
// colon cut, no article strip). These columns already carry a real
// `UNIQUE(user_id, normalized_title, normalized_author)` index, so
// `old_match` is structurally false for every pair of DISTINCT existing
// works — reported anyway: that's the honest zero, not a harness artifact.
//
// NEW: exact main-title equality (`ParsedTitle.main`, bare — NOT the fuller
// `title_verdict` state machine, since a DB key can't express "grey") plus
// author Agree. This is the literal Unit-E `identity_key` recipe
// (REQ-014: the DB key and the already-in-library key of Seat 3 become ONE
// function post-cutover, which is why Seats 2 and 3 share this formula).

struct Seat2Row {
    a: PairRef,
    b: PairRef,
    old_match: bool,
    new_match: bool,
    reason: String,
    id_verdict: IdVerdict,
}

fn seat2_pair(a: &WorkRow, b: &WorkRow) -> Seat2Row {
    let old_match =
        a.normalized_title == b.normalized_title && a.normalized_author == b.normalized_author;

    let pa = parse_title(&a.title);
    let pb = parse_title(&b.title);
    let av = author_verdict(
        std::slice::from_ref(&a.author_name),
        std::slice::from_ref(&b.author_name),
    );
    let new_match = pa.main == pb.main && av == AuthorVerdict::Agree;

    let reason = format!(
        "old: normalized_title/author {}; new: main-title {} + author={av:?}",
        if old_match { "equal" } else { "differ" },
        if pa.main == pb.main {
            "equal"
        } else {
            "differ"
        },
    );

    Seat2Row {
        a: PairRef::from(a),
        b: PairRef::from(b),
        old_match,
        new_match,
        reason,
        id_verdict: id_verdict(&to_id_evidence(a), &to_id_evidence(b)),
    }
}

// ===========================================================================
// Seat 3 — strict already-in-library key
// ===========================================================================
//
// OLD: `work_dedup::normalize_title_for_match` equality
// (crates/livrarr-matching/src/work_dedup.rs:209-224 — cuts at the first
// `:` and at " - ", strips a leading article, folds punctuation) combined
// with the public `authors_match` helper. This mirrors the richer of its
// two real call sites (anchor-graft / cover-borrow,
// crates/livrarr-metadata/src/work_service.rs:3153-3220); the other call
// site (bibliography "already in library" flag,
// crates/livrarr-metadata/src/author_service.rs:616-624) is title-only
// because it is pre-scoped to one author's own bibliography — folding
// author equality back in here is the correct generalization across a
// whole library, not a divergence from it.
//
// NEW: same recipe as Seat 2 (REQ-014 unification) — main-title equality +
// author Agree.

struct Seat3Row {
    a: PairRef,
    b: PairRef,
    old_match: bool,
    new_match: bool,
    reason: String,
    id_verdict: IdVerdict,
}

fn seat3_pair(a: &WorkRow, b: &WorkRow) -> Seat3Row {
    let old_match = work_dedup::normalize_title_for_match(&a.title)
        == work_dedup::normalize_title_for_match(&b.title)
        && work_dedup::authors_match(&a.author_name, &b.author_name);

    let pa = parse_title(&a.title);
    let pb = parse_title(&b.title);
    let av = author_verdict(
        std::slice::from_ref(&a.author_name),
        std::slice::from_ref(&b.author_name),
    );
    let new_match = pa.main == pb.main && av == AuthorVerdict::Agree;

    let reason = format!(
        "old: normalize_title_for_match {}; new: main-title {} + author={av:?}",
        if old_match { "equal" } else { "differ" },
        if pa.main == pb.main {
            "equal"
        } else {
            "differ"
        },
    );

    Seat3Row {
        a: PairRef::from(a),
        b: PairRef::from(b),
        old_match,
        new_match,
        reason,
        id_verdict: id_verdict(&to_id_evidence(a), &to_id_evidence(b)),
    }
}

// ===========================================================================
// Seat 4 — variant-fold (title_similarity_with_variants)
// ===========================================================================
//
// OLD rebuilt from the crate's PUBLIC surface only — never a private-item
// copy — reproducing crates/livrarr-matching/src/m4_scoring.rs:111-122
// line for line: full string similarity first; if BOTH sides carry a
// known, differing series position, return the full score (no fold);
// otherwise, if the two titles' variant-fold keys (colon-cut,
// crates/livrarr-matching/src/lib.rs:37-80) are equal, force 1.0.
// `old_forced` flags the risky case: the fold pushed a sub-1.0 full score
// up to a certain 1.0 "match".
//
// NEW: identity_matching::title_verdict_with_positions, fed the same
// caller-supplied series positions from the DB.
//
// The measured population is the `old_forced == true` rows ONLY — pairs
// where the fold actually changed the old scorer's answer. A changed
// decision is a forced pair the new authority no longer calls Same.
// (Identical-title pairs score 1.0 with or without the fold; nothing was
// forced, so nothing can have changed at this seat.)

struct Seat4Row {
    a: PairRef,
    b: PairRef,
    old_full_score: f64,
    old_forced: bool,
    new_verdict: TitleVerdict,
    new_same: bool,
    changed: bool,
    class: &'static str,
}

/// Rebuilt from `livrarr_matching::string_similarity` +
/// `livrarr_matching::normalize_title_variants` (both public) — reproduces
/// the private `m4_scoring::title_similarity_with_variants` exactly, per
/// the doc comment above.
fn old_title_similarity_with_variants(
    a: &str,
    b: &str,
    pos_a: Option<f64>,
    pos_b: Option<f64>,
) -> f64 {
    let full = livrarr_matching::string_similarity(a, b);
    if let (Some(x), Some(y)) = (pos_a, pos_b) {
        if (x - y).abs() >= 0.01 {
            return full;
        }
    }
    if livrarr_matching::normalize_title_variants(a)
        == livrarr_matching::normalize_title_variants(b)
    {
        return 1.0;
    }
    full
}

fn seat4_pair(a: &WorkRow, b: &WorkRow) -> Seat4Row {
    let old_full_score = livrarr_matching::string_similarity(&a.title, &b.title);
    let old_score = old_title_similarity_with_variants(
        &a.title,
        &b.title,
        a.series_position,
        b.series_position,
    );
    let old_forced = (old_score - 1.0).abs() < 1e-9 && (old_full_score - 1.0).abs() > 1e-9;

    let pa = parse_title(&a.title);
    let pb = parse_title(&b.title);
    let tv = title_verdict_with_positions(&pa, a.series_position, &pb, b.series_position);
    let new_same = tv == TitleVerdict::Same;

    let changed = old_forced && !new_same;
    let class = if changed {
        "old_forced_new_blocks"
    } else {
        "agree"
    };

    Seat4Row {
        a: PairRef::from(a),
        b: PairRef::from(b),
        old_full_score,
        old_forced,
        new_verdict: tv,
        new_same,
        changed,
        class,
    }
}

// ===========================================================================
// Seat 5 — language census (new-only; D7/REQ-007 impact preview)
// ===========================================================================
//
// For every work: what would identity_matching::language_verdict say for a
// HYPOTHETICAL language-silent payload (no declared language at all)? No
// seat in the current codebase asks this question, so there is no OLD side
// to compare — this is a pure census of the future grey-zone's size.

struct Seat5Row {
    work: PairRef,
    language: Option<String>,
    verdict: LanguageVerdict,
}

fn seat5_census(works: &[WorkRow], default_language: &str) -> Vec<Seat5Row> {
    works
        .iter()
        .map(|w| Seat5Row {
            work: PairRef::from(w),
            language: w.language.clone(),
            verdict: language_verdict(w.language.as_deref(), None, default_language),
        })
        .collect()
}

// ===========================================================================
// Report assembly — JSON
// ===========================================================================

fn seat1_summary(rows: &[Seat1Row]) -> serde_json::Value {
    let old_absorb = rows.iter().filter(|r| r.old_absorb).count();
    let new_match = rows
        .iter()
        .filter(|r| r.new_bucket == NewBucket::Match)
        .count();
    let new_grey = rows
        .iter()
        .filter(|r| r.new_bucket == NewBucket::Grey)
        .count();
    let new_conflict = rows
        .iter()
        .filter(|r| r.new_bucket == NewBucket::Conflict)
        .count();
    let new_veto = rows
        .iter()
        .filter(|r| r.new_bucket == NewBucket::VetoVolume)
        .count();
    let new_no = rows
        .iter()
        .filter(|r| r.new_bucket == NewBucket::NoMatch)
        .count();
    let changed = rows.iter().filter(|r| r.changed).count();
    let class_a = rows
        .iter()
        .filter(|r| r.class == "old_absorbed_new_blocks")
        .count();
    let class_b = rows
        .iter()
        .filter(|r| r.class == "new_matches_old_didnt")
        .count();
    json!({
        "total_pairs": rows.len(),
        "old_absorb_count": old_absorb,
        "new_match_count": new_match,
        "new_grey_count": new_grey,
        "new_conflict_count": new_conflict,
        "new_veto_volume_count": new_veto,
        "new_no_match_count": new_no,
        "changed_count": changed,
        "old_absorbed_new_blocks": class_a,
        "new_matches_old_didnt": class_b,
    })
}

fn seat1_changed_json(rows: &[Seat1Row]) -> Vec<serde_json::Value> {
    rows.iter()
        .filter(|r| r.changed)
        .map(|r| {
            json!({
                "work_a": pair_ref_json(&r.a),
                "work_b": pair_ref_json(&r.b),
                "old_verdict": if r.old_absorb { "absorb" } else { "no_absorb" },
                "old_reason": r.old_reason,
                "new_verdict": r.new_bucket.label(),
                "new_reason": r.new_reason,
                "id_verdict": describe_id_verdict(r.id_verdict),
                "class": r.class,
            })
        })
        .collect()
}

/// Every pair the new authority lands in the grey band, changed or not —
/// the D2 grey-zone accuracy baseline requires judging each grey pair on
/// its merits, so the pairs themselves are listed, not just counted.
fn seat1_grey_json(rows: &[Seat1Row]) -> Vec<serde_json::Value> {
    rows.iter()
        .filter(|r| r.new_bucket == NewBucket::Grey)
        .map(|r| {
            json!({
                "work_a": pair_ref_json(&r.a),
                "work_b": pair_ref_json(&r.b),
                "old_verdict": if r.old_absorb { "absorb" } else { "no_absorb" },
                "new_reason": r.new_reason,
                "id_verdict": describe_id_verdict(r.id_verdict),
            })
        })
        .collect()
}

/// Every pair the new authority calls a CONFLICT: same main title but a
/// same-provider work-key contradiction (REQ-006) — contradictory identity
/// evidence the library already holds, surfaced distinctly from grey.
fn seat1_conflict_json(rows: &[Seat1Row]) -> Vec<serde_json::Value> {
    rows.iter()
        .filter(|r| r.new_bucket == NewBucket::Conflict)
        .map(|r| {
            json!({
                "work_a": pair_ref_json(&r.a),
                "work_b": pair_ref_json(&r.b),
                "old_verdict": if r.old_absorb { "absorb" } else { "no_absorb" },
                "new_reason": r.new_reason,
                "id_verdict": describe_id_verdict(r.id_verdict),
            })
        })
        .collect()
}

fn seat2_summary(rows: &[Seat2Row]) -> serde_json::Value {
    let old = rows.iter().filter(|r| r.old_match).count();
    let new = rows.iter().filter(|r| r.new_match).count();
    let xor = rows.iter().filter(|r| r.old_match != r.new_match).count();
    json!({
        "total_pairs": rows.len(),
        "old_collide_count": old,
        "new_collide_count": new,
        "collide_under_exactly_one": xor,
    })
}

fn seat2_changed_json(rows: &[Seat2Row]) -> Vec<serde_json::Value> {
    rows.iter()
        .filter(|r| r.old_match != r.new_match)
        .map(|r| {
            json!({
                "work_a": pair_ref_json(&r.a),
                "work_b": pair_ref_json(&r.b),
                "old_collides": r.old_match,
                "new_collides": r.new_match,
                "reason": r.reason,
                "id_verdict": describe_id_verdict(r.id_verdict),
            })
        })
        .collect()
}

fn seat3_summary(rows: &[Seat3Row]) -> serde_json::Value {
    let old = rows.iter().filter(|r| r.old_match).count();
    let new = rows.iter().filter(|r| r.new_match).count();
    let xor = rows.iter().filter(|r| r.old_match != r.new_match).count();
    json!({
        "total_pairs": rows.len(),
        "old_collide_count": old,
        "new_collide_count": new,
        "collide_under_exactly_one": xor,
    })
}

fn seat3_changed_json(rows: &[Seat3Row]) -> Vec<serde_json::Value> {
    rows.iter()
        .filter(|r| r.old_match != r.new_match)
        .map(|r| {
            json!({
                "work_a": pair_ref_json(&r.a),
                "work_b": pair_ref_json(&r.b),
                "old_collides": r.old_match,
                "new_collides": r.new_match,
                "reason": r.reason,
                "id_verdict": describe_id_verdict(r.id_verdict),
            })
        })
        .collect()
}

fn seat4_summary(rows: &[Seat4Row]) -> serde_json::Value {
    let old_forced = rows.iter().filter(|r| r.old_forced).count();
    let new_same = rows.iter().filter(|r| r.new_same).count();
    let changed = rows.iter().filter(|r| r.changed).count();
    json!({
        "total_pairs": rows.len(),
        "old_forced_count": old_forced,
        "new_same_count": new_same,
        "changed_count": changed,
        "old_forced_new_blocks": changed,
    })
}

fn seat4_changed_json(rows: &[Seat4Row]) -> Vec<serde_json::Value> {
    rows.iter()
        .filter(|r| r.changed)
        .map(|r| {
            json!({
                "work_a": pair_ref_json(&r.a),
                "work_b": pair_ref_json(&r.b),
                "old_full_score": r.old_full_score,
                "old_forced_to_1_0": r.old_forced,
                "new_verdict": format!("{:?}", r.new_verdict),
                "class": r.class,
            })
        })
        .collect()
}

fn seat5_summary(rows: &[Seat5Row], default_language: &str) -> serde_json::Value {
    let neutral = rows
        .iter()
        .filter(|r| r.verdict == LanguageVerdict::Neutral)
        .count();
    let grey = rows
        .iter()
        .filter(|r| r.verdict == LanguageVerdict::Grey)
        .count();
    let mut by_lang: BTreeMap<String, i64> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.verdict == LanguageVerdict::Grey) {
        *by_lang
            .entry(r.language.clone().unwrap_or_else(|| "?".into()))
            .or_insert(0) += 1;
    }
    json!({
        "total_works": rows.len(),
        "default_language": default_language,
        "neutral_count": neutral,
        "grey_count": grey,
        "grey_by_language": by_lang,
    })
}

fn seat5_grey_json(rows: &[Seat5Row]) -> Vec<serde_json::Value> {
    rows.iter()
        .filter(|r| r.verdict == LanguageVerdict::Grey)
        .map(|r| json!({ "work": pair_ref_json(&r.work), "language": r.language }))
        .collect()
}

// ===========================================================================
// Report assembly — markdown (the PO-facing report)
// ===========================================================================

struct ReportData<'a> {
    generated_at: &'a str,
    snapshot_path: &'a str,
    work_count: usize,
    user_count: usize,
    default_language: &'a str,
    seat1: &'a [Seat1Row],
    seat2: &'a [Seat2Row],
    seat3: &'a [Seat3Row],
    seat4: &'a [Seat4Row],
    seat5: &'a [Seat5Row],
}

fn render_markdown(d: &ReportData) -> String {
    let mut md = String::new();
    md.push_str("# Matching Diff — Old vs New Identity Authority\n\n");
    md.push_str(&format!(
        "Generated {} from snapshot `{}`. {} works across {} user(s). Install default \
         language: `{}`.\n\n",
        d.generated_at, d.snapshot_path, d.work_count, d.user_count, d.default_language
    ));
    md.push_str(
        "This is the Phase 5 Unit C harness (REQ-018/D8): a zero-write, offline \
         comparison of what the OLD matching code decides vs what the NEW identity \
         authority decides, run against a snapshot copy of the real library. Nothing on \
         this page changed any data. Every table below lists ONLY pairs where the two \
         systems disagree — pairs both systems already treat the same way are omitted \
         for readability, but are counted in the summaries and listed in full in \
         `matching-diff.json`.\n\n",
    );

    // --- Seat 1 ---
    md.push_str("## Seat 1 — Library dedup / absorb\n\n");
    md.push_str(
        "Would the app silently merge one work into another? The new verdict combines \
         title, author, and identifier evidence. **Class old_absorbed_new_blocks** is \
         the June-incident shape: the OLD code would have absorbed, the NEW authority \
         blocks it (veto, conflict, or grey) — these are wins. **Class \
         new_matches_old_didnt** is the OLD code refusing something the NEW authority \
         would now allow; should be rare, scrutinize each one. **Conflict** pairs \
         carry contradictory identity evidence (same title, different same-provider \
         work keys) — never merged, listed separately below.\n\n",
    );
    let s1_changed: Vec<&Seat1Row> = d.seat1.iter().filter(|r| r.changed).collect();
    md.push_str(&format!(
        "- Pairs compared: {}\n- OLD would absorb: {}\n- NEW would auto-match: {}\n\
         - NEW flags as conflict: {}\n\
         - Changed decisions: {} (old_absorbed_new_blocks: {}; new_matches_old_didnt: {})\n\n",
        d.seat1.len(),
        d.seat1.iter().filter(|r| r.old_absorb).count(),
        d.seat1
            .iter()
            .filter(|r| r.new_bucket == NewBucket::Match)
            .count(),
        d.seat1
            .iter()
            .filter(|r| r.new_bucket == NewBucket::Conflict)
            .count(),
        s1_changed.len(),
        d.seat1
            .iter()
            .filter(|r| r.class == "old_absorbed_new_blocks")
            .count(),
        d.seat1
            .iter()
            .filter(|r| r.class == "new_matches_old_didnt")
            .count(),
    ));
    if s1_changed.is_empty() {
        md.push_str("No changed decisions at this seat.\n\n");
    } else {
        md.push_str(
            "| Work A | Work B | Old | New | Reason | Class |\n|---|---|---|---|---|---|\n",
        );
        for r in &s1_changed {
            md.push_str(&format!(
                "| {} — {} | {} — {} | {} | {} | {} ({}) | {} |\n",
                md_escape(&r.a.title),
                md_escape(&r.a.author),
                md_escape(&r.b.title),
                md_escape(&r.b.author),
                if r.old_absorb { "absorb" } else { "no" },
                r.new_bucket.label(),
                md_escape(&r.new_reason),
                describe_id_verdict(r.id_verdict),
                r.class,
            ));
        }
        md.push('\n');
    }

    let s1_grey: Vec<&Seat1Row> = d
        .seat1
        .iter()
        .filter(|r| r.new_bucket == NewBucket::Grey)
        .collect();
    if !s1_grey.is_empty() {
        md.push_str(
            "Grey pairs — every pair the new authority flags as \"likely but not \
             certain\". The action at this seat is unchanged (grey never absorbs, and \
             the old code didn't match these either unless listed above) — these are \
             listed so each can be judged on its merits as the D2 grey-zone accuracy \
             baseline.\n\n",
        );
        md.push_str("| Work A | Work B | Old | Why grey |\n|---|---|---|---|\n");
        for r in &s1_grey {
            md.push_str(&format!(
                "| {} — {} | {} — {} | {} | {} ({}) |\n",
                md_escape(&r.a.title),
                md_escape(&r.a.author),
                md_escape(&r.b.title),
                md_escape(&r.b.author),
                if r.old_absorb { "absorb" } else { "no match" },
                md_escape(&r.new_reason),
                describe_id_verdict(r.id_verdict),
            ));
        }
        md.push('\n');
    }

    let s1_conflict: Vec<&Seat1Row> = d
        .seat1
        .iter()
        .filter(|r| r.new_bucket == NewBucket::Conflict)
        .collect();
    if !s1_conflict.is_empty() {
        md.push_str(
            "Conflict pairs — same main title but contradictory same-provider work \
             keys: the library already holds two different identity claims for one \
             title. Never merged; each of these is worth a metadata look (one side's \
             attribution is likely wrong).\n\n",
        );
        md.push_str("| Work A | Work B | Old | Evidence |\n|---|---|---|---|\n");
        for r in &s1_conflict {
            md.push_str(&format!(
                "| {} — {} | {} — {} | {} | {} ({}) |\n",
                md_escape(&r.a.title),
                md_escape(&r.a.author),
                md_escape(&r.b.title),
                md_escape(&r.b.author),
                if r.old_absorb { "absorb" } else { "no match" },
                md_escape(&r.new_reason),
                describe_id_verdict(r.id_verdict),
            ));
        }
        md.push('\n');
    }

    // --- Seat 2 ---
    md.push_str("## Seat 2 — DB identity key (`works.normalized_title`/`normalized_author`)\n\n");
    md.push_str(
        "This is the raw uniqueness key the database already enforces today (no two \
         works can share it), so OLD can never show a collision between two distinct \
         existing works — the interesting number is how many pairs the NEW key (main \
         title + author) would now treat as the same identity that the OLD key kept \
         apart. Those are latent near-duplicates already sitting in the library; the \
         planned identity_key migration (Unit E) will need to reconcile them.\n\n",
    );
    let s2_xor: Vec<&Seat2Row> = d
        .seat2
        .iter()
        .filter(|r| r.old_match != r.new_match)
        .collect();
    md.push_str(&format!(
        "- Pairs compared: {}\n- OLD key collides: {}\n- NEW key would collide: {}\n\
         - Collide under exactly one: {}\n\n",
        d.seat2.len(),
        d.seat2.iter().filter(|r| r.old_match).count(),
        d.seat2.iter().filter(|r| r.new_match).count(),
        s2_xor.len(),
    ));
    if s2_xor.is_empty() {
        md.push_str("No pairs collide under exactly one key.\n\n");
    } else {
        md.push_str("| Work A | Work B | Old key | New key | Reason |\n|---|---|---|---|---|\n");
        for r in &s2_xor {
            md.push_str(&format!(
                "| {} — {} | {} — {} | {} | {} | {} |\n",
                md_escape(&r.a.title),
                md_escape(&r.a.author),
                md_escape(&r.b.title),
                md_escape(&r.b.author),
                if r.old_match { "collide" } else { "distinct" },
                if r.new_match { "collide" } else { "distinct" },
                md_escape(&r.reason),
            ));
        }
        md.push('\n');
    }

    // --- Seat 3 ---
    md.push_str("## Seat 3 — Strict already-in-library key\n\n");
    md.push_str(
        "Governs the bibliography \"already in library\" flag and the anchor-graft / \
         cover-borrow same-work checks. Same reading as Seat 2: the interesting number \
         is pairs the NEW key would now treat as one identity that the OLD colon-cut key \
         kept apart (or vice versa).\n\n",
    );
    let s3_xor: Vec<&Seat3Row> = d
        .seat3
        .iter()
        .filter(|r| r.old_match != r.new_match)
        .collect();
    md.push_str(&format!(
        "- Pairs compared: {}\n- OLD key collides: {}\n- NEW key would collide: {}\n\
         - Collide under exactly one: {}\n\n",
        d.seat3.len(),
        d.seat3.iter().filter(|r| r.old_match).count(),
        d.seat3.iter().filter(|r| r.new_match).count(),
        s3_xor.len(),
    ));
    if s3_xor.is_empty() {
        md.push_str("No pairs collide under exactly one key.\n\n");
    } else {
        md.push_str("| Work A | Work B | Old key | New key | Reason |\n|---|---|---|---|---|\n");
        for r in &s3_xor {
            md.push_str(&format!(
                "| {} — {} | {} — {} | {} | {} | {} |\n",
                md_escape(&r.a.title),
                md_escape(&r.a.author),
                md_escape(&r.b.title),
                md_escape(&r.b.author),
                if r.old_match { "collide" } else { "distinct" },
                if r.new_match { "collide" } else { "distinct" },
                md_escape(&r.reason),
            ));
        }
        md.push('\n');
    }

    // --- Seat 4 ---
    md.push_str("## Seat 4 — Variant-fold (same-series sibling volumes)\n\n");
    md.push_str(
        "The OLD scorer treats two titles as a CERTAIN match (score forced to 1.0) \
         whenever they fold to the same key — even when a known series position says \
         they're different volumes, as long as ONE side's position is missing. That's \
         the \"position-guard hole\". The measured population is the pairs where the \
         fold actually changed the old scorer's answer (forced a sub-1.0 score up to \
         1.0); a changed decision is a forced pair the NEW authority no longer calls \
         the same title (veto, grey, or different) — OLD was certain, NEW says no.\n\n",
    );
    let s4_changed: Vec<&Seat4Row> = d.seat4.iter().filter(|r| r.changed).collect();
    md.push_str(&format!(
        "- Pairs compared: {}\n- OLD forces a 1.0 match: {}\n- NEW says Same: {}\n\
         - Changed decisions: {}\n\n",
        d.seat4.len(),
        d.seat4.iter().filter(|r| r.old_forced).count(),
        d.seat4.iter().filter(|r| r.new_same).count(),
        s4_changed.len(),
    ));
    if s4_changed.is_empty() {
        md.push_str("No changed decisions at this seat.\n\n");
    } else {
        md.push_str(
            "| Work A | Work B | Old full score | Old forced? | New verdict | Class |\n\
             |---|---|---|---|---|---|\n",
        );
        for r in &s4_changed {
            md.push_str(&format!(
                "| {} — {} | {} — {} | {:.2} | {} | {:?} | {} |\n",
                md_escape(&r.a.title),
                md_escape(&r.a.author),
                md_escape(&r.b.title),
                md_escape(&r.b.author),
                r.old_full_score,
                if r.old_forced { "yes" } else { "no" },
                r.new_verdict,
                r.class,
            ));
        }
        md.push('\n');
    }

    // --- Seat 5 ---
    md.push_str("## Seat 5 — Language census (new-only; no OLD equivalent exists today)\n\n");
    md.push_str(
        "For every work: if a provider ever returns a payload with NO language field at \
         all, would the new authority treat that as fine (Neutral, auto-applies) or flag \
         it for review (Grey, parked)? This previews the size of REQ-007's new grey zone \
         before it ships.\n\n",
    );
    let s5_grey: Vec<&Seat5Row> = d
        .seat5
        .iter()
        .filter(|r| r.verdict == LanguageVerdict::Grey)
        .collect();
    md.push_str(&format!(
        "- Works censused: {}\n- Neutral (silent payload would auto-apply): {}\n\
         - Grey (silent payload would park for review): {}\n\n",
        d.seat5.len(),
        d.seat5
            .iter()
            .filter(|r| r.verdict == LanguageVerdict::Neutral)
            .count(),
        s5_grey.len(),
    ));
    if !s5_grey.is_empty() {
        md.push_str("| Work | Language |\n|---|---|\n");
        for r in &s5_grey {
            md.push_str(&format!(
                "| {} — {} | {} |\n",
                md_escape(&r.work.title),
                md_escape(&r.work.author),
                r.language.as_deref().unwrap_or("?"),
            ));
        }
        md.push('\n');
    }

    md
}

// ===========================================================================
// Orchestration
// ===========================================================================

#[tokio::test]
#[ignore = "manual: point MATCHING_DIFF_DB_PATH at a read-only snapshot copy, never \
            the live DB; run with -- --ignored --nocapture"]
async fn matching_diff_harness() {
    let db_path = std::env::var("MATCHING_DIFF_DB_PATH").expect(
        "set MATCHING_DIFF_DB_PATH to a snapshot copy \
         (e.g. `sqlite3 live.db \".backup 'snap.db'\"`) — never the live database",
    );
    let out_dir = std::env::var("MATCHING_DIFF_OUT_DIR").unwrap_or_else(|_| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../reports")
            .to_string_lossy()
            .to_string()
    });
    std::fs::create_dir_all(&out_dir).expect("create report output directory");

    let pool = open_snapshot_readonly(Path::new(&db_path)).await;
    let works = load_works(&pool).await;
    let default_language = load_default_language(&pool).await;

    let by_user = group_by_user(&works);
    let user_count = by_user.len();

    let mut seat1 = Vec::new();
    let mut seat2 = Vec::new();
    let mut seat3 = Vec::new();
    let mut seat4 = Vec::new();
    for user_works in by_user.values() {
        for (i, j) in pairs(user_works) {
            let a = user_works[i];
            let b = user_works[j];
            seat1.push(seat1_pair(a, b));
            seat2.push(seat2_pair(a, b));
            seat3.push(seat3_pair(a, b));
            seat4.push(seat4_pair(a, b));
        }
    }
    let seat5 = seat5_census(&works, &default_language);

    let generated_at = chrono::Utc::now().to_rfc3339();

    let report = json!({
        "generated_at": generated_at,
        "snapshot_path": db_path,
        "work_count": works.len(),
        "user_count": user_count,
        "default_language": default_language,
        "seats": {
            "1_dedup_absorb": {
                "summary": seat1_summary(&seat1),
                "changed": seat1_changed_json(&seat1),
                "grey_pairs": seat1_grey_json(&seat1),
                "conflict_pairs": seat1_conflict_json(&seat1),
            },
            "2_db_identity_key": {
                "summary": seat2_summary(&seat2),
                "changed": seat2_changed_json(&seat2),
            },
            "3_already_in_library_key": {
                "summary": seat3_summary(&seat3),
                "changed": seat3_changed_json(&seat3),
            },
            "4_variant_fold": {
                "summary": seat4_summary(&seat4),
                "changed": seat4_changed_json(&seat4),
            },
            "5_language_census": {
                "summary": seat5_summary(&seat5, &default_language),
                "grey_examples": seat5_grey_json(&seat5),
            },
        },
    });

    let json_path = Path::new(&out_dir).join("matching-diff.json");
    std::fs::write(
        &json_path,
        serde_json::to_string_pretty(&report).expect("serialize report"),
    )
    .expect("write matching-diff.json");

    let md = render_markdown(&ReportData {
        generated_at: &generated_at,
        snapshot_path: &db_path,
        work_count: works.len(),
        user_count,
        default_language: &default_language,
        seat1: &seat1,
        seat2: &seat2,
        seat3: &seat3,
        seat4: &seat4,
        seat5: &seat5,
    });
    let md_path = Path::new(&out_dir).join("matching-diff.md");
    std::fs::write(&md_path, md).expect("write matching-diff.md");

    println!("wrote {json_path:?}");
    println!("wrote {md_path:?}");
    println!(
        "seat1(dedup): {} pairs, {} changed | seat2(db key): {} xor | \
         seat3(lib key): {} xor | seat4(variant): {} changed | seat5(lang): {} grey / {} works",
        seat1.len(),
        seat1.iter().filter(|r| r.changed).count(),
        seat2.iter().filter(|r| r.old_match != r.new_match).count(),
        seat3.iter().filter(|r| r.old_match != r.new_match).count(),
        seat4.iter().filter(|r| r.changed).count(),
        seat5
            .iter()
            .filter(|r| r.verdict == LanguageVerdict::Grey)
            .count(),
        seat5.len(),
    );
}
