#![allow(dead_code)]

use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, FieldDissentDb, WorkDbCreate};
use livrarr_domain::{DissentReason, EnrichmentStatus, MetadataProvider, OutcomeClass, Work};
use livrarr_external_data::NormalizedWorkDetail;
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentMode, MergeEngine, MergeInput, PriorityModel,
    ReconstructedOutcome,
};

fn current_work(language: &str) -> Work {
    Work {
        id: 401,
        user_id: 17,
        title: "Le Comte de Monte-Cristo".to_string(),
        author_name: "Alexandre Dumas".to_string(),
        language: Some(language.to_string()),
        enrichment_status: EnrichmentStatus::Unenriched,
        ..Work::default()
    }
}

fn detail(title: &str, description: &str, language: Option<&str>) -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: Some(title.to_string()),
        description: Some(description.to_string()),
        cover_url: Some(format!("https://example.test/{title}.jpg")),
        language: language.map(str::to_string),
        ..NormalizedWorkDetail::default()
    }
}

#[tokio::test]
async fn test_mc_provider_conflict_isolates_dissent_and_clean_providers_still_merge() {
    // REQ-014 / AC-016
    let results = std::collections::HashMap::from([
        (
            MetadataProvider::Goodreads,
            ReconstructedOutcome {
                class: OutcomeClass::Conflict,
                payload: Some(detail("Wrong Book", "wrong payload", Some("en"))),
            },
        ),
        (
            MetadataProvider::GoogleBooks,
            ReconstructedOutcome {
                class: OutcomeClass::Success,
                payload: Some(detail(
                    "Le Comte de Monte-Cristo",
                    "clean description",
                    Some("fr"),
                )),
            },
        ),
        (
            MetadataProvider::Audible,
            ReconstructedOutcome {
                class: OutcomeClass::Success,
                payload: Some(detail(
                    "Le Comte de Monte-Cristo",
                    "audio description",
                    Some("fr"),
                )),
            },
        ),
    ]);

    let output = DefaultMergeEngine
        .merge(MergeInput {
            current_work: current_work("fr"),
            current_provenance: Vec::new(),
            provider_results: results,
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::for_language(Some("fr")),
        })
        .await
        .unwrap();

    assert!(!output.dissents.is_empty());
    assert_eq!(output.dissents[0].reason, DissentReason::PayloadMismatch);
    let update = output
        .work_update
        .expect("clean provider contributions must merge");
    assert_eq!(update.0.description.as_deref(), Some("clean description"));
}

#[tokio::test]
async fn test_mc_language_incompatible_known_payload_records_dissent_unknown_language_unaffected() {
    // REQ-013 / AC-015
    let results = std::collections::HashMap::from([
        (
            MetadataProvider::Goodreads,
            ReconstructedOutcome {
                class: OutcomeClass::Success,
                payload: Some(detail("Known English", "english text", Some("en"))),
            },
        ),
        (
            // GoogleBooks: in the foreign description priority list (Audnexus
            // is audio-only there and could never win this field).
            MetadataProvider::GoogleBooks,
            ReconstructedOutcome {
                class: OutcomeClass::Success,
                payload: Some(detail("Unknown Language", "unknown language text", None)),
            },
        ),
    ]);

    let output = DefaultMergeEngine
        .merge(MergeInput {
            current_work: current_work("fr"),
            current_provenance: Vec::new(),
            provider_results: results,
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::for_language(Some("fr")),
        })
        .await
        .unwrap();

    assert!(output
        .dissents
        .iter()
        .any(|row| row.reason == DissentReason::LanguageIncompatible
            && row.field == "description"
            && row.offered_value == "english text"));
    let update = output
        .work_update
        .expect("unknown-language payload remains eligible");
    assert_eq!(
        update.0.description.as_deref(),
        Some("unknown language text")
    );
}

#[tokio::test]
async fn test_mc_field_dissent_rows_from_merge_are_queryable_in_db() {
    // REQ-014 / AC-016
    let db = create_test_db().await;
    // work_field_dissents carries FK references to users/works (migration 060,
    // the 029 schema pattern) — seed the referenced rows.
    let user_id = create_test_user(&db).await;
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Le Comte de Monte-Cristo".to_string(),
            author_name: "Alexandre Dumas".to_string(),
            normalized_title: "le comte de monte-cristo".to_string(),
            normalized_author: "alexandre dumas".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();
    let work_id = work.id;
    let row = livrarr_domain::FieldDissent {
        work_id,
        provider: "goodreads".to_string(),
        field: "description".to_string(),
        offered_value: "wrong book".to_string(),
        winning_value: Some("clean description".to_string()),
        reason: DissentReason::PayloadMismatch,
        merge_generation: 1,
        recorded_at: chrono::Utc::now(),
    };

    db.record_field_dissents(user_id, work_id, vec![row.clone()])
        .await
        .unwrap();
    let rows = db.list_field_dissents(user_id, work_id).await.unwrap();

    assert_eq!(rows, vec![row]);
}

