# Identity review cards — the one mint authority, the dismissal ledger, and every door into the one continuation

Re-enumerated from the live tree at `cd506d15` (2026-08-26), after fix-wave 1a
(U1 `c71af24a`, U2 `44895d7b`, U7 `43c2554b`, U5 `cd506d15`). The previous
enumeration was at `8353c0bc` and is superseded — all four commits changed these
exact files. Read this before writing any spec, packet or test that says "every
card", "every door", "manual import attaches", or "dismissed".

**Amended 2026-08-31 after the deletion pass** (identity-conflict-authority,
buckets A–E): the legacy conflict endpoints, handlers, service and page are gone;
the legacy `WorkIdentityRepository` trait shrank from 33 methods to 14 (every
zero-caller method deleted with its SqliteDb impl); the pre-rewrite settle road,
merge chain and startup backfill are deleted. Line anchors below were re-read
against that tree. Open `work_identity_conflicts` rows are untouched — B4 adopts
them.

Two review rounds once failed because this census was sampled instead of
enumerated. **Code is authoritative; re-enumerate by walking the tree, not by
searching an index** (the code-index was ~300 lines stale in `identity_layer.rs`
the day the first census was written). **Re-enumerate after any change to these
files.**

## One authority mints every runtime card (REQ-007, U7)

A tree walk for `INSERT INTO identity_review_cards` now finds **exactly two
statements**, down from seven at six sites:

| # | Statement | Role |
|---|---|---|
| 1 | `crates/livrarr-db/src/identity_layer.rs:488`, inside `mint_reuse_or_suppress_review_card_in_tx` (defined `:417`) | **THE runtime authority.** Every production mint goes through here. |
| 2 | `crates/livrarr-db/src/identity_layer.rs:6163`, inside `stage_legacy_identity_rows` (defined `:6043`) | Pre-activation cutover staging only. Excluded from the authority **by design** — wave-3 territory. |

The helper owns, in one transaction: the reuse keys (PendingRoute owner-free
trimmed on the durable Work; GroupIdentity insight-92 tuple; EditionEvidence
user+edition), oldest-wins duplicate cleanup, the dismissal-suppression check,
and the notification decision — exactly one `identityReviewNeeded`, emitted in
the mint transaction, and only for a Minted still-pending non-inline
PendingRoute. A reuse emits none.

**Its seven callers** — this is the complete production mint surface:

| Caller | File:line | What it is |
|---|---|---|
| `commit_settlement_in_tx` | `identity_layer.rs:602` and `:919` | generic settlement (two arms) |
| `commit_pending_route_review` | `identity_layer.rs:1570` | captured-route handoff / search fallback |
| `commit_pending_route_review_with_review_context` | `identity_layer.rs:1612` | same, review-context variant |
| `apply_evidence` | `identity_layer.rs:2272` | edition writer |
| `apply_work_evidence` | `identity_layer.rs:2414` | edition writer (the live manual-import door reaches this one) |
| `heal_identity_title_policy` | `pool.rs:1125` | boot-time title heal |

PendingRoute keys resolve placeholder `work_id 0` to the allocated Work through
one shared scope function, so a real-id replay reuses its card and a second new
Work cannot steal one.

`ImportIdentity` no longer has any writer: `commit_unattached_import_review` is
**deleted** (0 hits in the tree). The variant survives only as a type — including
in the U1 refusal arm at `services.rs:607`.

No production producer: IdentityConflict, FieldResolution, ContributorOrder,
MigrationRepair, InvariantRepair, ImportIdentity.

## Seven of nine kinds now refuse the continuation by name (REQ-001, U1)

Before U1, seven of nine kinds accepted a user's decision, performed nothing,
then wrote a `resolved` audit and bumped the generation. One wildcard-free
`require_continuation()` (`crates/livrarr-domain/src/identity_layer/services.rs:600`)
now gates the single continuation decision point **before any validation or
write**:

