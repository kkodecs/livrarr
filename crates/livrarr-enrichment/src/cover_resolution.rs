use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use livrarr_domain::services::{
    FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::{
    CoverMediaType, CoverResolution, CoverTrust, MetadataProvider, OutcomeClass, RequestPriority,
    Work,
};

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

#[derive(Debug, Clone)]
pub struct CoverUpgradeResult {
    pub url: String,
    pub source: String,
    pub trust: CoverTrust,
    pub width: u32,
    pub height: u32,
    pub media_type: CoverMediaType,
}

#[derive(Debug, thiserror::Error)]
pub enum CoverError {
    #[error("download failed: {0}")]
    Download(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

const COVER_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10);
const COVER_MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Cover provider priority for same-tier comparison, per media type.
/// Lower index = higher priority. Audiobook covers must prefer audiobook-native
/// sources (Audible, Audnexus) over ebook providers, otherwise a refresh can
/// silently overwrite an existing Audible audiobook cover with a Hardcover
/// ebook one at the same trust tier.
const EBOOK_COVER_PRIORITY: &[&str] = &[
    "goodreads",
    "hardcover",
    "openlibrary",
    "googlebooks",
    "audnexus",
    "audible",
];

const AUDIOBOOK_COVER_PRIORITY: &[&str] = &[
    "audible",
    "audnexus",
    "hardcover",
    "goodreads",
    "openlibrary",
    "googlebooks",
];

fn provider_priority_index(source: &str, media_type: CoverMediaType) -> usize {
    let list = match media_type {
        CoverMediaType::Ebook => EBOOK_COVER_PRIORITY,
        CoverMediaType::Audiobook => AUDIOBOOK_COVER_PRIORITY,
    };
    list.iter().position(|&s| s == source).unwrap_or(list.len())
}

fn should_upgrade_same_tier(
    current_w: u32,
    current_h: u32,
    new_w: u32,
    new_h: u32,
    current_source: Option<&str>,
    new_source: &str,
    media_type: CoverMediaType,
) -> bool {
    let current_good = is_good_enough(current_w, current_h);
    let new_good = is_good_enough(new_w, new_h);

    match (new_good, current_good) {
        (true, false) => true,
        (false, true) => false,
        (true, true) => {
            let new_pri = provider_priority_index(new_source, media_type);
            let cur_pri = provider_priority_index(current_source.unwrap_or(""), media_type);
            new_pri < cur_pri
        }
        (false, false) => {
            let new_area = (new_w as u64) * (new_h as u64);
            let cur_area = (current_w as u64) * (current_h as u64);
            if new_area != cur_area {
                new_area > cur_area
            } else {
                let new_pri = provider_priority_index(new_source, media_type);
                let cur_pri = provider_priority_index(current_source.unwrap_or(""), media_type);
                new_pri < cur_pri
            }
        }
    }
}

pub async fn maybe_upgrade_cover<H: HttpFetcher>(
    work: &Work,
    resolution: Option<CoverResolution>,
    covers_dir: &Path,
    http: &H,
) -> Result<Option<CoverUpgradeResult>, CoverError> {
    let resolution = match resolution {
        Some(r) => r,
        None => return Ok(None),
    };

    let (current_trust, current_w_db, current_h_db, current_source) = match resolution.media_type {
        CoverMediaType::Ebook => (
            work.cover_trust,
            work.cover_width,
            work.cover_height,
            work.cover_source.as_deref(),
        ),
        CoverMediaType::Audiobook => (
            work.audiobook_cover_trust,
            work.audiobook_cover_width,
            work.audiobook_cover_height,
            work.audiobook_cover_source.as_deref(),
        ),
    };

    if current_trust == CoverTrust::User {
        return Ok(None);
    }

    if !current_trust.allows_replacement_by(resolution.trust) {
        return Ok(None);
    }

    let suffix = resolution.media_type.suffix();
    let cover_path = covers_dir.join(format!("{}{suffix}.jpg", work.id));
    let candidate_path = covers_dir.join(format!("{}{suffix}.candidate.tmp", work.id));

    // Lazy backfill: measure current if dimensions are 0×0
    let (current_w, current_h) = if current_w_db == 0 && current_h_db == 0 {
        measure_dimensions(&cover_path).unwrap_or((0, 0))
    } else {
        (current_w_db as u32, current_h_db as u32)
    };

    // Download candidate to temp path
    tokio::fs::create_dir_all(covers_dir).await?;

    let req = FetchRequest {
        url: resolution.url.clone(),
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: COVER_DOWNLOAD_TIMEOUT,
        rate_bucket: RateBucket::None,
        max_body_bytes: COVER_MAX_BODY_BYTES,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority: RequestPriority::Normal,
    };

    let resp = http
        .fetch_ssrf_safe(req)
        .await
        .map_err(|e| CoverError::Download(e.to_string()))?;

    if resp.status >= 400 {
        return Err(CoverError::Download(format!("HTTP {}", resp.status)));
    }

    let cpath = candidate_path.clone();
    let bytes = resp.body;
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&cpath)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        Ok(())
    })
    .await
    .map_err(|e| CoverError::Download(e.to_string()))??;

    // Measure candidate dimensions
    let (new_w, new_h) = measure_dimensions(&candidate_path).unwrap_or((0, 0));

    // Same-tier comparison
    let do_upgrade = if resolution.trust != current_trust {
        true // higher trust always wins (already gated above)
    } else {
        should_upgrade_same_tier(
            current_w,
            current_h,
            new_w,
            new_h,
            current_source,
            &resolution.source,
            resolution.media_type,
        )
    };

    if do_upgrade {
        tokio::fs::rename(&candidate_path, &cover_path).await?;

        // Invalidate thumbnail
        let thumb_path = covers_dir.join(format!("{}{suffix}_thumb.jpg", work.id));
        let _ = tokio::fs::remove_file(&thumb_path).await;

        // If ebook upgrade and audiobook has no dedicated cover, invalidate audio thumb too
        if resolution.media_type == CoverMediaType::Ebook && work.audiobook_cover_url.is_none() {
            let audio_thumb = covers_dir.join(format!("{}_audio_thumb.jpg", work.id));
            let _ = tokio::fs::remove_file(&audio_thumb).await;
        }

        Ok(Some(CoverUpgradeResult {
            url: resolution.url,
            source: resolution.source,
            trust: resolution.trust,
            width: new_w,
            height: new_h,
            media_type: resolution.media_type,
        }))
    } else {
        let _ = tokio::fs::remove_file(&candidate_path).await;
        Ok(None)
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
        assert_eq!(
            derive_cover_trust(&OutcomeClass::Suppressed),
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

    // -- should_upgrade_same_tier tests --

    #[test]
    fn same_tier_new_good_current_bad_upgrades() {
        assert!(should_upgrade_same_tier(
            200,
            300,
            400,
            600,
            Some("goodreads"),
            "hardcover",
            CoverMediaType::Ebook,
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
            CoverMediaType::Ebook,
        ));
    }

    #[test]
    fn same_tier_both_good_higher_priority_wins() {
        // ebook: goodreads index 0 < hardcover index 1 → goodreads wins
        assert!(should_upgrade_same_tier(
            400,
            600,
            500,
            700,
            Some("hardcover"),
            "goodreads",
            CoverMediaType::Ebook,
        ));
        // reverse: hardcover does not beat goodreads
        assert!(!should_upgrade_same_tier(
            400,
            600,
            500,
            700,
            Some("goodreads"),
            "hardcover",
            CoverMediaType::Ebook,
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
            CoverMediaType::Ebook,
        ));
        // 200×300=60k vs 300×400=120k → keep
        assert!(!should_upgrade_same_tier(
            300,
            400,
            200,
            300,
            Some("goodreads"),
            "goodreads",
            CoverMediaType::Ebook,
        ));
    }

    #[test]
    fn same_tier_neither_good_equal_area_priority_wins() {
        // goodreads (index 0) beats hardcover (index 1) for ebook
        assert!(should_upgrade_same_tier(
            200,
            300,
            200,
            300,
            Some("hardcover"),
            "goodreads",
            CoverMediaType::Ebook,
        ));
        assert!(!should_upgrade_same_tier(
            200,
            300,
            200,
            300,
            Some("goodreads"),
            "hardcover",
            CoverMediaType::Ebook,
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
            CoverMediaType::Audiobook,
        ));
        // Inverse: HC must NOT replace an existing Audible audiobook cover.
        assert!(!should_upgrade_same_tier(
            500,
            700,
            500,
            700,
            Some("audible"),
            "hardcover",
            CoverMediaType::Audiobook,
        ));
    }

    #[test]
    fn provider_priority_known_sources_ebook() {
        assert_eq!(
            provider_priority_index("goodreads", CoverMediaType::Ebook),
            0
        );
        assert_eq!(
            provider_priority_index("hardcover", CoverMediaType::Ebook),
            1
        );
        assert_eq!(
            provider_priority_index("openlibrary", CoverMediaType::Ebook),
            2
        );
        assert_eq!(
            provider_priority_index("audnexus", CoverMediaType::Ebook),
            4
        );
    }

    #[test]
    fn provider_priority_known_sources_audiobook() {
        assert_eq!(
            provider_priority_index("audible", CoverMediaType::Audiobook),
            0
        );
        assert_eq!(
            provider_priority_index("audnexus", CoverMediaType::Audiobook),
            1
        );
        assert_eq!(
            provider_priority_index("hardcover", CoverMediaType::Audiobook),
            2
        );
    }

    #[test]
    fn provider_priority_unknown_source_is_last() {
        assert_eq!(
            provider_priority_index("unknown", CoverMediaType::Ebook),
            EBOOK_COVER_PRIORITY.len()
        );
        assert_eq!(
            provider_priority_index("unknown", CoverMediaType::Audiobook),
            AUDIOBOOK_COVER_PRIORITY.len()
        );
    }
}
