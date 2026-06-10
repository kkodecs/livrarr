# Design — WCC re-integration onto the metadata-modularization trunk

**Status:** PLAN (pre-implementation). Verification-first, D-first. Endorsed by cross-family confer (gemini-3.1-pro + codex gpt-5.5), 2026-06-05.
**Trunk:** `feat/metadata-modularization` (worktree `/tmp/livrarr-modularization`). Build env `CARGO_TARGET_DIR=/mnt/opt/livrarr-mod-target`.
**Source of the stranded work:** `~/Projects/kk-build/build/state/wcc-wip-snapshot-2026-06-05.patch` + live `/mnt/opt/livrarr` (branch `wcc-stage5-green`).

## 0. Decision (locked)
Re-apply the stranded WCC feature onto `feat` (don't rebase the 16 extraction commits backward). **Verification-first replay**: the WCC behavioral tests are the gate; write code only to turn each red test green. **D-first ordering** (harden the identity winner rule before widening discovery). **Keep `feat`'s newer two-status model** (`IdentityStatus` + `EnrichmentStatus{Unenriched/Enriched/Thin}`); do NOT backport WCC's older unified status.

## 1. Foundation already on `feat` (de-riskers)
- **The test net is ~half-built.** 14 `test_wcc_*.rs` files are already synced into the worktree (gitignored). They map to chunks (see §3).
- **LookupResult anchors + `candidate_id`** already on feat (domain/services/work.rs) — chunk A's foundation is done.
- **Cache seam already supported:** `WorkServiceImpl.resolver` (work_service:68) → `LiveEnglishIdentityResolver.cache: Arc<TransportCache>` (identity:56), populated at resolve-time (identity:116). Chunk B consumes, doesn't build.
- **`merge_from_cached` exists** on feat (enrichment/lib.rs:363) — but inherent-only, not on the `MergeEngine` trait, and unused. Chunk B promotes + wires it.

## 2. The verification net (Step 1 — before any code)
1. **Establish the red baseline.** Run the WCC behavioral suite on `feat` (build env). Record which tests are red / green / fail-to-compile. Confer already identified two reds: `test_wcc_resolver_ac_032...isbn_locked` and `test_wcc_add_reqs_014_015...reuses_cached_payloads`.
2. **Golden-master `run_quorum`.** It's a pure fn `(HashMap<Provider, NormalizedWorkDetail>, &WorkSeed) -> Resolution`. Capture input/output fixtures (from spec ACs + the WCC monolith) and replay against the ported version — proves identical clustering/tie-break behavior across the re-homing.
3. **6-path convergence test.** `test_wcc_path_seams.rs` already covers list / Readarr / author-monitor. Extend to all six paths (Add Work, manual, list, Readarr, series, author) asserting final `{ol_key, gr_key, hc_key, isbn_13, asin}` + `IdentityStatus` + `EnrichmentStatus` via stub providers (REQ-006/022). This is the single best regression net for the spine.

## 3. Chunks (D → A → B → C), each gated: red tests → re-apply → build green → cross-family review → commit

### Chunk D — `run_quorum` anchored-cluster rule (FIRST; protects the spine)
- **Delta:** add `has_work_anchor()`; filter clusters to anchored-only before they compete; if no anchored cluster → `Unresolved` carrying seed + edition bridges (not a false `Resolved`).
- **Target:** `livrarr-identity/src/english_identity_resolver.rs:245-309`. Single caller: `resolve()` (identity:139).
- **Blast radius:** anchorless-ISBN seeds now resolve to identity-pending (intended — they converge later via cached payloads + background pass). Watch non-WCC tests that assume ISBN→Resolved.
- **Goalposts:** `test_wcc_resolver.rs` (ac_032 isbn-not-locked, ac_020_033 conflict+edition-variant, ac_001/030/024/018/022/021). Golden-master fixtures.

### Chunk A — `lookup_filtered` discovery fan-out (Add Work search box)
- **Delta:** feat 3-way `join!(gb,ol,hc)` (work_service:1279) → 4-way (+Goodreads), `take_lookup`, `interleave_by(chunk=3)`, per-provider cap 9, language-aware ordering, query-aware `llm_filter_search` on the tail only (KEEP_HEAD=9).
- **Target:** work_service:1275/1311/1490.
- **Note:** distinct from the resolver's `resolve()` discovery (chunk D) — this is the *search UI* path.
- **Goalposts:** `test_wcc_discovery_fanout.rs` (extend to 4-way + interleave), `test_wcc_add.rs::ac_024`.

### Chunk B — cached-payload reuse (REQ-014/015)
- **Delta:** (1) promote `merge_from_cached` to the `MergeEngine` trait; (2) extract `build_apply_request` shared helper; (3) thread `candidate_id` through the DTO chain; (4) `try_reuse_cached_payloads` + `cached_payloads_match_work` + `finish_created_work(candidate_id)`.
- **The candidate_id boundary chain (Codex's #1 risk — audit every hop):** resolver cache → `LookupResult.candidate_id` → handler `WorkSearchResult` DTO → frontend `WorkSearchResult` type → `AddWorkRequest` → `WorkCandidate` → `finish_created_work`. A drop anywhere silently reverts to network enrichment.
- **Target:** enrichment/lib.rs (trait@337, merge_from_cached@363); work_service finish_created_work@2089; handlers types/work + work.rs; frontend.
- **Goalposts:** `test_wcc_merge_reuse.rs` (ac_010 zero-http, ac_012 priority/null, ac_027 foreign-drop), `test_wcc_transport_cache.rs` (req_014/015), `test_wcc_add.rs::reqs_014_015`.

### Chunk C — Tier-A manual-import auto-match (the #97 user win)
- **Delta:** `eager_match_by_author` (work_service), `SuggestedMatch` (handlers manual_import), scan_service wiring, `EagerQuery` (domain), `best_candidate_index` (matching), `MatchCluster` harvest (isbn/asin/year), ManualImportPage UI.
- **Goalposts:** `test_wcc_eager_match.rs` (4 fns), `test_wcc_add.rs::ac_008/009`.

### E/F — specs + drop superseded
- Fold the WCC IR/spec design (harvest/discovery/reuse/Tier-A) into the metadata-modularization artifacts. Confirm `feat`'s two-status split fully covers REQ-030; drop the stranded status backport. Carry the GR-autocomplete reconcile (`a21c643` → external-data/goodreads.rs).

## 4. Risks & mitigations
| Risk | Mitigation |
|---|---|
| **Unanchored locks** (apply A before D → old run_quorum lets ISBN-only win → permanent unanchored works) — Gemini's #1 | **D first.** Golden-master + ac_032 gate. |
| **`candidate_id` dropped** in the 7-hop chain → silent revert to network enrichment — Codex's #1 | The add-reuse test asserts a cache-only field appears immediately; explicit chain audit in chunk B. |
| **Status-semantics drift:** WCC code sets `EnrichmentStatus::Conflict/IdentityPending`; feat owns those in `IdentityStatus` | Map to `IdentityStatus` inline per chunk; don't backport WCC's status shape (E/F). |
| **Non-WCC regressions** from the run_quorum behavior change | Run the FULL behavioral suite (not just test_wcc_*) after D; the 6-path convergence net. |
| `tests/` is gitignored — edits don't travel/commit | Sync discipline; never `git add tests/`. |

## 5. Logistics
- Per-chunk gate: red tests → re-apply → `cargo build/clippy/fmt/test` green (build env) → cross-family review (gemini+codex, author=anthropic auto-excluded) → commit on `feat`.
- **Handoff every 1–2 chunks** (tech-lead context cadence) — durable state is this doc + the on-disk green tests + the session log.
- Frontend chunks: typecheck via the main-tree node_modules symlink (disk full; no worktree install).

## 6. Open questions for the PO
1. **Golden-master source:** generate run_quorum fixtures by running the WCC monolith, or hand-author from the spec ACs?
2. **Goodreads in the *search* fan-out (chunk A):** feat's 3-way works today; WCC adds GR (anti-bot fragile). Worth the fragility in the interactive search box, or keep 3-way + just the richer interleave/tail-filter?
3. **6-path convergence test:** build the full master net now, or lean on the existing per-path seam tests + per-chunk coverage?
4. **Run the full WCC suite now** to lock the real red baseline before D, or start D and discover reds as we go?

## 7. PO decisions (live)
- **2026-06-05 — GR in search: RESOLVED → ADD Goodreads to the Add Work search fan-out via `/book/auto_complete`** (goodreads.rs:547 — fast HTTP GET, structured title/author/id, **no LLM**; user's pick = the identity vote). Verified: the LLM in GR is only the dead `/search` scrape path + foreign-language detail extraction, NOT id resolution. Consequence: the **GR-autocomplete reconcile moves UP-FRONT into chunk A** (carry a21c643's autocomplete path onto feat's `livrarr-external-data/goodreads.rs`), not deferred to E/F. Wrong-match risk covered by chunk D (winner-rule) + user pick. Open-question #2 (§6) closed.
- **2026-06-05 — Safety-net level: RESOLVED → synthetic-first, no corpus.** Use the 14 existing WCC tests + a **dense synthetic decision-matrix** for `run_quorum` (made-up provider answers + expected decision; cheap, deterministic, no network) + the **six-ways convergence** test (stub providers). Real books: only opportunistic edge cases **if already in the dev DB** (livrarr.db) — do NOT assemble a real-book corpus, no exhaustive golden snapshots. Closes §6 Q1/Q3/Q4. Rationale (PO): diminishing returns — belt-and-suspenders isn't perfect, and the effort passes below the worthwhile threshold fast.
