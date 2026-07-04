# Handoff: Metadata Remediation — Phase 4 (data completeness + convergence)

Generated 2026-07-02. Fresh CC session to BUILD Phase 4 of the remediation plan.
Working dir `/mnt/opt/livrarr`, branch `metadata-remediation` (HEAD `97963cf`).
IGNORE `/mnt/opt/scryer/livrarr` (stale duplicate — never read/edit it).

## Session-start — do these FIRST (mandatory, before any work)
1. **Read `build/foundation/principles.md`** — the 15 principles, HIGHEST authority.
   P10 (failure isolation), P14 (simplicity), P15 (fast) are load-bearing for Phase 4.
2. **Read `wiki/architecture/overview.md`** — crate graph. All arrows point to
   `livrarr-domain`; `livrarr-server` is the composition root.
3. **Read the wiki**: `wiki/insights.md` FIRST (insight 30 now describes the Phase-3
   outbound queue — the transport architecture every Phase 4 change sits on top of),
   then `wiki/domain/metadata-principles.md` (M1-M10; M5 user-sovereign fields and
   M9 never-dead-end govern this phase). Check `wiki/index.md` before re-deriving
   any subsystem.
4. **Run `/kk-reindex`** if you will use code search — code-index (Zoekt) is a
   snapshot and was STALE all through Phase 3 (wrong line numbers, phantom code).
   Serena is live (LSP). Raw `grep`/`find` are DENIED in this sandbox; use rg via
   permitted tools, python line-scanners, or Read.
5. **Read the governing docs for THIS phase**:
   - `docs/metadata-remediation-plan-2026-06-29.md` — the phase plan (Phase 4 row +
     group E) and the lean process contract.
   - `docs/metadata-audit-2026-06-28.md` — findings M-012, M-013, M-014, M-017 in
     full (the WHY and the cited code). ⚠ Audit line citations are against
     `da2a839`; the code has moved substantially (Phases 0-3 landed) — re-verify
     every citation against HEAD before building anything on it.

## Where things stand
- **Phases 0-3 are DONE and committed.** Phase 3 (transport/rate-limit consolidation)
  completed 2026-07-02: one process-global outbound queue paces / caps (in-flight 2) /
  circuit-breaks (six book buckets, explicit allowlist) / priority-orders every
  provider HTTP call. Commits `19af4d5, 7e76dec, 556e327, f03b537, 1657d26, f557e07,
  97963cf` — every stage dual-family reviewed (Gemini+Codex PASS). Tests 1030→1084.
  PO validated live (adds + series expand, clean logs). ALL COMMITS ARE LOCAL — the
  branch is not pushed.
- Workspace green: `cargo test --workspace --no-fail-fast` = 1084 passed; fmt/clippy
  clean. Dev server deployed at localhost:8789.

## Phase 4 scope (plan group E + PO-approved fold-ins)
1. **M-013** — an empty genre list from one provider blocks other providers' genres
   at the merge (empty-value override class; cf. merge-null-override precedent).
2. **M-012** — the cover quality gate is skipped on the cache path.
3. **M-014** — app-level CAS (compare-and-set) with no DB-level guard.
4. **M-017, SCOPED TIGHT** — convergence reports "done" but re-selects the same work.
   Fix ONLY the reported-done-but-re-selects defect. The broader convergence restore
   is the separate id-completeness feature (green-impl handoff at
   `~/Projects/kk-build/build/state/handoff-id-completeness.md`) — do NOT absorb its
   scope; PO chose the tight scoping explicitly (2026-07-02).
5. **Fold-in: delete `crates/livrarr-enrichment/src/pacing_queue.rs`** —
   dead unfinished machinery (`LivePacingQueue::submit` is `todo!()`, never wired in
   production; sole reference is one non-functional assertion in
   `tests/behavioral/test_metadata_refactor_pipeline.rs:359` + re-exports).
6. **Fold-in candidate (ASK PO before building): refresh vs refresh_all priority
   split** — they share `work_service::refresh()`; splitting needs a service-surface
   param (bulk → Low). Parked from Phase 3 B4; cheap here if wanted.
7. **Surface, do NOT fix: the suppression machinery is production-idle** — after
   Phase 3 stage C, nothing in production produces `ProviderOutcome::Suppressed`
   (memory: project_suppression_machinery_idle). If M-017's fix touches that area,
   flag the interaction to the PO instead of deleting/reworking suppression.

## Process (lean — proven over 6 stages in Phase 3, keep it)
- Per unit: **read-only recon agent** (Sonnet) with file:line-cited output → YOU
  verify the load-bearing citations against source → dense packet in `build/plans/`
  → **Sonnet implementer** against the packet → **run the full gate YOURSELF**
  (`cargo build` / `cargo fmt --all -- --check` / `cargo clippy --workspace
  --all-targets -- -D warnings` / `cargo test --workspace --no-fail-fast`; keep 1084
  green + name every new test) — NEVER trust an agent's "clean" claim, and ignore
  stale IDE diagnostics (they show mid-edit states; the gate run is the arbiter) →
  **cross-family review**: `cd ~/Projects/kk-build && python3 hooks/dispatch-review.py
  metadata-remediation-phase4 code /mnt/opt/livrarr --prompt-file <p> --reviewers
  gemini,codex` (NO --model flags).
- **GEMINI GOTCHA:** inline the FULL diff + context into the review prompt and tell
  gemini NOT to open files (600s file-read wall). Codex reads files fine. A
  reviewer FAIL you believe is wrong: refute with verbatim source evidence in a
  focused re-review — never self-certify past a FAIL (worked 3/3 in Phase 3).
- **Commit only when the PO says** (per-stage go-ahead, or an explicit standing
  authorization like Phase 3's overnight run). `tests/` is gitignored but its files
  are tracked — stage with `git add -u`; NEW files need explicit `git add <path>`.
  Commit messages end with the Co-Authored-By trailer.
- Deploy after each committed unit: `scripts/dev-restart.sh` (run it yourself).

## Key open threads
- M-017 vs id-completeness overlap: tight scope chosen; if the fix can't be separated
  cleanly from convergence-restore ground, STOP and bring options to the PO.
- Branch not pushed — PO was offered a backup push; not yet done.
- M-014's "DB guard" likely means a WHERE-clause/version-check on the UPDATE — check
  what the audit actually says before designing; never edit applied migrations
  (new migration only, if schema is touched).
- Pre-existing log noise, NOT Phase 4: SABnzbd polls fail 403 (~1400/day), MAM
  indexer 410. Separate fix if PO asks.
- Later phases: Phase 5 (one matching authority, M-002/M-008) is LAST and needs the
  PO's cover-threshold decision first.

## DO NOT
- Re-open Phase 3 decisions (queue design, breaker semantics, R-11 pause mapping,
  priority table) — all locked and dual-family reviewed.
- Trust code-index line numbers without a fresh /kk-reindex; audit citations are
  against `da2a839` and MUST be re-verified against HEAD.
- Delete or rework the suppression machinery (PO decision pending).
- Absorb the id-completeness feature's convergence scope into M-017.
- Advance any review gate on one-family coverage; both Gemini AND Codex must return
  a real verdict.

## Next move
Dispatch a read-only Sonnet recon agent to re-ground M-012/M-013/M-014/M-017
against HEAD `97963cf`: for each finding, locate the current code (the audit's
citations are stale), confirm the defect still reproduces in source, and map the
blast radius (callers, tests, DB writes). Verify its load-bearing citations
yourself, then write `build/plans/packet-phase4.md` (or one packet per unit if the
blast radii are disjoint) and start with the lowest-risk unit (pacing_queue deletion
or M-013).
