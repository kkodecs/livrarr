# Handoff: Metadata Remediation — Phase 1 (cleanup)

Generated 2026-06-29. For a fresh Claude Code session picking up the metadata
remediation. This effort runs **lean** (not formal kk-build): Sonnet implements,
Opus reviews, cross-family (Gemini + Codex) reviews the code. There is no
kk-build state YAML — the artifacts below are the source of truth.

## Where we are
- Branch `metadata-remediation` (off main). Working dir `/mnt/opt/livrarr`.
- **Phase 0 (identity stuck-states) is DONE.** Commits `229f2d4` (audit doc) →
  `e792849`. All green: `cargo fmt/clippy/test` clean, **1021 tests pass**.
  Cross-family reviewed clean (Codex PASS; all Gemini findings closed).
- Phase 0 fixed: conflict resolution now respects user picks + actually applies +
  recomputes the badge (M-019/M-016/M-015), Readarr import data reaches the merge
  (M-010), affirm sets the badge synchronously (M-020), plus the review-found P0
  (Merge was overwriting User-set anchors) and R-007/8/9.

## Read first (in order)
1. Memory `project_metadata_remediation.md` — project tracker: Phase 0 outcome, learnings, the 6-phase plan.
2. `docs/metadata-remediation-plan-2026-06-29.md` — the plan (6 phases, sequence, decisions, traps).
3. `docs/metadata-audit-2026-06-28.md` — the audit (findings M-001..M-021); cross-family verified.
4. `PRINCIPLES.md` + `ARCHITECTURE.md` — the target state the findings violate.
5. Phase 0 review artifacts (the pattern to copy): `build/reviews/metadata-remediation-phase0/*.json` and `build/reviews/phase0-fix-plan.md`.

## Phase 1 — the work (cleanup; low-risk)
- **M-006** — delete dead code: `livrarr-http/src/rate_limit.rs` (whole module), the `goodreads_rate_limiter` field, dead enrichment LLM fields (`validator`/`llm` on `EnrichmentServiceImpl`), `bulk_resolver::resolve_bulk` (migrate/remove its ignored behavioral tests FIRST — `tests/behavioral/test_ewl_bulk_resolver.rs`), `llm_ewl`, `ResolverConfig::confirm_title_jaccard`, `trigger_monitor` (empty stub — delete or wire).
- **M-011** — DELETE the dead 24h `metadata_cache` (migration 056 table + `MetadataCacheDb` trait + `sqlite_metadata_cache.rs` impl). Decision LOCKED: delete.
- **M-007** — wire the audiobook cover-dimensions writer (only the ebook slot writes dims today; `audiobook_cover_width/height` read 0 forever).

## TRAP (critical)
Phase 1 must **NOT** delete `RateBucket::Audnexus` / `RateBucket::Audible`. They
look dead only because those clients bypass the fetcher; **Phase 3 revives them.**
Leave them.

## Process to follow (lean)
- Dispatch a Sonnet implementer → Opus reviews the diff → cross-family code review:
  `cd ~/Projects/kk-build && python3 hooks/dispatch-review.py metadata-remediation-phase1 code /mnt/opt/livrarr --prompt-file <prompt>` — NO `--model` (models pinned in `config.yaml`: gemini-3.1-pro-preview + gpt-5.5); ~1200s; run in background. Read `build/reviews/metadata-remediation-phase1/review-code-{google,openai}-r*.json`.
- **`dispatch-review.py` reviews CODE (git-diffs)** — a plan/design review can't PASS until code exists. Use design-review to vet a plan, then implement, then code-review.
- ALWAYS pass `model=sonnet` on implementer Agent dispatches.

## Pending / deferred — don't lose
- **R-005 (from Phase 0):** verify by counting works with unparseable `setter` values in the real dev DB (expect 0); if any exist, add a legacy→User migration. NOT done yet.
- Deferred to **Phase 2-3:** provider-redirect detection (a `TODO(phase2-3)` marker sits in `detect_conflicting_anchors`); the add-time QuorumTie "pick at add" reshape.
- Gaps the audit never covered (verify before claiming conformance): tag-write = user-initiated-only invariant (unaudited); privacy boundary "only public info leaves" (unverified); compile wall only partially built (`livrarr-handlers` depends on http+matching, not jobs).

## DO NOT
- Don't re-derive Phase 0 decisions — read the plan + memory first.
- **QuorumTie is a recurring blind spot:** any QuorumTie claim — trace `existing_work_id` in source first (`async_resolver::llm_identity_verify` creates work-scoped ties, `existing_work_id = work.id`). It has produced a false "out-of-scope" assumption 3×.
- Don't treat a green aggregate test count as proof a specific new test ran — name it (`cargo test -p <crate> -- <names>`).

## Next move
Start Phase 1: dispatch a Sonnet agent to (1) delete the M-006 dead code **excluding `RateBucket::Audnexus/Audible`** and migrate the `bulk_resolver` ignored tests first, (2) delete the M-011 dead `metadata_cache`, (3) wire the M-007 audiobook cover-dims writer — all with tests; then Opus review → cross-family code review. Phase 0 and Phase 1 share files, but Phase 0 is committed, so the tree is clear.
