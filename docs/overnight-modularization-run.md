# Overnight Autonomous Run — metadata-modularization (Phase 1 → Phase 2)

You are an autonomous overnight driver. Execute this plan **one step per iteration**, fail-safe, and leave the PO a clear status to review in the morning. **Re-read this file at the start of every iteration.**

## Goal
Push the `metadata-modularization` feature forward through the kk-build process, *up to the human gates*:
- **Phase 1 (guaranteed target):** formalize the design into proper kk-build artifacts + cross-family review.
- **Phase 2 (best-effort, only if Phase 1 completes clean and budget remains):** the `livrarr-providers` extraction **canary** → GO/NO-GO.

PO chose this sequence so the morning has a **reliable deliverable (Phase 1)** plus a **shot at a testable result (Phase 2)**.

## Inputs (read these first, once)
- `docs/decision-metadata-modularization-sequencing.md` (v3 FINAL) — **the decision + the §8 first move**. Authoritative.
- `design-metadata-modularization.md` + `diagrams/metadata-modularization.html` — the target architecture (Q1–Q7).
- `spec-work-creation-consistency.md` (v5) + `ir-v1/v2-work-creation-consistency.yaml` — the metadata-pipeline features that become Track-2 work.
- Memory `project_metadata_modularization_decision`.
- Project rules: `CLAUDE.md` (Rust quality gate, family separation), `wiki/insights.md`.

## HARD RULES (safety — never violate)
1. **Work only in an isolated git worktree.** First iteration: `git worktree add /tmp/livrarr-modularization -b feat/metadata-modularization` off current HEAD. Do ALL work there. **Never** touch the main working tree, `main`, or the PO's uncommitted WCC changes.
2. **Never push. Never merge. Never run `tools/audit.sh`.** (PO's job.)
3. **Commit only at green** (`cargo build` + `cargo test` pass for what you changed). Small, labeled commits.
4. **STOP and write status** on: any unrecoverable red you can't fix in ≤3 attempts; a Phase-2 **NO-GO** (back-edge `providers→metadata`); no forward progress for 2 consecutive iterations; or budget exhaustion.
5. **Budget:** stop after ~40 iterations OR if you sense context bloat (>70%) — write status, end. Do not thrash.
6. **No new third-party deps.** Reuse the workspace's existing crates/patterns.
7. **Update `docs/overnight-status.md` every few iterations** and at every STOP (see Morning Status).

## SETUP (iteration 1)
- Create the worktree (Rule 1). `cd /tmp/livrarr-modularization`.
- Confirm baseline green there: `cargo build` (the worktree is at HEAD = committed state; the PO's uncommitted WCC work is intentionally NOT here — the providers extraction is about the existing committed structure, so HEAD is a valid base).
- Initialize `docs/overnight-status.md` with "STARTED" + timestamp-less marker (date unavailable in-run; use iteration counts).

## PHASE 1 — kk-build design artifacts (guaranteed target)
Produce, in the worktree:
1. **`spec-metadata-modularization.md`** — a proper kk-build spec, distilled from the decision brief + seed doc: problem (the tangle), the target (4 crates, one-way Identity→Enrichment, the 3 boxes + shared providers), REQs (the boundary invariants + the Track-2 features: GR ladder, cover-decouple/two-state-machines, Bug #2, de-facto identity, Q4 gating, user-ID-edit), non-requirements, acceptance criteria. Reuse the WCC spec's REQ/AC style.
2. **`ir-v1-metadata-modularization.yaml`** — architecture: the crate decomposition (providers/identity/enrichment/materialize + domain contract), the dependency graph (acyclic, one-way), the module→crate map from the brief §2.2, the seams (§2.3), `approved_libraries` (= WCC's, no new deps).
3. **`ir-v2-metadata-modularization.yaml`** — design: the extraction sequence (Track 1 stable-first, Track 2 feature-cuts), the `livrarr-providers` port design (contracts + a **search/discovery surface** + the enrichment-fetch surface; move `NormalizedWorkDetail`/`ProviderOutcome` out of lib.rs), the seam-cut pseudocode for each (discovery/cover/status), the falsifiable canary as the first build step.
- **Cross-family review** each artifact: dispatch `gemini -p "<artifact + review ask>"` and `codex exec "<...>"` (no `--model`); fold findings; iterate to convergence (≤3 rounds). If a family is quota-blocked, proceed with the available one + note it. (Family separation: you (Claude) author; they review.)
- Self-check: YAML parses; deps acyclic; every REQ maps to a module. Commit at green.
- **When Phase 1 is converged + committed → write status "PHASE 1 COMPLETE" and proceed to Phase 2.**

## PHASE 2 — the `livrarr-providers` canary (best-effort)
Only if Phase 1 is complete AND budget remains. Per brief §8:
1. Create `crates/livrarr-providers` (new workspace member).
2. Move out of `lib.rs`/`work_service`: the contract types (`NormalizedWorkDetail`, `ProviderOutcome`), a **search/discovery** surface + the detail/fetch surface, the parsing modules (`goodreads`/`google_books`/`hardcover`/`openlibrary`), `transport_cache`, the queue/client. Leave `lookup_filtered` + `enrich_work` as **consumers** (`use livrarr_providers::…`).
3. Fix imports across `livrarr-metadata` (+ server + tests). Behavior-preserving — do NOT change logic.
4. **The GO/NO-GO:**
   - ✅ **GO** — `cargo build` + `cargo test` green with **no back-edge** `providers→metadata`. Commit. Write status "CANARY GO".
   - ❌ **NO-GO** — extraction forces a back-edge (providers needing `LookupResult`/`EnrichmentContext`/queue traits from metadata) you can't cleanly resolve by moving the type into `domain`/`providers`. **STOP**, leave the worktree as-is, write status "CANARY NO-GO" with the exact offending edge.
- Tests for any new seam: dispatch to `codex exec` (OpenAI) per family separation; review to `gemini`. Graceful-degrade on quota.

## STOP conditions (write status, then end the loop)
- Phase 1 done + no budget for Phase 2 → "PHASE 1 COMPLETE, Phase 2 deferred".
- Canary GO or NO-GO (either is a complete result).
- 3 failed attempts on one compile error · 2 idle iterations · budget cap.

## Morning status — `docs/overnight-status.md` (keep current)
- What completed (Phase 1 artifacts? canary GO/NO-GO?).
- The worktree + branch to review (`/tmp/livrarr-modularization`, `feat/metadata-modularization`), and `git log --oneline` of what you committed.
- What needs **PO sign-off** before merge (the kk-build gates you stopped at).
- If NO-GO: the exact back-edge + recommendation (isolate which type / fall back to plan A).
- One-line "how to pick this up" for the next session.

## Process notes
- kk-build family separation: you (Anthropic) author code/artifacts; OpenAI (codex) writes tests; Google (gemini) reviews. No family reviews its own output. Dispatch with `gemini -p` / `codex exec`, **no `--model`** (models pinned). Degrade gracefully on quota.
- PO sign-off gates and `verify.py`/audit are **deferred to the morning** — do not attempt to cross them; stop at them and report.
- Use Serena + code-index for code nav (per CLAUDE.md). `grep`/`find` are sandbox-blocked → `awk`/code-index. Don't `git add` `tests/behavioral/*` (gitignored).
