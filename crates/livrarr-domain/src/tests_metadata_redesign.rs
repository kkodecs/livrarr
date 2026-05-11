//! Behavioral contract tests for Phase 1 metadata domain model redesign.

use chrono::{DateTime, Utc};

use crate::services::{
    AddWorkRequest, AddWorkResult, SourceProviderData, TagSyncItemResult, UpdateWorkRequest,
};
use crate::{
    EnrichmentStatus, LibraryItem, MediaType, MetadataProvider, ProvenanceSetter, TagStatus, Work,
};

fn fixed_time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-05-11T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn assert_option_f64(_: &Option<f64>) {}

fn assert_option_option_f64(_: &Option<Option<f64>>) {}

fn assert_option_string(_: &Option<String>) {}

fn enrichment_status_wire_name(status: EnrichmentStatus) -> &'static str {
    match status {
        EnrichmentStatus::Unenriched => "unenriched",
        EnrichmentStatus::Enriched => "enriched",
        EnrichmentStatus::Failed => "failed",
        EnrichmentStatus::Conflict => "conflict",
    }
}

fn tag_status_wire_name(status: TagStatus) -> &'static str {
    match status {
        TagStatus::Synced => "synced",
        TagStatus::Pending => "pending",
        TagStatus::Failed => "failed",
    }
}

