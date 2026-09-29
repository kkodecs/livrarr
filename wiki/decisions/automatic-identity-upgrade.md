# Decision: existing libraries convert to the new identity layer automatically (2026-09-27)

## Executive summary

An existing alpha6 library with books now converts itself on the first start of a newer build,
through migration 091. This replaces the manual rule ratified on 2026-08-06, which refused to start
such a library until an operator rehearsed on a copy, approved a report and ran the cutover
commands. The PO accepted the replacement on 2026-09-27. The manual `identity-cutover` command stays
for developers; upgrades have no approval step. Odd cases follow fixed rules and are logged, never
asked. The contract and IR lines that state the old rule are not yet amended; this page records the
supersession and the amendment is a follow-up. See [the old rule](#the-old-rule-and-why-it-existed),
[what changed](#what-changed), [the new rule](#the-new-rule), [odd cases](#odd-case-policies) and
[what this supersedes](#what-this-supersedes).

## The old rule and why it existed

From the upgrade research, section 2 ([research](../../build/reports/identity-cutover-upgrade-research-2026-09-27.md)):

- **Refuse to serve.** "A non-empty database with an inactive marker fails startup … it never
  serves in a mixed-authority state" (`ir-v1-identity-layer-rewrite.yaml:343`). An architecture
  reviewer had raised a "split-brain window" between new writers and legacy readers.
- **Snapshot first.** The contract forbids activating a non-empty database "without two
  byte-identical approved snapshot rehearsals" (`contract-identity-layer-rewrite.yaml:332`).
- **Human review channel.** Look-alike books became review cards settled through the local CLI
  (list/show/resolve) rather than a maintenance web page.
- **Ratified** by the PO on 2026-08-06: the CLI, refuse-to-serve, and automatic activation for
  empty databases only. The research's summary: "the manual step exists to keep a single authority
  and a human check. Automation for existing libraries was never designed."

## What changed

- **Merging is off for good (PO, 2026-09-24).** "Manual merging is dropped altogether, not
  deferred" ([merge/undo rewrite dropped](merge-undo-rewrite-dropped.md)). The only question the
  cutover can raise, "are these books the same?", now has one accepted answer, "keep them separate",
  and a machine can give it (research section 3).
- **The manual path lost data and dead-ended.** On a copy of a 139-book library, 301 user-set
  identities came out unconfirmed, and one open alpha6 conflict blocked startup forever. In Docker the
  refusal became a restart loop writing a full backup each time (research sections 3 and 5).
- **PO decision, 2026-09-27** ([DECISION.md](../../build/reviews/identity-cutover-inplace/review-plan-r2/DECISION.md)):
  plan r2 accepted as the build basis. "No merging; automatic conversion on first start; shared
  identifier stays with the first book and is logged; open legacy questions closed keeping the
  current match; alpha6 'picked this search result' counts as confirmed; backup only when an upgrade
  or tracked repair is pending." The spec records these as closed questions Q-005, Q-006, Q-007 and
  Q-009.

## The new rule

- Migration 091 converts an existing library in place on the first start; the unchanged startup
  readiness check then activates it. "No commands, no rehearsal, no approval" (spec REQ-003).
- Fresh installs and already-active libraries are no-ops. A failure inside 091 undoes only 091.
- The manual `identity-cutover` CLI remains; its command code is untouched (spec PA-001, REQ-003).
  It is a developer tool, not an upgrade step, and it takes no backup (spec Q-002, parked).

## Odd-case policies

Each case is written to the audit log and never stops startup (spec REQ-004, REQ-005):

- **Look-alike books** are kept separate; nothing is merged.
- **A shared identifier** stays with the first book; the others get none, and the log names the
  value and the keeping book.
- **Old unanswered match questions** are closed, keeping the current match.
- **Alpha6 "picked this search result"** counts as confirmed; those books show "Confirmed".
- **Backup:** one protected copy per upgrade, not one per restart. See
  [Upgrading from alpha6](../deployment/upgrading.md).

## What this supersedes

- `contract-identity-layer-rewrite.yaml:332` (FP-024): "activates a non-empty database without two
  byte-identical approved snapshot rehearsals", for existing libraries converted by 091.
- `ir-v1-identity-layer-rewrite.yaml:343` (`normal_startup`): the non-empty, inactive-marker refusal
  for those libraries.

Neither file is edited here. Amending the contract and IR is a follow-up.

## Sources

[Plan r2](../../build/reports/identity-cutover-inplace-plan-2026-09-27.md) ·
[bug spec](../../spec-identity-upgrade-inplace.md) (REQ-007, PA-001) ·
[Astra review r1](../../build/reviews/identity-cutover-inplace/review-plan-r1/REVIEW-astra-r1.md) ·
[Astra review r2](../../build/reviews/identity-cutover-inplace/review-plan-r2/REVIEW-astra-r2.md)
