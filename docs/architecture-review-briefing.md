# Architecture Review Briefing

Prepared 2026-07-04 for the top-to-bottom architecture review.
Review tip: `main @ 00daf3a` (the remediation-epoch merge — everything from the
2026-06-28 audit through N2 cover consolidation and id-completeness). Pushed to
origin + backup. Workspace at merge: 1331 tests / 0 failures; fmt + clippy clean;
canonical drift audit: exit 0 (no hard drift).

## Reading order

1. **Principles** — ⚠ TWO copies exist and must be reconciled first (see
   Day-one items): tracked `PRINCIPLES.md` (root, 2026-06-29) vs newer
   untracked `build/foundation/principles.md` (2026-07-03, 15 principles —
   the copy CLAUDE.md names as highest authority).
2. **Structure contract:** `docs/canonical-model.yaml` — entity spine, legal
   crate seams, invariants, amendments log. Machine-audited clean today
   (`audit_canonical.py`, kk-build; path config fixed 2026-07-04).
3. **Flow contract:** `wiki/architecture/roads.md` — 14 roads, every door,
   per-road status (9 CLEAN / 4 DEBT / 1 DECISION). ⚠ Provenance: authored
   2026-07-04, self-declares same-day cross-family verification; PO to
   confirm origin. One stale row corrected 2026-07-04 (cover_backfill —
   already deleted by N2).
4. **Architecture overview:** `ARCHITECTURE.md` (root, 16K, 2026-06-29) — ⚠
   duplicate at `docs/ARCHITECTURE.md` (2026-05-20, 5.5K, presumed stale).
5. `wiki/architecture/overview.md` (crate graph) → `metadata-pathway.md` →
   `work-creation-pipeline.md` → `wiki/crates/*` per-crate pages.
6. `wiki/insights.md` — 63 items; the operational truth that never made it
   into formal docs.

## Verification status of the references (be honest about trust)

| Doc | Verified against code | Notes |
|---|---|---|
| canonical-model.yaml | TODAY (machine audit, exit 0) | 2 sanctioned deltas on record |
| roads.md | Spot-checked today; self-claims full verification | provenance pending PO confirm |
| wiki insights | Continuously maintained | most reliable wiki content |
| metadata-pathway.md | Partially corrected today | ⚠ GR sections still describe pre-Phase-5 LLM disambiguation (superseded by insight 13/59) |
| wiki/crates/*.md | server.md corrected today (trait count, import_pipeline network claim) | others: bulk-ingested, verify before trusting |
| grab-system.md, library-management.md | NOT re-verified | roads.md queues corrections (missing Transmission; confirm_scan is dead, not the orchestration surface) |

## Intentional-debt register (known, decided or pending — don't re-discover)

- **Import-path forks (roads R7/R8/R9):** manual import, Readarr import, and
  scan adoption each materialize files outside `ImportWorkflow` — the
  file→LibraryItem universe is 4 sites, not 1. Accepted pending a
  consolidation decision ("Phase-2" in roads.md).
- **User-cover fork (roads R3):** select/upload cover handlers bypass the N2
  write gate mechanics (product semantics intact; small).
- **Convergence job ships disabled** (`[convergence] enabled=false`) — the
  id-completeness engine is fully built (commit 11d2238); enabling is a PO
  call. Until then, batch-created works converge only on manual retry, and
  the N4 matcher fix stays live-unvalidated. One pre-enable check queued:
  the flag-7 suppression-test disposition (removed in 11d2238 without an
  explicit design-call record; ~70% legitimately resolved).
- **Suppression machinery idle:** `ProviderOutcome::Suppressed` has zero
  production producers (PO decision pending: keep/remove).
- **Canonical-model deltas:** `Release` rename (#141). The `library →
  materialize` edge now appears in the audit's seam list — the #143 cutover
  may have landed; verify and update the amendments log.
- **Audio tag writers disabled** (OOM class; m4b matters, revival needs a
  streaming writer).
- **Cover quality residue:** 85 works below the 400×600 floor where no
  provider offers better art (empirically probed 2026-07-04 — provider art
  for this class tops out ~500px). Parked rungs: vision-LLM judge, web image
  search, quality scoring, fall-through rescue (probed, rejected — 28
  would-swaps, ~4 visible, 1 harmful). Works 30/92: stored GB cover URL,
  image never downloads (self-heals if GB serves it). Work 136: cover file
  doesn't decode.
- **713 unsanctioned pub types** (campaign metric, audit decision-3 backlog;
  652 at the June baseline).
- **Known flake:** goodreads tracer test under full parallel load (process-
  global breaker leak, insight 58).

## Day-one review items (found during this prep)

1. Reconcile the duplicate authority docs: root `PRINCIPLES.md` vs
   `build/foundation/principles.md`; root `ARCHITECTURE.md` vs stale
   `docs/ARCHITECTURE.md`. Pick canonical locations, delete or redirect the
   others, update CLAUDE.md pointers.
2. Confirm roads.md provenance; then commit it as the flow contract and
   adopt its queued wiki corrections + dead-code deletions (8 items, one
   already done).
3. Branch archaeology: 13 legacy feature branches hold commits not in main
   (features all shipped — presumed pre-merge leftovers). Verify + archive
   or delete. GitHub: main's branch protections (no merge commits,
   PR-required, 2 status checks) were admin-bypassed by the epoch push —
   decide whether the rules or the workflow should change.

## Tooling state for the review

- code-index (Zoekt): rebuilt TODAY against 00daf3a. Serena: live LSP.
- graphify-out/ + understand-anything graph: STALE (pre-N2, June). Rebuild
  on request before any graph-driven review pass.
- `audit_canonical.py /mnt/opt/livrarr` — structure drift, runnable any time.
- Live server on :8789 runs content identical to the review tip. Pre-N2
  data snapshots: `testdata/livrarr.db.pre-n2-20260704`,
  `testdata/covers.pre-n2-20260704/`.

## Host/ops footnotes (not architecture, will show up in logs)

- SABnzbd poller 403s every tick (config/credential issue, pre-existing).
- nodebb container crash-loop (18k+ restarts) and swap 100% full on this
  host — queued hygiene, unrelated to livrarr code.
- ~13 duplicate legacy root-level cover files: kept by design (orphan
  policy), unservable, re-warn on each boot until archived.