#[tokio::test]
async fn test_mc_foreign_work_drops_hc_ol_payloads_on_cached_and_network_merge_paths() {
    // REQ-012 / AC-014
    let engine = DefaultMergeEngine;
    let payloads = std::collections::HashMap::from([
        (
            MetadataProvider::Hardcover,
            detail("English HC", "hardcover english text", Some("en")),
        ),
        (
            MetadataProvider::OpenLibrary,
            detail("English OL", "openlibrary english text", Some("en")),
        ),
    ]);

    let cached = engine
        .merge_from_cached(current_work("pl"), payloads.clone(), Vec::new(), Some("pl"))
        .await
        .unwrap();
    let network = engine
        .merge(MergeInput {
            current_work: current_work("pl"),
            current_provenance: Vec::new(),
            provider_results: payloads
                .into_iter()
                .map(|(provider, payload)| {
                    (
                        provider,
                        ReconstructedOutcome {
                            class: OutcomeClass::Success,
                            payload: Some(payload),
                        },
                    )
                })
                .collect(),
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::foreign(),
        })
        .await
        .unwrap();

    assert!(cached.work_update.is_none());
    assert!(network.work_update.is_none());
}

#[tokio::test]
async fn test_mc_language_incompatible_content_field_page_count_year_dissented_and_suppressed() {
    // Bug #133: the language guard must extend beyond text fields to every
    // Content-category field (page_count, year, etc.), not just
    // Description/Subtitle/SeriesName/Genres.
    let results = std::collections::HashMap::from([
        (
            MetadataProvider::GoogleBooks,
            ReconstructedOutcome {
                class: OutcomeClass::Success,
                payload: Some(NormalizedWorkDetail {
                    page_count: Some(111),
                    year: Some(1850),
                    ..detail("Le Comte de Monte-Cristo", "english edition", Some("en"))
                }),
            },
        ),
        (
            // Goodreads, not Hardcover/OpenLibrary: those two are hard-dropped
            // from every foreign-language merge by drop_language_incompatible_providers
            // (REQ-027) before merge_impl ever runs, regardless of this fix.
            MetadataProvider::Goodreads,
            ReconstructedOutcome {
                class: OutcomeClass::Success,
                payload: Some(NormalizedWorkDetail {
                    page_count: Some(999),
                    year: Some(1844),
                    ..detail("Le Comte de Monte-Cristo", "french edition", Some("fr"))
                }),
            },
        ),
    ]);

    let output = DefaultMergeEngine
        .merge(MergeInput {
            current_work: current_work("fr"),
            current_provenance: Vec::new(),
            provider_results: results,
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::for_language(Some("fr")),
        })
        .await
        .unwrap();

    let update = output
        .work_update
        .expect("french-language provider still contributes");
    assert_eq!(update.0.page_count, Some(999));
    assert_eq!(update.0.year, Some(1844));

    assert!(output.dissents.iter().any(|row| {
        row.reason == DissentReason::LanguageIncompatible
            && row.field == "page_count"
            && row.provider == "google_books"
    }));
}

#[tokio::test]
async fn test_mc_language_incompatible_audio_fields_exempt_from_dissent_guard() {
    // Bug #133 audio exemption: Audible-only audio metadata (narrator,
    // duration) must not be suppressed or dissented even though the
    // audiobook's language differs from the (foreign) work language — a
    // foreign work can legitimately have a different-language audiobook
    // edition, since audio only ever comes from Audible/Audnexus.
    let results = std::collections::HashMap::from([(
        MetadataProvider::Audible,
        ReconstructedOutcome {
            class: OutcomeClass::Success,
            payload: Some(NormalizedWorkDetail {
                narrator: Some(vec!["Jean Reno".to_string()]),
                duration_seconds: Some(36000),
                ..detail("Le Comte de Monte-Cristo", "audio edition", Some("en"))
            }),
        },
    )]);

    let output = DefaultMergeEngine
        .merge(MergeInput {
            current_work: current_work("fr"),
            current_provenance: Vec::new(),
            provider_results: results,
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::for_language(Some("fr")),
        })
        .await
        .unwrap();

    let update = output
        .work_update
        .expect("audio-only provider still contributes");
    assert_eq!(update.0.narrator, Some(vec!["Jean Reno".to_string()]));
    assert_eq!(update.0.duration_seconds, Some(36000));

    assert!(!output
        .dissents
        .iter()
        .any(|row| row.field == "narrator" || row.field == "duration_seconds"));
}

#[tokio::test]
async fn test_mc_language_incompatible_series_position_dissented_and_suppressed() {
    // Bug #133: SeriesPosition (Content-category) now receives the same
    // language-dissent guard SeriesName already had.
    let results = std::collections::HashMap::from([
        (
            MetadataProvider::GoogleBooks,
            ReconstructedOutcome {
                class: OutcomeClass::Success,
                payload: Some(NormalizedWorkDetail {
                    series_position: Some(2.0),
                    ..detail("Le Comte de Monte-Cristo", "english edition", Some("en"))
                }),
            },
        ),
        (
            // Goodreads, not Hardcover/OpenLibrary: those two are hard-dropped
            // from every foreign-language merge by drop_language_incompatible_providers
            // (REQ-027) before merge_impl ever runs, regardless of this fix.
            MetadataProvider::Goodreads,
            ReconstructedOutcome {
                class: OutcomeClass::Success,
                payload: Some(NormalizedWorkDetail {
                    series_position: Some(3.0),
                    ..detail("Le Comte de Monte-Cristo", "french edition", Some("fr"))
                }),
            },
        ),
    ]);

    let output = DefaultMergeEngine
        .merge(MergeInput {
            current_work: current_work("fr"),
            current_provenance: Vec::new(),
            provider_results: results,
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::for_language(Some("fr")),
        })
        .await
        .unwrap();

    let update = output
        .work_update
        .expect("french-language provider still contributes");
    assert_eq!(update.0.series_position, Some(3.0));

    assert!(output.dissents.iter().any(|row| {
        row.reason == DissentReason::LanguageIncompatible
            && row.field == "series_position"
            && row.provider == "google_books"
    }));
}
