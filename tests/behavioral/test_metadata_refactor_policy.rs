use std::collections::HashMap;

use livrarr_db::{create_test_db, ProviderPolicyDb};
use livrarr_domain::{
    services::{ListKind, ProviderList, ProviderPolicy, ProviderPolicySnapshot, ProviderRef},
    MetadataProvider, Work,
};
use livrarr_external_data::NormalizedWorkDetail;
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentMode, MergeEngine, MergeInput, PriorityModel,
    ReconstructedOutcome,
};

fn provider(provider: MetadataProvider, rank: u8) -> ProviderRef {
    ProviderRef { provider, rank }
}

fn list(entries: Vec<ProviderRef>) -> ProviderList {
    ProviderList::new(entries).expect("test policy list should be valid")
}

fn snapshot_list(entries: Vec<ProviderRef>) -> ProviderList {
    ProviderList { entries }
}

#[tokio::test]
async fn foreign_language_policy_snapshot_excludes_hardcover_and_openlibrary() {
    // AC-006
    let db = create_test_db().await;

    let snapshot = db
        .load_provider_policy_snapshot()
        .await
        .expect("provider policy snapshot should load");

    let policy = snapshot
        .by_language
        .get("es")
        .cloned()
        .unwrap_or_else(|| snapshot.generic.clone());
    let ebook = policy.list_for(ListKind::Ebook);
    let audiobook = policy.list_for(ListKind::Audiobook);

    let all = ebook.entries.iter().chain(audiobook.entries.iter());
    assert!(
        all.clone()
            .all(|p| p.provider != MetadataProvider::Hardcover),
        "foreign-language policies must not write Hardcover fields"
    );
    assert!(
        all.clone()
            .all(|p| p.provider != MetadataProvider::OpenLibrary),
        "foreign-language policies must not write OpenLibrary fields"
    );
}

#[tokio::test]
async fn audiobook_list_returns_audible_first_independent_priority_order_for_merge() {
    // AC-007
    let policy = ProviderPolicy {
        ebook: list(vec![
            provider(MetadataProvider::Hardcover, 0),
            provider(MetadataProvider::Goodreads, 1),
        ]),
        audiobook: list(vec![
            provider(MetadataProvider::Audible, 0),
            provider(MetadataProvider::Hardcover, 1),
            provider(MetadataProvider::Goodreads, 2),
        ]),
    };

    let audio = policy.list_for(ListKind::Audiobook);

    assert_eq!(audio.entries[0].provider, MetadataProvider::Audible);
    assert_eq!(audio.entries[1].provider, MetadataProvider::Hardcover);
    assert_eq!(audio.entries[2].provider, MetadataProvider::Goodreads);

    let audio_priority = audio.entries.iter().map(|p| p.provider).collect();
    let priority_model = PriorityModel {
        content: vec![MetadataProvider::Hardcover, MetadataProvider::Goodreads],
        description: vec![MetadataProvider::Hardcover, MetadataProvider::Goodreads],
        cover: vec![MetadataProvider::Audible, MetadataProvider::Hardcover],
        audio: audio_priority,
    };
    let engine = DefaultMergeEngine::new(priority_model.clone());
    let audible = NormalizedWorkDetail {
        narrator: Some(vec!["Audible Narrator".to_string()]),
        duration_seconds: Some(11_111),
        cover_url: Some("https://covers.example.test/audible.jpg".to_string()),
        ..Default::default()
    };
    let hardcover = NormalizedWorkDetail {
        narrator: Some(vec!["Hardcover Narrator".to_string()]),
        duration_seconds: Some(22_222),
        cover_url: Some("https://covers.example.test/hardcover.jpg".to_string()),
        ..Default::default()
    };

    let output = engine
        .merge(MergeInput {
            current_work: Work {
                id: 1,
                user_id: 1,
                title: "Audio Contract".to_string(),
                author_name: "Contract Author".to_string(),
                language: Some("en".to_string()),
                ..Default::default()
            },
            current_provenance: vec![],
            provider_results: HashMap::from([
                (
                    MetadataProvider::Audible,
                    ReconstructedOutcome {
                        class: livrarr_domain::OutcomeClass::Success,
                        payload: Some(audible),
                    },
                ),
                (
                    MetadataProvider::Hardcover,
                    ReconstructedOutcome {
                        class: livrarr_domain::OutcomeClass::Success,
                        payload: Some(hardcover),
                    },
                ),
            ]),
            mode: EnrichmentMode::Manual,
            priority_model,
        })
        .await
        .expect("audiobook merge should complete");
    let update = output
        .work_update
        .expect("audiobook merge should update fields")
        .into_inner();

    assert_eq!(update.narrator, Some(vec!["Audible Narrator".to_string()]));
    assert_eq!(update.duration_seconds, Some(11_111));
    assert_eq!(
        update.cover_url,
        Some("https://covers.example.test/audible.jpg".to_string())
    );
}

