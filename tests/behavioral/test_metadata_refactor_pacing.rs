use livrarr_domain::{
    services::{EnrichmentMode as DomainEnrichmentMode, PacingLane, ProviderCallOutcome},
    MetadataProvider,
};

#[test]
fn domain_enrichment_mode_maps_cache_and_lane_semantics() {
    // AC-011
    assert!(!DomainEnrichmentMode::Manual.bypasses_cache());
    assert!(!DomainEnrichmentMode::Background.bypasses_cache());
    assert!(DomainEnrichmentMode::HardRefresh.bypasses_cache());
    assert_eq!(
        PacingLane::from(DomainEnrichmentMode::Manual),
        PacingLane::Foreground
    );
    assert_eq!(
        PacingLane::from(DomainEnrichmentMode::HardRefresh),
        PacingLane::Foreground
    );
    assert_eq!(
        PacingLane::from(DomainEnrichmentMode::Background),
        PacingLane::Background
    );
}

#[ignore = "pk-impl: blocked pending green budget schema (AC-008)"]
#[tokio::test]
async fn google_books_quota_exhausted_returns_without_network_call_other_providers_continue() {
    // AC-008
    let _intended_assertion = (
        MetadataProvider::GoogleBooks,
        PacingLane::Foreground,
        ProviderCallOutcome::QuotaExhaustedUntil,
        MetadataProvider::Hardcover,
        ProviderCallOutcome::Ok,
        "seed the DB daily-budget ledger above the Google Books limit; submit(GoogleBooks) \
         must return QuotaExhaustedUntil before any network call, while submit(Hardcover) \
         still returns Ok",
    );
}

#[ignore = "pk-impl: blocked pending green lane pools (AC-016)"]
#[tokio::test]
async fn foreground_lane_drains_before_queued_background_lane() {
    // AC-016
    let _intended_ordering_assertion = (
        PacingLane::Background,
        PacingLane::Foreground,
        "saturate the provider worker pool or contend on one provider, enqueue a background \
         provider call, then enqueue a foreground provider call; an execution-order spy must \
         observe the foreground call completing before the queued background call",
    );
}
