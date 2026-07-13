# Livrarr Quality-Remediation Plan — 2026-07-12

This document is the execution plan for fixing the findings of the 2026-07-12 code-quality probe (`code-quality-probe-2026-07-12.md`, repo root — cross-family reviewed, zero findings refuted): three fix waves ordered by risk, with the process level, parallelization layout, and open decisions for each. Item numbers (#N) refer to the probe document.

**Status: PLAN — owned by the CC orchestrator (PO handed ownership 2026-07-12); PO go still required per wave.**

---

## Journey state (LIVING — edit at the end of every session, read at the start of the next)

This section is the multi-session state carrier for the quality journey. Rules: the closing
session updates Done/Next/Blockers here (with dates); the opening session reads THIS section
plus the newest `~/Projects/kk-build/build/state/handoff-*.md` before touching anything.
kk-build state files remain authoritative for gates; this section is the narrative index.

- **As of 2026-07-13 ~01:00 UTC (session: pipeline-hygiene item-2 verification):**
  - Quality waves: NOT started. Precondition (pipeline-hygiene committed) still pending —
    the unit is now COMPLETE and verified, awaiting PO sign-off → commit.
  - Item 1 (suppression deletion): DONE + reviewed (unchanged from 2026-07-12; see that
    session's log entries).
  - Item 2 (door-gate suite): VERIFIED 2026-07-13. Per-row conformance read vs
    `design-door-gate.md` done — 19/22 rows conformant as authored; 3 weakened-expectation
    gaps found and FIXED (Layer B now asserts seam work_ids on every row, incl. the
    load-bearing B8 dedup-adoption pin; C1 asserts `source_provider_data: None`; roads.md
    R2 got its convention line; B14 aligned to the packet's bridge-only seed). Gates:
    fmt clean · clippy 0 · door-gate 22/22 · workspace 1522/0/299 (148 suites).
    `git add -f tests/behavioral/test_door_gate.rs` done (staged). Gemini review: both
    schema rounds failed (r1 bare PASS, r2 non-schema → INCOMPLETE); free-form fallback r3
    was the real review — per-row table 20/22 CONFORMS, VERDICT FAIL on 3 findings, all
    dispositioned on evidence (P0 + P1 refuted mechanically: bounded 4750ms advance < 5s;
    start_paused ⇒ current-thread determinism; P2 declined out-of-packet-scope). Closed per
    item-1 calibration; PO adjudicates at sign-off. verify.py tests + review both
    unstampable by design → 2 override_log entries in pipeline-hygiene.yaml mirror item 1.
  - kk-build friction FILED (TASKS.md 2026-07-13): dispatch-authoring 600s budget not
    enforced (new task); gemini schema-mode datapoint appended to the reliability item
    (the model-drift/fabrication item was already closed 2026-07-12 — config re-pins 3.5-flash).
  - PO directives folded 2026-07-12/13 (see Process calls): explicit cross-family review
    map · Serena-first with the worktree exception + reindex cadence · docs-sync subagent
    at every wave close.
  - CC adjustments folded into the waves below (marked `[CC 2026-07-12]`): #23 moved to
    Wave 2 · roads-map dead-code candidates added to Wave 1 · #38 pinned to zero-upgrades ·
    #15 also drops the `content_type.parse().unwrap()` · #37 recommended PARK.
  - Known editorial debt: "#39" refers to three different items across sections — run one
    numbering reconciliation pass against the probe doc before Wave 1 dispatches.
  - Next move: PO sign-off → commit the whole pipeline-hygiene unit (items 1+2: suppression
    deletion + migration 073, the suite + stubs + Cargo.toml, design-door-gate.md, wiki
    edits, stale-marked script/brief, this plan doc) → push (linear; a "Bypassed rule
    violations" line on push is expected) → then Wave 1 on PO go. Item 3 (N4 identity-edit
    check, ~10 min hands-on with PO) can slot anywhere.

---

## Sequencing precondition [CC 2026-07-12]

The pipeline-hygiene unit (suppression deletion — done, reviewed; door-gate suite — in
flight) is UNCOMMITTED on this tree and its Wave-adjacent crates overlap agents 1b/1e.
Commit pipeline-hygiene FIRST; every wave starts from a clean base.

---

## Process calls

- **No item runs the full kk-build pipeline.** Nothing here adds functionality, entities, or flows — there is no spec/IR content to write. Kept from the pipeline: a state file for tracking, red-test-first on behavior-affecting fixes, and the cross-family review gate per wave (review → fix → re-review; both families must return real verdicts).
- **Cross-family review map [PO directive 2026-07-12] — the explicit, complete list:**
  1. Per-wave merged-diff review (both families, unprimed prompt: no embedded conclusions,
     explicit license to reject; inline whole units for gemini, file-reading pass for codex).
  2. Wave 2 D1: the qBittorrent state truth TABLE is cross-family verified against qBit
     docs/poller history BEFORE agent 2a implements — the table is the artifact, not the diff.
  3. Wave 3: items #26, #30, #32, #9-part-2, #37 get their short design note cross-family
     reviewed BEFORE code (#9p2 touches the one-matching-authority; #37 keeps its dedicated
     round). Pure moves (#28, #24, #25, #27, #29, #31) ride the wave diff review only.
  4. Any red pinning test authored for Wave 2 is Codex-authored (test_write family policy)
     and Gemini-reviewed, as usual.
- **Serena-first [PO directive 2026-07-12]:** all MAIN-SESSION code navigation and editing
  goes through Serena (symbol lookup, references, symbolic edits) — no raw grep/file-spelunking.
  Parallel worktree agents are the ONE exception and must NOT edit through Serena
  (cross-contamination gotcha: Serena writes to the activated project, not the worktree) —
  they use plain file edits + code-index. Run `/kk-reindex` before each wave and after each
  wave's merge (code-index/Zoekt are snapshot indexes; a wave's deletions/renames stale them).
- **Docs-sync subagent at every wave close [PO directive 2026-07-12]:** after a wave merges
  and gates pass, dispatch a dedicated Sonnet docs subagent to sweep `wiki/` + `docs/` + this
  plan for statements the wave falsified (deleted symbols, moved files, renamed fns, DONE-able
  queue rows) and apply the mechanical fixes + a `wiki/log.md` entry. The subagent returns a
  claim list; the orchestrator spot-verifies load-bearing claims before commit — judgment
  edits to `wiki/insights.md` stay with the orchestrator. Doc updates are part of the wave's
  definition of done, not an afterthought.
- **One exception in kind:** #37 (ID newtypes) gets a dedicated design-review round before any code — it rewrites every persistence/service interface signature.
- Verification cadence per wave: `cargo fmt --check` / `cargo clippy --workspace --all-targets` / full test suite after merge; Wave 2 additionally gets `scripts/dev-restart.sh` + live smoke of the touched flows.

---

## Wave 1 — mechanical, zero intended behavior change (1 session)

Deletions and dedups where the compiler and existing tests are the safety net. **Parallel: 7 agents, disjoint crates, no file overlap.** Merge in any order; each agent runs crate-scoped checks before handing back.

| Agent | Crate scope | Probe items |
|---|---|---|
| 1a | livrarr-enrichment | #4 delete llm_validator.rs (+ mod decl); #7 remove unreachable cover-priority field |
| 1b | livrarr-server | #5 delete infra/rate_limiter.rs + its tests in state.rs + fix stale `wiki/crates/server.md:40-41`; #39 hoist per-book regex (`readarr_import_workflow.rs:419`); [CC 2026-07-12] close the roads.md dead-code queue: `create_test_library_item` + sibling helpers (api_secondary_impl.rs) and `build_tag_metadata`/`read_cover_bytes` (infra/import_pipeline.rs) — both queued as NEW candidates in wiki/architecture/roads.md since 2026-07-04; update the roads table rows to DONE in the same change |
| 1c | livrarr-download | #6 delete dead traits/structs (ProwlarrClient, QBitClient, QueueItem*); #16 replace hand-rolled `urlencoded()` with `urlencoding::encode` |
| 1d | livrarr-db | #10 one `parse_media_type` in sqlite_common (pick ONE error variant); #11 one `to_str`/`from_str` in sqlite_common |
| 1e | livrarr-metadata | #13 fence-strip helper ×3→1; #12 route cover-path formatting through cover_write_gate builders; #14 dimension-backfill twin blocks → one helper; #8 drop reserved `db` field/param on EnrichmentWorkflowImpl |
| 1f | livrarr-handlers | #15 dedupe download/stream serve block — the shared block also loses `content_type.parse().unwrap()` (panic-on-bad-data) [CC 2026-07-12] |
| 1g | frontend | #18 listImportPreview onto the shared client (also fixes the 401 auth-store desync) |

**Solo passes after merge** (touch many crates; do not parallelize): #8 remove the 7 stale `#[allow(dead_code)]` on call_sink fields; #38 centralize repeated external deps into `[workspace.dependencies]` — [CC 2026-07-12] STRICTLY unify-in-place, ZERO version upgrades (upgrades are their own reviewed change, never a rider); #17 rename `normalize_isbn` → `strip_isbn_punctuation` (cross-crate rename).

Close the wave: full gate + cross-family review of the whole diff → fix → re-review.

## Wave 2 — behavior fixes; red pinning test first on each (1-2 sessions)

Two up-front decisions, then parallel by crate:

- **Decision D1 (proposed in-wave, PO ratifies):** the single qBittorrent state truth table — which states mean "completed, safe to trigger import." Source it from qBit docs + the poller's history, not assumption.
- **Decision D2 (policy, one line, PO ratifies):** swallowed DB writes become warn-and-continue on best-effort paths, propagate where the caller can act on the failure.

| Group | Crates | Probe items |
|---|---|---|
| 2a | download + server | #1 one shared qBit classifier per D1; poller consumes it |
| 2b | db | #2 Readarr import path onto the canonical row mapper (kill the hard-coded tag fields); #36 work+anchor creation in one transaction (confirm_anchor_in_tx path — verify it exists at execution) |
| 2c | external-data | #3 OL ISBN/key tiers mirror HC's per-error-variant handling (CircuitOpen → circuit outcome, transport → retry-later; no silent downgrade to fuzzy fallback) |
| 2d | library + metadata + server | #19-#22 swallowed-writes sweep per D2 (incl. the mis-counted `linked += 1`, the unconditional success log in resolve_ol_key, CWA fire-and-forget logging, and the canonical mapper's silent try_get defaults) |
| 2e | metadata + server jobs | #33 CancellationToken through the GR pagination loops; #34 ticks that ignore `_cancel` (download_poller, rss_sync, maintenance) actually consult it |
| 2f | http | #35 poison-tolerant locks throughout outbound_queue (match the guard's own discipline) |
| 2g | matching | #9 (part 1) replace the identity-function `unicode_general_category` + partial table with `unicode_normalization::char::is_combining_mark` — match scores can change for non-Latin scripts; pin with tests |
| 2h | handlers + frontend | #23 route cover.rs's three ad-hoc error schemes through ApiError — moved from Wave 1 [CC 2026-07-12]: error RESPONSE BODIES change shape, so it belongs with the live-smoke wave; verify frontend tolerance in the smoke |

Close the wave: full gate + dev-restart + live smoke (add a book, poll a download, refresh) + cross-family review → fix → re-review.

## Wave 3 — structural moves, one at a time, interleaved with normal work

Each is a focused session with a short design note; land only on a quiet tree. Ranked lowest by both reviewers — approve piecemeal; nothing else depends on these.

Order (cheapest/safest first):
1. #28 merge engine → its own module in livrarr-enrichment (pure move)
2. #24 db/lib.rs → per-entity trait/DTO modules (pure move)
3. #25 domain/lib.rs → entities/infra/util modules + delete the stale `TEMP(pk-tdd)` scaffolding banner
4. #27 goodreads.rs → client/parsers/llm_repair modules (pure move)
5. #29 main() → named init functions
6. #26 series_query_service split (reuse the work-service-split playbook)
7. #30 manual-import business logic (audio grouping, work resolution) behind a service trait
8. #32 7-arg DB methods → request structs (removes most `too_many_arguments` allows)
9. #9 (part 2) consolidate m4_scoring's fuzzy engine onto domain text_norm primitives — one matching authority
10. #31 WorkDetailPage.tsx → per-component files (frontend; can run parallel to any Rust item)
11. #37 **ID newtypes** — design-review round first (pattern, serde/sqlx impls, staging — possibly one ID per commit). Highest type-safety payoff, widest churn. **Explicitly parkable — PO call. [CC 2026-07-12] recommendation: PARK — widest churn in the list, competes with alpha feature momentum; revisit after user feedback settles.**

## Out of scope (tracked, not fixed here)

- #39 anchor-redirect TODO (`sqlite_work_identity.rs:330-340`) — needs redirect-detection machinery that doesn't exist; future feature.
- #39 ignored dedup bug (`test_verify_d2.rs:187`) — functional bug with its own pending fix constraint, not a quality item; keep on the bug backlog.
- Everything in the probe's "deliberately not listed" section (decided/tracked elsewhere).

## Open PO decisions

1. Go/no-go per wave (Wave 1 needs no other decisions).
2. Wave 2: ratify D1 (qBit truth table, proposed in-wave) and D2 (error policy).
3. Wave 3: which items to run, and whether #37 (ID newtypes) runs or parks.
