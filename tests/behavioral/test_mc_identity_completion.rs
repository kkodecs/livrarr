//! NOTE (id-completeness cutover): this file's five `complete_anchors` directive
//! tests were removed when that legacy function was deleted. `complete_anchors`'
//! refresh-time anchor completion — its suppressed-provider list and the
//! not_found / ambiguous / suppressed skip taxonomy — is superseded:
//!  - the hard/fuzzy harvest + monotonic badge now live in `settle_identity`
//!    (covered by test_id_completeness + test_unified_identity_path);
//!  - retry-bounding moved from per-call provider suppression to the durable
//!    per-(work, anchor) dead-end counters + the background convergence loop
//!    (covered by test_id_completeness dead_end_counters / converge_work_terminal);
//!  - the refresh "chase only still-obtainable ids" gate is covered by
//!    test_id_completeness refresh_gate.