#[test]
fn missing_language_uses_english_policy_source_default() {
    // AC-013
    let english = ProviderPolicy {
        ebook: snapshot_list(vec![provider(MetadataProvider::Hardcover, 0)]),
        audiobook: snapshot_list(vec![provider(MetadataProvider::Audible, 0)]),
    };
    let generic = ProviderPolicy {
        ebook: snapshot_list(vec![provider(MetadataProvider::GoogleBooks, 0)]),
        audiobook: snapshot_list(vec![provider(MetadataProvider::Audnexus, 0)]),
    };
    let snapshot = ProviderPolicySnapshot {
        by_language: HashMap::from([("en".to_string(), english.clone())]),
        generic,
    };

    let policy = snapshot.for_language("");

    assert_eq!(policy, english);
}

#[test]
fn unlisted_language_resolves_to_generic_row_alone() {
    // AC-014
    let french = ProviderPolicy {
        ebook: snapshot_list(vec![provider(MetadataProvider::GoogleBooks, 0)]),
        audiobook: snapshot_list(vec![provider(MetadataProvider::Audible, 0)]),
    };
    let generic = ProviderPolicy {
        ebook: snapshot_list(vec![provider(MetadataProvider::Goodreads, 0)]),
        audiobook: snapshot_list(vec![provider(MetadataProvider::Audnexus, 0)]),
    };
    let snapshot = ProviderPolicySnapshot {
        by_language: HashMap::from([("fr".to_string(), french)]),
        generic: generic.clone(),
    };

    let policy = snapshot.for_language("zu");

    assert_eq!(policy, generic);
    assert_eq!(policy.ebook.entries.len(), 1);
    assert_eq!(policy.audiobook.entries.len(), 1);
}

#[test]
fn provider_list_rejects_duplicate_within_one_list_but_accepts_cross_list_reuse() {
    // AC-015
    let dup = ProviderList::new(vec![
        provider(MetadataProvider::Hardcover, 0),
        provider(MetadataProvider::Hardcover, 1),
    ]);
    assert!(
        dup.is_err(),
        "a provider appearing twice within one list must be rejected"
    );

    let policy = ProviderPolicy {
        ebook: list(vec![provider(MetadataProvider::Hardcover, 0)]),
        audiobook: list(vec![provider(MetadataProvider::Hardcover, 0)]),
    };

    assert_eq!(
        policy.list_for(ListKind::Ebook).entries[0].provider,
        MetadataProvider::Hardcover
    );
    assert_eq!(
        policy.list_for(ListKind::Audiobook).entries[0].provider,
        MetadataProvider::Hardcover
    );
}