- **PendingRoute** and **GroupIdentity** proceed unchanged — including
  GroupIdentity's known wave-B defects below.
- The other seven refuse with `IdentityRoadError::ContinuationUnavailable { kind }`,
  which names the kind.

The refusal is mapped explicitly to **HTTP 409** at every door
(`handlers/identity_layer.rs:137`, `handlers/work.rs:589`) and by name to the cutover CLI's
`ContinuationUnavailable` (stderr, exit 2) — never the database error class. The
gate is called at `identity_road.rs:791` and mirrored in the behavioral stub
(`stubs.rs:1101`), so the test road cannot drift from production guard ordering.

The legacy conflict endpoints (`/identity-conflict/*`), their handlers, the
`LiveIdentityConflictService`, the page SQL and the Conflicts section of the
review page were **deleted** in the deletion pass (2026-08-31, bucket D). Until
the card-based conflict authority (B4) lands there is no conflict door; the
legacy `work_identity_conflicts` rows stay in place for it to adopt.

## Every door into the one continuation (`resolve_review`)

Six (eight before the deletion pass removed the two conflict endpoints), by LSP
references plus the CLI's direct call. Doors 1–2 and 6 hit the refusal gate for
the seven unbuilt kinds; doors 3–5 submit GroupIdentity / PendingRoute for a card
minted in the same request.

1. `/identity-review-card/{id}/resolve` — typed (`handlers/identity_layer.rs`)
2. `/identity-review/{work_id}/resolve` — legacy alias, same handler
3. Work title/author update (`handlers/work.rs` `update`) — mints a GroupIdentity
   card through `WorkUpdateRekey` and resolves it `DifferentFromAll` in the same
   request
4. merge with choices (`merge`) — `ManualWorkMerge`, resolved `AttachOrMerge` inline
5. affirm pending anchor (`affirm_pending_anchor`) — `AffirmPendingRoute`, resolved
   `Affirm` inline
6. cutover CLI `identity-cutover resolve` (`livrarr-server/src/identity_layer.rs`)

The former `/identity-conflict/{id}/resolve|dismiss` doors loaded a typed
`IdentityConflict` card nothing ever minted; they are gone, and the one conflict
authority is this feature's B-phase (IR v2).

A notification rule keyed on "minted" would alert on already-resolved cards from
doors 3–5 — which is why the U7 rule excludes inline PendingRoute.

## Dismiss is a standing decision, not a card state (REQ-005, U5)

A user's Dismiss of a **keyed** question (GroupIdentity, PendingRoute,
EditionEvidence) is durable. In one transaction, `dismiss_pending_review`
(`identity_layer.rs:1694`):

1. cancels the selected card **and every equivalent pending sibling** (`:1751`),
2. writes one `ReviewActor` audit naming all of them,
3. upserts an active **version-1** row in the dismissal ledger (`:1786`,
   migration 086), keyed by one canonical `ReviewDismissalKeyV1` shared
   **verbatim** with U7's pending-reuse and duplicate-cleanup paths.

One canonical key for reuse, cleanup and suppression is the load-bearing part: a
second key definition would let a dismissed question come back under a different
name.

### Suppression is decided once, before any settlement mutation

The mint helper computes its disposition in a **preflight pass** and the
materialization step reuses it — no second ledger read. So an equivalent machine
proposal is refused while **unrelated evidence in the same settlement still
commits**. Per producer:

| Producer | Behaviour when suppressed |
|---|---|
| Generic settlement | defers with "standing dismissal" |
| Captured-route handoff | skips **per proposal**, still returns a surviving changed-key sibling; `None` only when every candidate is suppressed |
| Startup title heal | no-ops, marker left unset |
| Both edition writers | return the complete existing Edition aggregate as **normal success** — the live manual-import caller still imports the file |
| Retry-All | decides each work's final state **after** its identity handoff resolves, so a suppressed chase leaves the book incomplete and counted as such, not reported recovered |

