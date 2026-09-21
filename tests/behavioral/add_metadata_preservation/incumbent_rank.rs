//! Ordinary-field precedence at the production merge boundary, persisted in SQLite.
//!
//! Normalized provider contributions are intentional inputs here: this module tests
//! merge authority, not provider parsing, Add-time selection, or cache admission.
//! Every pass rereads the saved Work and provenance and applies the real engine's
//! output unchanged through the production merge writer.

use std::{collections::HashMap, sync::Arc};

use livrarr_db::{
    create_test_db, sqlite::SqliteDb, CreateWorkDbRequest, ProvenanceDb, ProviderPolicyDb,
    SetFieldProvenanceRequest, WorkDb, WorkDbCreate,
};
use livrarr_domain::{
    ApplyMergeOutcome, FieldProvenance, MetadataProvider as P, OutcomeClass, ProvenanceSetter,
    UserId, WorkField, WorkId,
};
use livrarr_enrichment::{
    build_apply_request, DefaultMergeEngine, EnrichmentMode, MergeEngine, MergeInput, MergeOutput,
    ReconstructedOutcome,
};
use livrarr_external_data::NormalizedWorkDetail;

#[path = "ownership.rs"]
mod ownership;

#[path = "readarr_source.rs"]
mod readarr_source;

const SAVED: &str = "An explorer crosses the winter sea in search of a missing expedition.";
const OFFERED: &str = "A revised account of the explorer's voyage and the expedition's fate.";

#[derive(Clone, Copy, Debug)]
pub(super) enum Entry {
    Live,
    Cached,
}

/// Shared real-writer fixture for saved field values and valid provenance.
pub(super) struct Fixture {
    db: SqliteDb,
    engine: DefaultMergeEngine,
    user_id: UserId,
    work_id: WorkId,
}

impl Fixture {
    async fn for_persisted_work(db: SqliteDb, user_id: UserId, work_id: WorkId) -> Self {
        let engine = DefaultMergeEngine::from_policy(Arc::new(
            db.load_provider_policy_snapshot()
                .await
                .expect("load the persisted Work's normal provider policy"),
        ));
        Self {
            db,
            engine,
            user_id,
            work_id,
        }
    }

    pub(super) async fn new(
        language: &str,
        description: Option<&str>,
        provenance: Option<(ProvenanceSetter, Option<P>, bool)>,
    ) -> Self {
        let db = create_test_db().await;
        let user_id = livrarr_behavioral::stubs::create_test_user(&db).await;
        let (work, created) = db
            .create_work(CreateWorkDbRequest {
                language: Some(language.into()),
                description: description.map(str::to_owned),
                ..super::work_req(user_id, "The Winter Expedition", "Ada Rivers")
            })
            .await
            .expect("seed the incumbent through the real SQLite Work writer");
        assert!(created);
        if let Some((setter, source, cleared)) = provenance {
            db.set_field_provenance(SetFieldProvenanceRequest {
                user_id,
                work_id: work.id,
                field: WorkField::Description,
                source,
                setter,
                cleared,
            })
            .await
            .expect("the real provenance writer must support the accepted source/setter fixture");
        }
        let engine = DefaultMergeEngine::from_policy(Arc::new(
            db.load_provider_policy_snapshot()
                .await
                .expect("load the same immutable database policy used at startup"),
        ));
        let fixture = Self {
            db,
            engine,
            user_id,
            work_id: work.id,
        };
        fixture.assert_description(description, provenance).await;
        fixture
    }

    async fn provider(description: Option<&str>, source: P) -> Self {
        Self::new(
            "en",
            description,
            Some((ProvenanceSetter::Provider, Some(source), false)),
        )
        .await
    }

