---
feature: identity-lifecycle-audit
stage: spec
status: approved  # PO 2026-08-21: read-only lane, all eleven lenses, trimmed pipeline, "go"
version: 1
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009]
---

# Spec: identity-lifecycle-audit

A **pure read-only audit** of the F2 identity lifecycle — the layer rebuilt by
identity-layer-rewrite (merged `ba99290e`, closed 2026-08-20). No fixes ship from
this feature; every finding becomes backlog. PO decisions of record (2026-08-21):
read-only lane, eleven lenses, machine-built coverage ledger, and process steps may
be skipped when judged unworthwhile — the pipeline for this feature is
**spec → audit run → findings report + cross-family review of the report**. No
architecture, design, tests, or code gates. No spec review round (the review effort
is spent on the findings, where accuracy lives).

## 0a. Design Principles

- **Accuracy over coverage speed.** A finding without `file:line` evidence (or a
  named DB-snapshot query result) does not enter the report. "I don't know" rows are
  legal and preferred over confident inference.
- **No repeated work.** The coverage ledger is the single source of truth for what
  has been read, by whom. A function is read once; a finding is recorded once;
  known residuals are pre-loaded so they are not re-discovered.
- **Enumerated, never sampled.** The audit's completeness claim rests on a
  machine-built inventory (Serena symbol enumeration), not on a reader's judgment of
  what looked relevant. (Wisdom-loop guard: sampled-not-enumerated.)
- **Production wiring is verified by caller scan and DB state, never by
  test-greenness.** The rewrite's worst defect was correct, fully-tested code with
  zero production feeders (insight 99).
- **Read-only is absolute.** No source edits, no live-DB writes, no external
  provider calls. DB evidence comes from a `VACUUM INTO` snapshot only.

## 0b. System Truths

The audit itself makes no external calls, so no new provider facts are sampled here.
The truths below are **already-banked facts the auditors check the code against**
(lens L9); each cites its live capture or proven artifact.

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | Insights 96/97; rounds 19–21 live fixes; detail parser harvesting `gr_work_key` separately since 2026-07-17 | Goodreads Work ids (`/work/<id>`) and Book ids (`/book/show/<id>`) are disjoint namespaces. Only `GoodreadsBookEdition` is Book-page-fetchable; `GoodreadsWork` is identity evidence only | Any code path sending a Work id to a Book endpoint, or minting one namespace's value as the other | high |
| ST-002 | Ten zero-byte captures banked 2026-08-20 (`testdata/captures/goodreads/`); insight 96 | GR soft-blocks with EMPTY-200 bodies on `/book/show/` and ~59KB Next-shell bodies (real pages ~700–870KB); autocomplete unaffected | Treating a 200 as a healthy read without body inspection; scoping GR work off the reversed "layout drift" diagnosis | high |
| ST-003 | Insight 78 empirical migration-replay verification 2026-07-24 | Anchor uniqueness is per-user (migration 044); identity-edit's 076 narrowed it to work-keys-only per-user | Audit claims about "current schema state" derived from reading migration endpoints — enumerate the full range or replay on a scratch DB | high |
| ST-004 | GR breaker state, live 2026-08-20 (handoff env notes) | GR is currently empty-200 blocked; most of the library re-dues hourly with no burn, by design | Reading busy hourly convergence logs as an audit finding — it is known, deferred to the GR feature | high |

## 0c. Prior Art

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | Metadata audit 2026-06-28 (canonical report under `docs/`; memory `project_metadata_audit_2026_06_28`) | The proven method: top-down read of every function in a domain; M-001..21 findings format. This audit copies the method and the report shape. Do NOT re-run its domain. |
| PA-002 | `~/Projects/kk-build/build/state/retro-identity-layer-rewrite.md` | This audit is that retro's structural answer (recommendation 6). Its failure-cause table seeds lenses L6–L11. |
| PA-003 | `wiki/insights.md` 85–99 | The F2 model's mechanics and failure classes — required reading for every auditor before their lane. |
| PA-004 | `spec-identity-layer-rewrite.md` v11 | The specification the code is audited against (lens L5). Covers containment note carries the corrected GR diagnosis. |
| PA-005 | `docs/canonical-model.yaml` + `ARCHITECTURE.md`/`PRINCIPLES.md` | The authority for lens L4 (architecture consistency). `audit_canonical.py` is the existing model↔code tool — run it, don't re-derive it. |
| PA-006 | `docs/roads.md` (roads map, memory `project_roads_map`) | The R1/R2 door inventory — the starting checklist for lane 1's door trace. |
| PA-007 | identity-layer-rewrite close-out handoff — residuals list | Pre-loaded known items (see REQ-006); finding them again is not a finding. |

## 1. Problem Statement

