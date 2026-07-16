# Plan — work-service-split, Option 2 (approved 2026-07-10)

**Mode:** Fable orchestrates; Sonnet 5 subagents implement. Cross-family review AFTER code
complete (PO's call — one review round at the end, not per-slice).
**Scope:** Option 2 from `assessment-work-service-split-fresh.md` §5/§7 — baseline → dead
weight → Discovery extracted as its own service. The add/refresh/enrich core is NOT restructured.
Pattern-A polish of the remainder is explicitly OUT of this plan (separate PO decision later).

## The contract (binds every slice)

- **Zero behavior change.** Pure deletes, moves, and re-wiring. Any slice needing a semantic
  decision stops and escalates to the orchestrator → PO.
- **Green gate per slice:** `cargo fmt --all -- --check` (zero diffs) · `cargo clippy
  --workspace --all-targets` (zero warnings) · `cargo test --workspace --no-fail-fast` ·
  `cargo test -p livrarr-behavioral --no-fail-fast` (the behavioral suite is wired as
  `[[test]]` targets in `crates/livrarr-behavioral/Cargo.toml` pointing at
  `tests/behavioral/*.rs` — it compiles against the workspace, so signature changes hit it).
  Known flake exception (wiki insight 58): `goodreads_through_queue_returns_success_for_
  direct_gr_key_lookup` failing ALONE under full parallel load → re-run once before judging.
- **A slice that can't go green is reverted, not patched forward.**
- **One commit per green slice** (direct linear push to main at the end, after review + PO OK).
- **Compile wall intact:** `cargo tree -p livrarr-handlers` must show no dep on livrarr-db /
  livrarr-metadata / livrarr-tagwrite / livrarr-download (insight 9b) — checked at S2b.
- **Every moved/deleted symbol gets a Serena reference check first.** A symbol with an
  unexpected referent is flagged to the orchestrator, never silently dragged along or deleted.

## Roles & dispatch rules

- **Orchestrator (me):** writes each agent's packet (dense, contract-carrying), runs ALL gates
  myself after each agent reports (agent claims are never the evidence), spot-reads the diff
  (`git diff` + Serena on load-bearing symbols), sequences agents, escalates decisions, owns
  commits and the session log.
- **Subagents:** `model=sonnet` always. **Edit agents run strictly sequentially in the main
  tree** — never in parallel, never in worktrees (Serena writes to the activated project;
  parallel edit agents cross-contaminate). Read-only recon may overlap S0 only.
- Agents implement and report; they make no judgment calls and author no artifacts.

## Slices

### S0 — Pin the baseline (orchestrator) + R0 recon (read-only Sonnet agent, concurrent)

- **S0 (me):** run the four gate commands on untouched `main`; record pass/fail counts in this
  file. A red baseline STOPS the plan (triage first — nothing moves onto a red base).
- **S0 RESULT (2026-07-10, HEAD `4f0ef232` + untracked docs only): GREEN.**
  `fmt --check` clean · `clippy --workspace --all-targets` zero warnings ·
  `cargo test --workspace --no-fail-fast` = **1,383 passed / 0 failed / 298 ignored, 135
  suites, 97.9s** (one run covers both test gates — livrarr-behavioral is a workspace member).
  Every post-slice run must reproduce: 0 failed, same 298-ignored shape, ≥1,383 passed.
- **R0 (agent, read-only):** produce the wiring map the S2 packets need:
  1. Every call site of `WorkService::lookup`, `lookup_filtered`, `eager_match_by_author`
     (handlers, composite handler-context traits per insight 41, jobs, tests).
  2. Every behavioral test file that constructs `WorkServiceImpl` and calls those 3 methods.
  3. The full contents/shape of the three `new_with_all` construction sites —
     `livrarr-server/src/main.rs:533, :665, :708` (verified 3 sites, not 1) — and what each
     instance is for; plus any other `WorkServiceImpl` constructor calls anywhere.
  4. External referents (if any) of: `StubNoLlm`, `StubTagService`, `CachedLookup`, the 8
     discovery free fns, `llm_filter_search`, the 4 `lookup_<provider>` methods.
- I verify R0's load-bearing lines myself before baking them into packets.
- **R0 RESULT (2026-07-10): complete, spot-verified (new_with_llm refs, state.rs alias, test
  alias — all match source).** Deltas vs plan assumptions:
  (a) 3 handler call sites — `handlers/work.rs:131` (fn lookup, `HasWorkService`),
  `manual_import.rs:795` (fn search, ad-hoc bound), `manual_import.rs:688` (fn scan via
  composite `ManualImportHandlerContext:12-40`, called in a tokio::spawn).
  (b) TWO hand-rolled `impl WorkService for StubWorkService` blocks in behavioral tests
  (test_consolidation_author_monitor.rs:55, test_consolidation_import_workflow.rs:1476) define
  the 3 methods → S2b must delete those method bodies or they E0407.
  (c) main.rs sites 2 (:661, feeds list_service) and 3 (:704, feeds author_monitor_workflow)
  never use discovery and get no resolver — after S2b they need NO discovery wiring; only
  site 1 (:526, the shared `work_service_arc`) feeds AppState.
  (d) `new_with_llm` has ZERO referents → delete in S1.
  (e) StubTagService is spelled in 16 behavioral-test type aliases (paired StubNoLlm lines) —
  S1 strips the M/T params there; 9 test files call `new_with_all` and lose 3 args.
  (f) 5 test files call discovery methods and repoint in S2b: test_wcc_add, 
  test_mc_filters_covers_pins, test_stress_phase4a (×2), test_wcc_eager_match (×12),
  test_wcc_discovery_fanout. `llm`/`lookup_cache` have zero leakage (private, LSP+text-confirmed).

### S1 — Dead weight (one Sonnet edit agent)

All items verified dead/duplicate this session:
1. Delete fields `http_client`, `merge_engine`, `tag_service` (`work_service.rs:40-56`) and
   generic params `M`, `T` → struct becomes `WorkServiceImpl<D, E, H, L>`. Update all 5
   constructors, every impl-block header, `convergence_service.rs`'s two where-clauses, the 3
   `main.rs` construction sites, and any test constructors. **Only the WorkServiceImpl copies
   die** — the live `DefaultMergeEngine` (enrichment crate) and `TagService` instances held
   elsewhere in `main.rs`/AppState are untouched.
2. Delete the commented-out `refresh_all` block (`work_service.rs:1515-1536`).
3. `unproxy_cover_url`: repoint the ONE caller (`cover.rs:239`) to
   `livrarr_domain::unproxy_cover_url` (verified behavior-identical,
   `livrarr-domain/src/lib.rs:1041-1048`), delete the metadata copy (`work_service.rs:3459`).
4. `StubTagService` / `StubNoLlm`: delete only if R0 found zero external referents; else leave
   and report.
- Gate → commit `refactor(metadata): S1 — drop dead fields, shrink WorkServiceImpl generics`.

### S2a — Discovery extraction, file level (one Sonnet edit agent) — MECHANISM REVISED per R0

R0 showed main.rs sites 2/3 never use discovery — plumbing a delegate Arc through all 3
construction sites in S2a would be churn that S2b immediately deletes. Revised mechanism:
- New `crates/livrarr-metadata/src/discovery_service.rs`: the discovery code moves as
  `pub(crate)` free functions over a small borrowed context struct (crate-private), e.g.
  `DiscoveryCtx<'a, C, H, L> { config: &'a C, http: &'a H, llm: &'a L, lookup_cache: &'a …,
  resolver: &'a Option<Arc<…>> }` — the convergence-precedent pattern, but with narrow bounds
  (`C: ConfigDb`) instead of the whole service.
- Move: bodies of `lookup`, `lookup_filtered`, `eager_match_by_author`, `llm_filter_search`,
  the 4 `lookup_<provider>` methods, free fns `take_lookup`, `interleave_by`,
  `lookup_result_from_captured`, `lookup_results_from_resolution`, `dedupe_lookup_results`,
  `cover_source_rank`, `finalize_eager_pick`, `best_same_work_cover`, `CachedLookup`, and the
  `discovery_tests` module. (`search_works`, `resolve_identity`, `is_supported_image`,
  `delete_cover_files` STAY — verified non-discovery owners.)
- The 3 `WorkService` trait methods in work_service.rs become thin adapters that build the ctx
  from `&self` and call the free fns. **Trait, constructors, main.rs, handlers, AppState, and
  all behavioral tests untouched in this slice.**
- Gate → commit `refactor(metadata): S2a — discovery concern extracted to discovery_service.rs`.

### S2b — Trait cutover (one Sonnet edit agent)

- New trait `DiscoveryService` in `crates/livrarr-domain/src/services/` (own file;
  `trait_variant::make(Send)` per insight 8): `lookup`, `lookup_filtered`,
  `eager_match_by_author` — moved OFF `WorkService` (20 → 17 methods,
  `services/work.rs:350`). Request/response types stay where they live in domain.
- `DiscoveryServiceImpl` implements it. `WorkServiceImpl` drops the S2a delegation field, the
  3 shims, the `llm`/`lookup_cache` fields, and the `L` generic → `WorkServiceImpl<D, E, H>`;
  constructors simplify (drop/absorb `new_with_llm` per what R0 says tests use).
- Handlers: `HasDiscoveryService` in `livrarr-handlers/src/context.rs`, impl on `AppState` in
  server `state.rs`, rebind the call sites R0 mapped (incl. any composite module trait, e.g.
  manual-import's). AppState carries `Arc<DiscoveryServiceImpl>` directly.
- Behavioral tests from R0's map: constructors/imports updated to call the discovery service —
  test BODIES/assertions unchanged (behavior-preservation is the point; an assertion that must
  change = STOP + escalate).
- Gate + compile-wall check → commit `refactor(metadata): S2b — DiscoveryService trait; god
  struct down to <D,E,H>`.

### S3 — Live smoke (orchestrator)

- `scripts/dev-restart.sh`, then exercise the moved surface for real: one UI-search lookup
  (term + identifier form), one manual-import eager-match if cheap. Server boots + search
  answers = code complete.
- Declare **code complete** to PO with the numbers (lines moved/deleted, file sizes before/after,
  test counts).

## EXECUTION RESULTS (2026-07-10/11)

- **S1** `2734fd02` (+25/−231, 16 files): dead trio + M/T generics + new_with_llm + StubTagService
  + commented refresh_all + duplicate unproxy_cover_url all gone. Gates: orchestrator-run,
  1383/0/298 exact.
- **S2a** `7c1de013` (+1393/−1296, 3 files): discovery moved to `discovery_service.rs` as free
  fns over Copy `DiscoveryCtx`; verbatim bodies (orchestrator full-read verified); adapters in
  place. Gates: 1383/0/298 exact. work_service.rs 3,628 → 2,350.
- **S2b** `3521c940` (+249/−316, 28 files): `DiscoveryService` trait (WorkService 20→17),
  `DiscoveryServiceImpl`, `HasDiscoveryService` + 3 handler rewires, AppState wiring, god
  struct → `WorkServiceImpl<D,E,H>` (7 fields), 21 test files repointed (assertions
  byte-identical), 2 hand-rolled stubs trimmed. Compile wall re-verified by orchestrator
  (handlers: domain/http/matching only). Gates: 1383/0/298 exact.
- **S3 smoke**: dev-restart clean (backend 32.8s, frontend deployed, "Server up"); health 200,
  UI root 200, `GET /api/v1/work/lookup` → 401 unauthenticated (route mounted via
  HasDiscoveryService binding, auth intact). NOT driven live: an authenticated search
  (API key is stored hashed; not available to the orchestrator) — the road is covered
  end-to-end by the behavioral discovery suites; one real UI search by the PO closes it.
- **Net**: work_service.rs 3,742 → 2,280 (−39%); discovery_service.rs 1,451 (new);
  repo net −176 lines. Behavior-equivalence traces run: lookup-cache topology unchanged
  (one shared instance serves both search handlers), resolver same shared Arc, second
  HttpFetcher instance neutral (process-global outbound queue, M-009).
- **Pending post-review**: wiki updates (crates/handlers.md Has* table + insights entry for
  the new service/trait; stale doc comments in db/lib.rs:179 + readarr_import_workflow.rs:1594
  mention the old shape), and the push to main (PO validates first).

## Then: the review (separate step, PO-flagged)

Cross-family adversarial review of the full S1→S2b diff — Gemini + Codex via
`hooks/dispatch-review.py`, both verdicts required, prompt carries NO conclusions/option-framing
(the review-priming lesson from this feature). Findings assessed together, fixed in one pass,
re-reviewed per the independent-review rule. PO validates after.

## Risk register

- Baseline may be red (suite is local-only, point-in-time) → S0 stops the plan; triage decision
  to PO.
- 3 construction sites in `main.rs` mean wiring churn ×3 — R0 maps them before any edit.
- `add`/`refresh`/enrichment core: untouched by design; any agent finding itself editing those
  bodies is off-plan → stop.
- Rollback: every slice is one commit on a clean base; revert = `git revert`/reset of that slice.

## Estimate

S0+R0 ~half a session; S1 ~1 agent-hour; S2a/S2b ~1–2 agent-hours each + my verification
between. Realistically 1–2 working sessions to code-complete, review round after.