fn sample_work(series_position: Option<f64>) -> Work {
    Work {
        id: 42,
        user_id: 7,
        title: "Contract Work".to_string(),
        sort_title: Some("contract work".to_string()),
        subtitle: Some("Phase 1".to_string()),
        original_title: None,
        author_name: "Contract Author".to_string(),
        author_id: Some(9),
        description: Some("Domain model contract fixture".to_string()),
        year: Some(2026),
        series_id: Some(11),
        series_name: Some("Contract Series".to_string()),
        series_position,
        genres: Some(vec!["fiction".to_string(), "metadata".to_string()]),
        language: Some("eng".to_string()),
        page_count: Some(321),
        duration_seconds: Some(12_345),
        publisher: Some("Example Press".to_string()),
        publish_date: Some("2026-05-11".to_string()),
        ol_key: Some("OL123W".to_string()),
        hc_key: Some("HC123".to_string()),
        gr_key: Some("GR123".to_string()),
        isbn_13: Some("9780000000001".to_string()),
        asin: Some("B000000001".to_string()),
        narrator: Some(vec!["Narrator".to_string()]),
        narration_type: None,
        abridged: false,
        rating: Some(4.25),
        rating_count: Some(128),
        enrichment_status: EnrichmentStatus::Unenriched,
        enrichment_retry_count: 0,
        enriched_at: None,
        enrichment_source: None,
        cover_url: Some("https://example.test/cover.jpg".to_string()),
        cover_manual: false,
        monitor_ebook: true,
        monitor_audiobook: true,
        import_id: Some("import-1".to_string()),
        added_at: fixed_time(),
    }
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn enrichment_status_has_only_phase1_variants_default_and_snake_case_serde() {
    let variants = [
        EnrichmentStatus::Unenriched,
        EnrichmentStatus::Enriched,
        EnrichmentStatus::Failed,
        EnrichmentStatus::Conflict,
    ];

    assert_eq!(variants.len(), 4);
    assert_eq!(EnrichmentStatus::default(), EnrichmentStatus::Unenriched);

    for status in variants {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, format!("\"{}\"", enrichment_status_wire_name(status)));
        assert_eq!(
            serde_json::from_str::<EnrichmentStatus>(&json).unwrap(),
            status
        );
    }
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn metadata_provider_has_readarr_variant_and_serializes_as_readarr() {
    let provider = MetadataProvider::Readarr;

    let json = serde_json::to_string(&provider).unwrap();

    assert_eq!(json, "\"readarr\"");
    assert_eq!(
        serde_json::from_str::<MetadataProvider>(&json).unwrap(),
        MetadataProvider::Readarr
    );
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn tag_status_has_three_variants_default_and_snake_case_serde() {
    let variants = [TagStatus::Synced, TagStatus::Pending, TagStatus::Failed];

    assert_eq!(variants.len(), 3);
    assert_eq!(TagStatus::default(), TagStatus::Pending);

    for status in variants {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, format!("\"{}\"", tag_status_wire_name(status)));
        assert_eq!(serde_json::from_str::<TagStatus>(&json).unwrap(), status);
    }
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn series_position_is_optional_f64_on_work_add_request_and_update_request() {
    let work = sample_work(Some(1.5));
    assert_option_f64(&work.series_position);
    assert_eq!(work.series_position, Some(1.5));

    let add_req = AddWorkRequest {
        title: "Contract Work".to_string(),
        author_name: "Contract Author".to_string(),
        year: Some(2026),
        language: Some("eng".to_string()),
        ol_key: Some("OL123W".to_string()),
        gr_key: Some("GR123".to_string()),
        author_ol_key: Some("OL456A".to_string()),
        cover_url: Some("https://example.test/cover.jpg".to_string()),
        detail_url: Some("https://example.test/work".to_string()),
        series_id: Some(11),
        series_name: Some("Contract Series".to_string()),
        series_position: Some(1.5),
        monitor_ebook: Some(true),
        monitor_audiobook: Some(false),
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: Some("import-1".to_string()),
        source_provider_data: None,
    };
    assert_option_f64(&add_req.series_position);
    assert_eq!(add_req.series_position, Some(1.5));

    let update_req = UpdateWorkRequest {
        title: None,
        author_name: None,
        series_name: None,
        series_position: Some(Some(2.0)),
        monitor_ebook: None,
        monitor_audiobook: None,
    };
    assert_option_option_f64(&update_req.series_position);
    assert_eq!(update_req.series_position, Some(Some(2.0)));
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn add_work_request_uses_source_provider_data_import_context_and_optional_monitoring() {
    let source_provider_data = SourceProviderData {
        description: Some("Readarr supplied description".to_string()),
        isbn: Some("9780000000001".to_string()),
        asin: Some("B000000001".to_string()),
        publisher: Some("Example Press".to_string()),
        genres: Some(vec!["fiction".to_string()]),
        page_count: Some(321),
        rating: Some(4.25),
        rating_count: Some(128),
        cover_url: Some("https://example.test/cover.jpg".to_string()),
        series_name: Some("Contract Series".to_string()),
        series_position: Some("1-3".to_string()),
    };

    let req = AddWorkRequest {
        title: "Contract Work".to_string(),
        author_name: "Contract Author".to_string(),
        year: Some(2026),
        language: Some("eng".to_string()),
        ol_key: Some("OL123W".to_string()),
        gr_key: Some("GR123".to_string()),
        author_ol_key: Some("OL456A".to_string()),
        cover_url: Some("https://example.test/cover.jpg".to_string()),
        detail_url: Some("https://example.test/work".to_string()),
        series_id: Some(11),
        series_name: Some("Contract Series".to_string()),
        series_position: None,
        monitor_ebook: None,
        monitor_audiobook: Some(false),
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: Some("import-1".to_string()),
        source_provider_data: Some(source_provider_data),
    };

    assert_eq!(req.series_id, Some(11));
    assert_eq!(req.import_id.as_deref(), Some("import-1"));
    assert_eq!(req.monitor_ebook, None);
    assert_eq!(req.monitor_audiobook, Some(false));
    assert_eq!(
        req.source_provider_data
            .as_ref()
            .and_then(|data| data.series_position.as_deref()),
        Some("1-3")
    );
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn add_work_result_reports_creation_and_final_enrichment_status() {
    let result = AddWorkResult {
        work: sample_work(None),
        created: true,
        author_created: false,
        author_id: Some(9),
        messages: vec!["added".to_string()],
        cover_mtime: Some(1_777_000_000),
        enrichment_status: EnrichmentStatus::Enriched,
    };

    assert!(result.created);
    assert!(!result.author_created);
    assert_eq!(result.author_id, Some(9));
    assert_eq!(result.enrichment_status, EnrichmentStatus::Enriched);
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn tag_sync_item_result_reports_per_library_item_success_or_error() {
    let success = TagSyncItemResult {
        library_item_id: 100,
        succeeded: true,
        error: None,
    };
    let failure = TagSyncItemResult {
        library_item_id: 101,
        succeeded: false,
        error: Some("permission denied".to_string()),
    };

    assert_eq!(success.library_item_id, 100);
    assert!(success.succeeded);
    assert_eq!(success.error, None);
    assert_eq!(failure.library_item_id, 101);
    assert!(!failure.succeeded);
    assert_eq!(failure.error.as_deref(), Some("permission denied"));
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn source_provider_data_fields_are_all_optional_and_default_to_none() {
    let data = SourceProviderData::default();

    assert_eq!(data.description, None);
    assert_eq!(data.isbn, None);
    assert_eq!(data.asin, None);
    assert_eq!(data.publisher, None);
    assert_eq!(data.genres, None);
    assert_eq!(data.page_count, None);
    assert_eq!(data.rating, None);
    assert_eq!(data.rating_count, None);
    assert_eq!(data.cover_url, None);
    assert_eq!(data.series_name, None);
    assert_eq!(data.series_position, None);
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn work_omits_metadata_source_and_library_item_tracks_tag_status() {
    let work = sample_work(Some(0.5));
    assert_eq!(work.series_position, Some(0.5));
    assert_eq!(work.enrichment_status, EnrichmentStatus::Unenriched);

    let item = LibraryItem {
        id: 100,
        user_id: 7,
        work_id: work.id,
        root_folder_id: 3,
        path: "/library/Contract Author/Contract Work.epub".to_string(),
        media_type: MediaType::Ebook,
        file_size: 2048,
        import_id: Some("import-1".to_string()),
        imported_at: fixed_time(),
        tag_status: TagStatus::Pending,
        tagged_at_generation: 0,
    };

    assert_eq!(item.tag_status, TagStatus::Pending);
    assert_eq!(item.tagged_at_generation, 0);
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn provenance_setter_has_import_variant() {
    let setter = ProvenanceSetter::Import;

    assert!(matches!(setter, ProvenanceSetter::Import));
}