**Intent, not door, controls bypass.** DirectAdd, explicit manual-import and
validated list choices, and the three inline actions mint exactly as before.
ListImport no longer synthesises an explicit choice for anchorless rows.

### Exactly two doors revoke — each inside its own action's transaction

1. `sqlite_work_identity.rs:1587` in `apply_identity_edit_in_tx` — a successful
   **certified identity edit** (same-value included)
2. `identity_layer.rs:2202` in `commit_review_continuation` — a successful
   **PendingRoute Affirm** resolved through its continuation

EditionEvidence has **no** revocation in this wave. Machine cancellations never
write the ledger.

### One-time adoption of historical dismissals

`adopt_identity_review_dismissals` (`pool.rs:518`), marker
`identity_review_dismissal_adoption_v1`, adopts pre-ledger user dismissals so an
upgrade keeps prior decisions: latest equivalent wins, only a later user Affirm
blocks, malformed rows are logged and skipped, idempotent, with rollback proven
at the pre-commit failpoint (`identity_layer.rs:6482`).

## Cancellation is five sites; only one is the user

`UPDATE identity_review_cards SET status='cancelled'` appears at five sites
(four before U7 added duplicate cleanup):

| Site | Actor |
|---|---|
| `dismiss_pending_review` (`identity_layer.rs:1751`) | **the user** — the only one whose `review-dismissal` audit carries a `ReviewActor` JSON actor, and the only one that writes the ledger |
| `cancel_satisfied_pending_route_cards` (`:3309`) | `identity-engine` |
| `cancel_equivalent_duplicate_review_cards` (`:4370`) | U7 oldest-wins duplicate cleanup (distinct non-`ReviewActor` audit) |
| `cancel_pending_group_card` (`:5718`) | machine |
| `heal_identity_sweep_findings` (`pool.rs:1452`) | `identity-sweep-heal` |

"Cancelled" therefore **never** implies a user decision — check the ledger or the
audit actor. Nothing in the tree sets a card back to `pending`. A card's
`(user_id, work_id)` FK is `ON DELETE CASCADE` with `foreign_keys=ON`, so a card
whose own Work is deleted vanishes with it.

## The GroupIdentity continuation is still known-broken until wave B

Unchanged by this wave, and deliberately so — U1 lets GroupIdentity **proceed**
rather than refuse, because refusing it would break working doors.

`DifferentFromAll` with a parked proposal runs
`UPDATE works SET title=…, author_id=… WHERE id = <the card's work>` — it
overwrites the established work with the proposal. `AttachOrMerge` absorbs with
no archive (`identity_route_archives` still has no writer). Any change that
routes a NEW flow into a GroupIdentity card hands the user buttons that corrupt
identity. **A bounded fix must defer such cases, not park them.** Fixing the two
buttons is wave-1's second half, alongside the one conflict authority.

## Manual import never parks, and the coordinator is atomic (REQ-003, U2)

A minimum-only manual import (title + author + owned file, no provider key) no
longer parks a dead unattached `ImportIdentity` card. **One repository
`BEGIN IMMEDIATE` transaction** owns hint revalidation, Author resolution, the
complete-group read, the one five-rule decision, and every Author/Work write —
or none of them. A defer rolls back and surfaces the exact recovery message.

The ordinary road keeps a minimum-only backstop: Review, or a 2+-member
AutoMerge, becomes Deferred **before** commit. Provider-bearing items are
byte-identical (pinned).

The shared complete-group evaluator, certainty predicate, Author predicates and
defer formatter live in `livrarr-domain::identity_layer::reconciliation` — the
road and the coordinator call **the same functions**, never two copies.

**What the door feeds the matcher** (unchanged): `find_existing_work` →
`work_dedup::find_matching_work` with `ProviderKeys { ol_key, ..Default }` — the
item's ISBN / ASIN / GR / HC keys never reach the matcher (no HC slot exists), so
only the OL-key arm and the text tier can fire. The work list is loaded ONCE per
request, so item N cannot dedup-attach to a work item 1 created — it reaches the
road and AutoMerges onto it.

