//! Identity-grade matching authority: the single vocabulary for deciding
//! whether two records describe the same book.
//!
//! Pure functions only — no I/O, no async, no database access. Callers parse
//! each title once into a [`ParsedTitle`], then combine the per-dimension
//! verdicts ([`TitleVerdict`], [`AuthorVerdict`], [`LanguageVerdict`],
//! [`IdVerdict`]) at their own policy seat.
//!
//! Title model: parse, don't truncate. A title splits into a main title and a
//! tail (subtitle / series marker / edition junk). Only main titles can make
//! two records match. Tails can veto (conflicting volume numbers), demote to
//! grey (substantive disagreement or a tail on one side only), or be ignored
//! (edition junk). A tail that cannot be confidently classified is treated as
//! a true subtitle — the safest class: it can only demote, never veto, never
//! silently pass.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::text_norm;
use crate::title_cleanup::{self, collapse_whitespace, normalize_last_first};

/// Token-set Jaccard floor below which two main titles are considered
/// different rather than grey candidates.
pub const TITLE_GREY_FLOOR: f64 = 0.75;

/// A series/volume marker extracted from a title tail or a trailing
/// parenthetical, e.g. "Book 3", "Vol. IV", "(The Dresden Files, #1)".
///
/// The marker's text never participates in title comparison; only the
/// extracted number carries signal.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesMarker {
    /// Series name text accompanying the marker, when present
    /// ("The Dresden Files" in "The Dresden Files, Book 1"). Canonical
    /// lowercase form; never compared as title text.
    pub series: Option<String>,
    /// The extracted volume/position number.
    pub number: f64,
}

/// A title decomposed for identity comparison. All fields hold canonical
/// matching forms (lowercased, accent-stripped, punctuation-folded), not
/// display text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedTitle {
    /// Canonical main title — the only part that can make a match.
    pub main: String,
    /// True subtitle: substantive tail text that is neither a series marker
    /// nor recognized edition junk. Can only demote a pair to grey.
    pub subtitle: Option<String>,
    /// Series/volume markers extracted from the tail and trailing
    /// parentheticals. Conflicting numbers veto a match.
    pub series_markers: Vec<SeriesMarker>,
    /// Recognized edition-junk phrases ("a novel", "unabridged", …).
    /// Ignored for identity.
    pub junk: Vec<String>,
}

impl ParsedTitle {
    /// Volume numbers carried by this title's series markers.
    pub fn volume_numbers(&self) -> Vec<f64> {
        self.series_markers.iter().map(|m| m.number).collect()
    }
}

/// Why a title pair landed in the grey band instead of `Same`. Computed once,
/// inside the verdict; the highest-risk trigger wins when several co-occur
/// (`VolumeAsymmetry` > `SubtitleDisagreement` > `OneSidedSubtitle`), so
/// `OneSidedSubtitle` always means *solely* that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GreyCause {
    /// Equal mains; the only demotion trigger was a true subtitle on exactly
    /// one side. No volume asymmetry, no subtitle disagreement.
    OneSidedSubtitle,
    /// Equal mains; both sides carry true subtitles and they differ.
    SubtitleDisagreement,
    /// Equal mains; volume evidence on exactly one side.
    VolumeAsymmetry,
    /// Mains not equal; similarity at or above [`TITLE_GREY_FLOOR`].
    NearMain,
}

/// Identity-grade comparison of two parsed titles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TitleVerdict {
    /// Cleaned main titles are exactly equal and no tail evidence conflicts.
    Same,
    /// Close but not certain: near-equal mains (token-set Jaccard at or above
    /// [`TITLE_GREY_FLOOR`]), or exact mains demoted by tail evidence
    /// (one-sided tail, disagreeing subtitles, one-sided volume info).
    /// `score` is the computed main-title token-set Jaccard; `cause` names
    /// the demotion trigger.
    Grey { score: f64, cause: GreyCause },
    /// Below the grey floor, or no usable title on either side.
    Different,
    /// Conflicting volume numbers (from parsed tails or caller-supplied
    /// series positions): a hard stop regardless of title similarity.
    VetoVolume,
}

/// Identity-grade comparison of two credited-author lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AuthorVerdict {
    /// At least one unambiguous full-name match. Extra credited names on
    /// either side are non-evidence and never subtract.
    Agree,
    /// Shared surname without a full-name match, or initials compatible with
    /// more than one candidate name.
    Grey,
    /// No overlap of any kind.
    Disagree,
    /// One or both sides carry no usable author names.
    Abstain,
}

/// Language compatibility between a work and an incoming payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageVerdict {
    /// Both sides declare a language and they differ: never merge.
    Veto,
    /// One side is silent and the declared side differs from the user's
    /// default language: eligible for review, never auto-apply.
    Grey,
    /// Declared and equal, silent-but-default, or both silent.
    Neutral,
}

/// Identifier evidence for one record. Work-level keys identify the work;
/// edition-level ids (ISBN/ASIN) identify a printing of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IdEvidence<'a> {
    pub ol_key: Option<&'a str>,
    pub gr_key: Option<&'a str>,
    pub hc_key: Option<&'a str>,
    pub isbn_13: Option<&'a str>,
    pub asin: Option<&'a str>,
}

/// Identifier-level comparison of two records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdVerdict {
    /// A same-provider work key matches: the strongest positive evidence.
    WorkKeyEqual,
    /// A same-provider work key differs on the two sides: a hard veto that
    /// outranks any edition-id agreement (a shared ISBN with contradicting
    /// work keys is an ISBN collision, never a merge).
    WorkKeyContradiction,
    /// No work-key evidence either way, but a shared ISBN or ASIN bridges
    /// the pair as editions of one work.
    EditionBridge,
    /// No identifier evidence. Edition-id inequality lands here too: two
    /// editions of one work legitimately carry different ISBNs/ASINs, so
    /// inequality is no evidence and never vetoes.
    NoEvidence,
}

/// Decompose a raw title into its canonical matching parts.
pub fn parse_title(raw: &str) -> ParsedTitle {
    let mut markers: Vec<SeriesMarker> = Vec::new();
    let mut junk: Vec<String> = Vec::new();
    let mut subtitle_parts: Vec<String> = Vec::new();

    let mut rest = collapse_whitespace(raw);

    // Peel trailing parentheticals right-to-left, classifying each.
    loop {
        let found = title_cleanup::RE_TRAILING_PAREN
            .captures(&rest)
            .map(|caps| {
                (
                    caps.get(1)
                        .map(|m| m.as_str().trim().to_string())
                        .unwrap_or_default(),
                    caps.get(0).map(|m| m.start()).unwrap_or(0),
                )
            });
        let Some((inner, start)) = found else { break };
        classify_paren(&inner, &mut markers, &mut junk, &mut subtitle_parts);
        rest = rest[..start].trim_end().to_string();
        if rest.is_empty() {
            break;
        }
    }

    // Split main from tail at the first unambiguous separator.
    let (main_part, tail_part) = split_main_tail(&rest);
    let mut main_raw = main_part.to_string();
    let tail_owned = tail_part.map(str::to_string);

    // A trailing comma-led volume marker binds to the main ("Foo, Vol. 3").
    if let Some((prefix, marker)) = extract_comma_marker(&main_raw) {
        main_raw = prefix;
        markers.push(marker);
    }

    if let Some(tail) = tail_owned {
        classify_tail(&tail, &mut markers, &mut junk, &mut subtitle_parts);
    }

    let subtitle = {
        let joined = subtitle_parts.join(" ");
        let canonical = canonical_phrase(&joined);
        (!canonical.is_empty()).then_some(canonical)
    };

    ParsedTitle {
        main: canonical_phrase(&main_raw),
        subtitle,
        series_markers: markers,
        junk,
    }
}

/// Compare two parsed titles at identity grade, using only volume evidence
/// carried by the titles themselves.
pub fn title_verdict(a: &ParsedTitle, b: &ParsedTitle) -> TitleVerdict {
    title_verdict_with_positions(a, None, b, None)
}

/// Compare two parsed titles at identity grade, folding in caller-supplied
/// series positions (e.g. from series metadata) as additional volume
/// evidence.
pub fn title_verdict_with_positions(
    a: &ParsedTitle,
    a_position: Option<f64>,
    b: &ParsedTitle,
    b_position: Option<f64>,
) -> TitleVerdict {
    let mut volumes_a = a.volume_numbers();
    volumes_a.extend(a_position);
    let mut volumes_b = b.volume_numbers();
    volumes_b.extend(b_position);

    if volumes_conflict(&volumes_a, &volumes_b) {
        return TitleVerdict::VetoVolume;
    }
    if a.main.is_empty() || b.main.is_empty() {
        return TitleVerdict::Different;
    }

    let score = text_norm::jaccard(
        &text_norm::title_tokens(&a.main),
        &text_norm::title_tokens(&b.main),
    );

    if a.main == b.main {
        let one_sided_volume = volumes_a.is_empty() != volumes_b.is_empty();
        let subtitles_demote = match (&a.subtitle, &b.subtitle) {
            (Some(x), Some(y)) => x != y,
            (None, None) => false,
            _ => true,
        };
        if one_sided_volume || subtitles_demote {
            // Highest-risk trigger wins, so OneSidedSubtitle means solely that.
            let cause = if one_sided_volume {
                GreyCause::VolumeAsymmetry
            } else if a.subtitle.is_some() && b.subtitle.is_some() {
                GreyCause::SubtitleDisagreement
            } else {
                GreyCause::OneSidedSubtitle
            };
            return TitleVerdict::Grey { score, cause };
        }
        return TitleVerdict::Same;
    }

    if score >= TITLE_GREY_FLOOR {
        return TitleVerdict::Grey {
            score,
            cause: GreyCause::NearMain,
        };
    }
    TitleVerdict::Different
}

/// Raw-name cap per side for [`author_verdict`] (D3 / PRINCIPLES.md §5): a
/// side carrying more than this many raw credited-name strings abstains
/// rather than building comparison state proportional to an unbounded
/// input. Chosen so the worst-case comparison count
/// (`AUTHOR_VERDICT_MAX_NAMES_PER_SIDE`^2 = 65,536) stays small.
pub const AUTHOR_VERDICT_MAX_NAMES_PER_SIDE: usize = 256;

