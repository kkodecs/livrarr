//! Domain-level behavioral pins for author-provider linking.
//!
//! These tests enter through the canonical parser, guard, and display-name
//! policy functions. They intentionally run red against the Stage 4a
//! `todo!()` bodies.

use chrono::{TimeZone, Utc};
use livrarr_domain::identity_matching::AuthorVerdict;
use livrarr_domain::{
    guard_author_route, AuthorLinkError, AuthorNameSource, AuthorNameVariant, AuthorProvider,
    AuthorRouteEvidenceSource, AuthorRouteGuardResult, AuthorRouteKey, OpenLibraryNameRole,
    ProviderAuthorRef,
};
use livrarr_metadata::author_linking::{
    author_name_rank_table, choose_author_display_name, AuthorNameRankModel,
};

fn route(provider: AuthorProvider, raw: &str) -> AuthorRouteKey {
    AuthorRouteKey::parse(provider, raw).expect("fixture route must parse through production code")
}

fn provider_ref(provider: AuthorProvider, raw: &str, name: &str) -> ProviderAuthorRef {
    ProviderAuthorRef {
        key: route(provider, raw),
        name: name.to_string(),
        role: Some("author".to_string()),
    }
}

fn variant(id: i64, source: AuthorNameSource, name: &str, selected: bool) -> AuthorNameVariant {
    AuthorNameVariant {
        id,
        user_id: 7,
        author_id: 11,
        name: name.to_string(),
        source,
        source_route_id: None,
        open_library_role: (source == AuthorNameSource::OpenLibrary)
            .then_some(OpenLibraryNameRole::Primary),
        user_selected_at: selected.then(|| {
            Utc.with_ymd_and_hms(2026, 7, 30, 12, 0, 0)
                .single()
                .expect("valid fixture timestamp")
        }),
        observed_at: Utc
            .with_ymd_and_hms(2026, 7, 30, 11, 0, 0)
            .single()
            .expect("valid fixture timestamp"),
    }
}

/// Door: every typed route entry, before any DB route operation.
/// AC-005 / REQ-001: provider aliases converge on one canonical identity.
#[test]
fn ac005_route_key_parser_accepts_only_documented_aliases_and_canonicalizes() {
    let ol = route(AuthorProvider::OpenLibrary, "OL123A");
    assert_eq!(ol, route(AuthorProvider::OpenLibrary, " /authors/OL123A "));
    assert_eq!(
        ol,
        route(
            AuthorProvider::OpenLibrary,
            "https://openlibrary.org/authors/OL123A"
        )
    );

    assert_eq!(
        route(AuthorProvider::Goodreads, "00042"),
        route(AuthorProvider::Goodreads, "42")
    );
    assert_eq!(
        route(AuthorProvider::Hardcover, "00073"),
        route(AuthorProvider::Hardcover, "73")
    );
}

/// Door: every typed route entry, before any DB route operation.
/// AC-005 / REQ-001: malformed, zero, overflowed, and cross-provider values
/// never reach persistence.
#[test]
fn ac005_route_key_parser_rejects_empty_zero_overflow_malformed_and_cross_provider_values() {
    let cases = [
        (AuthorProvider::OpenLibrary, ""),
        (AuthorProvider::OpenLibrary, "OL0A"),
        (AuthorProvider::OpenLibrary, "OL12W"),
        (AuthorProvider::OpenLibrary, "42"),
        (AuthorProvider::Goodreads, "0"),
        (AuthorProvider::Goodreads, "OL12A"),
        (AuthorProvider::Goodreads, "18446744073709551616"),
        (AuthorProvider::Hardcover, "-1"),
        (AuthorProvider::Hardcover, "12.5"),
    ];

    for (provider, raw) in cases {
        assert!(
            matches!(
                AuthorRouteKey::parse(provider, raw),
                Err(AuthorLinkError::InvalidRoute(_))
            ),
            "{provider:?} unexpectedly accepted {raw:?}"
        );
    }
}

/// Door: Readarr import author resolution -> `guard_author_route`.
/// AC-003 / AC-013: only canonical `Agree` evidence mints the automatic
/// route-write capability, and the evidence snapshot is retained.
#[test]
fn ac003_ac013_guard_mints_agreed_capability_only_for_agree() {
    let key = route(AuthorProvider::Goodreads, "101");
    let guarded = guard_author_route(
        &["J.K. Rowling".to_string()],
        ProviderAuthorRef {
            key: key.clone(),
            name: "Joanne Kathleen Rowling".to_string(),
            role: Some("author".to_string()),
        },
        Some(9001),
        AuthorRouteEvidenceSource::Tier1SettledWork,
    );

    let AuthorRouteGuardResult::Agreed(agreed) = guarded else {
        panic!("Agree must be the sole capability-minting verdict");
    };
    assert_eq!(agreed.evidence().key, key);
    assert_eq!(agreed.evidence().observed_name, "Joanne Kathleen Rowling");
    assert_eq!(agreed.evidence().evidence_work_id, Some(9001));
}

