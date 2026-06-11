#![allow(dead_code)]
// Behavioral contract tests for v2.1 domain extensions.
// Tests new enum variants (EnrichmentStatus::Exhausted, NotificationType::JobPanicked/RateLimitHit)
// and new Work field (enrichment_retry_count).
//
// State transition invariants (Exhausted only from Failed when retry_count >= 3)
// are enforced at the service/DB layer — tested in test_enrichment_retry_db and test_job_runner.

use librarr_domain::{EnrichmentStatus, NotificationType, Work};

#[test]
fn test_domain_v21_enrichment_status_exhausted_serde_round_trip() {
    // REQ-ID: IMPL-JOBS-005
    // IR contract: EnrichmentStatus derives Serialize/Deserialize with #[serde(rename_all = "lowercase")];
    // new v2.1 variant `Exhausted` serializes as "exhausted".
    let status = EnrichmentStatus::Exhausted;
    let json = serde_json::to_string(&status).expect("serialize EnrichmentStatus::Exhausted");
    assert_eq!(json, "\"exhausted\"");
    let de: EnrichmentStatus =
        serde_json::from_str(&json).expect("deserialize EnrichmentStatus::Exhausted");
    assert_eq!(de, status);
}

#[test]
fn test_domain_v21_notification_type_job_panicked_serde_round_trip() {
    // REQ-ID: IMPL-JOBS-001
    // IR contract: NotificationType derives Serialize/Deserialize and JobPanicked serializes as "jobPanicked".
    let notification = NotificationType::JobPanicked;
    let json =
        serde_json::to_string(&notification).expect("serialize NotificationType::JobPanicked");
    assert_eq!(json, "\"jobPanicked\"");
    let de: NotificationType =
        serde_json::from_str(&json).expect("deserialize NotificationType::JobPanicked");
    assert_eq!(de, notification);
}

#[test]
fn test_domain_v21_notification_type_rate_limit_hit_serde_round_trip() {
    // REQ-ID: IMPL-JOBS-004
    // IR contract: NotificationType derives Serialize/Deserialize and RateLimitHit serializes as "rateLimitHit".
    let notification = NotificationType::RateLimitHit;
    let json =
        serde_json::to_string(&notification).expect("serialize NotificationType::RateLimitHit");
    assert_eq!(json, "\"rateLimitHit\"");
    let de: NotificationType =
        serde_json::from_str(&json).expect("deserialize NotificationType::RateLimitHit");
    assert_eq!(de, notification);
}

#[test]
fn test_domain_v21_work_enrichment_retry_count_default() {
    // REQ-ID: IMPL-JOBS-005
    // IR contract: Work defaults enrichment_retry_count to 0.
    let work = Work::default();
    assert_eq!(work.enrichment_retry_count, 0);
}

#[test]
fn test_domain_v21_work_enrichment_retry_count_serde_default_when_missing() {
    // REQ-ID: IMPL-JOBS-005
    // IR contract: Work deserialization applies the default value 0 when enrichment_retry_count is omitted.
    let default_work = Work::default();
    let mut value = serde_json::to_value(&default_work).expect("serialize default Work");
    value
        .as_object_mut()
        .unwrap()
        .remove("enrichment_retry_count");
    let de: Work =
        serde_json::from_value(value).expect("deserialize Work without enrichment_retry_count");
    assert_eq!(de.enrichment_retry_count, 0);
}