    /// Override only this isolated database's English ordinary-field policy.
    /// Reloading represents startup composition; the engine never receives a
    /// hand-built PriorityModel or a priority lookup implementation from the test.
    async fn google_before_hardcover(&mut self) {
        sqlx::query("DELETE FROM provider_policy WHERE language = 'en'")
            .execute(self.db.pool())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO provider_policy (language, kind, provider, rank) VALUES
             ('en','ebook','google_books',0), ('en','ebook','hardcover',1),
             ('en','ebook','goodreads',2), ('en','audiobook','audible',0)",
        )
        .execute(self.db.pool())
        .await
        .unwrap();
        self.engine = DefaultMergeEngine::from_policy(Arc::new(
            self.db.load_provider_policy_snapshot().await.unwrap(),
        ));
    }

    pub(super) async fn offer(&self, entry: Entry, source: P, description: Option<&str>) {
        let work = self.db.get_work(self.user_id, self.work_id).await.unwrap();
        self.payloads(
            entry,
            HashMap::from([(
                source,
                NormalizedWorkDetail {
                    title: Some(work.title),
                    author_name: Some(work.author_name),
                    language: work.language,
                    description: description.map(str::to_owned),
                    ..Default::default()
                },
            )]),
        )
        .await;
    }

    async fn payloads(&self, entry: Entry, payloads: HashMap<P, NormalizedWorkDetail>) {
        match entry {
            Entry::Live => {
                let outcomes = payloads
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
                    .collect();
                self.live(EnrichmentMode::Manual, outcomes).await;
            }
            Entry::Cached => {
                let work = self.db.get_work(self.user_id, self.work_id).await.unwrap();
                let language = work.language.clone();
                let provenance = self
                    .db
                    .list_work_provenance(self.user_id, self.work_id)
                    .await
                    .unwrap();
                let generation = self
                    .db
                    .get_merge_generation(self.user_id, self.work_id)
                    .await
                    .unwrap();
                // This directly executes the public cached entry. There is no
                // service, queue, provider client, or network fallback in scope.
                let output = self
                    .engine
                    .merge_from_cached(work, payloads, provenance, language.as_deref())
                    .await
                    .expect("production cached merge");
                self.persist(output, generation).await;
            }
        }
    }

    pub(super) async fn live(
        &self,
        mode: EnrichmentMode,
        provider_results: HashMap<P, ReconstructedOutcome>,
    ) {
        let work = self.db.get_work(self.user_id, self.work_id).await.unwrap();
        let priority_model = self.engine.priority_model(work.language.as_deref());
        let current_provenance = self
            .db
            .list_work_provenance(self.user_id, self.work_id)
            .await
            .unwrap();
        let generation = self
            .db
            .get_merge_generation(self.user_id, self.work_id)
            .await
            .unwrap();
        let output = self
            .engine
            .merge(MergeInput {
                current_work: work,
                current_provenance,
                provider_results,
                mode,
                priority_model,
            })
            .await
            .expect("production live merge");
        self.persist(output, generation).await;
    }

    async fn persist(&self, output: MergeOutput, generation: i64) {
        let request = build_apply_request(&output, self.user_id, self.work_id, generation);
        let outcome = self.db.apply_enrichment_merge(request).await.unwrap();
        assert_eq!(outcome, ApplyMergeOutcome::Applied, "real merge write");
    }

    pub(super) async fn provenance(&self) -> Option<FieldProvenance> {
        let mut rows: Vec<_> = self
            .db
            .list_work_provenance(self.user_id, self.work_id)
            .await
            .unwrap()
            .into_iter()
            .filter(|row| row.field == WorkField::Description)
            .collect();
        assert!(rows.len() <= 1, "one provenance row per ordinary field");
        rows.pop()
    }

    pub(super) async fn assert_description(
        &self,
        expected: Option<&str>,
        provenance: Option<(ProvenanceSetter, Option<P>, bool)>,
    ) {
        let saved = self.db.get_work(self.user_id, self.work_id).await.unwrap();
        assert_eq!(
            saved.description.as_deref(),
            expected,
            "persisted description"
        );
        let actual = self.provenance().await;
        assert_eq!(
            actual.as_ref().map(|p| (p.setter, p.source, p.cleared)),
            provenance,
            "persisted description setter, provider source, and clear state"
        );
        if let Some(row) = actual {
            assert_eq!((row.user_id, row.work_id), (self.user_id, self.work_id));
        }
    }

    async fn assert_provider_description(&self, value: &str, source: P) {
        self.assert_description(
            Some(value),
            Some((ProvenanceSetter::Provider, Some(source), false)),
        )
        .await;
    }

    async fn assert_retained(&self, before: &Option<FieldProvenance>) {
        let saved = self.db.get_work(self.user_id, self.work_id).await.unwrap();
        assert_eq!(saved.description.as_deref(), Some(SAVED));
        assert_eq!(
            &self.provenance().await,
            before,
            "retention must preserve the description's source, setter, clear state, and set_at"
        );
    }
}

// REQ-005 / AC-005
#[tokio::test]
async fn rank_001_listed_provider_fills_blank_description() {
    let fixture = Fixture::new("en", None, None).await;
    fixture
        .offer(Entry::Live, P::GoogleBooks, Some(OFFERED))
        .await;
    fixture
        .assert_provider_description(OFFERED, P::GoogleBooks)
        .await;
}

// REQ-005 / AC-005
#[tokio::test]
async fn rank_002_live_lower_offer_preserves_configured_higher_source() {
    let mut fixture = Fixture::provider(Some(SAVED), P::GoogleBooks).await;
    fixture.google_before_hardcover().await;
    let before = fixture.provenance().await;
    fixture
        .offer(Entry::Live, P::Hardcover, Some(OFFERED))
        .await;
    fixture.assert_retained(&before).await;
}

// REQ-005 / AC-005
#[tokio::test]
async fn rank_003_cached_lower_offer_preserves_configured_higher_source() {
    let mut fixture = Fixture::provider(Some(SAVED), P::GoogleBooks).await;
    fixture.google_before_hardcover().await;
    let before = fixture.provenance().await;
    fixture
        .offer(Entry::Cached, P::Hardcover, Some(OFFERED))
        .await;
    fixture.assert_retained(&before).await;
}