// OpenAI tests to append to the existing policy test target after writer handback.
// Replace ../../crates/livrarr-db/migrations/088_provider_policy_enrichment_defaults.sql with the author's actual new migration filename.
mod priority_migration_regression {
    use super::*;
    const NEW_POLICY: &str = include_str!(
        "../../crates/livrarr-db/migrations/088_provider_policy_enrichment_defaults.sql"
    );
    async fn legacy() -> livrarr_db::sqlite::SqliteDb {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../../crates/livrarr-db/migrations/057_provider_policy.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        livrarr_db::sqlite::SqliteDb::new(pool)
    }
    fn entries(list: &ProviderList) -> Vec<(MetadataProvider, u8)> {
        list.entries.iter().map(|p| (p.provider, p.rank)).collect()
    }
    #[tokio::test]
    async fn upgrades_legacy_defaults_and_preserves_explicit_custom_lists() {
        let db = legacy().await;
        sqlx::query("INSERT INTO provider_policy (language,kind,provider,rank) VALUES ('en','ebook','goodreads',0),('en','ebook','hardcover',1),('fr','ebook','readarr',0),('fr','audiobook','audnexus',0)").execute(db.pool()).await.unwrap();
        let before = db.load_provider_policy_snapshot().await.unwrap();
        sqlx::raw_sql(NEW_POLICY).execute(db.pool()).await.unwrap();
        let after = db.load_provider_policy_snapshot().await.unwrap();
        assert_eq!(
            entries(&before.for_language("en").ebook),
            entries(&after.for_language("en").ebook)
        );
        assert_eq!(before.for_language("fr"), after.for_language("fr"));
        assert_eq!(
            after
                .generic
                .ebook
                .entries
                .iter()
                .map(|p| p.provider)
                .collect::<Vec<_>>(),
            vec![
                MetadataProvider::GoogleBooks,
                MetadataProvider::Goodreads,
                MetadataProvider::Readarr,
                MetadataProvider::Audible
            ]
        );
        assert_eq!(
            after.for_language("en").audiobook.entries[0].provider,
            MetadataProvider::Audible
        );
    }
    #[tokio::test]
    async fn preserves_nondefault_generic_order_and_does_not_reseed_on_restart() {
        let db = legacy().await;
        sqlx::query("DELETE FROM provider_policy WHERE language='*' AND kind='ebook'")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO provider_policy(language,kind,provider,rank) VALUES('*','ebook','readarr',0),('*','ebook','google_books',1)").execute(db.pool()).await.unwrap();
        let before = db.load_provider_policy_snapshot().await.unwrap();
        sqlx::raw_sql(NEW_POLICY).execute(db.pool()).await.unwrap();
        assert_eq!(
            entries(&before.generic.ebook),
            entries(
                &db.load_provider_policy_snapshot()
                    .await
                    .unwrap()
                    .generic
                    .ebook
            )
        );
        sqlx::query("UPDATE provider_policy SET rank=7 WHERE language='en' AND kind='ebook' AND provider='google_books'").execute(db.pool()).await.unwrap();
        let first = db.load_provider_policy_snapshot().await.unwrap();
        let second = db.load_provider_policy_snapshot().await.unwrap();
        assert_eq!(first.for_language("en"), second.for_language("en"));
        assert_eq!(
            second
                .for_language("en")
                .ebook
                .entries
                .iter()
                .find(|p| p.provider == MetadataProvider::GoogleBooks)
                .unwrap()
                .rank,
            7
        );
    }
    #[tokio::test]
    async fn rejects_invalid_persisted_values() {
        for sql in [
            "UPDATE provider_policy SET rank=-1",
            "UPDATE provider_policy SET rank=256",
            "UPDATE provider_policy SET kind='unknown-kind'",
            "UPDATE provider_policy SET provider='unknown-provider'",
        ] {
            let db = legacy().await;
            // One row avoids the PK collision hiding the invalid-value check.
            sqlx::query("DELETE FROM provider_policy WHERE kind='audiobook'")
                .execute(db.pool())
                .await
                .unwrap();
            sqlx::query(sql).execute(db.pool()).await.unwrap();
            assert!(
                db.load_provider_policy_snapshot().await.is_err(),
                "invalid policy accepted: {sql}"
            );
        }
    }
    #[tokio::test]
    async fn initializes_missing_generic_groups() {
        for kind in ["ebook", "audiobook"] {
            let db = legacy().await;
            sqlx::query("DELETE FROM provider_policy WHERE language='*' AND kind=?")
                .bind(kind)
                .execute(db.pool())
                .await
                .unwrap();
            sqlx::raw_sql(NEW_POLICY).execute(db.pool()).await.unwrap();
            let snapshot = db.load_provider_policy_snapshot().await.unwrap();
            let list = if kind == "ebook" {
                snapshot.generic.ebook
            } else {
                snapshot.generic.audiobook
            };
            assert!(
                !list.entries.is_empty(),
                "missing generic {kind} list must receive defaults"
            );
        }
    }
}