/// Door: Readarr import author resolution -> rejected review evidence.
/// AC-004 / AC-013: a shared-surname `Grey` result is retained for review and
/// never promoted to agreed route evidence.
#[test]
fn ac004_ac013_guard_preserves_grey_as_rejected_evidence() {
    let guarded = guard_author_route(
        &["John Smith".to_string()],
        provider_ref(AuthorProvider::Goodreads, "102", "Jane Smith"),
        Some(9002),
        AuthorRouteEvidenceSource::ReadarrImport,
    );

    let AuthorRouteGuardResult::Rejected(rejected) = guarded else {
        panic!("Grey must not mint agreed evidence");
    };
    assert_eq!(rejected.verdict(), AuthorVerdict::Grey);
    assert!(matches!(
        rejected.evidence().source,
        AuthorRouteEvidenceSource::ReadarrImport
    ));
}

/// Door: Readarr import author resolution -> rejected review evidence.
/// AC-004 / AC-013: an unusable comparison is `Abstain`, not automatic proof.
#[test]
fn ac004_ac013_guard_preserves_abstain_as_rejected_evidence() {
    let guarded = guard_author_route(
        &[],
        provider_ref(AuthorProvider::Hardcover, "103", "Octavia Butler"),
        None,
        AuthorRouteEvidenceSource::ReadarrImport,
    );

    let AuthorRouteGuardResult::Rejected(rejected) = guarded else {
        panic!("Abstain must not mint agreed evidence");
    };
    assert_eq!(rejected.verdict(), AuthorVerdict::Abstain);
}

/// Door: Readarr import author resolution -> rejected review evidence.
/// AC-004 / AC-013: zero-overlap `Disagree` remains review evidence and cannot
/// enter the guarded writer.
#[test]
fn ac004_ac013_guard_preserves_disagree_as_rejected_evidence() {
    let guarded = guard_author_route(
        &["Frank Herbert".to_string()],
        provider_ref(AuthorProvider::OpenLibrary, "OL104A", "Ursula Le Guin"),
        None,
        AuthorRouteEvidenceSource::ReadarrImport,
    );

    let AuthorRouteGuardResult::Rejected(rejected) = guarded else {
        panic!("Disagree must not mint agreed evidence");
    };
    assert_eq!(rejected.verdict(), AuthorVerdict::Disagree);
}

/// Door: Author rename / stored-name variant pick -> local ranking.
/// AC-008: English-or-undetermined order is User, GR, HC, GB, OL, then
/// import/legacy fallback.
#[test]
fn ac008_english_or_undetermined_rank_table_is_exact() {
    assert_eq!(
        author_name_rank_table(AuthorNameRankModel::EnglishOrUndetermined),
        &[
            AuthorNameSource::User,
            AuthorNameSource::Goodreads,
            AuthorNameSource::Hardcover,
            AuthorNameSource::GoogleBooks,
            AuthorNameSource::OpenLibrary,
            AuthorNameSource::Readarr,
            AuthorNameSource::Import,
            AuthorNameSource::Legacy,
        ]
    );
}

/// Door: Author rename / stored-name variant pick -> local ranking.
/// AC-008: foreign-dominant order moves GB/HC ahead of GR/OL while retaining
/// User first and import/legacy only as fallback.
#[test]
fn ac008_foreign_dominant_rank_table_is_exact() {
    assert_eq!(
        author_name_rank_table(AuthorNameRankModel::ForeignDominant),
        &[
            AuthorNameSource::User,
            AuthorNameSource::GoogleBooks,
            AuthorNameSource::Hardcover,
            AuthorNameSource::Goodreads,
            AuthorNameSource::OpenLibrary,
            AuthorNameSource::Readarr,
            AuthorNameSource::Import,
            AuthorNameSource::Legacy,
        ]
    );
}

/// Door: Author rename / stored-name variant pick -> display cascade.
/// AC-008: an explicit user choice wins for English, foreign-dominant, tied,
/// and absent language evidence.
#[test]
fn ac008_user_selected_name_wins_across_language_models_and_ties_default_english() {
    let variants = vec![
        variant(
            1,
            AuthorNameSource::GoogleBooks,
            "Gabriel García Márquez",
            false,
        ),
        variant(
            2,
            AuthorNameSource::Goodreads,
            "Gabriel Garcia Marquez",
            false,
        ),
        variant(3, AuthorNameSource::User, "Gabo", true),
    ];

    for languages in [
        vec![Some("en"), Some("es")],
        vec![Some("es"), Some("es"), Some("en")],
        vec![Some("es"), Some("en")],
        vec![None, None],
    ] {
        let selected = choose_author_display_name(&variants, languages.into_iter())
            .expect("a display name must be selected");
        assert_eq!(selected.id, 3);
        assert_eq!(selected.name, "Gabo");
    }
}

/// Door: Enrichment-completion author-name observation -> dirty local rank.
/// AC-008: absent an explicit selection, provider order follows the dominant
/// language model and never elevates import/legacy above provider evidence.
#[test]
fn ac008_provider_ranking_changes_with_dominant_language_without_guessing() {
    let variants = vec![
        variant(1, AuthorNameSource::Legacy, "Legacy Name", false),
        variant(2, AuthorNameSource::Goodreads, "Goodreads Name", false),
        variant(3, AuthorNameSource::GoogleBooks, "Google Books Name", false),
    ];

    let english = choose_author_display_name(&variants, [Some("en"), Some("en")].into_iter())
        .expect("English selection");
    assert_eq!(english.id, 2);

    let foreign = choose_author_display_name(&variants, [Some("fr"), Some("fr")].into_iter())
        .expect("foreign selection");
    assert_eq!(foreign.id, 3);
}