// REQ-005 / AC-005
#[tokio::test]
async fn rank_004_higher_offer_replaces_value_and_provenance() {
    let mut fixture = Fixture::provider(Some(SAVED), P::Hardcover).await;
    fixture.google_before_hardcover().await;
    fixture
        .offer(Entry::Live, P::GoogleBooks, Some(OFFERED))
        .await;
    fixture
        .assert_provider_description(OFFERED, P::GoogleBooks)
        .await;
}

// REQ-005 / AC-005
#[tokio::test]
async fn rank_005_same_provider_refreshes_description() {
    let fixture = Fixture::provider(Some(SAVED), P::GoogleBooks).await;
    fixture
        .offer(Entry::Live, P::GoogleBooks, Some(OFFERED))
        .await;
    fixture
        .assert_provider_description(OFFERED, P::GoogleBooks)
        .await;
}

// REQ-005 / AC-005: every pass rereads SQLite so losing source ownership
// cannot quietly authorize a subsequent lower-ranked replacement.
#[tokio::test]
async fn rank_006_absent_empty_and_failed_offers_retain_value_and_provenance() {
    let fixture = Fixture::provider(Some(SAVED), P::Hardcover).await;
    let before = fixture.provenance().await;
    fixture.payloads(Entry::Live, HashMap::new()).await;
    fixture.assert_retained(&before).await;
    for empty in [None, Some(""), Some(" \n\t ")] {
        fixture.offer(Entry::Live, P::Hardcover, empty).await;
        fixture.assert_retained(&before).await;
    }
    fixture
        .live(
            EnrichmentMode::Manual,
            HashMap::from([(
                P::Hardcover,
                ReconstructedOutcome {
                    class: OutcomeClass::PermanentFailure,
                    payload: None,
                },
            )]),
        )
        .await;
    fixture.assert_retained(&before).await;
    fixture
        .offer(Entry::Live, P::GoogleBooks, Some(OFFERED))
        .await;
    fixture.assert_retained(&before).await;
}

// REQ-005 / AC-005: missing provenance, unknown source, and a source
// excluded from the applicable language list confer no ranked precedence.
#[tokio::test]
async fn rank_008_listed_offer_replaces_unranked_incumbents() {
    let missing = Fixture::new("en", Some(SAVED), None).await;
    missing
        .offer(Entry::Live, P::GoogleBooks, Some(OFFERED))
        .await;
    missing
        .assert_provider_description(OFFERED, P::GoogleBooks)
        .await;

    // The real add-time writer below produces Import + source=None. There is
    // no synthetic provider name or invalid Import + provider provenance.
    source_free_series_is_unranked().await;

    let excluded = Fixture::new(
        "fr",
        Some("Une exploratrice traverse la mer hivernale."),
        Some((ProvenanceSetter::Provider, Some(P::Hardcover), false)),
    )
    .await;
    let offered = "Le nouveau récit de la traversée et de l'expédition disparue.";
    excluded
        .offer(Entry::Live, P::GoogleBooks, Some(offered))
        .await;
    excluded
        .assert_provider_description(offered, P::GoogleBooks)
        .await;
}

async fn source_free_series_is_unranked() {
    let setter = ProvenanceSetter::Import;
    let db = create_test_db().await;
    let user_id = livrarr_behavioral::stubs::create_test_user(&db).await;
    let (work, created) = db
        .create_work(CreateWorkDbRequest {
            series_name: Some("The Winter Voyages".into()),
            ..super::work_req(user_id, "The Winter Expedition", "Ada Rivers")
        })
        .await
        .expect("create the populated add-time series through the SQLite writer");
    assert!(created);
    livrarr_metadata::provenance::write_addtime_provenance(&db, user_id, &work, setter).await;
    let fixture = Fixture::for_persisted_work(db, user_id, work.id).await;
    let before = fixture.db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(before.series_name.as_deref(), Some("The Winter Voyages"));
    let row = fixture
        .db
        .get_field_provenance(user_id, work.id, WorkField::SeriesName)
        .await
        .unwrap()
        .expect("the actual add-time writer records the source-free setter");
    assert_eq!((row.setter, row.source, row.cleared), (setter, None, false));

    fixture
        .payloads(
            Entry::Live,
            HashMap::from([(
                P::OpenLibrary,
                NormalizedWorkDetail {
                    title: Some(before.title),
                    author_name: Some(before.author_name),
                    language: before.language,
                    series_name: Some("Northern Expeditions".into()),
                    ..Default::default()
                },
            )]),
        )
        .await;

    let after = fixture.db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(after.series_name.as_deref(), Some("Northern Expeditions"));
    let row = fixture
        .db
        .get_field_provenance(user_id, work.id, WorkField::SeriesName)
        .await
        .unwrap()
        .expect("replacement records the actual contributing provider");
    assert_eq!(
        (row.setter, row.source, row.cleared),
        (ProvenanceSetter::Provider, Some(P::OpenLibrary), false)
    );
}
