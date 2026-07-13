use std::collections::HashMap;
use std::path::Path;

use livrarr_domain::{
    CoverMediaType, CoverResolution, CoverTrust, MetadataProvider, OutcomeClass, Work,
};

use crate::cover_rank::{rank_index, CoverRankModel};
use crate::{NormalizedWorkDetail, ReconstructedOutcome};

pub fn is_good_enough(width: u32, height: u32) -> bool {
    width >= 400 && height >= 600
}

pub fn derive_cover_trust(outcome: &OutcomeClass) -> CoverTrust {
    match outcome {
        OutcomeClass::Success => CoverTrust::Validated,
        _ => CoverTrust::Unvalidated,
    }
}

pub fn phase1_trust(user_initiated: bool, is_fallback: bool) -> CoverTrust {
    if is_fallback {
        return CoverTrust::Unvalidated;
    }
    if user_initiated {
        CoverTrust::Validated
    } else {
        CoverTrust::Unvalidated
    }
}

const REJECT_SUBSTRINGS: &[&str] = &[
    "summary",
    "study guide",
    "sparknotes",
    "cliffsnotes",
    "bookrags",
    "analysis",
    "supersummary",
    "companion",
    "reader's guide",
];

pub fn should_reject_cover(edition_title: Option<&str>, work_title: &str) -> bool {
    let edition = match edition_title {
        Some(t) => t,
        None => return false,
    };

    let work_lower = work_title.to_lowercase();
    if REJECT_SUBSTRINGS.iter().any(|sub| work_lower.contains(sub)) {
        return false;
    }

    let edition_lower = edition.to_lowercase();
    REJECT_SUBSTRINGS
        .iter()
        .any(|sub| edition_lower.contains(sub))
}

pub fn measure_dimensions(path: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}

pub fn resolve_cover(
    current_work: &Work,
    media_type: CoverMediaType,
    priority_list: &[MetadataProvider],
    eligible: &HashMap<MetadataProvider, Option<&NormalizedWorkDetail>>,
    outcomes: &HashMap<MetadataProvider, &ReconstructedOutcome>,
) -> Option<CoverResolution> {
    let current_trust = match media_type {
        CoverMediaType::Ebook => current_work.cover_trust,
        CoverMediaType::Audiobook => current_work.audiobook_cover_trust,
    };
    if current_trust == CoverTrust::User {
        return None;
    }

    struct Candidate {
        provider: MetadataProvider,
        url: String,
        trust: CoverTrust,
    }

    let mut candidates: Vec<Candidate> = Vec::new();

    for &provider in priority_list {
        let detail = match eligible.get(&provider) {
            Some(Some(d)) => d,
            _ => continue,
        };
        let url = match &detail.cover_url {
            Some(u) if !u.is_empty() => u.clone(),
            _ => continue,
        };
        if should_reject_cover(detail.title.as_deref(), &current_work.title) {
            continue;
        }
        let outcome = match outcomes.get(&provider) {
            Some(o) => o,
            None => continue,
        };
        let trust = derive_cover_trust(&outcome.class);
        candidates.push(Candidate {
            provider,
            url,
            trust,
        });
    }

    // Prefer validated over unvalidated; within same tier, priority_list order wins.
    let winner = candidates
        .iter()
        .find(|c| c.trust == CoverTrust::Validated)
        .or_else(|| {
            candidates
                .iter()
                .find(|c| c.trust == CoverTrust::Unvalidated)
        })?;

    if !current_trust.allows_replacement_by(winner.trust) {
        return None;
    }

    Some(CoverResolution {
        url: winner.url.clone(),
        source: format!("{:?}", winner.provider).to_lowercase(),
        trust: winner.trust,
        media_type,
    })
}

/// Same-tier comparison used by the consolidated save gate
/// (`livrarr_metadata::cover_write_gate`) when a candidate's trust equals the
/// incumbent's: the 400x600 floor, good-beats-bad, both-good-> unified rank,
/// both-bad-> larger pixel area, tie-> rank (S2, AS BUILT — unchanged rules).
/// `rank_model` supplies the ONE rank table (S1): callers derive it from the
/// work's language + media type via `CoverRankModel::for_media`.
pub fn should_upgrade_same_tier(
    current_w: u32,
    current_h: u32,
    new_w: u32,
    new_h: u32,
    current_source: Option<&str>,
    new_source: &str,
    rank_model: CoverRankModel,
) -> bool {
    let current_good = is_good_enough(current_w, current_h);
    let new_good = is_good_enough(new_w, new_h);

    let priority_of = |source: &str| -> usize {
        match provider_from_source_str(source) {
            Some(p) => rank_index(p, rank_model),
            None => usize::MAX,
        }
    };

    match (new_good, current_good) {
        (true, false) => true,
        (false, true) => false,
        (true, true) => priority_of(new_source) < priority_of(current_source.unwrap_or("")),
        (false, false) => {
            let new_area = (new_w as u64) * (new_h as u64);
            let cur_area = (current_w as u64) * (current_h as u64);
            if new_area != cur_area {
                new_area > cur_area
            } else {
                priority_of(new_source) < priority_of(current_source.unwrap_or(""))
            }
        }
    }
}