/// Compare two credited-author lists at identity grade.
///
/// Bounded (D3): more than [`AUTHOR_VERDICT_MAX_NAMES_PER_SIDE`] raw names
/// on either side abstains outright — the cap is checked before any
/// canonicalization or comparison work. Below the cap, this preserves the
/// exact compatibility-based verdict the naive O(N*M)-pair-vector
/// implementation computed (see the `author_verdict_matches_the_naive_authority_*`
/// tests below): the row/column saturating counts plus each row's first
/// matching column let an `Agree` pair be recovered without materializing
/// every matching pair, since a row with exactly one match has, by definition,
/// only the one column its saturating count already implies.
pub fn author_verdict(a: &[String], b: &[String]) -> AuthorVerdict {
    if a.len() > AUTHOR_VERDICT_MAX_NAMES_PER_SIDE || b.len() > AUTHOR_VERDICT_MAX_NAMES_PER_SIDE {
        return AuthorVerdict::Abstain;
    }

    let ca: Vec<CanonicalName> = a.iter().filter_map(|n| canonical_author_name(n)).collect();
    let cb: Vec<CanonicalName> = b.iter().filter_map(|n| canonical_author_name(n)).collect();
    if ca.is_empty() || cb.is_empty() {
        return AuthorVerdict::Abstain;
    }

    // A full-name match pair counts only when it is unambiguous on both
    // sides; a name compatible with several candidates is grey evidence.
    // O(N+M) auxiliary memory: row/column saturating counts, plus each
    // row's FIRST matching column — sufficient because a row whose count
    // is exactly 1 has, definitionally, only that one matching column, so
    // "first match" and "only match" coincide exactly when it matters.
    let mut row_counts = vec![0usize; ca.len()];
    let mut col_counts = vec![0usize; cb.len()];
    let mut row_first_match: Vec<Option<usize>> = vec![None; ca.len()];
    let mut any_match = false;
    for (i, x) in ca.iter().enumerate() {
        for (j, y) in cb.iter().enumerate() {
            if full_name_match(x, y) {
                row_counts[i] = row_counts[i].saturating_add(1);
                col_counts[j] = col_counts[j].saturating_add(1);
                row_first_match[i].get_or_insert(j);
                any_match = true;
            }
        }
    }
    let agrees = (0..ca.len()).any(|i| {
        row_counts[i] == 1
            && row_first_match[i]
                .map(|j| col_counts[j] == 1)
                .unwrap_or(false)
    });
    if agrees {
        return AuthorVerdict::Agree;
    }
    if any_match {
        return AuthorVerdict::Grey;
    }
    let shared_surname = ca.iter().any(|x| cb.iter().any(|y| x.surname == y.surname));
    if shared_surname {
        AuthorVerdict::Grey
    } else {
        AuthorVerdict::Disagree
    }
}

/// Shared identity-grade provider hit-picker (matching-conformance unit).
///
/// Chooses the best candidate `(title, author)` pair for a seed, routing the
/// decision through the ONE authority (`parse_title` / `title_verdict` /
/// `author_verdict`) — the single replacement for the loose whole-string
/// 0.75-jaccard pickers (`score_provider_candidates`, `score_candidates`) and
/// the home of `gr_best_match`'s selection logic (REQ-001 / AC-001).
///
/// `accept_grey`:
/// * `false` — Same-only. The four newly-conformed pickers (Audible, OpenLibrary,
///   Hardcover, Google Books) pass this: a `Grey` title match never becomes their
///   provider answer, because those payloads reach a background field-merge via the
///   anchor-only reuse cache and REQ-008 forbids writing grey-matched provider data.
/// * `true` — Same-or-Grey. Goodreads only: its grey subtitled-from-bare picks are
///   the input the ratified `verify_gr_payload` / AC-004 corroboration hatch consumes
///   (settle-road unit), and are gated downstream, not here.
///
/// Author bar: a `Same` title accepts `author_verdict ∈ {Agree, Abstain}`; a `Grey`
/// title requires `Agree` strictly (any-shared-token dies — D9 / AC-008). Ranking:
/// `Same` beats `Grey`, higher grey score beats lower, earliest hit breaks ties.
/// Returns `None` when nothing clears the bar (the provider abstains).
pub fn pick_best_candidate(
    seed_title: &str,
    seed_author: &str,
    candidates: &[(String, String)],
    accept_grey: bool,
) -> Option<usize> {
    let seed_parsed = parse_title(seed_title);
    let seed_authors = [seed_author.to_string()];
    // (index, tier: Same=2 / Grey=1, grey score)
    let mut best: Option<(usize, u8, f64)> = None;
    for (idx, (cand_title, cand_author)) in candidates.iter().enumerate() {
        let (tier, score) = match title_verdict(&seed_parsed, &parse_title(cand_title)) {
            TitleVerdict::Same => {
                if matches!(
                    author_verdict(&seed_authors, std::slice::from_ref(cand_author)),
                    AuthorVerdict::Agree | AuthorVerdict::Abstain
                ) {
                    (2u8, 1.0)
                } else {
                    continue;
                }
            }
            TitleVerdict::Grey { score, .. } => {
                if accept_grey
                    && matches!(
                        author_verdict(&seed_authors, std::slice::from_ref(cand_author)),
                        AuthorVerdict::Agree
                    )
                {
                    (1u8, score)
                } else {
                    continue;
                }
            }
            TitleVerdict::Different | TitleVerdict::VetoVolume => continue,
        };
        let beats = match best {
            None => true,
            Some((_, best_tier, best_score)) => {
                tier > best_tier || (tier == best_tier && score > best_score)
            }
        };
        if beats {
            best = Some((idx, tier, score));
        }
    }
    best.map(|(idx, ..)| idx)
}

/// Exactly-one-unambiguous-match author adoption gate (author-dedup).
///
/// `Some(i)` iff `candidate` adoption-matches `stored[i]` and nothing else.
/// Adoption matching is deliberately TIGHTER than [`full_name_match`]: given
/// equal canonical surnames, it matches on (1) equal given-token counts with
/// pairwise initial/word compatibility, (2) a symmetric glued-initials
/// reading ("jk" ⇄ "j k") that equalizes counts with first-char-equal pairs,
/// or (3) unequal counts only when every zipped pair is an exact multi-char
/// word — a lone initial never spans unchecked surplus given names. Ambiguity
/// (two or more compatible stored names) refuses: grey never absorbs; the
/// merge action is the recovery for a wrong split.
pub fn unambiguous_author_match(candidate: &str, stored: &[String]) -> Option<usize> {
    let candidate_name = canonical_author_name(candidate)?;
    let mut matches = stored.iter().enumerate().filter_map(|(i, name)| {
        canonical_author_name(name)
            .filter(|stored_name| adoption_match(&candidate_name, stored_name))
            .map(|_| i)
    });
    let first = matches.next()?;
    match matches.next() {
        Some(_) => None,
        None => Some(first),
    }
}

/// Compare a work's language against an incoming payload's declared
/// language, in the context of the user's default language.
pub fn language_verdict(
    work: Option<&str>,
    payload: Option<&str>,
    user_default: &str,
) -> LanguageVerdict {
    let norm = |v: Option<&str>| v.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let default = user_default.trim().to_lowercase();
    match (norm(work), norm(payload)) {
        (Some(w), Some(p)) if w != p => LanguageVerdict::Veto,
        (Some(_), Some(_)) | (None, None) => LanguageVerdict::Neutral,
        (Some(declared), None) | (None, Some(declared)) => {
            if declared == default {
                LanguageVerdict::Neutral
            } else {
                LanguageVerdict::Grey
            }
        }
    }
}

fn present(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|s| !s.is_empty())
}
fn id_eq(x: Option<&str>, y: Option<&str>) -> bool {
    matches!((present(x), present(y)), (Some(p), Some(q)) if p == q)
}
fn id_differs(x: Option<&str>, y: Option<&str>) -> bool {
    matches!((present(x), present(y)), (Some(p), Some(q)) if p != q)
}

/// Any same-provider WORK key (OL/GR/HC) present on both sides and different.
/// Checked per provider over raw evidence: [`id_verdict`]'s equality-first
/// collapse deliberately reports `WorkKeyEqual` for mixed agree+contradict
/// evidence, which a trust decision must not inherit.
fn work_key_contradiction(a: &IdEvidence, b: &IdEvidence) -> bool {
    id_differs(a.ol_key, b.ol_key)
        || id_differs(a.gr_key, b.gr_key)
        || id_differs(a.hc_key, b.hc_key)
}

/// Compare the identifier evidence of two records.
pub fn id_verdict(a: &IdEvidence, b: &IdEvidence) -> IdVerdict {
    if id_eq(a.ol_key, b.ol_key) || id_eq(a.gr_key, b.gr_key) || id_eq(a.hc_key, b.hc_key) {
        return IdVerdict::WorkKeyEqual;
    }
    if work_key_contradiction(a, b) {
        return IdVerdict::WorkKeyContradiction;
    }
    if id_eq(a.isbn_13, b.isbn_13) || id_eq(a.asin, b.asin) {
        return IdVerdict::EditionBridge;
    }
    IdVerdict::NoEvidence
}

/// AC-004/D4/REQ-006 trust shape for a text-corroborated identity: may this
/// pair's identifiers + title evidence be trusted at an identity seat?
/// NOT a full acceptance decision — callers apply their seat's author bar.
/// Takes RAW evidence rather than a collapsed [`IdVerdict`]: the collapsed
/// verdict reports `WorkKeyEqual` as soon as any work key matches, which would
/// mask a contradicting sibling key on a mixed payload.
pub fn title_id_trust(title: &TitleVerdict, a: &IdEvidence, b: &IdEvidence) -> bool {
    // REQ-006: a same-provider work-key contradiction is the collision shape —
    // never auto-same, regardless of title evidence, in both directions.
    if work_key_contradiction(a, b) {
        return false;
    }
    match title {
        TitleVerdict::Same => true,
        // A subtitle is edition-level, not work-level: a work record carries the
        // bare title while an edition record carries title + subtitle, so one
        // side having a subtitle is not evidence of a different work. Demanding
        // an agreeing hard identifier here asked an unanswerable question —
        // `EditionBridge` needs ISBN/ASIN equality between two different
        // printings, which by construction it will not find. Equal main titles
        // stand on their own; every caller applies its own author bar on top.
        TitleVerdict::Grey {
            cause: GreyCause::OneSidedSubtitle,
            ..
        } => true,
        _ => false,
    }
}

