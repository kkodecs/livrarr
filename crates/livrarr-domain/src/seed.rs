//! The single construction point for work-creation seeds.
//!
//! Every creation door builds its [`WorkCandidate`] here; no door assembles
//! [`WorkSeedFields`] directly (the only other legal site is `#[cfg(test)]`
//! code). The module owns seed-language policy: [`SeedLanguage::resolve`] is
//! the one place the system default lives, and every constructor stamps the
//! resolved language into the identity payload the candidate carries, so the
//! seed fields and the identity harvest can never disagree.

use crate::identity::{CandidateId, IdentityState, WorkCandidate, WorkSeed, WorkSeedFields};
use crate::services::SourceProviderData;
use crate::ProvenanceSetter;

/// The system-wide last-resort seed language.
pub const DEFAULT_SEED_LANGUAGE: &str = "en";

/// A resolved seed language. Constructed only via [`SeedLanguage::resolve`]:
/// normalized, with missing or empty input resolving to
/// [`DEFAULT_SEED_LANGUAGE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedLanguage(String);

impl SeedLanguage {
    pub fn resolve(input: Option<&str>) -> Self {
        let lang = input
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(crate::normalize_language)
            .unwrap_or_else(|| DEFAULT_SEED_LANGUAGE.to_string());
        SeedLanguage(lang)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a door knows about the work it is creating: [`WorkSeedFields`] with
/// the language already resolved through [`SeedLanguage`].
#[derive(Debug, Clone)]
pub struct SeedInput {
    pub title: String,
    pub author_name: String,
    pub language: SeedLanguage,
    pub author_ol_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub detail_url: Option<String>,
    pub description: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

/// Stamps the resolved seed language into the identity payload the candidate
/// carries. Resolver-produced `top_candidates` are other works' payloads and
/// are left untouched.
fn stamp_identity_language(identity: &mut IdentityState, language: &SeedLanguage) {
    match identity {
        IdentityState::Confirmed { anchors, .. } => {
            anchors.language = Some(language.as_str().to_string());
        }
        IdentityState::Pending { seed_anchors, .. } => {
            if let Some(anchors) = seed_anchors {
                anchors.language = Some(language.as_str().to_string());
            }
        }
    }
}

fn assemble(input: SeedInput, mut identity: IdentityState) -> (WorkSeedFields, IdentityState) {
    stamp_identity_language(&mut identity, &input.language);
    let fields = WorkSeedFields {
        title: input.title,
        author_name: input.author_name,
        language: input.language.0,
        author_ol_key: input.author_ol_key,
        year: input.year,
        cover_url: input.cover_url,
        detail_url: input.detail_url,
        description: input.description,
        series_name: input.series_name,
        series_position: input.series_position,
    };
    (fields, identity)
}

/// Add-box / GR-link door: interactive add from search results or a pasted
/// provider link. Language comes from the request (provider lookup result).
pub fn seed_add_box(
    input: SeedInput,
    identity: IdentityState,
    candidate_id: Option<CandidateId>,
    cover_manual: bool,
) -> WorkCandidate {
    let (fields, identity) = assemble(input, identity);
    WorkCandidate {
        fields,
        identity,
        candidate_id,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: None,
        import_id: None,
        cover_manual,
    }
}

/// Manual-import door: scan-review confirm. Language comes from the file's
/// embedded `dc:language`, then the picked candidate.
pub fn seed_manual_import(
    input: SeedInput,
    identity: IdentityState,
    candidate_id: Option<CandidateId>,
) -> WorkCandidate {
    let (fields, identity) = assemble(input, identity);
    WorkCandidate {
        fields,
        identity,
        candidate_id,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: None,
        import_id: None,
        cover_manual: false,
    }
}

/// List-import door: CSV confirm. Language is the import's user choice.
pub fn seed_list_import(
    input: SeedInput,
    identity: IdentityState,
    candidate_id: Option<CandidateId>,
) -> WorkCandidate {
    let (fields, identity) = assemble(input, identity);
    WorkCandidate {
        fields,
        identity,
        candidate_id,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: Some(ProvenanceSetter::Imported),
        import_id: None,
        cover_manual: false,
    }
}

/// Author-monitor door: bibliography auto-add. Language is the author's
/// persisted `monitor_language` choice.
pub fn seed_author_monitor(input: SeedInput, identity: IdentityState) -> WorkCandidate {
    let (fields, identity) = assemble(input, identity);
    WorkCandidate {
        fields,
        identity,
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: Some(ProvenanceSetter::AutoAdded),
        import_id: None,
        cover_manual: false,
    }
}

/// Series-monitor door: missing roster-work creation. Language is the series'
/// persisted `monitor_language` choice.
pub fn seed_series_monitor(
    input: SeedInput,
    identity: IdentityState,
    series_id: i64,
    monitor_ebook: bool,
    monitor_audiobook: bool,
) -> WorkCandidate {
    let (fields, identity) = assemble(input, identity);
    WorkCandidate {
        fields,
        identity,
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: Some(series_id),
        monitor_ebook: Some(monitor_ebook),
        monitor_audiobook: Some(monitor_audiobook),
        provenance_setter: Some(ProvenanceSetter::AutoAdded),
        import_id: None,
        cover_manual: false,
    }
}

/// Readarr-import door. Language comes from the Readarr edition record.
pub fn seed_readarr_import(
    input: SeedInput,
    identity: IdentityState,
    source_provider_data: SourceProviderData,
    monitor_ebook: bool,
    monitor_audiobook: bool,
    import_id: String,
) -> WorkCandidate {
    let (fields, identity) = assemble(input, identity);
    WorkCandidate {
        fields,
        identity,
        candidate_id: None,
        source_provider_data: Some(source_provider_data),
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: Some(monitor_ebook),
        monitor_audiobook: Some(monitor_audiobook),
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: Some(import_id),
        cover_manual: false,
    }
}

/// The dominant language among a set of works' language values: the unique
/// most-common non-empty value. Returns `None` on a tie or an all-empty set —
/// the caller decides the fallback (the UI leaves the selector at its default;
/// the monitor-enable guard persists [`DEFAULT_SEED_LANGUAGE`]). This is the
/// one definition of "smart default" the whole system shares (REQ-003 Q-001).
pub fn dominant_language<'a>(langs: impl IntoIterator<Item = Option<&'a str>>) -> Option<String> {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for l in langs.into_iter().flatten().filter(|l| !l.is_empty()) {
        *counts.entry(l).or_default() += 1;
    }
    let max = counts.values().copied().max()?;
    let mut at_max = counts.iter().filter(|(_, n)| **n == max);
    let winner = (*at_max.next()?.0).to_string();
    // A second value sharing the max is a tie — no dominant language.
    at_max.next().is_none().then_some(winner)
}

/// ISO 639-1 → 639-2/B code mapping (the form OpenLibrary's search language
/// filter expects).
pub fn iso639_1_to_3(code: &str) -> &str {
    match code {
        "nl" => "dut",
        "fr" => "fre",
        "de" => "ger",
        "it" => "ita",
        "ja" => "jpn",
        "ko" => "kor",
        "pl" => "pol",
        "es" => "spa",
        "en" => "eng",
        other => other,
    }
}

/// Parse a `lookup_filtered` search term into a discovery [`WorkSeed`]. An
/// `isbn:` prefix with a valid ISBN seeds the bridge; anything else seeds the
/// title.
pub fn lookup_term_to_seed(term: &str, lang: &str) -> WorkSeed {
    let isbn_13 = term
        .strip_prefix("isbn:")
        .and_then(|rest| crate::normalization::normalize_isbn13(rest.trim()));
    let title = if isbn_13.is_some() {
        None
    } else {
        Some(term.to_string())
    };
    WorkSeed {
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13,
        asin: None,
        title,
        author_name: None,
        language: Some(lang.to_string()),
        series_name: None,
        year: None,
        user_confirmed: false,
    }
}

/// True when a search term resolved to a hard identifier (e.g. an `isbn:`
/// lookup) — the signal the identity resolver needs. A bare-title term carries
/// none and is served as a free-text discovery search by the legacy chain.
pub fn seed_carries_identifier(seed: &WorkSeed) -> bool {
    seed.isbn_13.is_some()
        || seed.asin.is_some()
        || seed.ol_key.is_some()
        || seed.gr_key.is_some()
        || seed.hc_key.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{CapturedIdentity, IdentityMethod, PendingReason};

    fn input_with(language: SeedLanguage) -> SeedInput {
        SeedInput {
            title: "T".into(),
            author_name: "A".into(),
            language,
            author_ol_key: None,
            year: None,
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        }
    }

    #[test]
    fn dominant_language_unique_max_tie_and_empty() {
        let de = || Some("de");
        let en = || Some("en");
        // unique max wins
        assert_eq!(dominant_language([de(), de(), en()]).as_deref(), Some("de"));
        // tie → None (caller falls back to "en")
        assert_eq!(dominant_language([de(), en()]), None);
        // empty / all-None / all-empty → None
        assert_eq!(dominant_language(std::iter::empty::<Option<&str>>()), None);
        assert_eq!(dominant_language([None, None]), None);
        assert_eq!(dominant_language([Some(""), None]), None);
    }

    #[test]
    fn resolve_defaults_on_none_and_empty() {
        assert_eq!(SeedLanguage::resolve(None).as_str(), DEFAULT_SEED_LANGUAGE);
        assert_eq!(
            SeedLanguage::resolve(Some("  ")).as_str(),
            DEFAULT_SEED_LANGUAGE
        );
    }

    #[test]
    fn resolve_normalizes_real_input() {
        assert_eq!(
            SeedLanguage::resolve(Some("fr")).as_str(),
            crate::normalize_language("fr")
        );
    }

    #[test]
    fn confirmed_anchors_carry_the_seed_language() {
        let identity = IdentityState::Confirmed {
            anchors: CapturedIdentity {
                ol_key: Some("OL1W".into()),
                gr_key: None,
                hc_key: None,
                isbn_13: None,
                asin: None,
                title: "T".into(),
                author_name: "A".into(),
                language: None,
            },
            method: IdentityMethod::TitleAuthorSearch,
            score: None,
        };
        let c = seed_author_monitor(input_with(SeedLanguage::resolve(Some("fr"))), identity);
        let IdentityState::Confirmed { anchors, .. } = &c.identity else {
            panic!("variant changed");
        };
        assert_eq!(
            anchors.language.as_deref(),
            Some(c.fields.language.as_str())
        );
    }

    #[test]
    fn pending_seed_anchors_carry_the_seed_language() {
        let identity = IdentityState::Pending {
            reason: PendingReason::NoCandidates,
            seed_anchors: Some(CapturedIdentity {
                ol_key: None,
                gr_key: Some("123".into()),
                hc_key: None,
                isbn_13: None,
                asin: None,
                title: "T".into(),
                author_name: "A".into(),
                language: None,
            }),
            top_candidates: vec![],
        };
        let c = seed_series_monitor(
            input_with(SeedLanguage::resolve(None)),
            identity,
            7,
            true,
            false,
        );
        let IdentityState::Pending { seed_anchors, .. } = &c.identity else {
            panic!("variant changed");
        };
        assert_eq!(
            seed_anchors.as_ref().unwrap().language.as_deref(),
            Some(DEFAULT_SEED_LANGUAGE)
        );
        assert_eq!(c.series_id, Some(7));
    }
}
