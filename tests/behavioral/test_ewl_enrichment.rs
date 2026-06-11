#![allow(dead_code, unused_imports)]
//! Behavioral tests for english-work-lifecycle `enrichment` directives.

/// REQ-IDs: REQ-017
/// Directive: Apply path: GR returns cover_url + gr_key, gate returns Apply; payload reaches merge with cover_url + gr_key populated; merge picks GR cover when priority allows.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_enrichment_service_invoke_cover_gate_apply_path_gr_cover_url_gr_key_gate_apply() {
    todo!()
}

/// REQ-IDs: REQ-017
/// Directive: Skip-deterministic path: jaccard < 0.6, llm_enabled=false; payload reaches merge with cover_url=None and gr_key=None; merge takes cover from HC or OL or leaves blank.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_enrichment_service_invoke_cover_gate_skip_deterministic_path_jaccard_0_6_llm_enabled_payload(
) {
    todo!()
}

/// REQ-IDs: REQ-017
/// Directive: AskLlm + SameBook: jaccard < 0.6, llm_enabled=true, mocked LLM returns SameBook; cover_url + gr_key preserved; merge applies.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_enrichment_service_invoke_cover_gate_askllm_samebook_jaccard_0_6_llm_enabled_mocked_llm(
) {
    todo!()
}

/// REQ-IDs: REQ-017
/// Directive: AskLlm + NotSameBook: same setup, LLM returns NotSameBook; cover stripped.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_enrichment_service_invoke_cover_gate_askllm_notsamebook_same_setup_llm_notsamebook_cover_stripped(
) {
    todo!()
}

/// REQ-IDs: REQ-017
/// Directive: AskLlm + Failed: LLM times out → Failed → Skip; cover stripped.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_enrichment_service_invoke_cover_gate_askllm_failed_llm_times_failed_skip_cover_stripped(
) {
    todo!()
}

/// REQ-IDs: REQ-017
/// Directive: Non-English work: gate not invoked; cover_url flows through unchanged.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_enrichment_service_invoke_cover_gate_english_work_gate_invoked_cover_url_flows_through_unchanged(
) {
    todo!()
}

/// REQ-IDs: REQ-017
/// Directive: Non-GR provider (HC payload with cover): gate not invoked.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_enrichment_service_invoke_cover_gate_gr_provider_hc_payload_cover_gate_invoked() {
    todo!()
}

/// REQ-IDs: REQ-013
/// Directive: Identity-pending work appears in list_works_due_for_retry output.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_enrichment_retry_due_works_query_identity_pending_work_appears_list_works_due_retry_output(
) {
    todo!()
}

/// REQ-IDs: REQ-013
/// Directive: Retry job calls resolver on identity-pending; Confirmed result promotes the anchor and enqueues enrichment.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_enrichment_retry_due_works_query_retry_job_calls_resolver_identity_pending_confirmed_promotes_anchor(
) {
    todo!()
}

/// REQ-IDs: REQ-013
/// Directive: Still-pending: retry_at advances per existing backoff; counter increments; work stays in identity_pending state.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_enrichment_retry_due_works_query_still_pending_retry_advances_existing_backoff_counter_increments_work(
) {
    todo!()
}