/// Deterministic identity key for storage and exact-equality lookups: the
/// title component encodes the FULL parse triple — cleaned main title, true
/// subtitle, and sorted volume-marker numbers — joined by `\u{1}` (a
/// character the cleaned segments can never contain: `canonical_phrase`
/// folds all non-alphanumerics to token boundaries); the author component
/// is the canonical author string (the same name canonicalization
/// `author_verdict` uses under the hood). Built entirely from the
/// authority's own internals. Feeds `works.normalized_title`/
/// `normalized_author` (REQ-014) and any other site that needs a single
/// deterministic string pair rather than a full [`TitleVerdict`]/
/// [`AuthorVerdict`] comparison.
///
/// Why the triple and not the bare main title: the stored key backs a
/// UNIQUE index and an `ON CONFLICT DO NOTHING` create backstop, so two
/// DIFFERENT books must never share a key. Series siblings ("Mistborn: The
/// Final Empire" vs "Mistborn: The Well of Ascension") share a main title
/// and differ only in subtitle/volume — the triple keeps them distinct, so
/// both persist. Junk tails ("A Novel", "(Unabridged)") are stripped by the
/// parse and never enter the key, so a junk-tail or accented variant of a
/// stored title computes the SAME key and can adopt (ST-04). A one-sided
/// true-subtitle/volume pair computes DIFFERENT keys — it correctly misses
/// at this exact-equality seat and falls to the dedup cascade, which lands
/// grey → a visible duplicate, never a silent absorb (REQ-008; the
/// merge-two-works action is the resolution path).
///
/// Trailing empty segments are dropped, so a plain title's key is just its
/// cleaned main ("Dune" → "dune") — unambiguous, since `\u{1}` cannot
/// appear in cleaned text.
///
/// Deliberately blunter than `title_verdict`/`author_verdict`: a plain
/// string pair can only express exact equality, never grey, and an
/// initials-vs-full-name author variant that `author_verdict` would call
/// [`AuthorVerdict::Agree`] (e.g. "J.K. Rowling" vs "Joanne Kathleen
/// Rowling") can still produce different strings here. The richer verdict
/// functions remain the primary decision seats; this is the storable
/// backstop their unique-index guard needs.
///
/// Either side may be passed empty when only the other half is needed (e.g.
/// normalizing a bare series or title-only string has no author) — the
/// unused side's component is still returned deterministically as an empty
/// string.
pub fn identity_key(title: &str, author: &str) -> (String, String) {
    let parsed = parse_title(title);
    let volume_segment = rendered_volume_numbers(&parsed).join(",");

    let mut segments = vec![
        parsed.main,
        parsed.subtitle.unwrap_or_default(),
        volume_segment,
    ];
    while segments.len() > 1 && segments.last().is_some_and(|s| s.is_empty()) {
        segments.pop();
    }
    let title_key = segments.join("\u{1}");

    (title_key, canonical_author_key(author))
}

/// The scan/filename comparison form of the SAME recipe (never stored):
/// the [`identity_key`] parse triple flattened into one space-joined,
/// whitespace-collapsed token string — volume numbers rendered as bare
/// digits — with the canonical article vocabulary (a/an/the) dropped at
/// EVERY token position. The author component is identical to
/// [`identity_key`]'s.
///
/// Why this form exists: filesystem sanitization erases the `:` separator
/// (`sanitize_path_component` writes `:` as `_`), so a rescanned file stem
/// ("Mistborn_ The Final Empire") parses FLAT while the stored work title
/// ("Mistborn: The Final Empire") parses segmented. Comparing two
/// flattened renders of the one parse reconciles the two shapes — the
/// scan matcher compares `identity_key_flat(stem)` against
/// `identity_key_flat(work.title)`, never the segmented stored key.
///
/// Why articles drop at every position here (vs segment-leading only in
/// the segmented form): position-sensitive article dropping is defined by
/// segment boundaries, and flattening is precisely the erasure of those
/// boundaries — the stem side never had them, so its buried "the" would
/// otherwise never reconcile with the work side's dropped subtitle
/// article. Both sides coarsen identically, so sibling distinctness
/// survives: differing subtitles still differ token-wise, a bare-titled
/// stem still misses a subtitled work (grey territory — the existing
/// fuzzy/manual import path), and junk-tail/accent folding hold exactly
/// as in the segmented form. Never used for storage, adopt, or dedup —
/// those keep the segmented [`identity_key`].
pub fn identity_key_flat(title: &str, author: &str) -> (String, String) {
    let parsed = parse_title(title);
    let volumes = rendered_volume_numbers(&parsed);
    let joined = format!(
        "{} {} {}",
        parsed.main,
        parsed.subtitle.unwrap_or_default(),
        volumes.join(" ")
    );
    let flat_title = joined
        .split_whitespace()
        .filter(|t| !matches!(*t, "a" | "an" | "the"))
        .collect::<Vec<_>>()
        .join(" ");
    (flat_title, canonical_author_key(author))
}

/// Sorted, deduplicated volume numbers of a parsed title, rendered as bare
/// digit strings ("1", "3.5") — the shared volume normalization behind
/// both [`identity_key`] (comma-joined segment) and [`identity_key_flat`]
/// (space-joined tokens).
fn rendered_volume_numbers(parsed: &ParsedTitle) -> Vec<String> {
    let mut volumes = parsed.volume_numbers();
    volumes.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    volumes.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
    volumes.iter().map(|n| n.to_string()).collect()
}

/// The canonical author string shared by [`identity_key`] and
/// [`identity_key_flat`]: order-normalized, accent-stripped,
/// suffix-dropped tokens (the same name canonicalization `author_verdict`
/// uses under the hood), or empty when no usable author name is present.
/// Also the recipe for the stored author identity key
/// (`authors.normalized_name`): one normalization authority for both the
/// works-side and authors-side stored forms.
pub fn canonical_author_key(author: &str) -> String {
    canonical_author_name(author)
        .map(|name| {
            let mut tokens = name.given;
            tokens.push(name.surname);
            tokens.join(" ")
        })
        .unwrap_or_default()
}

/// One author's own names, with canonically identical spellings collapsed.
///
/// Two spellings of the same person that reduce to the same
/// [`canonical_author_key`] are one name, not two, and leaving both in the
/// snapshot makes an exact provider match look ambiguous to `author_verdict` —
/// which is correct when comparing lists of *different* people and wrong when
/// one side is one person's own alias list.
///
/// The first spelling of each key survives, so the order is deterministic:
/// author name first, then stored variant order. Empty and blank names are
/// dropped.
pub fn dedupe_associated_names(names: &[String]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut deduped = Vec::with_capacity(names.len());
    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(canonical_author_key(trimmed)) {
            deduped.push(trimmed.to_string());
        }
    }
    deduped
}

// --- title parsing internals ---

/// Volume-marker phrase: optional "Series Name, " prefix, a volume token,
/// a number token, and an optional "of/in the Series" suffix. Anchored to
/// the whole string.
static RE_SERIES_VOLUME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:(.+?),\s*)?(?:book|volume|vol\.?|part|no\.|#)\s*([a-z0-9.]+)(?:\s+(?:of|in)\s+(?:the\s+)?(.+?))?$",
    )
    .unwrap()
});

/// Hash-number series form without a comma: "Series Name #4" or "#4".
static RE_HASH_SERIES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:(.+?)\s+)?#\s*([0-9.]+)$").unwrap());

/// A bare number standing alone as a tail.
static RE_BARE_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+(\.\d+)?$").unwrap());

/// Trailing comma-led volume marker bound to a main title ("Foo, Vol. 3").
static RE_COMMA_VOLUME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i),\s*(?:book|volume|vol\.?|part|no\.|#)\s*([a-z0-9.]+)\s*$").unwrap()
});

/// Edition-junk phrases matched whole against a normalized tail. Extending
/// this list requires a matching trap-corpus test.
const JUNK_VOCAB: &[&str] = &[
    "a novel",
    "a memoir",
    "a novella",
    "a story",
    "a tale",
    "a poem",
    "unabridged",
    "abridged",
    "large print",
    "annotated",
    "illustrated",
    "complete",
    "tie in",
    "special edition",
    "deluxe edition",
    "collectors edition",
    "anniversary edition",
    "expanded edition",
    "audiobook",
    "ebook",
    "kindle edition",
    "hardcover",
    "paperback",
    "mass market",
    "original edition",
    "reissue edition",
    "revised edition",
    "updated edition",
    "definitive edition",
    "directors cut edition",
];

fn is_junk_phrase(normalized: &str) -> bool {
    JUNK_VOCAB.contains(&normalized)
}

/// Lowercased, accent-stripped, apostrophe-dropped, punctuation-folded form
/// used for junk-vocabulary comparison.
fn normalize_vocab(s: &str) -> String {
    let stripped = text_norm::strip_combining_marks(s);
    let lower = stripped.to_lowercase();
    let no_apostrophe: String = lower.chars().filter(|c| !matches!(c, '\'' | '’')).collect();
    no_apostrophe
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Canonical comparison form: accent-stripped, lowercased, punctuation
/// folded to token boundaries, leading article dropped. CJK text folds to
/// its bare character sequence instead (bigram comparison happens at
/// scoring time).
fn canonical_phrase(s: &str) -> String {
    if text_norm::has_cjk(s) {
        return s
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase();
    }
    let stripped = text_norm::strip_combining_marks(s);
    let lower = stripped.to_lowercase();
    let mut tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.len() > 1 && matches!(tokens[0], "a" | "an" | "the") {
        tokens.remove(0);
    }
    tokens.join(" ")
}

/// Split at the first unambiguous separator: a colon, a spaced dash, or an
/// em-dash. A hyphen inside a word never splits.
fn split_main_tail(s: &str) -> (&str, Option<&str>) {
    let candidates = [
        s.find(':').map(|i| (i, ':'.len_utf8())),
        s.find(" - ").map(|i| (i, " - ".len())),
        s.find('—').map(|i| (i, '—'.len_utf8())),
    ];
    let best = candidates.into_iter().flatten().min_by_key(|&(i, _)| i);
    match best {
        Some((i, len)) => {
            let main = s[..i].trim_end();
            let tail = s[i + len..].trim_start();
            (main, (!tail.is_empty()).then_some(tail))
        }
        None => (s, None),
    }
}

/// Parse a volume number token: digits (with an optional decimal part),
/// a spelled cardinal (one..twenty), or a roman numeral (i..xx — the forms
/// the existing series-marker stripper recognizes).
fn parse_volume_number(token: &str) -> Option<f64> {
    let t = token.trim().trim_end_matches('.');
    if t.is_empty() {
        return None;
    }
    if t.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return t.parse::<f64>().ok();
    }
    let lower = t.to_lowercase();
    const SPELLED: [&str; 20] = [
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    if let Some(i) = SPELLED.iter().position(|w| *w == lower) {
        return Some((i + 1) as f64);
    }
    const ROMAN: [&str; 20] = [
        "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv",
        "xv", "xvi", "xvii", "xviii", "xix", "xx",
    ];
    ROMAN
        .iter()
        .position(|w| *w == lower)
        .map(|i| (i + 1) as f64)
}

/// Recognize a whole string as a series-volume marker and extract its
/// number. Bare numbers qualify only below four integer digits (a four
/// digit standalone number reads as a year, not a volume).
fn parse_series_marker(text: &str) -> Option<SeriesMarker> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if RE_BARE_NUMBER.is_match(t) {
        let int_digits = t.split('.').next().unwrap_or("").len();
        if int_digits <= 3 {
            return t.parse::<f64>().ok().map(|number| SeriesMarker {
                series: None,
                number,
            });
        }
        return None;
    }
    if let Some(caps) = RE_SERIES_VOLUME.captures(t) {
        if let Some(number) = caps.get(2).and_then(|m| parse_volume_number(m.as_str())) {
            let series = caps
                .get(1)
                .or(caps.get(3))
                .map(|m| canonical_phrase(m.as_str()))
                .filter(|s| !s.is_empty());
            return Some(SeriesMarker { series, number });
        }
    }
    if let Some(caps) = RE_HASH_SERIES.captures(t) {
        if let Some(number) = caps.get(2).and_then(|m| parse_volume_number(m.as_str())) {
            let series = caps
                .get(1)
                .map(|m| canonical_phrase(m.as_str()))
                .filter(|s| !s.is_empty());
            return Some(SeriesMarker { series, number });
        }
    }
    None
}

