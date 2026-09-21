//! Existing ownership guards, exercised through the merger and SQLite writer.
//! These are merge-boundary controls, not edit-route or provider-parsing tests.

use super::{Entry, Fixture, OFFERED, SAVED};
use std::collections::HashMap;

use livrarr_db::{
    ProvenanceDb, SetFieldProvenanceRequest, UpdateWorkEnrichmentDbRequest,
    UpdateWorkUserFieldsDbRequest, WorkDb,
};
use livrarr_domain::{FieldProvenance, MetadataProvider as P, ProvenanceSetter, WorkField};
use livrarr_enrichment::{EnrichmentMode, ReconstructedOutcome};
use livrarr_external_data::NormalizedWorkDetail;

async fn set_provenance(
    fixture: &Fixture,
    field: WorkField,
    setter: ProvenanceSetter,
    source: Option<P>,
    cleared: bool,
) -> FieldProvenance {
    fixture
        .db
        .set_field_provenance(SetFieldProvenanceRequest {
            user_id: fixture.user_id,
            work_id: fixture.work_id,
            field,
            setter,
            source,
            cleared,
        })
        .await
        .expect("persist valid ownership through the production writer");
    fixture
        .db
        .get_field_provenance(fixture.user_id, fixture.work_id, field)
        .await
        .unwrap()
        .expect("saved ownership")
}

// REQ-005 / AC-005. Reused from the archived personal-edit control, narrowed
// to one ordinary field and one unlocked field proving the offer was admitted.
#[tokio::test]
async fn own_003_personal_edit_survives_hard_refresh() {
    const PERSONAL: &str = "My notes on the expedition, corrected after reading.";
    let fixture = Fixture::provider(Some(SAVED), P::Hardcover).await;
    fixture
        .db
        .update_work_enrichment(
            fixture.user_id,
            fixture.work_id,
            UpdateWorkEnrichmentDbRequest {
                description: Some(PERSONAL.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let before = set_provenance(
        &fixture,
        WorkField::Description,
        ProvenanceSetter::User,
        None,
        false,
    )
    .await;
    fixture
        .assert_description(Some(PERSONAL), Some((ProvenanceSetter::User, None, false)))
        .await;

    fixture
        .live(
            EnrichmentMode::HardRefresh,
            HashMap::from([(
                P::Hardcover,
                ReconstructedOutcome {
                    class: livrarr_domain::OutcomeClass::Success,
                    payload: Some(NormalizedWorkDetail {
                        title: Some("The Winter Expedition".into()),
                        author_name: Some("Ada Rivers".into()),
                        language: Some("en".into()),
                        description: Some(OFFERED.into()),
                        publisher: Some("Northern Press".into()),
                        ..Default::default()
                    }),
                },
            )]),
        )
        .await;

    let after = fixture
        .db
        .get_work(fixture.user_id, fixture.work_id)
        .await
        .unwrap();
    assert_eq!(after.description.as_deref(), Some(PERSONAL));
    assert_eq!(fixture.provenance().await, Some(before));
    assert_eq!(after.publisher.as_deref(), Some("Northern Press"));
}

// REQ-005 / AC-005. Some(None) drives the real user writer's explicit clear.
#[tokio::test]
async fn own_005_explicit_clear_survives_manual_refresh() {
    let fixture = Fixture::new("en", None, None).await;
    fixture
        .db
        .update_work_user_fields(
            fixture.user_id,
            fixture.work_id,
            UpdateWorkUserFieldsDbRequest {
                series_name: Some(Some("The Northern Voyages".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let populated = fixture
        .db
        .get_work(fixture.user_id, fixture.work_id)
        .await
        .unwrap();
    assert_eq!(
        populated.series_name.as_deref(),
        Some("The Northern Voyages")
    );
    fixture
        .db
        .update_work_user_fields(
            fixture.user_id,
            fixture.work_id,
            UpdateWorkUserFieldsDbRequest {
                series_name: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let before = set_provenance(
        &fixture,
        WorkField::SeriesName,
        ProvenanceSetter::User,
        None,
        true,
    )
    .await;
    let cleared = fixture
        .db
        .get_work(fixture.user_id, fixture.work_id)
        .await
        .unwrap();
    assert_eq!(cleared.series_name, None);

    fixture
        .payloads(
            Entry::Live,
            HashMap::from([(
                P::Hardcover,
                NormalizedWorkDetail {
                    title: Some("The Winter Expedition".into()),
                    author_name: Some("Ada Rivers".into()),
                    language: Some("en".into()),
                    series_name: Some("The Northern Voyages".into()),
                    description: Some(OFFERED.into()),
                    ..Default::default()
                },
            )]),
        )
        .await;

    let after = fixture
        .db
        .get_work(fixture.user_id, fixture.work_id)
        .await
        .unwrap();
    assert_eq!(after.series_name, None, "retain the intentional clear");
    assert_eq!(
        fixture
            .db
            .get_field_provenance(fixture.user_id, fixture.work_id, WorkField::SeriesName)
            .await
            .unwrap(),
        Some(before),
    );
    fixture
        .assert_provider_description(OFFERED, P::Hardcover)
        .await;
}

// REQ-005 / AC-005. A higher-ranked admitted offer must still respect all
// three populated identity fields, independently of personal ownership.
#[tokio::test]
async fn prot_001_populated_title_author_and_language_survive_admitted_offer() {
    let fixture = Fixture::new("en", None, None).await;
    let mut before = Vec::new();
    for field in [WorkField::Title, WorkField::AuthorName, WorkField::Language] {
        before.push(
            set_provenance(
                &fixture,
                field,
                ProvenanceSetter::Provider,
                Some(P::OpenLibrary),
                false,
            )
            .await,
        );
    }
    fixture
        .payloads(
            Entry::Live,
            HashMap::from([(
                P::GoogleBooks,
                NormalizedWorkDetail {
                    title: Some("The Winter Expedition: A Journey North".into()),
                    author_name: Some("Ada M. Rivers".into()),
                    // An English Work admits this payload's description, so
                    // foreign-Work rejection cannot hide the language guard.
                    language: Some("fr".into()),
                    description: Some(OFFERED.into()),
                    ..Default::default()
                },
            )]),
        )
        .await;

    let after = fixture
        .db
        .get_work(fixture.user_id, fixture.work_id)
        .await
        .unwrap();
    assert_eq!(after.title, "The Winter Expedition");
    assert_eq!(after.author_name, "Ada Rivers");
    assert_eq!(after.language.as_deref(), Some("en"));
    for row in before {
        assert_eq!(
            fixture
                .db
                .get_field_provenance(fixture.user_id, fixture.work_id, row.field)
                .await
                .unwrap(),
            Some(row),
        );
    }
    fixture
        .assert_provider_description(OFFERED, P::GoogleBooks)
        .await;
}
