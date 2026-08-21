# IMPL-NOTES — AUD-P1-10: provider failure must never burn the REQ-027 attempt ledger

## Commits

- Red (test-only, on frozen base 4b4acf25): 43f6b07b
- Implementation: 3f5d441e

## Red evidence

Four cases failed by assertion on the frozen base, all driving the real
convergence tick (`run_identity_convergence_tick`) with scripted transports /
stub clients at the ProviderClient seam:

- `ac025b_goodreads_propose_probe_failure_remains_a_miss` — defect pin amended
  (cards, burns) (0,1) → contract (0,0); failed with left (0,1)
- `aud_p1_10_probe_failure_leg_beside_honest_miss_never_burns` — left (0,1)
- `aud_p1_10_anchored_provider_failure_blocks_pass_burn` — left 1, right 0
- `aud_p1_10_bridge_arm_anchored_failure_never_burns` — left 1, right 0
- `aud_p1_10_clean_miss_threshold_and_settle_semantics_unchanged` — green on
  the frozen base: clean miss burns once per pass, threshold parks, settle
  burns none

Tests live in the already-registered `tests/behavioral/test_ilr_contracts.rs`
(no new root file — its route harness is file-local), committed with
`git add -f`. `create_activated_test_db()` is the activated-schema variant of
the real in-memory test DB; the route-authoritative road requires it.

## Chosen fold shape

One typed value replaces the `search_ledger_burnable` bool at every accounting
boundary: `livrarr_domain::services::LedgerPassAccounting { Idle, CardOrMiss,
Settled, LegFailed }` with one commutative fold `combine` — `Idle` the
identity; `LegFailed` absorbs, then `Settled`, then `CardOrMiss`. Every leg
contributes one value (search leg: card/miss, settle, or failed;
proposal-grade probe failure: `LegFailed`, no card; anchored fetch:
`LegFailed` on WillRetry/PermanentFailure, nothing on completed verdicts;
cache hits and skips: nothing), and every layer — queue, convergence, server —
folds with the same `combine`. "One leg failed" stays visible at the burn site
and can never be OR-collapsed away by a burnable sibling — the common shape of
all three finding legs. The burn site charges only `CardOrMiss` on the search
arm; the legacy bridge arm keeps its `!has_work_route` policy, now gated by
`!leg_failed()` — the missing failure discrimination was the defect, the
policy stays per the out-of-scope list. A second boolean would reopen the same
drift (a future fold site can OR the wrong one); the enum makes the fold total
and the compiler enumerates every accounting site.

## Changed files

- domain: `services/enrichment.rs` (new type; two fields), `services/work.rs`
- enrichment: `provider_queue.rs` (typed capture, probe-failure fix, pass fold
  incl. anchored legs), `lib.rs`
- metadata: `convergence_service.rs` (combine replaces the OR fold),
  `work_service.rs`, `enrichment_workflow_service.rs`, `lib.rs` (test stubs)
- server: `identity_layer.rs` (failure-discriminated burn decision)
- behavioral stubs + 11 test files (mechanical field adaptation; the only
  amended expectation is the defect pin above)

## Gates (inside the worktree)

- `cargo fmt --all -- --check`: 0 diffs
- `cargo clippy --workspace --all-targets`: 0 warnings
- `cargo test --no-fail-fast`: exit 0, zero failures
  (`test_ilr_contracts` 196/196)

## Limitations

- A permanently failing provider/probe suppresses burns indefinitely: the work
  stays cadence-selectable, never parking until the provider recovers — the
  contract's intended trade.
- Anchored `Conflict`/`NotConfigured` count as completed verdicts, not
  failures (the contract lists provider error, probe, task, breaker/queue
  pause).
- `Settled` vs `LegFailed` precedence is unobservable at the one existing burn
  site (settled evidence also routes through the fresh-handoff gate).