/// Pull a trailing comma-led volume marker off a main title.
fn extract_comma_marker(s: &str) -> Option<(String, SeriesMarker)> {
    let caps = RE_COMMA_VOLUME.captures(s)?;
    let number = parse_volume_number(caps.get(1)?.as_str())?;
    let start = caps.get(0)?.start();
    Some((
        s[..start].trim_end().to_string(),
        SeriesMarker {
            series: None,
            number,
        },
    ))
}

/// Classify a colon/dash tail: series marker, junk, or true subtitle.
/// Anything unrecognized is a true subtitle — the class that can only
/// demote, never veto, never silently pass.
fn classify_tail(
    tail: &str,
    markers: &mut Vec<SeriesMarker>,
    junk: &mut Vec<String>,
    subtitle_parts: &mut Vec<String>,
) {
    if let Some(marker) = parse_series_marker(tail) {
        markers.push(marker);
        return;
    }
    let normalized = normalize_vocab(tail);
    if !normalized.is_empty() && is_junk_phrase(&normalized) {
        junk.push(normalized);
        return;
    }
    subtitle_parts.push(tail.to_string());
}

/// Classify a trailing parenthetical: year and recognized series/format/
/// edition tags are junk (number-carrying series tags become markers);
/// anything unrecognized is a true subtitle.
fn classify_paren(
    inner: &str,
    markers: &mut Vec<SeriesMarker>,
    junk: &mut Vec<String>,
    subtitle_parts: &mut Vec<String>,
) {
    if inner.is_empty() {
        return;
    }
    let normalized = normalize_vocab(inner);
    if title_cleanup::RE_YEAR_PAREN.is_match(inner) {
        junk.push(normalized);
        return;
    }
    if let Some(marker) = parse_series_marker(inner) {
        markers.push(marker);
        return;
    }
    if title_cleanup::RE_SERIES_PAREN.is_match(inner)
        || title_cleanup::RE_FORMAT_PAREN.is_match(inner)
        || title_cleanup::RE_EDITION_PAREN.is_match(inner)
        || is_junk_phrase(&normalized)
    {
        junk.push(normalized);
        return;
    }
    subtitle_parts.push(inner.to_string());
}

// --- verdict internals ---

/// Both sides carry volume evidence and share no number.
fn volumes_conflict(a: &[f64], b: &[f64]) -> bool {
    !a.is_empty() && !b.is_empty() && !a.iter().any(|x| b.iter().any(|y| (x - y).abs() < 1e-9))
}

/// Parenthesized role tags in author credits ("(narrator)", "(translator)").
static RE_AUTHOR_PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\([^)]*\)").unwrap());

/// One credited name in canonical comparison form: lowercased,
/// accent-stripped, order-normalized tokens with name suffixes dropped.
/// The last token is the surname; the rest are given-name tokens (a
/// single-character token is an initial).
struct CanonicalName {
    given: Vec<String>,
    surname: String,
}

fn canonical_author_name(raw: &str) -> Option<CanonicalName> {
    let no_parens = RE_AUTHOR_PAREN.replace_all(raw, " ");
    let reordered = normalize_last_first(&collapse_whitespace(&no_parens));
    let stripped = text_norm::strip_combining_marks(&reordered);
    let lower = stripped.to_lowercase();
    let mut tokens: Vec<String> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .filter(|t| !text_norm::AUTHOR_SUFFIX_STOPWORDS.contains(t))
        .map(String::from)
        .collect();
    let surname = tokens.pop()?;
    Some(CanonicalName {
        given: tokens,
        surname,
    })
}

/// An initial is compatible with any name sharing its first character;
/// full words must match exactly.
fn given_token_compatible(a: &str, b: &str) -> bool {
    let a_initial = a.chars().count() == 1;
    let b_initial = b.chars().count() == 1;
    if a_initial || b_initial {
        a.chars().next() == b.chars().next()
    } else {
        a == b
    }
}

/// Full-name match: equal surnames plus pairwise-compatible given names.
/// Surplus given tokens (an extra middle name on one side) do not block.
/// A surname-only name never fully matches a name that carries given
/// names — that is shared-surname (grey) territory.
fn full_name_match(a: &CanonicalName, b: &CanonicalName) -> bool {
    if a.surname != b.surname {
        return false;
    }
    match (a.given.is_empty(), b.given.is_empty()) {
        (true, true) => true,
        (true, false) | (false, true) => false,
        (false, false) => a
            .given
            .iter()
            .zip(b.given.iter())
            .all(|(x, y)| given_token_compatible(x, y)),
    }
}

/// Adoption match between two canonical names — the rule set behind
/// [`unambiguous_author_match`], deliberately tighter than [`full_name_match`].
fn adoption_match(x: &CanonicalName, y: &CanonicalName) -> bool {
    if x.surname != y.surname {
        return false;
    }
    match (x.given.is_empty(), y.given.is_empty()) {
        (true, true) => return true,
        (true, false) | (false, true) => return false,
        (false, false) => {}
    }
    if x.given.len() == y.given.len() {
        return x
            .given
            .iter()
            .zip(y.given.iter())
            .all(|(a, b)| given_token_compatible(a, b));
    }
    if let Some(matched) = glued_initials_match(&x.given, &y.given) {
        return matched;
    }
    exact_prefix_match(&x.given, &y.given)
}

/// Symmetric glued-initials reading: a single 2-4 char all-alphabetic given
/// token on either side may be read as a run of single-char initials when
/// that reading equalizes the given-token counts. `None` when neither side
/// qualifies — the rule is inapplicable, not a verdict, and the caller falls
/// through to [`exact_prefix_match`].
fn glued_initials_match(a: &[String], b: &[String]) -> Option<bool> {
    try_glued_side(a, b).or_else(|| try_glued_side(b, a))
}

/// `glued`'s lone token, when 2-4 alphabetic chars, exploded into initials
/// and compared pairwise against `other`. `None` if `glued` isn't a single
/// eligible token or the explosion doesn't equalize the counts.
fn try_glued_side(glued: &[String], other: &[String]) -> Option<bool> {
    if glued.len() != 1 {
        return None;
    }
    let token = &glued[0];
    let chars: Vec<char> = token.chars().collect();
    if !(2..=4).contains(&chars.len()) || !token.chars().all(|c| c.is_alphabetic()) {
        return None;
    }
    if chars.len() != other.len() {
        return None;
    }
    Some(
        chars
            .iter()
            .map(|c| c.to_string())
            .zip(other.iter())
            .all(|(initial, name)| given_token_compatible(&initial, name)),
    )
}

