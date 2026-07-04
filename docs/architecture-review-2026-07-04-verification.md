# Architecture review 2026-07-04 — cross-family verification record

Backing record for the refutation pass on `docs/architecture-review-2026-07-04.md`.
Dispatch: `hooks/dispatch-review.py arch-review architecture` (kk-build), refutation brief
(attack directions: sample-verify single-source claims at code level; attack completeness;
attack the two load-bearing recommendations). Raw verdicts:
`build/reviews/arch-review/review-architecture-{google,openai}-r*.json` (local, untracked).

## Verdicts

| Reviewer | Round | Verdict | Notes |
|---|---|---|---|
| Codex (gpt-5.5) | r1 | **PASS** | 4 findings: 3 refinements + sample-verification all-confirmed |
| Gemini (gemini-3.5-flash) | r1 | REVIEW_INCOMPLETE | known empty-payload flake (114B, no verdict field; tracked kk-build-side) |
| Gemini (gemini-3.5-flash) | r2 (retry) | **PASS** | confirms the same 4 items; its own sample-verification citations |

**Independence caveat (recorded, not hidden):** the Gemini retry ran as round 2, so it
received Codex's r1 findings as prior-round context (dispatch-script design). Its PASS is a
confirmation pass plus its own source citations — not a blind independent derivation. The
blind-independent slot this round was Codex.

## Sample-verification results (the report's (agent) claims)

Both reviewers independently confirmed, with their own path:line citations:
AR-02's three dependency-table errors (identity/materialize/library manifests vs
`ARCHITECTURE.md:148-153`); AR-02's tag-write resolution ("code conforms to P5" — correct);
AR-09's three bare startup spawns vs JobRunner's panic isolation
(`jobs/mod.rs:182-228`); AR-10's 4× merge-engine construction duplication. No sampled claim
was refuted. The (agent) marks in the report are therefore cross-verified as of this record.

## Refinements folded into the report (all three verified at source by the orchestrator
before folding)

1. **AR-05 revised** (Codex R-1 / Gemini R-1): deleting `livrarr-jobs` = canonical-model
   amendment (`docs/canonical-model.yaml:75,77,79`) + removal of the `JobService` surface
   (`crates/livrarr-jobs/src/lib.rs:3-17`) — presented as amend-and-delete, not free cleanup.
2. **AR-10 extended** (R-2): `state.rs:144-153` stale "Phase 1.5 plumbing… not yet on the
   live enrichment path" comments on `provider_queue`/`enrichment_service`, which are the
   live path (`main.rs:542-565`). Verified by reading state.rs:138-158 this session.
3. **AR-02 extended** (R-3): `ARCHITECTURE.md:269-270` — the whole provider-addition
   checklist is stale (trait claim AND "enrichment dispatch table in livrarr-enrichment";
   live registration point is the enum re-exported at
   `crates/livrarr-external-data/src/lib.rs:29-31`). Verified by reading both files.

## What was attacked and held

Completeness: beyond R-2/R-3, neither family produced a missed architecture-review-worthy
issue within scope. Judgments: AR-06 "land feat/playwright-e2e" unchallenged; AR-05's
recommendation refined (above) but its substance (don't leave the ghost seam) unchallenged;
no severity tier disputed. The report's excluded-claim decision (live `MetadataProvider`
enum ≠ dead trait) drew no objection.

## Family-asymmetry note

The refutation brief again did the work: the round's real deltas (R-2, R-3) came from the
family running blind (Codex). Keep the refutation framing and treat a round-2 retry's
agreement as confirmation, never as independent derivation.