Twenty-two fix rounds rebuilt the identity layer, and the final day alone surfaced
three instances of one namespace class plus an eligibility hole — all found by live
poking, none by gates. The June metadata audit proved that a systematic top-down
read catches what tests and review miss (dead fields, unreachable branches, missing
write paths). The F2 layer has never had that read. Before the next feature builds
on it (readarr-import-removal is queued behind this), audit the entire identity
lifecycle end-to-end: every function, eleven lenses, findings to backlog.

## 2. Requirements

- **REQ-001 — Machine-built coverage ledger.** Before any reading, build the
  function inventory by Serena symbol enumeration over the audit scope (§ Scope
  below). One ledger row per function: path, symbol, lane, status
  (unread/read/blocked), reader, date. The ledger lives at
  `build/audit-identity-lifecycle/LEDGER.md` (with a machine-readable twin
  `LEDGER.json`). The audit is complete only when zero rows are unread (or each
  residual `blocked` row carries a stated reason the PO accepted).

- **REQ-002 — Flow-order lanes.** The ledger is partitioned into six lanes read
  end-to-end in the life-of-a-book order, each lane tracing producers → consumers:
  L-A entry doors + seeding; L-B settlement road + quorum/matching; L-C route
  persistence + generations/CAS + cutover/heals; L-D convergence + retry/attempt
  ledgers; L-E enrichment handoff + covers; L-F review cards + user identity doors
  (+ their frontend consumers at the API boundary). Every seam crossed answers two
  mandatory questions in writing: **who feeds this in production** (repo-wide caller
  scan) and **what DB state actually changes**.

