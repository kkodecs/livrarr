# identity-edit test-suite merge record (2026-07-24)

Inputs: two independent red-first suites from the same r4 design — CC
(`test_identity_edit{,_durable}.rs`, 20+10 tests) and codex
(`test_identity_edit_codex{,_durable}.rs`, 26+6 tests). PO directed compare + merge.

## Outcome

- **Gated suite** (`test_identity_edit.rs`, `required-features = ["identity_edit_red"]`,
  2065 lines): **codex base** + 7 CC net-new tests appended (marked `cc_*` / "CC-merged").
- **Durable suite** (`test_identity_edit_durable.rs`, 10 tests, red-verified
  0 passed / 10 failed): **CC base** + codex's column-shim pattern adopted for the 7
  writer-bump tests (red now points at the missing writer bump, not the missing column).
- Codex's two files deleted after merge (content preserved via the base/adoptions).

## Why codex's gated base won

Fixture fidelity: stubs at the OUTBOUND HTTP boundary (local axum server serving real
GR HTML through the real GoodreadsClient, provider queue, workflow adapter, service,
handler, SqliteDb) vs CC's scripted-workflow stub; deterministic SQLite-trigger race
barriers (lost-claim + mid-tx abort); route-level retrofitted-door envelope tests
(pending_anchor_stale / identity_review_stale / identity_conflict_stale) that CC's suite
lacked; covers all four clusters CC had deferred (AC-13, AC-22 via real add_fast, AC-24
per-user caps, AC-12 partial via abort-trigger rollback).

## CC net-new adopted into the base

AC-3 column-only legacy-owner collision; AC-4 HC NotConfigured=keep through commit;
AC-9 route-level background-writer staleness (d-arm); AC-14 negatives ×2 (machine
setter → stamps user; column drift → repairs); AC-23 duplicate-work-key owner
preservation (existing owner kept over lower id; no owner → lowest id; loser column
intact); AC-16 populated-clear residue (superseded_by NULL + column NULL).

## Defects fixed at merge

- codex: conflicts table named `identity_conflicts` at 5 sites → `work_identity_conflicts`
  (source: sqlite_work_identity.rs SQL).
- codex: exact `== before + 1` generation asserts → monotonic `>` (design §Claims allows
  >1 increment per composite transaction; the edit tx = claim bump + chokepoint bump).
- codex: `with_retry_backoff(Duration::ZERO)` → `(0)` (real signature takes i64 secs).
- CC (fixed earlier, recorded here): fixture normalized-fields bug (same-user works
  deduped via idx_works_user_normalized → false test topology); GT6-derived AC-21
  baseline wording.

## Codex tests dropped (with reasons)

- Durable index test asserting 076 KEEPS the name `uniq_user_confirmed_ol_anchor` —
  contradicts the design's SQL (new name `uniq_user_confirmed_work_anchor`, old dropped).
- Durable pending/dead-end generation-bump test — asserts bumps the design deliberately
  does NOT require (pending/dead-end rows are not settled identity state; the commit
  handles post-preview rows unconditionally). Adopting it would force spurious 409s.
- Durable column/uniqueness duplicates of CC equivalents.

## Interpretation divergences settled (merged contract)

- Preview response shape: `resolved.{title,author,slot,canonicalValue}` nested +
  camelCase DTO fields (`grKey`, `identityStatus`) — codex's, matching the house
  camelCase API precedent. Sibling assessments: `slot` + `action` (keep/drop) + `cause`.
- `classify_identifier_input(input, hint: Option<AnchorType>)` — owned hint (codex).
- Backfill entry: `livrarr_db::backfill_work_identity_ledger(pool)` (precedent:
  backfill_gr_numeric lives in livrarr-db).
- Unwired-provider work-key sibling (test-only shape; prod GR/OL always wired): treated
  as unproven→drop; the HC NotConfigured=keep exception stays HC-only per the design.

## Gaps the merged suite still defers (add before the code gate closes)

AC-24 global-saturation 503 arm; AC-4 OL-agrees/disagrees/outage sibling arms (need OL/HC
legs on the local provider fixture); AC-12 BUSY/FULL 503 taxonomy injection; FE
vitest + Playwright. Listed in the gated file header.

## Verification state after merge

- Durable: compiles, **0 passed / 10 failed**, every failure designed (3 schema/index/
  bridge-delta + 7 writer-bump "must bump (0 -> 0)"). Named list in the 10:44Z+ log runs.
- Default workspace build: green (gated file excluded by required-features).
- Gated: staged compile-red until implementation signatures land, per protocol.