/// Map a stored `cover_source` string (the lowercased `MetadataProvider`
/// debug name every write path stamps, e.g. "goodreads", "googlebooks") back
/// to the provider for a rank lookup. Any other value (a non-provider source
/// string such as "epub"/"isbn_ol"/"user_upload", or unrecognized) has no
/// rank position.
fn provider_from_source_str(source: &str) -> Option<MetadataProvider> {
    match source {
        "goodreads" => Some(MetadataProvider::Goodreads),
        "hardcover" => Some(MetadataProvider::Hardcover),
        "openlibrary" => Some(MetadataProvider::OpenLibrary),
        "audnexus" => Some(MetadataProvider::Audnexus),
        "readarr" => Some(MetadataProvider::Readarr),
        "googlebooks" => Some(MetadataProvider::GoogleBooks),
        "audible" => Some(MetadataProvider::Audible),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_good_enough_boundary() {
        assert!(is_good_enough(400, 600));
        assert!(!is_good_enough(399, 600));
        assert!(!is_good_enough(400, 599));
        assert!(!is_good_enough(0, 0));
        assert!(is_good_enough(1400, 2100));
    }

    #[test]
    fn derive_cover_trust_maps_success_to_validated() {
        assert_eq!(
            derive_cover_trust(&OutcomeClass::Success),
            CoverTrust::Validated
        );
    }

    #[test]
    fn derive_cover_trust_maps_others_to_unvalidated() {
        assert_eq!(
            derive_cover_trust(&OutcomeClass::NotFound),
            CoverTrust::Unvalidated
        );
        assert_eq!(
            derive_cover_trust(&OutcomeClass::WillRetry),
            CoverTrust::Unvalidated
        );
        assert_eq!(
            derive_cover_trust(&OutcomeClass::PermanentFailure),
            CoverTrust::Unvalidated
        );
        assert_eq!(
            derive_cover_trust(&OutcomeClass::Conflict),
            CoverTrust::Unvalidated
        );
    }

    #[test]
    fn phase1_trust_user_initiated_not_fallback() {
        assert_eq!(phase1_trust(true, false), CoverTrust::Validated);
    }

    #[test]
    fn phase1_trust_auto_add() {
        assert_eq!(phase1_trust(false, false), CoverTrust::Unvalidated);
    }

    #[test]
    fn phase1_trust_fallback_overrides_user_initiated() {
        assert_eq!(phase1_trust(true, true), CoverTrust::Unvalidated);
    }

    #[test]
    fn should_reject_none_edition_title() {
        assert!(!should_reject_cover(None, "The Great Gatsby"));
    }

    #[test]
    fn should_reject_study_guide_edition() {
        assert!(should_reject_cover(
            Some("The Great Gatsby: Study Guide"),
            "The Great Gatsby"
        ));
    }

    #[test]
    fn should_reject_clean_edition() {
        assert!(!should_reject_cover(
            Some("The Great Gatsby"),
            "The Great Gatsby"
        ));
    }

    #[test]
    fn should_reject_work_is_study_guide_exemption() {
        assert!(!should_reject_cover(
            Some("CliffsNotes: Gatsby"),
            "CliffsNotes: Gatsby"
        ));
    }

    #[test]
    fn should_reject_case_insensitive() {
        assert!(should_reject_cover(
            Some("SUPERSUMMARY of The Great Gatsby"),
            "The Great Gatsby"
        ));
    }

    #[test]
    fn should_reject_readers_guide() {
        assert!(should_reject_cover(
            Some("A Reader's Guide to Moby Dick"),
            "Moby Dick"
        ));
    }

    #[test]
    fn should_reject_all_keywords() {
        let work = "Some Novel";
        for keyword in REJECT_SUBSTRINGS {
            let edition = format!("Some Novel: {keyword}");
            assert!(
                should_reject_cover(Some(&edition), work),
                "expected reject for keyword '{keyword}'"
            );
        }
    }

    #[test]
    fn measure_dimensions_nonexistent_file() {
        assert_eq!(measure_dimensions(Path::new("/nonexistent/path.jpg")), None);
    }

    #[test]
    fn measure_dimensions_non_image() {
        assert_eq!(
            measure_dimensions(Path::new("/mnt/opt/livrarr/Cargo.toml")),
            None
        );
    }

    fn test_work(cover_trust: CoverTrust) -> Work {
        Work {
            cover_trust,
            ..Default::default()
        }
    }

    fn test_detail(cover_url: Option<&str>) -> NormalizedWorkDetail {
        NormalizedWorkDetail {
            cover_url: cover_url.map(String::from),
            ..Default::default()
        }
    }

    fn test_outcome(class: OutcomeClass) -> ReconstructedOutcome {
        ReconstructedOutcome {
            class,
            payload: None,
        }
    }

    #[test]
    fn resolve_cover_user_locked_returns_none() {
        let work = test_work(CoverTrust::User);
        let eligible = HashMap::new();
        let outcomes = HashMap::new();
        assert!(resolve_cover(&work, CoverMediaType::Ebook, &[], &eligible, &outcomes).is_none());
    }

    #[test]
    fn resolve_cover_no_candidates_returns_none() {
        let work = test_work(CoverTrust::Unvalidated);
        let eligible = HashMap::new();
        let outcomes = HashMap::new();
        assert!(resolve_cover(
            &work,
            CoverMediaType::Ebook,
            &[MetadataProvider::Goodreads],
            &eligible,
            &outcomes
        )
        .is_none());
    }

    #[test]
    fn resolve_cover_validated_beats_unvalidated() {
        let work = test_work(CoverTrust::Unvalidated);
        let detail_gr = test_detail(Some("https://gr.test/cover.jpg"));
        let detail_hc = test_detail(Some("https://hc.test/cover.jpg"));
        let outcome_gr = test_outcome(OutcomeClass::Success);
        let outcome_hc = test_outcome(OutcomeClass::NotFound);

        let eligible: HashMap<MetadataProvider, Option<&NormalizedWorkDetail>> = [
            (MetadataProvider::Hardcover, Some(&detail_hc)),
            (MetadataProvider::Goodreads, Some(&detail_gr)),
        ]
        .into();
        let outcomes: HashMap<MetadataProvider, &ReconstructedOutcome> = [
            (MetadataProvider::Hardcover, &outcome_hc),
            (MetadataProvider::Goodreads, &outcome_gr),
        ]
        .into();

        let priority = [MetadataProvider::Hardcover, MetadataProvider::Goodreads];
        let result = resolve_cover(
            &work,
            CoverMediaType::Ebook,
            &priority,
            &eligible,
            &outcomes,
        )
        .unwrap();
        assert_eq!(result.trust, CoverTrust::Validated);
        assert_eq!(result.url, "https://gr.test/cover.jpg");
    }

    #[test]
    fn resolve_cover_same_tier_uses_priority_order() {
        let work = test_work(CoverTrust::Unvalidated);
        let detail_hc = test_detail(Some("https://hc.test/cover.jpg"));
        let detail_gr = test_detail(Some("https://gr.test/cover.jpg"));
        let outcome_hc = test_outcome(OutcomeClass::Success);
        let outcome_gr = test_outcome(OutcomeClass::Success);

        let eligible: HashMap<MetadataProvider, Option<&NormalizedWorkDetail>> = [
            (MetadataProvider::Hardcover, Some(&detail_hc)),
            (MetadataProvider::Goodreads, Some(&detail_gr)),
        ]
        .into();
        let outcomes: HashMap<MetadataProvider, &ReconstructedOutcome> = [
            (MetadataProvider::Hardcover, &outcome_hc),
            (MetadataProvider::Goodreads, &outcome_gr),
        ]
        .into();

        // GR first in priority → GR wins
        let priority = [MetadataProvider::Goodreads, MetadataProvider::Hardcover];
        let result = resolve_cover(
            &work,
            CoverMediaType::Ebook,
            &priority,
            &eligible,
            &outcomes,
        )
        .unwrap();
        assert_eq!(result.url, "https://gr.test/cover.jpg");
    }

    #[test]
    fn resolve_cover_validated_current_rejects_unvalidated_candidate() {
        let work = test_work(CoverTrust::Validated);
        let detail = test_detail(Some("https://hc.test/cover.jpg"));
        let outcome = test_outcome(OutcomeClass::NotFound); // → Unvalidated

        let eligible: HashMap<MetadataProvider, Option<&NormalizedWorkDetail>> =
            [(MetadataProvider::Hardcover, Some(&detail))].into();
        let outcomes: HashMap<MetadataProvider, &ReconstructedOutcome> =
            [(MetadataProvider::Hardcover, &outcome)].into();

        let priority = [MetadataProvider::Hardcover];
        assert!(resolve_cover(
            &work,
            CoverMediaType::Ebook,
            &priority,
            &eligible,
            &outcomes
        )
        .is_none());
    }

    #[test]
    fn resolve_cover_unvalidated_first_by_priority() {
        let work = test_work(CoverTrust::Unvalidated);
        let detail = test_detail(Some("https://hc.test/cover.jpg"));
        let outcome = test_outcome(OutcomeClass::NotFound); // → Unvalidated

        let eligible: HashMap<MetadataProvider, Option<&NormalizedWorkDetail>> =
            [(MetadataProvider::Hardcover, Some(&detail))].into();
        let outcomes: HashMap<MetadataProvider, &ReconstructedOutcome> =
            [(MetadataProvider::Hardcover, &outcome)].into();

        let priority = [MetadataProvider::Hardcover];
        let result = resolve_cover(
            &work,
            CoverMediaType::Ebook,
            &priority,
            &eligible,
            &outcomes,
        )
        .unwrap();
        assert_eq!(result.trust, CoverTrust::Unvalidated);
        assert_eq!(result.media_type, CoverMediaType::Ebook);
    }

    #[test]
    fn resolve_cover_rejects_study_guide_edition() {
        let work = Work {
            identity_status: Default::default(),
            title: "The Great Gatsby".to_string(),
            cover_trust: CoverTrust::Unvalidated,
            ..Default::default()
        };
        let detail = NormalizedWorkDetail {
            cover_url: Some("https://gr.test/cover.jpg".to_string()),
            title: Some("The Great Gatsby: Study Guide".to_string()),
            ..Default::default()
        };
        let outcome = test_outcome(OutcomeClass::Success);

        let eligible: HashMap<MetadataProvider, Option<&NormalizedWorkDetail>> =
            [(MetadataProvider::Goodreads, Some(&detail))].into();
        let outcomes: HashMap<MetadataProvider, &ReconstructedOutcome> =
            [(MetadataProvider::Goodreads, &outcome)].into();

        let priority = [MetadataProvider::Goodreads];
        assert!(resolve_cover(
            &work,
            CoverMediaType::Ebook,
            &priority,
            &eligible,
            &outcomes
        )
        .is_none());
    }

    #[test]
    fn resolve_cover_allows_study_guide_work_own_covers() {
        let work = Work {
            identity_status: Default::default(),
            title: "CliffsNotes: The Great Gatsby".to_string(),
            cover_trust: CoverTrust::Unvalidated,
            ..Default::default()
        };
        let detail = NormalizedWorkDetail {
            cover_url: Some("https://gr.test/cover.jpg".to_string()),
            title: Some("CliffsNotes: The Great Gatsby".to_string()),
            ..Default::default()
        };
        let outcome = test_outcome(OutcomeClass::Success);

        let eligible: HashMap<MetadataProvider, Option<&NormalizedWorkDetail>> =
            [(MetadataProvider::Goodreads, Some(&detail))].into();
        let outcomes: HashMap<MetadataProvider, &ReconstructedOutcome> =
            [(MetadataProvider::Goodreads, &outcome)].into();

        let priority = [MetadataProvider::Goodreads];
        assert!(resolve_cover(
            &work,
            CoverMediaType::Ebook,
            &priority,
            &eligible,
            &outcomes
        )
        .is_some());
    }

    // -- should_upgrade_same_tier tests (S1: rank model replaces the old
    // per-media-type-only priority arrays) --

    #[test]
    fn same_tier_new_good_current_bad_upgrades() {
        assert!(should_upgrade_same_tier(
            200,
            300,
            400,
            600,
            Some("goodreads"),
            "hardcover",
            CoverRankModel::EbookEnglish,
        ));
    }

    #[test]
    fn same_tier_new_bad_current_good_keeps() {
        assert!(!should_upgrade_same_tier(
            400,
            600,
            200,
            300,
            Some("goodreads"),
            "hardcover",
            CoverRankModel::EbookEnglish,
        ));
    }

    #[test]
    fn same_tier_both_good_higher_priority_wins() {
        // ebook english: goodreads index 0 < hardcover index 1 → goodreads wins
        assert!(should_upgrade_same_tier(
            400,
            600,
            500,
            700,
            Some("hardcover"),
            "goodreads",
            CoverRankModel::EbookEnglish,
        ));
        // reverse: hardcover does not beat goodreads
        assert!(!should_upgrade_same_tier(
            400,
            600,
            500,
            700,
            Some("goodreads"),
            "hardcover",
            CoverRankModel::EbookEnglish,
        ));
    }

    #[test]
    fn same_tier_neither_good_larger_area_wins() {
        // 300×400=120k vs 200×300=60k → upgrade
        assert!(should_upgrade_same_tier(
            200,
            300,
            300,
            400,
            Some("goodreads"),
            "goodreads",
            CoverRankModel::EbookEnglish,
        ));
        // 200×300=60k vs 300×400=120k → keep
        assert!(!should_upgrade_same_tier(
            300,
            400,
            200,
            300,
            Some("goodreads"),
            "goodreads",
            CoverRankModel::EbookEnglish,
        ));
    }

    #[test]
    fn same_tier_neither_good_equal_area_priority_wins() {
        // goodreads (index 0) beats hardcover (index 1) for ebook english
        assert!(should_upgrade_same_tier(
            200,
            300,
            200,
            300,
            Some("hardcover"),
            "goodreads",
            CoverRankModel::EbookEnglish,
        ));
        assert!(!should_upgrade_same_tier(
            200,
            300,
            200,
            300,
            Some("goodreads"),
            "hardcover",
            CoverRankModel::EbookEnglish,
        ));
    }

    #[test]
    fn same_tier_ebook_foreign_prefers_googlebooks_over_goodreads() {
        // S1: foreign order is GB -> GR -> HC -> ... — the opposite of English.
        assert!(should_upgrade_same_tier(
            400,
            600,
            500,
            700,
            Some("goodreads"),
            "googlebooks",
            CoverRankModel::EbookForeign,
        ));
        assert!(!should_upgrade_same_tier(
            400,
            600,
            500,
            700,
            Some("googlebooks"),
            "goodreads",
            CoverRankModel::EbookForeign,
        ));
    }

    #[test]
    fn audiobook_priority_audible_beats_hardcover() {
        // For an existing HC audiobook cover, an Audible candidate at the same
        // trust tier and same dimensions must win — Audible is the canonical
        // audiobook source. This is the regression #95 came back to fix.
        assert!(should_upgrade_same_tier(
            500,
            700,
            500,
            700,
            Some("hardcover"),
            "audible",
            CoverRankModel::Audiobook,
        ));
        // Inverse: HC must NOT replace an existing Audible audiobook cover.
        assert!(!should_upgrade_same_tier(
            500,
            700,
            500,
            700,
            Some("audible"),
            "hardcover",
            CoverRankModel::Audiobook,
        ));
    }

    #[test]
    fn provider_from_source_str_known_sources() {
        assert_eq!(
            provider_from_source_str("goodreads"),
            Some(MetadataProvider::Goodreads)
        );
        assert_eq!(
            provider_from_source_str("hardcover"),
            Some(MetadataProvider::Hardcover)
        );
        assert_eq!(
            provider_from_source_str("openlibrary"),
            Some(MetadataProvider::OpenLibrary)
        );
        assert_eq!(
            provider_from_source_str("googlebooks"),
            Some(MetadataProvider::GoogleBooks)
        );
        assert_eq!(
            provider_from_source_str("audnexus"),
            Some(MetadataProvider::Audnexus)
        );
        assert_eq!(
            provider_from_source_str("audible"),
            Some(MetadataProvider::Audible)
        );
        assert_eq!(
            provider_from_source_str("readarr"),
            Some(MetadataProvider::Readarr)
        );
    }

    #[test]
    fn provider_from_source_str_unknown_is_none() {
        assert_eq!(provider_from_source_str("unknown"), None);
        assert_eq!(provider_from_source_str("epub"), None);
        assert_eq!(provider_from_source_str("user_upload"), None);
    }

    #[test]
    fn same_tier_unknown_current_source_loses_to_any_known_provider() {
        // An unranked incumbent source (e.g. a legacy "add" stamp) must not
        // out-rank a real provider at the same trust tier and dimension tier.
        assert!(should_upgrade_same_tier(
            200,
            300,
            200,
            300,
            Some("add"),
            "openlibrary",
            CoverRankModel::EbookEnglish,
        ));
    }
}