/// Unequal given-token counts, non-glued: the shorter side's tokens must be
/// an exact multi-char word-for-word prefix of the longer side's; surplus
/// given names beyond that prefix are tolerated unchecked. A lone initial
/// can never occupy a prefix position — it is not a multi-char word.
fn exact_prefix_match(a: &[String], b: &[String]) -> bool {
    let (shorter, longer) = if a.len() < b.len() { (a, b) } else { (b, a) };
    shorter
        .iter()
        .zip(longer.iter())
        .all(|(x, y)| x.chars().count() > 1 && x == y)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_title: splitting and tail classification ---

    #[test]
    fn hyphenated_word_never_splits() {
        let p = parse_title("Catch-22");
        assert_eq!(p.main, "catch 22");
        assert!(p.subtitle.is_none());
        assert!(p.series_markers.is_empty());
    }

    #[test]
    fn spaced_dash_and_em_dash_split_as_separators() {
        let spaced = parse_title("Foo - And Other Stories");
        assert_eq!(spaced.main, "foo");
        assert!(spaced.subtitle.is_some());

        let em = parse_title("Foo—And Other Stories");
        assert_eq!(em.main, "foo");
        assert!(em.subtitle.is_some());
    }

    #[test]
    fn colon_book_spelled_ordinal_is_series_marker() {
        let p = parse_title("Foo: Book Three");
        assert_eq!(p.main, "foo");
        assert!(p.subtitle.is_none());
        assert_eq!(p.volume_numbers(), vec![3.0]);
    }

    #[test]
    fn comma_vol_abbreviation_is_series_marker() {
        let p = parse_title("Foo, Vol. 3");
        assert_eq!(p.main, "foo");
        assert!(p.subtitle.is_none());
        assert_eq!(p.volume_numbers(), vec![3.0]);
    }

    #[test]
    fn a_novel_tail_is_junk() {
        let p = parse_title("Foo: A Novel");
        assert_eq!(p.main, "foo");
        assert!(p.subtitle.is_none());
        assert!(p.series_markers.is_empty());
        assert!(!p.junk.is_empty());
    }

    #[test]
    fn substantive_tail_is_true_subtitle() {
        let p = parse_title("Foo: And Other Stories");
        assert_eq!(p.main, "foo");
        assert!(p.subtitle.is_some());
        assert!(p.series_markers.is_empty());
    }

    #[test]
    fn unclassifiable_tail_lands_as_subtitle() {
        let p = parse_title("Foo: Xyzzy Plugh Fnord");
        assert_eq!(p.main, "foo");
        assert!(p.subtitle.is_some());
        assert!(p.series_markers.is_empty());
        assert!(p.junk.is_empty());
    }

    #[test]
    fn paren_series_suffix_is_series_marker() {
        let p = parse_title("The Way of Kings (The Stormlight Archive, #1)");
        assert_eq!(p.main, "way of kings");
        assert!(p.subtitle.is_none());
        assert_eq!(p.volume_numbers(), vec![1.0]);
    }

    #[test]
    fn series_name_comma_book_n_tail_is_series_marker() {
        let p = parse_title("Storm Front: The Dresden Files, Book 1");
        assert_eq!(p.main, "storm front");
        assert_eq!(p.volume_numbers(), vec![1.0]);
        assert!(p.subtitle.is_none());
    }

    #[test]
    fn unabridged_paren_is_junk_not_subtitle() {
        let p = parse_title("Dune (Unabridged)");
        assert_eq!(p.main, "dune");
        assert!(p.subtitle.is_none());
        assert!(!p.junk.is_empty());
    }

    // --- title_verdict: volume vetoes ---

    #[test]
    fn conflicting_tail_volumes_veto() {
        let a = parse_title("History of Rome: Volume 1");
        let b = parse_title("History of Rome: Volume 2");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::VetoVolume);
    }

    #[test]
    fn conflicting_caller_positions_veto_without_tails() {
        let a = parse_title("History of Rome");
        let b = parse_title("History of Rome");
        assert_eq!(
            title_verdict_with_positions(&a, Some(1.0), &b, Some(2.0)),
            TitleVerdict::VetoVolume
        );
    }

    #[test]
    fn position_missing_sibling_is_not_same() {
        let a = parse_title("History of Rome");
        let b = parse_title("History of Rome");
        let v = title_verdict_with_positions(&a, Some(1.0), &b, None);
        assert_ne!(v, TitleVerdict::Same);
        assert!(matches!(v, TitleVerdict::Grey { .. }), "got {v:?}");
    }

    #[test]
    fn matching_volumes_do_not_veto() {
        let a = parse_title("History of Rome: Volume 1");
        let b = parse_title("History of Rome, Vol. 1");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Same);
    }

    // --- title_verdict: tails demote, junk is ignored ---

    #[test]
    fn one_sided_series_tail_is_grey_never_same() {
        let a = parse_title("Storm Front");
        let b = parse_title("Storm Front: The Dresden Files, Book 1");
        let v = title_verdict(&a, &b);
        assert_ne!(v, TitleVerdict::Same);
        assert!(matches!(v, TitleVerdict::Grey { .. }), "got {v:?}");
    }

    #[test]
    fn disagreeing_subtitles_never_auto_same() {
        let a = parse_title("Mistborn: The Final Empire");
        let b = parse_title("Mistborn: The Well of Ascension");
        let v = title_verdict(&a, &b);
        assert_ne!(v, TitleVerdict::Same);
        assert!(matches!(v, TitleVerdict::Grey { .. }), "got {v:?}");
    }

    #[test]
    fn one_sided_true_subtitle_is_grey() {
        let a = parse_title("Foo");
        let b = parse_title("Foo: And Other Stories");
        let v = title_verdict(&a, &b);
        assert!(matches!(v, TitleVerdict::Grey { .. }), "got {v:?}");
    }

    #[test]
    fn junk_only_tail_difference_is_ignored() {
        let a = parse_title("Dune: A Novel");
        let b = parse_title("Dune");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Same);
    }

    #[test]
    fn agreeing_subtitles_stay_same() {
        let a = parse_title("The Power Broker: Robert Moses and the Fall of New York");
        let b = parse_title("The Power Broker: Robert Moses and the Fall of New York");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Same);
    }

    // --- title_verdict: lookalikes and colon-cut regression guards ---

    #[test]
    fn study_guide_lookalike_never_same() {
        let real = parse_title("Storm Front");
        let guide = parse_title("Study Guide: Storm Front by Jim Butcher");
        let summary = parse_title("Summary of Storm Front");
        assert_ne!(title_verdict(&real, &guide), TitleVerdict::Same);
        assert_ne!(title_verdict(&real, &summary), TitleVerdict::Same);
    }

    #[test]
    fn colon_truncation_lookalikes_are_not_same() {
        // Titles today's first-colon truncation would wrongly equate.
        let a = parse_title("Star Wars: A New Hope");
        let b = parse_title("Star Wars: The Empire Strikes Back");
        assert_ne!(title_verdict(&a, &b), TitleVerdict::Same);
    }

    #[test]
    fn both_titles_empty_never_same() {
        let a = parse_title("");
        let b = parse_title("");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Different);
    }

    #[test]
    fn one_empty_title_cannot_pass_any_bar() {
        let a = parse_title("");
        let b = parse_title("Dune");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Different);
    }

    // --- title_verdict: cleaning baseline and the grey band ---

    #[test]
    fn leading_article_and_accents_fold_into_exact_same() {
        let a = parse_title("The Hobbit");
        let b = parse_title("Hobbit");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Same);

        let c = parse_title("Café");
        let d = parse_title("Cafe");
        assert_eq!(title_verdict(&c, &d), TitleVerdict::Same);
    }

    #[test]
    fn near_titles_land_grey_with_real_score() {
        let a = parse_title("The Wise Man's Fear");
        let b = parse_title("The Wise Man's Fear Chronicle");
        match title_verdict(&a, &b) {
            TitleVerdict::Grey { score, .. } => {
                assert!(score >= TITLE_GREY_FLOOR, "score {score}");
                assert!(score < 1.0, "score {score}");
            }
            other => panic!("expected Grey, got {other:?}"),
        }
    }

    #[test]
    fn unrelated_titles_are_different() {
        let a = parse_title("Storm Front");
        let b = parse_title("The Name of the Wind");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Different);
    }

    #[test]
    fn cjk_titles_compare_via_bigrams() {
        let a = parse_title("三体");
        let b = parse_title("三体");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Same);

        let c = parse_title("キッチン");
        let d = parse_title("ノルウェイの森");
        assert_ne!(title_verdict(&c, &d), TitleVerdict::Same);
    }

    // --- author_verdict ---

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unambiguous_author_match_adopts_po_pairs_in_both_directions() {
        let cases = [
            ("W.E.B. Griffin", "W. E. B. Griffin"),
            ("JK Rowling", "J.K. Rowling"),
            ("Robert Anson Heinlein", "Robert A. Heinlein"),
        ];

        for (candidate, stored) in cases {
            assert_eq!(
                unambiguous_author_match(candidate, &names(&["Other Author", stored])),
                Some(1),
                "{candidate:?} should adopt stored {stored:?}"
            );
            assert_eq!(
                unambiguous_author_match(stored, &names(&["Other Author", candidate])),
                Some(1),
                "{stored:?} should adopt stored {candidate:?}"
            );
        }
    }

    #[test]
    fn unambiguous_author_match_adopts_exact_full_word_prefix_in_both_directions() {
        assert_eq!(
            unambiguous_author_match("Robert Heinlein", &names(&["Robert A. Heinlein"])),
            Some(0)
        );
        assert_eq!(
            unambiguous_author_match("Robert A. Heinlein", &names(&["Robert Heinlein"])),
            Some(0)
        );
    }

    #[test]
    fn unambiguous_author_match_refuses_lone_initial_spanning_surplus_names() {
        assert_eq!(
            unambiguous_author_match("J. Rowling", &names(&["Jane Joanne Rowling"])),
            None
        );
        assert_eq!(
            unambiguous_author_match("Jane Joanne Rowling", &names(&["J. Rowling"])),
            None
        );
    }

    #[test]
    fn unambiguous_author_match_requires_exactly_one_compatible_stored_author() {
        assert_eq!(
            unambiguous_author_match("J. Smith", &names(&["John Smith", "Jane Smith"])),
            None
        );
    }

    #[test]
    fn unambiguous_author_match_refuses_glued_initials_against_single_given_name() {
        assert_eq!(
            unambiguous_author_match("JK Rowling", &names(&["Joanne Rowling"])),
            None
        );
    }

    #[test]
    fn unambiguous_author_match_refuses_surname_only_against_given_named_stored() {
        assert_eq!(
            unambiguous_author_match("Rowling", &names(&["J.K. Rowling"])),
            None
        );
    }

    #[test]
    fn unambiguous_author_match_refuses_empty_or_garbage_candidate() {
        assert_eq!(unambiguous_author_match("", &names(&["John Smith"])), None);
        assert_eq!(
            unambiguous_author_match("!!! ...", &names(&["John Smith"])),
            None
        );
    }

    #[test]
    fn unambiguous_author_match_accepts_documented_single_initial_residual() {
        assert_eq!(
            unambiguous_author_match("J. Smith", &names(&["John Smith"])),
            Some(0)
        );
    }

    #[test]
    fn shared_surname_only_is_grey_never_agree() {
        let v = author_verdict(&names(&["John Smith"]), &names(&["Jane Smith"]));
        assert_eq!(v, AuthorVerdict::Grey);
    }

    #[test]
    fn extra_credited_name_is_non_evidence() {
        let v = author_verdict(
            &names(&["Jim Butcher"]),
            &names(&["Jim Butcher", "James Marsters"]),
        );
        assert_eq!(v, AuthorVerdict::Agree);
    }

    #[test]
    fn last_first_and_initials_normalize_to_agree() {
        let v = author_verdict(&names(&["Rowling, J. K."]), &names(&["J.K. Rowling"]));
        assert_eq!(v, AuthorVerdict::Agree);
    }

    #[test]
    fn initials_compatible_with_expanded_name_agree() {
        let v = author_verdict(
            &names(&["J.K. Rowling"]),
            &names(&["Joanne Kathleen Rowling"]),
        );
        assert_eq!(v, AuthorVerdict::Agree);
    }

    #[test]
    fn initials_compatible_with_two_candidates_is_grey() {
        let v = author_verdict(&names(&["J. Smith"]), &names(&["John Smith", "Jane Smith"]));
        assert_eq!(v, AuthorVerdict::Grey);
    }

    #[test]
    fn zero_overlap_is_disagree() {
        let v = author_verdict(&names(&["Frank Herbert"]), &names(&["Ursula Le Guin"]));
        assert_eq!(v, AuthorVerdict::Disagree);
    }

    #[test]
    fn empty_author_list_abstains() {
        assert_eq!(
            author_verdict(&[], &names(&["Frank Herbert"])),
            AuthorVerdict::Abstain
        );
        assert_eq!(author_verdict(&[], &[]), AuthorVerdict::Abstain);
    }

    #[test]
    fn accented_and_plain_forms_agree() {
        let v = author_verdict(
            &names(&["Gabriel García Márquez"]),
            &names(&["Gabriel Garcia Marquez"]),
        );
        assert_eq!(v, AuthorVerdict::Agree);
    }

    // --- author_verdict: D3 bound (raw-name cap, pair-vector elimination) ---
    //
    // `naive_author_verdict` below is a FROZEN, byte-for-byte copy of
    // `author_verdict`'s pre-D3 body (the O(N*M) pair-vector
    // implementation) — a sanctioned duplication whose sole job is to
    // serve as the diff-oracle these tests check the bounded rewrite
    // against. Never "fix" or simplify this copy to match the real
    // function; if it ever needs to change, that means the real function's
    // semantics changed and the property tests below should catch it.
    fn naive_author_verdict(a: &[String], b: &[String]) -> AuthorVerdict {
        let ca: Vec<CanonicalName> = a.iter().filter_map(|n| canonical_author_name(n)).collect();
        let cb: Vec<CanonicalName> = b.iter().filter_map(|n| canonical_author_name(n)).collect();
        if ca.is_empty() || cb.is_empty() {
            return AuthorVerdict::Abstain;
        }

        let mut pairs: Vec<(usize, usize)> = Vec::new();
        let mut row_counts = vec![0usize; ca.len()];
        let mut col_counts = vec![0usize; cb.len()];
        for (i, x) in ca.iter().enumerate() {
            for (j, y) in cb.iter().enumerate() {
                if full_name_match(x, y) {
                    row_counts[i] += 1;
                    col_counts[j] += 1;
                    pairs.push((i, j));
                }
            }
        }
        if pairs
            .iter()
            .any(|&(i, j)| row_counts[i] == 1 && col_counts[j] == 1)
        {
            return AuthorVerdict::Agree;
        }
        if !pairs.is_empty() {
            return AuthorVerdict::Grey;
        }
        let shared_surname = ca.iter().any(|x| cb.iter().any(|y| x.surname == y.surname));
        if shared_surname {
            AuthorVerdict::Grey
        } else {
            AuthorVerdict::Disagree
        }
    }

    /// A deterministic corpus spanning the categories the D3 rewrite must
    /// preserve exactly: permutations (multi-name lists reordered),
    /// initials (bare + dotted + spaced + glued), surplus middle names,
    /// duplicate names (repeated within one list), and ambiguity (two+
    /// compatible candidates on one side). No `proptest`/`quickcheck`
    /// dependency exists anywhere in this workspace; a full cross-product
    /// over a rich, hand-built corpus is the deterministic equivalent —
    /// reproducible, no new dependency, no flake surface.
    fn author_verdict_corpus() -> Vec<Vec<String>> {
        let mut corpus: Vec<Vec<String>> = Vec::new();

        // Singletons: bare / initialed / accented / last-first / role-tag /
        // garbage / empty forms across several distinct people.
        let singles = [
            "John Smith",
            "Jane Smith",
            "J. Smith",
            "J.Smith",
            "Frank Herbert",
            "Ursula Le Guin",
            "Jim Butcher",
            "James Marsters",
            "Robert Anson Heinlein",
            "Robert A. Heinlein",
            "Robert Heinlein",
            "R. Heinlein",
            "Jane Joanne Rowling",
            "Joanne Kathleen Rowling",
            "J.K. Rowling",
            "JK Rowling",
            "Rowling, J. K.",
            "W.E.B. Griffin",
            "W. E. B. Griffin",
            "Gabriel García Márquez",
            "Gabriel Garcia Marquez",
            "Jim Butcher (Author)",
            "James Marsters (Narrator)",
            "",
            "   ",
            "!!! ...",
        ];
        for s in singles {
            corpus.push(names(&[s]));
        }

        // Duplicates within one list (the same raw string repeated, and a
        // canonicalization-equivalent repeat).
        corpus.push(names(&["John Smith", "John Smith"]));
        corpus.push(names(&["J.K. Rowling", "JK Rowling", "J.K. Rowling"]));
        corpus.push(names(&[
            "Robert Heinlein",
            "Robert Heinlein",
            "Robert Heinlein",
        ]));

        // Multi-name lists (extra credited names — non-evidence per the
        // authority) in several orders (permutations), including
        // ambiguity-inducing pairs (two names compatible with one initial).
        let multi_bases: Vec<Vec<&str>> = vec![
            vec!["Jim Butcher", "James Marsters"],
            vec!["John Smith", "Jane Smith"],
            vec!["John Smith", "Jane Smith", "J. Smith"],
            vec!["Frank Herbert", "Ursula Le Guin", "Jim Butcher"],
            vec!["Robert A. Heinlein", "Ursula Le Guin"],
            vec!["J.K. Rowling", "Frank Herbert", "Jim Butcher"],
            vec!["Jane Joanne Rowling", "J. Rowling"],
        ];
        for base in &multi_bases {
            // All rotations of the base list — a permutation sweep without
            // needing a combinatorics crate.
            for rot in 0..base.len() {
                let mut rotated = base.clone();
                rotated.rotate_left(rot);
                corpus.push(names(&rotated));
            }
            // Fully reversed order too.
            let mut reversed = base.clone();
            reversed.reverse();
            corpus.push(names(&reversed));
        }

        corpus
    }

    #[test]
    fn author_verdict_matches_the_naive_authority_across_the_generated_corpus() {
        let corpus = author_verdict_corpus();
        let mut checked = 0usize;
        for a in &corpus {
            for b in &corpus {
                assert_eq!(
                    author_verdict(a, b),
                    naive_author_verdict(a, b),
                    "author_verdict diverged from the naive authority for a={a:?} b={b:?}"
                );
                checked += 1;
            }
        }
        // Sanity: the corpus is actually exercising a meaningful number of
        // pairings, not silently degenerating to a handful of cases.
        assert!(
            checked >= 2500,
            "expected a substantial cross-product corpus, only checked {checked} pairs"
        );
    }

    #[test]
    fn author_verdict_abstains_past_the_256_raw_name_cap_on_either_side() {
        fn distinct_names(n: usize) -> Vec<String> {
            (0..n).map(|i| format!("Author{i} Surname{i}")).collect()
        }

        // Exactly at the cap: normal behavior (no size-based abstain) — two
        // disjoint 256-name rosters share no author, so the real verdict is
        // Disagree, proving the cap did NOT fire here.
        let a256 = distinct_names(256);
        let b256 = distinct_names(1000); // offsets so no names collide with a256
        let b256_disjoint: Vec<String> = b256[500..756].to_vec();
        assert_eq!(a256.len(), 256);
        assert_eq!(b256_disjoint.len(), 256);
        assert_eq!(
            author_verdict(&a256, &b256_disjoint),
            AuthorVerdict::Disagree,
            "256 raw names per side must NOT trigger the size-based Abstain"
        );

        // One side at 257 (one over the cap): must Abstain, regardless of
        // the other side.
        let a257 = distinct_names(257);
        assert_eq!(
            author_verdict(&a257, &names(&["Frank Herbert"])),
            AuthorVerdict::Abstain,
            "257 raw names on the LEFT side must Abstain"
        );
        assert_eq!(
            author_verdict(&names(&["Frank Herbert"]), &a257),
            AuthorVerdict::Abstain,
            "257 raw names on the RIGHT side must Abstain"
        );

        // Both sides over cap.
        let b257 = distinct_names(257);
        assert_eq!(
            author_verdict(&a257, &b257),
            AuthorVerdict::Abstain,
            "both sides over the cap must Abstain"
        );
    }

    // --- language_verdict ---

    #[test]
    fn declared_language_mismatch_vetoes() {
        assert_eq!(
            language_verdict(Some("en"), Some("fr"), "en"),
            LanguageVerdict::Veto
        );
    }

    #[test]
    fn silent_payload_on_non_default_work_is_grey() {
        assert_eq!(
            language_verdict(Some("fr"), None, "en"),
            LanguageVerdict::Grey
        );
    }

    #[test]
    fn silent_payload_on_default_language_work_is_neutral() {
        assert_eq!(
            language_verdict(Some("en"), None, "en"),
            LanguageVerdict::Neutral
        );
    }

    #[test]
    fn equal_or_both_silent_is_neutral() {
        assert_eq!(
            language_verdict(Some("fr"), Some("fr"), "en"),
            LanguageVerdict::Neutral
        );
        assert_eq!(language_verdict(None, None, "en"), LanguageVerdict::Neutral);
    }

    // --- id_verdict ---

    #[test]
    fn work_key_equality_wins() {
        let a = IdEvidence {
            ol_key: Some("OL1W"),
            ..Default::default()
        };
        let b = IdEvidence {
            ol_key: Some("OL1W"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::WorkKeyEqual);
    }

    #[test]
    fn shared_isbn_with_work_key_contradiction_is_collision() {
        let a = IdEvidence {
            gr_key: Some("111"),
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        let b = IdEvidence {
            gr_key: Some("222"),
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::WorkKeyContradiction);
    }

    #[test]
    fn differing_asin_is_no_penalty() {
        let a = IdEvidence {
            ol_key: Some("OL1W"),
            asin: Some("B001"),
            ..Default::default()
        };
        let b = IdEvidence {
            ol_key: Some("OL1W"),
            asin: Some("B002"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::WorkKeyEqual);

        let c = IdEvidence {
            isbn_13: Some("9780000000001"),
            asin: Some("B001"),
            ..Default::default()
        };
        let d = IdEvidence {
            isbn_13: Some("9780000000001"),
            asin: Some("B002"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&c, &d), IdVerdict::EditionBridge);
    }

    #[test]
    fn shared_isbn_alone_is_an_edition_bridge() {
        let a = IdEvidence {
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        let b = IdEvidence {
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::EditionBridge);
    }

    #[test]
    fn edition_id_inequality_is_no_evidence() {
        let a = IdEvidence {
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        let b = IdEvidence {
            isbn_13: Some("9780000000002"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::NoEvidence);
    }

    #[test]
    fn no_identifiers_is_no_evidence() {
        let a = IdEvidence::default();
        let b = IdEvidence {
            ol_key: Some("OL9W"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::NoEvidence);
        assert_eq!(id_verdict(&a, &a), IdVerdict::NoEvidence);
    }

    // --- settle-road matching pins ---

    #[test]
    fn cause_one_sided_subtitle_is_exposed_on_equal_main_grey() {
        let bare = parse_title("World War Z");
        let subtitled = parse_title("World War Z: An Oral History of the Zombie War");
        assert_eq!(bare.main, subtitled.main);
        assert!(bare.subtitle.is_none());
        assert!(subtitled.subtitle.is_some());

        assert!(matches!(
            title_verdict(&bare, &subtitled),
            TitleVerdict::Grey {
                cause: GreyCause::OneSidedSubtitle,
                ..
            }
        ));
    }

    #[test]
    fn cause_subtitle_disagreement_is_exposed_on_equal_main_grey() {
        let final_empire = parse_title("Mistborn: The Final Empire");
        let well = parse_title("Mistborn: The Well of Ascension");
        assert_eq!(final_empire.main, well.main);
        assert!(final_empire.subtitle.is_some());
        assert!(well.subtitle.is_some());
        assert_ne!(final_empire.subtitle, well.subtitle);

        assert!(matches!(
            title_verdict(&final_empire, &well),
            TitleVerdict::Grey {
                cause: GreyCause::SubtitleDisagreement,
                ..
            }
        ));
    }

    #[test]
    fn cause_volume_asymmetry_is_exposed_on_one_sided_volume_grey() {
        let bare = parse_title("History of Rome");
        let volume = parse_title("History of Rome, Vol. 2");
        assert_eq!(bare.main, volume.main);
        assert!(bare.volume_numbers().is_empty());
        assert_eq!(volume.volume_numbers(), vec![2.0]);

        assert!(matches!(
            title_verdict(&bare, &volume),
            TitleVerdict::Grey {
                cause: GreyCause::VolumeAsymmetry,
                ..
            }
        ));
    }

    #[test]
    fn cause_volume_asymmetry_wins_over_one_sided_subtitle() {
        let bare = parse_title("History of Rome");
        let subtitled_volume = parse_title("History of Rome, Vol. 2: Civil Wars");
        assert_eq!(bare.main, subtitled_volume.main);
        assert!(bare.subtitle.is_none());
        assert!(subtitled_volume.subtitle.is_some());
        assert!(bare.volume_numbers().is_empty());
        assert_eq!(subtitled_volume.volume_numbers(), vec![2.0]);

        assert!(matches!(
            title_verdict(&bare, &subtitled_volume),
            TitleVerdict::Grey {
                cause: GreyCause::VolumeAsymmetry,
                ..
            }
        ));
    }

    #[test]
    fn cause_near_main_and_junk_tail_guards_hold() {
        // Guard: near-main pairs are still grey, but for the near-main cause.
        let near_a = parse_title("The Wise Man's Fear");
        let near_b = parse_title("The Wise Man's Fear Chronicle");
        assert!(matches!(
            title_verdict(&near_a, &near_b),
            TitleVerdict::Grey {
                cause: GreyCause::NearMain,
                ..
            }
        ));

        // Guard: recognized junk tails do not demote exact mains.
        assert_eq!(
            title_verdict(&parse_title("Dune"), &parse_title("Dune: A Novel")),
            TitleVerdict::Same
        );

        // Guard: the equal-main causes are defined ONLY for equal mains — a
        // near-main pair with one-sided volume evidence still classifies as
        // NearMain (both causes are untrusted at every seat, so this is a
        // taxonomy pin, not a trust-behavior fork).
        let near_bare = parse_title("The Wise Man's Fear");
        let near_volume = parse_title("The Wise Man's Fear Chronicle, Vol. 2");
        assert_ne!(near_bare.main, near_volume.main);
        assert_eq!(near_volume.volume_numbers(), vec![2.0]);
        assert!(matches!(
            title_verdict(&near_bare, &near_volume),
            TitleVerdict::Grey {
                cause: GreyCause::NearMain,
                ..
            }
        ));
    }

    #[test]
    fn title_id_trust_vetoes_mixed_work_key_evidence_on_same_title() {
        let a = IdEvidence {
            ol_key: Some("OL-A"),
            hc_key: Some("HC-A"),
            ..Default::default()
        };
        let b = IdEvidence {
            ol_key: Some("OL-A"),
            hc_key: Some("HC-B"),
            ..Default::default()
        };

        assert!(!title_id_trust(&TitleVerdict::Same, &a, &b));
    }

    #[test]
    fn title_id_trust_allows_one_sided_subtitle_grey_with_edition_bridge() {
        let a = IdEvidence {
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        let b = IdEvidence {
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        let title = TitleVerdict::Grey {
            score: 1.0,
            cause: GreyCause::OneSidedSubtitle,
        };

        assert!(title_id_trust(&title, &a, &b));
    }

    #[test]
    fn title_id_trust_allows_one_sided_subtitle_grey_with_work_key_equality() {
        let a = IdEvidence {
            ol_key: Some("OL-A"),
            ..Default::default()
        };
        let b = IdEvidence {
            ol_key: Some("OL-A"),
            ..Default::default()
        };
        let title = TitleVerdict::Grey {
            score: 1.0,
            cause: GreyCause::OneSidedSubtitle,
        };

        assert!(title_id_trust(&title, &a, &b));
    }

    #[test]
    fn title_id_trust_allows_one_sided_subtitle_grey_without_id_evidence() {
        // A one-sided-subtitle grey no longer waits for a hard identifier to
        // agree. `EditionBridge` demanded ISBN/ASIN equality, and a provider
        // lists a different printing's ISBN than the one in the user's file, so
        // the corroboration this arm required could not arrive. Equal main
        // titles plus the agreeing author every caller already requires is the
        // operative bar.
        let title = TitleVerdict::Grey {
            score: 1.0,
            cause: GreyCause::OneSidedSubtitle,
        };

        assert!(title_id_trust(
            &title,
            &IdEvidence::default(),
            &IdEvidence::default()
        ));
    }

    #[test]
    fn title_id_trust_vetoes_one_sided_subtitle_grey_on_work_key_contradiction() {
        // The contradiction veto runs before any title arm and still applies.
        let a = IdEvidence {
            gr_key: Some("10884"),
            ..Default::default()
        };
        let b = IdEvidence {
            gr_key: Some("2059858"),
            ..Default::default()
        };
        let title = TitleVerdict::Grey {
            score: 1.0,
            cause: GreyCause::OneSidedSubtitle,
        };

        assert!(!title_id_trust(&title, &a, &b));
    }

    #[test]
    fn title_id_trust_accepts_named_omnibus_and_bounds_it_at_numbered_volumes() {
        // ACCEPTED REGRESSION, on the record by design
        // (`docs/design-subtitle-matching.md` r3, C1 "Accepted risk"): a named
        // omnibus volume is now accepted as the same work. Equal mains, prose
        // subtitle, no volume evidence either side. Asserted deliberately so the
        // trade is visible in the suite rather than implicit.
        let omnibus = parse_title("The Lord of the Rings");
        let volume = parse_title("The Lord of the Rings: The Fellowship of the Ring");
        let verdict = title_verdict(&omnibus, &volume);
        assert!(
            matches!(
                verdict,
                TitleVerdict::Grey {
                    cause: GreyCause::OneSidedSubtitle,
                    ..
                }
            ),
            "expected OneSidedSubtitle, got {verdict:?}"
        );
        assert!(title_id_trust(
            &verdict,
            &IdEvidence::default(),
            &IdEvidence::default()
        ));

        // What bounds that acceptance: a numbered marker parses into
        // `series_markers`, so the pair lands in VolumeAsymmetry or VetoVolume —
        // neither of which any arm of `title_id_trust` accepts. If this stops
        // holding, the acceptance above is unbounded and the rule change must be
        // revisited.
        for numbered in [
            "The Lord of the Rings: Book One",
            "The Lord of the Rings #2",
            "The Lord of the Rings, Vol. 2",
            "The Lord of the Rings, Volume II",
        ] {
            let numbered_verdict = title_verdict(&omnibus, &parse_title(numbered));
            assert!(
                !matches!(
                    numbered_verdict,
                    TitleVerdict::Grey {
                        cause: GreyCause::OneSidedSubtitle,
                        ..
                    }
                ),
                "{numbered} must not land in OneSidedSubtitle: got {numbered_verdict:?}"
            );
            assert!(
                !title_id_trust(
                    &numbered_verdict,
                    &IdEvidence::default(),
                    &IdEvidence::default()
                ),
                "{numbered} must not be trusted: got {numbered_verdict:?}"
            );
        }
    }

    #[test]
    fn title_id_trust_guards_for_unsupported_title_and_evidence_shapes() {
        // Guards: exact title needs no ID evidence; every grey cause other than
        // the one-sided subtitle stays untrusted even with a bridging ID.
        assert!(title_id_trust(
            &TitleVerdict::Same,
            &IdEvidence::default(),
            &IdEvidence::default()
        ));

        let bridge_a = IdEvidence {
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        let bridge_b = IdEvidence {
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        for cause in [
            GreyCause::NearMain,
            GreyCause::SubtitleDisagreement,
            GreyCause::VolumeAsymmetry,
        ] {
            assert!(!title_id_trust(
                &TitleVerdict::Grey { score: 1.0, cause },
                &bridge_a,
                &bridge_b
            ));
        }

        let work_a = IdEvidence {
            ol_key: Some("OL-A"),
            ..Default::default()
        };
        let work_b = IdEvidence {
            ol_key: Some("OL-A"),
            ..Default::default()
        };
        assert!(!title_id_trust(&TitleVerdict::Different, &work_a, &work_b));
        assert!(!title_id_trust(&TitleVerdict::VetoVolume, &work_a, &work_b));
    }

    #[test]
    fn title_id_trust_vetoes_isbn_bridge_when_same_provider_work_keys_differ() {
        let a = IdEvidence {
            gr_key: Some("1"),
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        let b = IdEvidence {
            gr_key: Some("2"),
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };

        assert!(!title_id_trust(&TitleVerdict::Same, &a, &b));
    }

    #[test]
    fn blank_keys_are_absent_not_equal() {
        let a = IdEvidence {
            ol_key: Some("  "),
            ..Default::default()
        };
        let b = IdEvidence {
            ol_key: Some("  "),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::NoEvidence);
    }

    // --- pick_best_candidate ---

    fn hit(title: &str, author: &str) -> (String, String) {
        (title.to_string(), author.to_string())
    }

    fn one_sided_subtitle_grey(seed: &str, candidate: &str) {
        assert!(matches!(
            title_verdict(&parse_title(seed), &parse_title(candidate)),
            TitleVerdict::Grey {
                cause: GreyCause::OneSidedSubtitle,
                ..
            }
        ));
    }

    fn assert_dune_same_agree() {
        assert_eq!(
            title_verdict(&parse_title("Dune"), &parse_title("Dune")),
            TitleVerdict::Same
        );
        assert_eq!(
            author_verdict(
                &["Frank Herbert".to_string()],
                &["Frank Herbert".to_string()]
            ),
            AuthorVerdict::Agree
        );
    }

    fn assert_dune_same_author_abstain() {
        assert_eq!(
            title_verdict(&parse_title("Dune"), &parse_title("Dune")),
            TitleVerdict::Same
        );
        assert_eq!(
            author_verdict(&["Frank Herbert".to_string()], &["".to_string()]),
            AuthorVerdict::Abstain
        );
    }

    fn assert_dune_messiah_rejected_before_same() {
        assert_eq!(
            title_verdict(&parse_title("Dune"), &parse_title("Dune Messiah")),
            TitleVerdict::Different
        );
        assert_eq!(
            title_verdict(&parse_title("Dune"), &parse_title("Dune")),
            TitleVerdict::Same
        );
    }

    fn world_war_z_grey_candidates() -> Vec<(String, String)> {
        vec![
            hit("Neuromancer", "William Gibson"),
            hit(
                "World War Z: An Oral History of the Zombie War",
                "Max Brooks",
            ),
        ]
    }

    #[test]
    fn pick_a1_accepts_same_agree_when_grey_disabled() {
        let candidates = vec![hit("Dune", "Frank Herbert")];
        assert_dune_same_agree();

        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &candidates, false),
            Some(0)
        );
    }

    #[test]
    fn pick_a1_accepts_same_agree_when_grey_enabled() {
        let candidates = vec![hit("Dune", "Frank Herbert")];
        assert_dune_same_agree();

        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &candidates, true),
            Some(0)
        );
    }

    #[test]
    fn pick_a2_accepts_same_abstain_when_grey_disabled() {
        let candidates = vec![hit("Dune", "")];
        assert_dune_same_author_abstain();

        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &candidates, false),
            Some(0)
        );
    }

    #[test]
    fn pick_a2_accepts_same_abstain_when_grey_enabled() {
        let candidates = vec![hit("Dune", "")];
        assert_dune_same_author_abstain();

        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &candidates, true),
            Some(0)
        );
    }

    #[test]
    fn pick_a3_rejects_one_sided_subtitle_grey_when_disabled() {
        let candidates = vec![hit(
            "World War Z: An Oral History of the Zombie War",
            "Max Brooks",
        )];
        one_sided_subtitle_grey(
            "World War Z",
            "World War Z: An Oral History of the Zombie War",
        );
        assert_eq!(
            author_verdict(&["Max Brooks".to_string()], &["Max Brooks".to_string()]),
            AuthorVerdict::Agree
        );

        assert_eq!(
            pick_best_candidate("World War Z", "Max Brooks", &candidates, false),
            None
        );
    }

    #[test]
    fn pick_a3_accepts_one_sided_subtitle_grey_when_enabled() {
        let candidates = vec![hit(
            "World War Z: An Oral History of the Zombie War",
            "Max Brooks",
        )];
        one_sided_subtitle_grey(
            "World War Z",
            "World War Z: An Oral History of the Zombie War",
        );
        assert_eq!(
            author_verdict(&["Max Brooks".to_string()], &["Max Brooks".to_string()]),
            AuthorVerdict::Agree
        );

        assert_eq!(
            pick_best_candidate("World War Z", "Max Brooks", &candidates, true),
            Some(0)
        );
    }

    #[test]
    fn pick_rejects_same_title_with_shared_surname_author_grey() {
        let candidates = vec![hit("Storm Front", "Jane Smith")];
        assert_eq!(
            title_verdict(&parse_title("Storm Front"), &parse_title("Storm Front")),
            TitleVerdict::Same
        );
        assert_eq!(
            author_verdict(&["John Smith".to_string()], &["Jane Smith".to_string()]),
            AuthorVerdict::Grey
        );

        assert_eq!(
            pick_best_candidate("Storm Front", "John Smith", &candidates, false),
            None
        );
        assert_eq!(
            pick_best_candidate("Storm Front", "John Smith", &candidates, true),
            None
        );
    }

    #[test]
    fn pick_a5_skips_rejected_hit_when_grey_disabled() {
        let candidates = vec![
            hit("Dune Messiah", "Frank Herbert"),
            hit("Dune", "Frank Herbert"),
        ];
        assert_dune_messiah_rejected_before_same();

        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &candidates, false),
            Some(1)
        );
    }

    #[test]
    fn pick_a5_skips_rejected_hit_when_grey_enabled() {
        let candidates = vec![
            hit("Dune Messiah", "Frank Herbert"),
            hit("Dune", "Frank Herbert"),
        ];
        assert_dune_messiah_rejected_before_same();

        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &candidates, true),
            Some(1)
        );
    }

    #[test]
    fn pick_a6_ranks_same_above_grey_when_grey_is_enabled() {
        let candidates = vec![
            hit("The Power Broker: The Life of Robert Moses", "Robert Caro"),
            hit("The Power Broker", "Robert Caro"),
        ];
        one_sided_subtitle_grey(
            "The Power Broker",
            "The Power Broker: The Life of Robert Moses",
        );
        assert_eq!(
            title_verdict(
                &parse_title("The Power Broker"),
                &parse_title("The Power Broker")
            ),
            TitleVerdict::Same
        );

        assert_eq!(
            pick_best_candidate("The Power Broker", "Robert Caro", &candidates, true),
            Some(1)
        );
    }

    #[test]
    fn pick_rejects_hard_title_and_author_mismatches() {
        let different_title = vec![hit("Neuromancer", "William Gibson")];
        assert_eq!(
            title_verdict(&parse_title("Dune"), &parse_title("Neuromancer")),
            TitleVerdict::Different
        );
        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &different_title, false),
            None
        );
        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &different_title, true),
            None
        );

        let veto_volume = vec![hit("Alpha, Vol. 3", "Ann Author")];
        assert_eq!(
            title_verdict(&parse_title("Alpha, Vol. 2"), &parse_title("Alpha, Vol. 3")),
            TitleVerdict::VetoVolume
        );
        assert_eq!(
            pick_best_candidate("Alpha, Vol. 2", "Ann Author", &veto_volume, false),
            None
        );
        assert_eq!(
            pick_best_candidate("Alpha, Vol. 2", "Ann Author", &veto_volume, true),
            None
        );

        let author_disagree = vec![hit("Dune", "William Gibson")];
        assert_eq!(
            author_verdict(
                &["Frank Herbert".to_string()],
                &["William Gibson".to_string()]
            ),
            AuthorVerdict::Disagree
        );
        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &author_disagree, false),
            None
        );
        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &author_disagree, true),
            None
        );
    }

    #[test]
    fn pick_a8_abstains_on_empty_candidates() {
        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &[], false),
            None
        );
        assert_eq!(
            pick_best_candidate("Dune", "Frank Herbert", &[], true),
            None
        );
    }

    #[test]
    fn pick_a8_rejects_only_grey_candidate_when_disabled() {
        let candidates = world_war_z_grey_candidates();
        one_sided_subtitle_grey(
            "World War Z",
            "World War Z: An Oral History of the Zombie War",
        );

        assert_eq!(
            pick_best_candidate("World War Z", "Max Brooks", &candidates, false),
            None
        );
    }

    #[test]
    fn pick_a8_accepts_only_grey_candidate_when_enabled() {
        let candidates = world_war_z_grey_candidates();
        one_sided_subtitle_grey(
            "World War Z",
            "World War Z: An Oral History of the Zombie War",
        );

        assert_eq!(
            pick_best_candidate("World War Z", "Max Brooks", &candidates, true),
            Some(1)
        );
    }

    // --- identity_key ---

    #[test]
    fn identity_key_is_deterministic() {
        let a = identity_key("The Hobbit", "J.R.R. Tolkien");
        let b = identity_key("The Hobbit", "J.R.R. Tolkien");
        assert_eq!(a, b);
    }

    #[test]
    fn identity_key_title_drops_leading_article_and_accents() {
        let (main, _) = identity_key("The Hobbit", "Author");
        assert_eq!(main, "hobbit");

        let (main, _) = identity_key("Café", "Author");
        assert_eq!(main, "cafe");
    }

    #[test]
    fn identity_key_junk_tail_folds_to_bare_title() {
        // A junk tail ("A Novel") is stripped by the parse and never enters
        // the key — the ST-04 acceptance shape: a junk-tail variant of a
        // stored title computes the SAME key and can adopt.
        let (key_a, _) = identity_key("Dune: A Novel", "Author");
        let (key_b, _) = identity_key("Dune", "Author");
        assert_eq!(key_a, key_b);
    }

    #[test]
    fn identity_key_one_sided_volume_marker_keeps_distinct() {
        // A volume marker on one side enters the key triple: the pair
        // misses at the exact-equality seat (falls to the dedup cascade,
        // which lands grey — a visible duplicate, never a silent absorb).
        let (key_a, _) = identity_key("Storm Front: The Dresden Files, Book 1", "Author");
        let (key_b, _) = identity_key("Storm Front", "Author");
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn identity_key_true_subtitle_siblings_stay_distinct() {
        // Series siblings share a main title but differ in subtitle — the
        // triple keeps their keys distinct so BOTH persist under the UNIQUE
        // index + ON CONFLICT DO NOTHING backstop.
        let (key_a, _) = identity_key("Mistborn: The Final Empire", "Brandon Sanderson");
        let (key_b, _) = identity_key("Mistborn: The Well of Ascension", "Brandon Sanderson");
        let (key_bare, _) = identity_key("Mistborn", "Brandon Sanderson");
        assert_ne!(key_a, key_b);
        assert_ne!(key_a, key_bare);
        assert_ne!(key_b, key_bare);
    }

    #[test]
    fn identity_key_volume_siblings_stay_distinct() {
        let (key_a, _) = identity_key("History of Rome: Volume 1", "Author");
        let (key_b, _) = identity_key("History of Rome: Volume 2", "Author");
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn identity_key_matching_volume_variants_fold() {
        // Different spellings of the SAME volume ("Volume 1" tail vs
        // ", Vol. 1" comma marker) extract the same number and compute the
        // same key — variant forms of one book still fold.
        let (key_a, _) = identity_key("History of Rome: Volume 1", "Author");
        let (key_b, _) = identity_key("History of Rome, Vol. 1", "Author");
        assert_eq!(key_a, key_b);

        let (key_c, _) = identity_key("Foo: Book Three", "Author");
        let (key_d, _) = identity_key("Foo, Vol. 3", "Author");
        assert_eq!(key_c, key_d);
    }

    #[test]
    fn identity_key_plain_title_key_is_bare_main() {
        // Trailing empty segments are dropped: a plain title's key is just
        // its cleaned main, with no separator characters.
        let (key, _) = identity_key("Dune", "Author");
        assert_eq!(key, "dune");
        assert!(!key.contains('\u{1}'));
    }

    #[test]
    fn identity_key_author_reorders_last_first_and_lowercases() {
        let (_, author_a) = identity_key("Title", "Herbert, Frank");
        let (_, author_b) = identity_key("Title", "frank herbert");
        assert_eq!(author_a, author_b);
        assert_eq!(author_a, "frank herbert");
    }

    #[test]
    fn identity_key_empty_side_yields_empty_component() {
        let (main, author) = identity_key("Dune", "");
        assert_eq!(main, "dune");
        assert_eq!(author, "");

        let (main, author) = identity_key("", "Frank Herbert");
        assert_eq!(main, "");
        assert_eq!(author, "frank herbert");
    }

    #[test]
    fn identity_key_differs_from_old_recipe_on_leading_article() {
        // The retired normalize_for_matching kept stopwords (no article
        // drop); identity_key's title component does drop them (REQ-014 —
        // this IS the behavior change the recompute migration exists for).
        let (main, _) = identity_key("The Hobbit", "Author");
        assert_ne!(main, "the hobbit");
    }

    // --- identity_key_flat (the scan/filename comparison form) ---

    #[test]
    fn flat_sanitized_colon_stem_matches_subtitled_work() {
        // The rescan regression case, dead: sanitize_path_component wrote
        // ":" as "_", so the stem carries no separator — its buried "The"
        // and the work side's segmented subtitle reconcile only in the
        // flattened form.
        let stem = identity_key_flat("Mistborn_ The Final Empire", "Brandon Sanderson");
        let work = identity_key_flat("Mistborn: The Final Empire", "Brandon Sanderson");
        assert_eq!(stem, work);
    }

    #[test]
    fn flat_sibling_stems_never_cross_match() {
        let final_empire = identity_key_flat("Mistborn: The Final Empire", "Brandon Sanderson");
        let well_of_ascension =
            identity_key_flat("Mistborn: The Well of Ascension", "Brandon Sanderson");
        assert_ne!(final_empire, well_of_ascension);

        // A sanitized sibling stem matches its OWN work only.
        let sibling_stem =
            identity_key_flat("Mistborn_ The Well of Ascension", "Brandon Sanderson");
        assert_eq!(sibling_stem, well_of_ascension);
        assert_ne!(sibling_stem, final_empire);
    }

    #[test]
    fn flat_bare_stem_does_not_match_subtitled_work() {
        // A bare-titled file against a subtitled work is grey territory —
        // it falls to the existing fuzzy/manual import path, never a silent
        // flat match.
        let bare = identity_key_flat("Mistborn", "Brandon Sanderson");
        let subtitled = identity_key_flat("Mistborn: The Final Empire", "Brandon Sanderson");
        assert_ne!(bare, subtitled);
    }

    #[test]
    fn flat_junk_tail_and_accent_folding_hold() {
        let junk = identity_key_flat("Dune: A Novel", "Frank Herbert");
        let bare = identity_key_flat("Dune", "Frank Herbert");
        assert_eq!(junk, bare);

        let accented = identity_key_flat("CAFÉ WORLD", "J. Author");
        let plain = identity_key_flat("Cafe World", "j. author");
        assert_eq!(accented, plain);
    }
}
