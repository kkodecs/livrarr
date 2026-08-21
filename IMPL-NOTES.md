# IMPL-NOTES — AUD-P1-10: provider failure must never burn the REQ-027 attempt ledger

## Commits

- Red (test-only, on frozen base 4b4acf25): 43f6b07b
- Implementation: 3f5d441e; r1 review fixes (EXP-SEM-R1-01/02/03): see log
- Notes: this commit

## Red evidence (r1)

Four cases failed by assertion on the frozen base, all driving the real
convergence tick with scripted transports / stub clients at the ProviderClient
seam and the activated real test DB: the amended round15 probe pin (0,1)→(0,0),
probe-failure beside an honest miss, anchored 5xx beside a search miss, and a
bridge-arm case later re-pinned (below). The clean-miss/threshold/settle
control test was green on base.

## Chosen fold shape

One typed value replaces the `search_ledger_burnable` bool at every accounting
boundary: `livrarr_domain::services::LedgerPassAccounting { Idle, CardOrMiss,
Settled, LegFailed }` with one commutative fold `combine` — `Idle` identity;
`LegFailed` absorbs, then `Settled`, then `CardOrMiss`. Every leg contributes
one value (search leg: card/miss, settle, or failed; ANY failed probe:
`LegFailed` — a text-decisive pick still emits its settlement evidence beside
the failed-leg fact, and a proposal-grade pick mints no card; anchored fetch:
`LegFailed` on WillRetry/PermanentFailure; cache hits and skips: nothing).
Queue, convergence, and server all fold with the same `combine`, so "one leg
failed" reaches the burn site un-collapsible. A second boolean would reopen
the OR-drift; the enum makes the fold total and compiler-enumerated.

Burn decision: a fired search pass burns iff the fold is `CardOrMiss`. The
legacy edition-only bridge arm (no search leg fired) is OUT OF SCOPE and
stands byte-equivalent to its frozen policy: burn iff no Work-level route,
provider failure or not (EXP-SEM-R1-01 corrected r1's over-reach; the pin
`aud_p1_10_bridge_arm_legacy_burn_survives_anchored_failure` proves it burns).

## r1 review fixes

- R1-01 (P1): failure gate now applies only to the search arm; bridge test
  re-pinned to legacy burns=1; notes corrected.
- R1-02 (P2): text-decisive probe failure folds `LegFailed` while still
  settling; provider-client comment no longer calls a transport/parse failure
  an honest miss.
- R1-03 (P2): zero-network candidate reuse reports
  `provider_chase_attempted=false` + `Idle`; asserted in the real reuse test.

## Changed files

domain services/{enrichment,work}.rs; enrichment provider_queue.rs + lib.rs;
metadata convergence_service.rs, work_service.rs, enrichment_workflow_service
.rs, lib.rs stubs; server identity_layer.rs; external-data provider_client.rs
(comment); behavioral stubs + test adaptations (only the round15 defect pin
changed expectation).

## Gates (inside the worktree, after r1 fixes)

- `cargo fmt --all -- --check`: 0 diffs
- `cargo clippy --workspace --all-targets`: 0 warnings
- `cargo test --no-fail-fast`: exit 0, zero failures (`test_ilr_contracts`
  196/196)

## Limitations

- On the search arm a permanently failing provider/probe suppresses burns
  indefinitely (never parks until recovery) — the contract's intended trade;
  the legacy bridge arm still parks such works.
- Anchored `Conflict`/`NotConfigured` count as completed verdicts, not
  failures (contract lists provider error, probe, task, breaker/queue pause).