- **REQ-003 — Eleven lenses.** Every function read and every finding filed is
  tagged with at least one lens:
  | Lens | Name | What it asks |
  |------|------|--------------|
  | L1 | Data accuracy | Is stored identity data correct? Code claims vs snapshot reality. |
  | L2 | Data completeness | Is anything missing that should exist (routes, events, files, covers)? |
  | L3 | Responsiveness | Obviously serial/wasteful paths on interactive roads — **flag suspects only**; measurement is backlog, never audit-time design (responsiveness retro rule). |
  | L4 | Architecture consistency | Conforms to ARCHITECTURE.md, PRINCIPLES.md, canonical-model.yaml, the compile wall, one-authority rules. |
  | L5 | Spec conformance | Code vs spec v11 REQ/AC text; deviations named per clause. |
  | L6 | Wiring | Every door reaches the road; every seam has ≥1 real production feeder; no self-feeding echo callers (insights 46, 99). |
  | L7 | Removals honesty | Frozen legacy columns/paths (`works.identity_status`, `ol_key/gr_key/hc_key/isbn_13/asin` scalars, pre-F2 roads): zero production readers/writers outside the sanctioned projections (insights 90, 94). |
  | L8 | Race safety | Every identity writer observes its claimed generation at decision time (insight 80's question). The two known-outside writers (convergence Pending→NeedsReview, add/adopt preflight) are pre-loaded; the audit sweeps for others. |
  | L9 | Honest failure states | No silent limbo; every terminal state has a user-visible exit; attempt/retry ledgers count what they claim; warn boundaries actually fire (M9, insights 95, 97, 99). |
  | L10 | Identifier taxonomy | Per-provider id namespaces kept apart at every mint and every fetch door — GR is done (ST-001); sweep OL/HC/GB/Audible/Audnexus/ISBN/ASIN for the same shape (insight 97's both-ends rule). |
  | L11 | Code quality / concision | Duplicated logic, dead code, needless complexity, `pub` leaks — PRINCIPLES.md § 7 and Red Flags. |
  Note: "user intent wins" defects (user choice overwritten by machine, missing
  `user_confirmed`) are filed under L1/L9 with an explicit `user-intent` tag — they
  are the product's top hierarchy rule and get called out separately in the report.

- **REQ-004 — DB snapshot evidence.** Take one `VACUUM INTO` snapshot of
  `testdata/livrarr.db` at audit start (never touch the live file). Each lane runs
  its lens-L1/L2/L9 claims as read-only SQL against the snapshot (route coverage,
  generation/audit consistency, attempt-ledger counts, orphaned rows, review-card
  states). Queries and their outputs are saved under
  `build/audit-identity-lifecycle/queries/` so every data claim is re-runnable.

- **REQ-005 — Findings ledger.** One file, `FINDINGS.md`, M-style: ID (`IA-001`…),
  severity (P0 data-damage or user-intent violation / P1 wrong behavior / P2 latent
  or drift / P3 hygiene), lens tag(s), one-sentence claim, `file:line` or query
  evidence, and a one-line suggested direction (pseudocode at most — no fixes).
  Duplicate-suspect findings are merged at PM triage, not by the finder.

- **REQ-006 — Known residuals pre-loaded.** Seeded into FINDINGS.md as OPEN rows
  before reading starts, so they are verified in passing rather than re-discovered:
  (1) missing library files — White Night + Franklin epubs absent at expected paths;
  (2) manual-import hand-picks lack `user_confirmed` (spec v11 REQ-020 gap);
  (3) work-21 dirty title; (4) r8-fixes P3 gated-startup-heal nit; (5) the round-10
  fileless door-matrix amendment (provider-ID-alone creation precedent) present in
  IMPL-REPORT-FIX-ROUND10 but never spec-folded; (6) insight 80's two
  outside-protocol identity writers (also under L8).

- **REQ-007 — Cross-family verification.** Lane reads are split: PM-side
  (Anthropic, via subagents) reads three lanes; the codex seat (gpt-5.6-sol max)
  reads three. Every P0/P1 finding is verified by the *other* family against the
  cited evidence before the report freezes; grok (4.6 xhigh) verifies any P0 and
  any finding the two families dispute. P2/P3 enter on the finder's evidence alone.

- **REQ-008 — Read-only guarantee.** The audit changes no source file, writes no
  live-DB row, calls no external provider. Its only writable surfaces are
  `build/audit-identity-lifecycle/**`, the final report under `docs/`, and (at
  close-out) wiki updates. `cargo` runs are limited to read-only checks
  (`cargo tree`, existing test suite is NOT run as part of this audit).

- **REQ-009 — Deliverables.** (1) Canonical report
  `docs/identity-lifecycle-audit-2026-08.md` — method, coverage proof (ledger
  totals), findings by severity with the user-intent call-out section, and a
  PO-facing plain-English summary up top; (2) the backlog handoff: each P0/P1 as a
  proposed next-feature/bugfix item with its evidence pointer; (3) wiki fold of any
  durable domain knowledge learned (per /pk-wiki), including corrections to
  insights the audit proves wrong.

## 3. UI/Interface Design

No UI. (Frontend code enters scope only at lane L-F's API-boundary consumers.)

## 4. Non-Requirements

- **No fixes.** Not even one-liners; everything is backlog. (PO: pure read-only.)
- **No performance measurement.** L3 flags suspects; probes/baselines are backlog.
- **No GR anti-bot work.** ST-002/ST-004 are known; the GR feature owns them.
- **No re-run of the 2026-06-28 metadata audit's domain** (enrichment merge/refresh
  pipeline body). Overlap seams (enrichment handoff, covers) are audited from the
  identity side only.
- **No kk-build changes** (PO 2026-08-21: ignore kk-build for now).
- **No test-suite execution or test authoring.** Untested seams are findable (L6/L9
  evidence), but writing tests is backlog.
- **No readarr-import-removal scoping.** Findings that touch Readarr doors are
  recorded and handed to that queued feature.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Exact lane→seat assignment (which three lanes to codex) | open | settle at ledger build, by lane size |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): `LEDGER.md` exists, was generated by symbol enumeration
      (generation command recorded in the file header), and at report freeze has
      zero `unread` rows; any `blocked` rows carry PO-accepted reasons.
- [ ] **AC-002** (REQ-002): every lane document records, per seam crossed, the
      production-feeder answer with the caller-scan evidence and the DB-state answer.
- [ ] **AC-003** (REQ-003): every FINDINGS.md row carries ≥1 lens tag; every ledger
      row was read under the lens set (spot-checkable via the lane documents).
- [ ] **AC-004** (REQ-004): the snapshot's creation command and every SQL claim's
      query + output are present under `build/audit-identity-lifecycle/queries/`;
      the live DB's mtime is unchanged by the audit.
- [ ] **AC-005** (REQ-005/006): FINDINGS.md is M-style with severities and evidence;
      the six pre-loaded residuals appear with a verified disposition each.
- [ ] **AC-006** (REQ-007): every P0/P1 row names its cross-family verifier and
      verdict; disputed rows carry the grok ruling.
- [ ] **AC-007** (REQ-008): `git status` on livrarr shows no source modifications
      attributable to the audit at report freeze (audit artifacts + report + wiki
      only).
- [ ] **AC-008** (REQ-009): the canonical report is under `docs/`, leads with the
      plain-English summary, and its backlog section lists every P0/P1 exactly once.

## Scope (the ledger's boundary rule)

A function is **in scope** iff it reads or writes identity state — provider routes,
identity generations/status (v2), anchors/ledger rows, review cards, attempt/bridge
ledgers, identity audits, `identity_key*` — or decides sameness (matching verdicts,
dedup/absorb, seed identity capture), or is a door/continuation that feeds any of
the above. Concretely that spans: `livrarr-identity` (all), `livrarr-domain`
(`identity_matching`, `seed`, identity-relevant service traits/types),
`livrarr-db` identity repositories (work-identity/anchors/review/cutover/attempt
tables), `livrarr-metadata` (settlement, convergence, the identity-facing parts of
work/discovery services), `livrarr-enrichment` (anchor derivation, route planning,
cover-candidate sourcing at the identity seam), identity/review/work handlers, the
cutover CLI, and startup heals/backfills touching identity tables. The enumeration
command and inclusion decisions are recorded in the LEDGER.md header; edge calls go
to PM triage, and an excluded-by-triage symbol is listed as `excluded` with a
reason, never silently dropped.
