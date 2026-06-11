#![allow(dead_code, unused_imports)]
//! Behavioral tests for english-work-lifecycle `work_service_add` directives.

/// REQ-IDs: REQ-001, REQ-002, REQ-003, REQ-005, REQ-011, REQ-013
/// Directive: Confirmed-fresh insert: AddWorkResult::Created; works row exists; anchor row at confirmed; works.ol_key set.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_service_add_confirmed_fresh_insert_addworkresult_created_works_row_exists_anchor(
) {
    todo!()
}

/// REQ-IDs: REQ-001, REQ-002, REQ-003, REQ-005, REQ-011, REQ-013
/// Directive: Existing-ol-key-match: candidate.ol_key matches an existing work's confirmed anchor; AddWorkResult::Existing returned; no new row.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_service_add_existing_ol_key_match_candidate_ol_key_matches_existing() {
    todo!()
}

/// REQ-IDs: REQ-001, REQ-002, REQ-003, REQ-005, REQ-011, REQ-013
/// Directive: REQ-005 adopt path: existing ol-key-less work with same normalized title+author; candidate has ol_key; AddWorkResult::Existing; existing row now carries the anchor; no new row; works.ol_key updated.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_service_add_req_005_adopt_path_existing_ol_key_less_work() {
    todo!()
}

/// REQ-IDs: REQ-001, REQ-002, REQ-003, REQ-005, REQ-011, REQ-013
/// Directive: Race-loser conflict: induce concurrent insert (two work_service.add calls with same normalized identity, different ol_keys); the loser raises conflict and returns Existing pointing at the winner's id; conflict row created.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_service_add_race_loser_conflict_induce_concurrent_insert_work_service_add() {
    todo!()
}

/// REQ-IDs: REQ-001, REQ-002, REQ-003, REQ-005, REQ-011, REQ-013
/// Directive: Pending insert: candidate.identity=Pending; AddWorkResult::IdentityPending; works row with ol_key=NULL, enrichment_status='identity_pending'; anchor row at confidence='pending' with empty value.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_service_add_pending_insert_candidate_identity_pending_addworkresult_identitypending_works_row(
) {
    todo!()
}

/// REQ-IDs: REQ-001, REQ-002, REQ-003, REQ-005, REQ-011, REQ-013
/// Directive: User-selected provenance: candidate.identity.method=UserSelected; resulting work_metadata_provenance rows for cover_url, title, author have setter='user'.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_service_add_user_selected_provenance_candidate_identity_method_userselected_resulting_work(
) {
    todo!()
}

/// REQ-IDs: REQ-001, REQ-002, REQ-003, REQ-005, REQ-011, REQ-013
/// Directive: Field preservation (AC-002): candidate.fields.cover_url='https://...', year=2024, author_ol_key='OL/A1A'; persisted works row carries all three exactly.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_service_add_field_preservation_ac_002_candidate_fields_cover_url_https() {
    todo!()
}
