# RACE PACKET — identity-edit implementation (side: FABLE)

You are the tech lead and implementer for ONE side of a PO-directed head-to-head
implementation race. Another lead is implementing the same feature from the same
contract in a separate worktree. Judging criteria: **code quality, speed, cost.**
You will be cross-family code-reviewed when done. Work autonomously; do not wait for
approvals between steps.

## Fence (absolute)

- Your worktree: `/mnt/opt/livrarr-race-fable` (branch `race/fable`). ALL reads and
  writes happen here. `cd` here first.
- NEVER write outside it. Specifically forbidden: `/mnt/opt/livrarr` (main checkout),
  `/mnt/opt/livrarr-race-cc`, `/mnt/opt/livrarr-race-codex`, `/mnt/opt/livrarr-race-opus` (rival implementations — do not read them either),
  `/mnt/opt/livrarr/testdata` (live DB). Tests use in-memory SQLite only.
- Do NOT use MCP tools (serena, code-index, graphify) — they point at the MAIN checkout
  and their consent dialogs will block you. Use rg / sed / direct file reads instead.
- Do not commit or push. Leave all work as uncommitted changes in the worktree.

## Contract (binding)

1. `docs/design-identity-edit.md` — the r4 design, both-family PASS. This is THE
   contract: §Slot roster, §Input classification, §Preview seam, §Preview, §Commit,
   §Durable identity generation (incl. the 17-row writer-coverage table), §Clear,
   §Migration 076 + startup ledger completion (the migration SQL is verbatim — use it
   exactly, filename `076_anchor_uniqueness_identity_generation.sql`), §add_fast
   multi-bridge abstention, §API error contract, §Frontend, §History door inventory,
   §Residuals (accepted — do not "fix" them), AC-1..AC-25.
2. `docs/identity-edit-merge-notes.md` — settled interpretation decisions (DTO shapes:
   camelCase + nested `resolved` record; `classify_identifier_input(input, hint:
   Option<AnchorType>)` owned hint; backfill lives at
   `livrarr_db::backfill_work_identity_ledger(pool)`; unwired-provider sibling = drop,
   HC NotConfigured=keep is HC-only).
3. The tests are the gate and are ALREADY in your worktree — do not weaken them:
   - `tests/behavioral/test_identity_edit_durable.rs` — 10 tests, RED right now
     (run first: `cargo test -p livrarr-behavioral --test test_identity_edit_durable`).
   - `tests/behavioral/test_identity_edit.rs` — 33 tests behind
     `required-features = ["identity_edit_red"]`; compile-red until your signatures
     land, then runtime-red, then green.
   - You may fix IMPORT PATHS in these files to match where you place symbols;
     assertions/contracts change ONLY via escalation (below).

## Scope

IN: everything the design requires in production code, backend AND frontend
(§Frontend: modal, api client `details` retention, exact invalidation keys, HistoryTab
line, bounded poll, `parkedByConflicts`). Plus migration 076 + startup backfill +
history door-inventory rows per §History door inventory.

OUT (post-race, do not build): the four deferred test clusters listed in the gated
file's header (AC-24 global-saturation arm, AC-4 OL/HC sibling arms on the fixture,
AC-12 BUSY/FULL injection, FE vitest/Playwright). FE production code IS in scope; FE
tests are not.

## Definition of done (all gates, named runs in your report)

1. `cargo test -p livrarr-behavioral --test test_identity_edit_durable` → 10/10 pass
2. `cargo test -p livrarr-behavioral --test test_identity_edit --features identity_edit_red` → 33/33 pass
3. `cargo test --workspace --no-fail-fast` → zero failures
   (known flake: `goodreads_through_queue_returns_success_for_direct_gr_key_lookup`
   under full parallel load — if ONLY that fails, re-run once)
4. `cargo fmt --all -- --check` → zero diffs
5. `cargo clippy --workspace --all-targets` → zero warnings
6. `cd frontend && npx tsc --noEmit` → clean (deps already installed)
7. `cd frontend && npx vite build` → succeeds

## Project rules that apply (violations = quality findings)

- Migrations 041/042/044 are IMMUTABLE. 076 contains EXACTLY the three ops in the
  design — no backfill inserts, no marker writes. The backfill is a Rust startup pass.
- `INSERT OR REPLACE` is banned — `INSERT ... ON CONFLICT ... DO UPDATE`.
- No SQL outside livrarr-db. No business logic in handlers. livrarr-handlers must not
  depend on livrarr-db/-metadata/-tagwrite/-download (compile wall) — handlers reach
  repos/services via `Has*` capability traits in `livrarr-handlers/src/context.rs`.
- Service traits: `trait_variant::make(Send)` pattern, trait in livrarr-domain, impl in
  the owning crate, stub in livrarr-behavioral (trait + impl + EVERY stub or the
  workspace won't compile).
- `chrono`, never `time`. Match surrounding code style. No comments about how the code
  came to be — comments describe what code IS.
- All blocking I/O in `spawn_blocking`. Async sleeps in background paths use
  `tokio::select!` with cancellation where the surrounding code does.

## Escalation

If the design is ambiguous or contradicts the code in a way that blocks you: write
`BLOCKED-<topic>.md` in the worktree root (the contradiction, your proposed reading,
file:line evidence), reply "BLOCKED — <path>", and continue on non-blocked work.
Do NOT silently reinterpret the contract.

## Work solo

Implement this yourself in one session. Do NOT spawn child agents, background worker
processes, or delegate any part of the implementation — this run measures a single
agent working alone. The cargo target dir is pre-warmed for the default feature set.

## Report

When done, write `RACE-RESULT.md` in the worktree root: the seven gate commands with
their ACTUAL tail output (pass counts), start/end wall-clock times, your total token
usage (from your session status), notable design decisions, anything you'd flag for
review. Then reply exactly: `DONE — RACE-RESULT.md`.
