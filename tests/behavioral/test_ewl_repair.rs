#![allow(dead_code, unused_imports)]
//! Behavioral tests for english-work-lifecycle `repair` directives.

/// REQ-IDs: REQ-014, REQ-015
/// Directive: Acceptance against prod snapshot: run repair on build/design/req-016/livrarr-prod.db; the four named works (503, 539, 575, 611) all exit Conflict status; 503 gets a GR cover via the gate (per cover_gate TDD); 539/575/611 may have blank covers per AC-012.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_repair_run_repair_acceptance_against_prod_snapshot_run_repair_build_design_req() {
    todo!()
}

/// REQ-IDs: REQ-014, REQ-015
/// Directive: User-set cover preserved: work with cover_url + setter='user'; repair does NOT overwrite (REQ-010 / AC-009).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_repair_run_repair_user_set_cover_preserved_work_cover_url_setter_user() {
    todo!()
}

/// REQ-IDs: REQ-014, REQ-015
/// Directive: Pending route: work with no ISBN and search returns no candidates; resolver returns Pending{NoCandidates}; repair records as Pending; enrichment_status='identity_pending' post-run.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_repair_run_repair_pending_route_work_no_isbn_search_no_candidates_resolver() {
    todo!()
}

/// REQ-IDs: REQ-014, REQ-015
/// Directive: Concurrency cap: 10 works + concurrency=2; peak in-flight resolver calls <= 2.
#[tokio::test]
#[ignore = "stage-5: requires implementation or integration infrastructure not available in unit tests"]
async fn test_ewl_repair_run_repair_concurrency_cap_10_works_concurrency_2_peak_flight_resolver() {
    todo!()
}

/// REQ-IDs: REQ-014, REQ-015
/// Directive: Foreign-language work: not in target set; repair skips it.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_repair_run_repair_foreign_language_work_target_set_repair_skips() {
    todo!()
}

/// REQ-IDs: REQ-014, REQ-015
/// Directive: Idempotent: running repair twice on the same target produces the same end state (re-resolving an already-confirmed work returns the same Confirmed).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_repair_run_repair_idempotent_running_repair_twice_same_target_produces_same_end()
{
    todo!()
}

/// REQ-IDs: REQ-014, REQ-015
/// Directive: (Round-2 R1-P1-003 / R1-P1-004 regression TDD — FP-009 + RP-009 English-path teeth) English enrichment refresh of works 503/539/575/611 completes with ZERO invocations of crates/livrarr-metadata/src/lib.rs::merge_impl_llm. Test harness wires a mock LlmCaller that counts calls by LlmPurpose. After repair + enrichment_service.enrich_work for each of the four fixture works, mock_llm.call_count_for(purpose=LlmPurpose::MergeArbitration) == 0 for each. Calls with purpose=LlmPurpose::CoverDisambiguation are permitted and expected when LLM is enabled AND a GR candidate falls below the deterministic threshold.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_repair_run_repair_round_2_r1_p1_003_r1_p1_004_regression() {
    todo!()
}