A one-sided subtitle tail is grey at the dedup tier ONLY (never absorbed there —
`identity_absorb_match` compares the raw incoming string against `Work.title`); at
the road's group reconciliation it is text-**certain**, because `candidate_core`
splits the incoming title into its identity tuple and `evaluate_match` compares
tuple mains only. So a lone sibling is ATTACHED and a sibling inside a 2+ cohort
is ABSORBED (PO ruling 2026-08-24; spec v11 ST-011). **An earlier version of this
page claimed the road "parks" it — that was wrong.**

## The import screen now has a title/author editor

**Corrected 2026-08-26 — the previous "there is no on-screen way to type a
title/author" claim is obsolete.** U2 added the "Edit title and author" recovery
control. The saved override submits minimum-only fields and **omits every
match-derived key**, so an edited row enters as a minimum-only item rather than
inheriting a stale provider identity.

The parsed row's title text remains a button that opens the OL search;
`correctedMatch` from a search-select still submits the candidate's provider ids
(a provider-identity item — a different door-matrix cell).

## The resolve-request `notes` field is gone

Removed end to end by U7: HTTP struct field and TS type deleted, old clients
still accepted, the value persists nowhere, and the cutover CLI rejects any
nested `notes` key **recursively** before action decoding. The legacy conflict
store's page SQL (the only other `notes` handling) went with the page on
2026-08-31.

## `user_confirmed=1` is NOT identity-edit-only, and `observed_at` refreshes

Three writers stamp `user_confirmed` routes: the certified identity edit
(`sqlite_work_identity.rs` → `sync_identity_edit_route:1206`), PendingRoute
Affirm, and any DirectAdd/ListImport settlement whose bundle carries a
`user_choice` (`identity_road.rs:958-989`). The settlement route upsert preserves
the bit and UserChoice/OwnedFile provenance but **REPLACES `observed_at` on every
machine re-observation** — so "a user-confirmed route observed after time T"
proves nothing about user activity after T.

The identity edit writes no `identity_audit_events` row; its durable trace is the
route row + generation bump only. Its HTTP handlers (preview/commit/clear) are
NOT in the router (IA-C15) — the production entry is
`WorkService::commit_identity_edit`.

## Settlement and continuation each claim a generation — by design

Verified 2026-08-26 against the tree, after a contest judgment mis-framed it as
an atomicity hole:

An update is **one settle plus one continuation**, and each claims a generation
(`tests/behavioral/test_irf_u1_refusal.rs:1658`: the continuation's
`expected_generation == generation + 1`, end state `generation + 2`; same pin at
`:1732`, `:1806`, and `test_ilr_contracts.rs:8966`).

So when a continuation **fails**, step one's committed generation claim
correctly survives — it is pinned as intended behaviour at
`test_irf_u5_durable_dismissal.rs:4878-4882`. A continuation that expects
`generation + 1` **requires** the settlement's bump to be already committed and
visible; deferring it would jam the CAS check. This is insight 91's decision-time
rule applied to the two-step road. Do not "fix" it.

## The road-service trait is four methods; the door cannot reach the impl

`IdentityRoadService` = `settle` / `resolve_review` /
`settle_manual_import_minimum` / `apply_captured_route_handoff`
(`identity_layer/services.rs:347-377`; corrected 2026-08-27 — an earlier version
of this page said three methods and a private predicate). `reconcile_complete_group`
is an inherent method on the impl in livrarr-metadata (`identity_road.rs:385`);
`authority_certain` is `pub fn` in domain (`identity_layer/reconciliation.rs:71-81`)
but the complete-group reconciliation still runs only on the impl — a handler
(compile wall: domain, http, matching, jobs only) cannot reach it. The review
continuation `resolve_review` runs on that same impl, so the group rule is
reachable where the decision is made. Any door-side decision that needs the group
rule needs a **trait surface**, never a re-implementation.
