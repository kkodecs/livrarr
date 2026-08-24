---
feature: "identity-review-fixes"
stage: spec
status: draft
version: 9
req_ids: [REQ-001, REQ-003, REQ-005, REQ-007]
---

# Spec: identity-review-fixes (audit fix-wave 1a — the review surface, bounded half)

v9 after round 8 (`review-spec-xai-r8.md`, FAIL p1=1 — both r7 findings
RESOLVED; the one finding was a leftover of the r7 fold itself: rules 4–5 of
the decision list lacked the "with no valid hint" gate rules 2–3 gained, so
the unchanged audited-distinction acceptance case contradicted the exclusive
hint-first rule; folded per the reviewer's suggested fix). v8 was v7 plus
the round-7 fold; v8 after the round-7 review of v7 (`build/reviews/identity-review-fixes/review-spec-xai-r7.md`,
FAIL p1=1 p3=1 — every round-6 finding dispositioned RESOLVED; the one P1 was
an internal collision between REQ-003's transaction recipe and its per-item
decision list, resolved below by making the list the ONE decision;
`review-spec-anthropic-r7.md` PASS records the contest judgment). v7 was the
PO-directed authoring contest's openai candidate plus two declared mechanical
fixes; both contest candidates and CHANGES files sit beside the reviews.

Bug Spec (bugfix lane). v7 is a complete revision after the round-6
cross-family review of v6
(`build/reviews/identity-review-fixes/review-spec-xai-r6.md`, FAIL
p1=2 p2=1; `build/reviews/identity-review-fixes/review-spec-openai-r6.md`,
FAIL p1=5 p2=3). It closes
the round's connected defects: the manual-import defer decision was
neither callable from the handler nor atomic with Author/Work writes; its
recovery was unavailable on the screen that displayed it; historical
`user_confirmed` routes could not date a certified edit; suppression
classified intent from the origin instead of the actual choice; and
`EditionEvidence` exposed a recurring Dismiss despite already having
a semantic key. It also pins every conflict-page action, every suppression
origin, and exact singular/plural defer copy.

This feature is the BOUNDED half of fix-wave 1 (PO split call 2026-08-23):
REQ-001, REQ-003, REQ-005, REQ-007. v2's REQ-002, REQ-004, REQ-006 and
REQ-008 left this feature for a design-first wave-B feature (§ 4). Their IDs
are retired here, not renumbered, so the r1-r6 findings and the unit names
(U1/U2/U5/U7) keep their meaning.

Every `file:line` in § 0b was read from the tree at
`8353c0bca1b52aab6f58e626e8d7e49959cc3b7c`
(main) for this revision (1-based lines; every complete set was enumerated
from the live tree by its storage target or symbol, never from an index).

Source of every defect: the identity lifecycle audit
(`docs/identity-lifecycle-audit-2026-08.md`, master ledger
`build/audit-identity-lifecycle/FINDINGS.md`; lane evidence in
`build/audit-identity-lifecycle/FINDINGS-C.md` /
`build/audit-identity-lifecycle/FINDINGS-F.md`), PO-blessed fix-wave 1
on 2026-08-22.

Units run in the order U1 → U2 → U7 → U5: U1 (refusal) first because every
later unit's tests assert through the same continuation; U2 (manual import)
next because it deletes a mint site that U7's helper would otherwise have to
cover; U7 (the one mint/reuse/suppress helper) before U5 so the dismissal-key
check of U5 lands inside that helper, not beside it. Seating (PO 2026-08-21):
grok authors each unit in its own worktree against a dense contract packet;
codex reviews with the census-grade instruction; the PM authors each unit's
red reproduction test (bugfix carve-out) and runs every gate independently.

## 0a. Design Principles

- **User intent is final — and nothing happens without it.** An accepted
  review command either performs exactly what it says or is refused before
  any write. A user Dismiss is durable against an equivalent machine
  proposal for every runtime-produced kind that can recur in this wave:
  `GroupIdentity`, `PendingRoute`, and
  `EditionEvidence`. The same tombstone never blocks a real explicit
  identity choice or an inline user continuation.
- **Intent is evidence, not a door label.** The governing REQ-018 definition
  controls: a user choice is an explicit identity-candidate pick or an
  explicit manual title+author creation, not a click on Add/Review. In
  particular, `ListImport` is conditional; its automatic/row-confirm
  path remains suppressible, while a request carrying a genuine choice
  bypasses the tombstone.
- **Uncertainty is visible, actionable, and safe.** A card kind whose
  continuation is not built is refused and labelled with its owner. A
  notification is a call to act, so this wave alerts only on
  `PendingRoute`; it does not invite users into the known-broken
  `GroupIdentity` continuation or a refused
  `EditionEvidence` continuation.
- **A human-watching door flags instead of parking when the card would
  corrupt.** Until wave B fixes `GroupIdentity` and its absorption
  archive, minimum-only manual import defers the grey and multi-member cases:
  the screen names the existing book(s), gives an operable recovery, creates
  no review card, and leaves no state except the ordinary failure-history
  event (Q-008).
- **A decision not to write is serialized before the first write.** U2
  reserves SQLite's one writer before its authoritative Author/group read and
  holds that transaction through Author and Work settlement. U5's
  suppression decision is made inside the transaction that would mint/apply
  the proposal. A read-only preview followed by independent writes is not a
  contract.
- **One authority per concern.** One review continuation; one shared,
  exhaustive continuation-availability function; one identity road; one
  canonical complete-group evaluator; one runtime mint/reuse/suppress helper;
  one versioned dismissal key per kind. The governing door matrix is
  conformed to, never amended.
- **Fail closed; smallest coherent change per unit.** Refusal is shippable.
  Kinds whose producer and continuation cannot both land remain refused with
  a named owner. Wave A neither repairs the `GroupIdentity` actions
  nor routes/alerts a new flow onto them.
- **No wire-contract change in this wave.** All HTTP request/response shapes
  and CLI flag/file shapes are unchanged except REQ-007's removal of
  `notes`. U2's title/author editor only changes which already-present
  `ImportItem` values the frontend sends.

## 0b. System Truths

Internal binding contracts this wave depends on (no external provider,
indexer, or download client is changed):

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | `spec-identity-layer-rewrite.md:506-542` (REQ-018, PO-approved) | Six doors share one road. Manual import has user choice and owned-file evidence `Always`, provider evidence and minimum-only `Conditional`. List import's user choice is conditional. “User choice” is an explicit candidate pick or explicit manual title+author creation, never merely Add/Review. A minimum-only machine door may create or defer; a human-watching surface may flag where a machine door decides. | Parking manual import as unattached `ImportIdentity`; calling dedup a user choice; suppressing every ListImport solely because of its origin; amending the matrix. | high |
| ST-002 | `wiki/architecture/roads.md:154-205` | R6 alone materializes LibraryItems: manual = Copy + unconditional retag → CWA → email; Readarr = HardlinkFirst with no import post-steps; scan = AdoptInPlace. | Replaying a parked Readarr item through manual policy; creating a LibraryItem outside R6. | high |
| ST-003 | `crates/livrarr-domain/src/identity_layer/services.rs:231-246`, `crates/livrarr-handlers/src/identity_layer.rs:28-39` | A `PendingReviewCard` payload is serialized to HTTP clients. | Putting a path, checksum, or other private file datum in a card. | high |
| ST-004 | `crates/livrarr-server/src/identity_layer.rs:766-872`, `crates/livrarr-domain/src/identity_layer/review.rs:14-121`, `crates/livrarr-domain/src/identity_layer/contributor.rs:14-24` | The CLI reads one JSON value. Ordinary actions use externally-tagged enum shapes; a second top-level key is invalid, while unknown fields inside a struct payload are ignored. `ContributorOrder` is the whole one-key command; unknowns in its command payload and object-valued `partition` entries are ignored, while `order` elements and `primary` are strings and objects there are invalid. | A blanket “unknowns ignored” claim; changing any non-`notes` accept/reject result. | high |
| ST-005 | `crates/livrarr-handlers/src/manual_import.rs:850-854`, `crates/livrarr-handlers/src/middleware.rs:4-24`, `crates/livrarr-handlers/src/identity_layer.rs:60-66` | Manual import requires `RequireAdmin`; ordinary card resolution takes `AuthContext`. | A review continuation that reads/materializes a file without re-authorizing import capability. | high |
| ST-006 | `crates/livrarr-handlers/src/manual_import.rs:222-274,850-977,983-1152,1154-1292,1319-1374`, `frontend/src/pages/manual-import/ManualImportPage.tsx:221-250,297-316,661-683` | Manual import is one admin batch with per-item results and request-level 200 for a well-formed request. The Work snapshot is loaded once. The request carries title/author but no existing-work id. Any book provider id makes the item provider-evidence, not minimum-only. A road defer has a per-item error channel. | A per-item whole-request 4xx; “pick the in-library result” as an existing-work choice; a new request field. | high |
| ST-007 | Every production `INSERT INTO identity_review_cards`, enumerated from every live `.rs`/`.sql` file under `crates/`: `crates/livrarr-db/src/identity_layer.rs:834,906,990,1792,1928,5402` and `crates/livrarr-db/src/pool.rs:1407`. Sites: generic settlement `crates/livrarr-db/src/identity_layer.rs:801-857`; captured-route handoff `crates/livrarr-db/src/identity_layer.rs:937-1051`; unattached manual import `crates/livrarr-db/src/identity_layer.rs:871-935`; both edition writers `crates/livrarr-db/src/identity_layer.rs:1759-1996` (live `apply_work_evidence` adapter at `crates/livrarr-server/src/state.rs:293-318`; no live reference to repository `apply_evidence`); startup title heal `crates/livrarr-db/src/pool.rs:1083-1546`; cutover staging `crates/livrarr-db/src/identity_layer.rs:5282-5446`. | Production mints exactly four kinds: `PendingRoute` (sites 1/2), `GroupIdentity` (1/5/6), `EditionEvidence` (4), and `ImportIdentity` (3, deleted by U2). Runtime sites after U2 are 1, 2, 4, 5; site 6 is pre-activation cutover. `IdentityConflict`, `FieldResolution`, `ContributorOrder`, `MigrationRepair`, `InvariantRepair` have no production producer. | Calling a settle-only helper a complete census; keying EditionEvidence on generation; testing an unproduced kind or callerless operation without naming a compatibility fixture. | high |
| ST-008 | `crates/livrarr-domain/src/identity_layer/services.rs:38-64`, `crates/livrarr-server/src/identity_layer.rs:375-393,909-927`, `crates/livrarr-server/src/main.rs:115-130`, `crates/livrarr-handlers/src/identity_layer.rs:116-139` | Neither error enum has a “continuation unavailable” case. Unnamed road errors collapse to CLI `Database`; CLI errors print on stderr and exit 2. HTTP already distinguishes 409 review/probe from 400 mismatch. | Reusing an existing error variant; giving the CLI an HTTP status; allowing the new road error to collapse to Database. | high |
| ST-009 | Every external `resolve_review` ingress, enumerated from the symbol's live references: typed route + legacy alias (`crates/livrarr-handlers/src/identity_layer.rs:60-114`), conflict resolve/dismiss (`crates/livrarr-handlers/src/identity_conflicts.rs:128-242`), inline update/merge/affirm (`crates/livrarr-handlers/src/work.rs:762-899,1066-1178,1604-1781`, calls at `crates/livrarr-handlers/src/work.rs:860,1125,1749`), and cutover CLI (`crates/livrarr-server/src/identity_layer.rs:766-872`). The other symbol references are recorder/enum pass-throughs, not doors (`crates/livrarr-server/src/identity_layer.rs:90-174`). Legacy list/detail read `work_identity_conflicts` (`crates/livrarr-handlers/src/identity_conflicts.rs:64-126`); actions load typed cards (`crates/livrarr-db/src/identity_layer.rs:1173-1218`). | Seven HTTP doors plus CLI reach one continuation. The conflict page lists a store its actions do not load; all listed rows 404 today. The three inline doors submit only `GroupIdentity`/`PendingRoute` cards minted in the same request. | Treating conflict or inline handlers as separate continuations; leaving the conflict mapper wildcard; calling a fixture-only 409 the real-page fix. | high |
| ST-010 | `crates/livrarr-metadata/src/identity_road.rs:830-943,777-827`, `crates/livrarr-domain/src/identity_layer/services.rs:532-539`, `crates/livrarr-handlers/src/manual_import.rs:1175-1235` | Road validation requires HumanWatching + choice + owned file for manual creation; candidate title parsing rejects an empty parsed main, including nonblank punctuation/marker-only input. The current door writes the Author first. | “Nonblank” as the title rule; relying on road validation after Author writes. | high |
| ST-011 | `crates/livrarr-handlers/src/manual_import.rs:1154-1173`, `crates/livrarr-matching/src/work_dedup.rs:48-55,80-144`, `crates/livrarr-metadata/src/identity_road.rs:243-305,389-479,1087-1098`, `crates/livrarr-db/src/identity_layer.rs:2550` | For a minimum-only item, handler dedup gets only title+author against the request-start Work snapshot. Exact text may attach; one-sided subtitle is grey. The live road group read can create, attach one text-certain member, absorb 2+, or park Review. The unique identity-v2 index makes the 2+ text-certain common tuple unreachable in an activated production DB. | Describing ISBN/ASIN/GR/HC as live dedup inputs; duplicating the road's authority rule in the handler. | high |
| ST-012 | `crates/livrarr-db/src/identity_layer.rs:1315-1363,1438-1451,1522-1579`, `frontend/src/pages/review/ReviewPage.tsx:216-352` | `GroupIdentity::DifferentFromAll` overwrites the card's established Work with the proposal; `AttachOrMerge` absorbs without an archive. The UI exposes those two actions. | Calling the continuation correct; routing/alerting a new flow onto it; fixing AUD-P0-2/3 in wave A. | high |
| ST-013 | All four `status='cancelled'` writers: user dismiss (`crates/livrarr-db/src/identity_layer.rs:1116-1170`), satisfied pending route (`crates/livrarr-db/src/identity_layer.rs:2732-2750`), dedup-residue heal (`crates/livrarr-db/src/identity_layer.rs:4948-4975`), sweep heal (`crates/livrarr-db/src/pool.rs:1729-1745`). Card FK/cascade: `crates/livrarr-db/migrations/082_identity_layer_foundation.sql:183-194`. | Only user dismiss has a `review-dismissal` whose actor parses as `ReviewActor`. Machine cancellations must not create tombstones. Nothing reopens a card. Deleting the card's own Work cascades it; deleting a non-anchor cohort member can leave a multi-work GroupIdentity card. | Treating every cancelled row as user intent; adopting a machine cancel; a “re-open card” action. | high |
| ST-014 | `frontend/src/api/client.ts:33-75`, `frontend/src/pages/review/ReviewPage.tsx:153-174` | The client preserves the server message; `ConflictCard` discards it for fixed Resolve/Dismiss toasts. | Backend-only conflict refusal; a new error-body shape. | high |
| ST-015 | `crates/livrarr-metadata/src/author_service.rs:75-182,554-598`, `crates/livrarr-db/src/sqlite_author.rs:418-434,614-626` | `AuthorService::add` can update/adopt an Author, enqueue linking, attach a user route, or create Author + first name variant + link task. Exact-name and unambiguous-match discovery are read-only before those calls. | Permitting “the Author row” as residue; invoking public `add` before a defer decision. | high |
| ST-016 | `crates/livrarr-metadata/src/identity_road.rs:310-387,587-684`, `crates/livrarr-domain/src/identity_layer/door.rs:14-38,108-149`; machine callers `crates/livrarr-metadata/src/author_monitor_workflow.rs:671-690`, `crates/livrarr-metadata/src/list_service.rs:328-335`, `crates/livrarr-metadata/src/series_query_service/service.rs:1135-1177` | A card changes settlement: emptying the card list would apply the incoming proposal and report Settled. Suppression needs a pre-commit disposition. Machine creation callers already treat non-Settled as not added; captured-route proposals can yield no proposal. Inline origins require their card. | Modelling suppression as “no cards” inside a normal commit; returning Settled; suppressing inline/user origins. | high |
| ST-017 | `crates/livrarr-db/src/pool.rs:6-46`, `crates/livrarr-domain/src/identity_layer/services.rs:321-381`, `crates/livrarr-handlers/src/manual_import.rs:13-49`, `crates/livrarr-handlers/Cargo.toml:6-31`, `crates/livrarr-metadata/src/identity_road.rs:389-479,1087-1098` | SQLite has one write-begin authority, `BEGIN IMMEDIATE`, which queues competing writers before any read-to-write upgrade. The road trait currently exposes only settle/resolve/handoff. Complete-group reconcile is an impl method and its certainty predicate is private to metadata; the handler compile wall cannot call metadata. | A handler-side copy of the group rule; citing an impl's `pub` method as a callable handler seam; a read-only preview outside the write transaction. | high |
| ST-018 | `frontend/src/pages/manual-import/ManualImportPage.tsx:14-19,221-250,297-316,526-659` | The screen has no title/author editor. Search-select installs a provider match; `handleImport` then sends that match's provider ids. The parsed title is a Search button, not an input. | Copy that says “type the title” without adding a real control; treating search-select as a minimum-only retry. | high |
| ST-019 | `crates/livrarr-db/src/sqlite_work_identity.rs:1953-2104,2153-2339`, `crates/livrarr-db/src/identity_layer.rs:1458-1483,1522-1579,2790-2839`, `crates/livrarr-metadata/src/identity_road.rs:957-990` | `user_confirmed=1` has FOUR production writers: certified edit, PendingRoute Affirm, DirectAdd/ListImport routes with true `user_choice`, and merge coalescing, which copies the bit onto the winner's route with `MAX(user_confirmed, ?1)` and `MergeCoalesced` provenance (`crates/livrarr-db/src/identity_layer.rs:3077-3084`) without any user-action time. Ordinary route upsert preserves that bit/provenance but replaces `observed_at` on later machine observations. No certified-edit-specific audit exists; the generic review audit identifies PendingRoute Affirm by its serialized command. | Using `user_confirmed` or `observed_at` to prove the time or kind of a historical user action; counting another review command as an Affirm. | high |
| ST-020 | `frontend/src/pages/review/ReviewPage.tsx:50-75,153-210`, `crates/livrarr-handlers/src/identity_conflicts.rs:44-55,128-196` | The conflict page has four Resolve actions. `AcceptSeparate` currently requires `winningWorkId`, but the page sends only `action`, so it fails before the road; the other three can reach the road if a typed card exists. | An AC that clicks only one unspecified Resolve; action-specific validation before the transitional kind refusal on an authorized open legacy row. | high |
| ST-021 | Closed origin enums at `crates/livrarr-domain/src/identity_layer/door.rs:14-38`. Live creation callers: DirectAdd `crates/livrarr-handlers/src/work.rs:329-348`, ManualImport `crates/livrarr-handlers/src/manual_import.rs:1254-1269`, ListImport `crates/livrarr-metadata/src/list_service.rs:315-327`, AuthorMonitor `crates/livrarr-metadata/src/author_monitor_workflow.rs:644-670`, SeriesMonitor `crates/livrarr-metadata/src/series_query_service/service.rs:1121-1134`, and ReadarrImport `crates/livrarr-server/src/readarr_import_workflow.rs:2592-2607,2674-2750`. Live continuation callers: EnrichmentPass `crates/livrarr-handlers/src/work.rs:50-90`, ManualRefresh `crates/livrarr-handlers/src/work.rs:1180-1203`, ConvergenceVisit `crates/livrarr-handlers/src/work.rs:1336-1368`. | Every suppressible `CreationDoor` and machine continuation has a named production observable. The no-caller statement in the r6 review applies only to the standalone `plan_readarr_identity` helper, not to the ReadarrImport road origin. The three inline origins are user actions and are covered separately. | Sampling only ListImport/AuthorMonitor/handoff/heal; using a compatibility-only test where the real Readarr workflow exists. | high |

## 0c. Prior Art

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | `spec-identity-layer-rewrite.md` REQ-018 (506-542) / REQ-019 (543-574) | Governs the door matrix. Manual import does not need an unattached card; ListImport intent is conditional; Readarr owns its import-review exits. |
| PA-002 | `spec-identity-layer-rewrite.md:593-616,698-724,764-770` | Promises future ContributorOrder, FieldResolution, and MigrationRepair producers/continuations; none is produced today. |
| PA-003 | `wiki/architecture/roads.md:109-132,154-205` | R4 is one continuation; R6 is one import road with per-door policy. |
| PA-004 | `wiki/insights.md` 80, 91, 92, 93, 99 | Decision-time generation; semantic pending GroupIdentity key; no-change machine observation is not settlement; real handoff/accounting rules. |
| PA-005 | Audit report + `build/audit-identity-lifecycle/FINDINGS.md:21-27,39,42,47,52` | AUD-P0-1/2/3, AUD-P1-7b/10/15/20 and IA-F04/05/08/09. Group continuation defects stay wave B. IA-F04 is equivalent MACHINE recurrence. |
| PA-006 | `crates/livrarr-domain/src/identity_layer/review.rs:69-121` | Existing commands retain one scalar generation claim; the per-work map remains wave B. |
| PA-007 | r1-r5 reports beside this candidate | v6 preserved the prior resolved findings: exact defer error without wrapper, no GroupIdentity notification, callable identity-edit service test, exact PendingRoute-Affirm historical parsing, ContributorOrder value shapes, and corrected inline/edit citations. |
| PA-008 | `build/reviews/identity-review-fixes/review-spec-openai-r6.md` and `build/reviews/identity-review-fixes/review-spec-xai-r6.md` | v7 closes OAI-R6-001..008 and XAI-R6-001..003: ST-017/U2 transaction, ST-018 editor, ST-019 migration policy, real choice classification, EditionEvidence durability, all conflict actions, plural grammar, and exhaustive origin ACs. |

## 1. Problem Statement

The review surface promises that the user's decision is final, but seven of
nine kinds currently accept a command, perform nothing, then fabricate a
resolved audit; cancelled cards can return; most cards do not notify; the
conflict page lists a legacy store while its buttons query a typed store and
hide errors; and one of its four Resolve buttons cannot even pass its own
handler validation. Manual import parks an unattached card that has no
continuation. Deleting that mint without a safe branch would instead expose
the known-corrupt GroupIdentity actions or absorb without archive.

The previous spec still allowed two ways for the same defects to survive:
manual defer was a read-only guess followed by separately committed Author
and road writes, and EditionEvidence's only visible action cancelled a row
without suppressing its immediate recurrence. It also promised a title retry
the production screen could not submit and treated mutable route timestamps
as historical proof. Wave A closes these concrete surfaces. Group merge
correctness, absorption archive, one conflict store, and multi-work generation
claims remain wave B.

## 2. Requirements

- **REQ-001** — Exhaustive continuation, fail closed (AUD-P0-1 / IA-C02,
  IA-F08; U1). One pure `require_continuation(kind)` function is the
  exhaustive, wildcard-free decision for all nine `ReviewKind`
  variants. `resolve_review` calls it after user-scoped existence
  loading and before action-specific validation or mutation. Adding a kind
  without deciding availability fails compilation.

  The available kinds remain exactly `PendingRoute` and
  `GroupIdentity`, byte-identical through every ingress including
  update/merge/affirm and their scalar claims. “Available” does not call the
  GroupIdentity actions correct: AUD-P0-2/3 remain pinned defects owned by
  wave B, and this feature creates no new route or notification to them.
  The other seven kinds refuse before any write with
  `IdentityRoadError::ContinuationUnavailable { kind }`. The CLI
  has the named peer
  `IdentityCutoverCommandError::ContinuationUnavailable(kind)` and
  maps by name, never through Database. Every HTTP mapper names the new
  variant as 409; proposal invalidation stays 409, mismatch/invalid action
  stays 400; the CLI emits the kind-bearing message on stderr and exits 2.

  Refusal is transitional and the owner label is fixed, not inferred:
  `IdentityConflict` → wave B; `EditionEvidence` → its post-wave-B
  continuation feature; `ImportIdentity` → the Readarr feature;
  `FieldResolution` → wave B's merge unit; and `ContributorOrder`,
  `MigrationRepair`, and `InvariantRepair` → the features that build their
  respective producers. The last three and FieldResolution have no runtime
  producer today (ST-007); EditionEvidence does, so U5/U7 make its visible
  Dismiss durable while its Resolve remains refused.

  The conflict adapter makes this true on the production page without
  inventing a typed row. For an authenticated id, each Resolve and Dismiss
  handler first loads either the same-user open legacy conflict served by
  list/detail or the same-user pending typed `IdentityConflict`
  card. Once existence/scope is proved, it calls the same exhaustive guard
  **before** translating Keep Existing, Treat as Separate, Use New Match, or
  Combine Both and therefore before checking `winningWorkId`,
  routes, or target edition. All five buttons receive the identical
  `IdentityConflict` 409 and no write. An unknown/foreign/closed id
  remains 404. `ConflictCard` shows the server's message for failed
  Resolve and Dismiss and performs none of its success invalidations/toasts.
  Generic `/identity-review-card/{id}/dismiss` remains a real user
  cancellation; U5 makes it durable for the three recurring keyed kinds.

- **REQ-003** — Manual import never parks, absorbs, or leaves defer residue
  (ST-001; AUD-P1-15 / IA-F03 producer half; U2; Q-008). Scope is a
  minimum-only item: explicit title+author, owned-file evidence, and none of
  `ol_key/gr_key/hc_key/isbn/asin`. Provider-bearing items remain
  byte-identical.

  **Callable, atomic seat.** Extend the domain
  `IdentityRoadService` with an internal (non-wire)
  `settle_manual_import_minimum(command: ManualImportMinimumCommand)`
  operation returning `Result<IdentityRoadOutcome, IdentityRoadError>`.
  The command carries user id, raw title/author, the already-read owned-file evidence,
  optional Author route discovered before the transaction, and the optional
  request-start dedup Work hint; it carries no path. The handler already
  depends on this trait and never imports metadata/db. For in-scope items it
  no longer invokes `AuthorService::add` or ordinary
  `settle`.

  The implementation delegates to one repository operation owning one
  `pool::begin_write` / `BEGIN IMMEDIATE` transaction. The complete-group
  evaluator, its authority-certainty predicate, and the pure Author-match
  predicates are extracted to a dependency-neutral
  `livrarr-domain::identity_layer` module; the repository supplies rows read
  under its transaction, and both this operation and ordinary road
  reconciliation invoke those same helpers. No handler reaches an inherent
  metadata implementation or duplicates either rule.
  Inside that transaction it executes the per-item decision list below as
  the ONE decision, in that order: (1) it revalidates the hinted exact-text
  attach — a still-valid hint attaches WITHOUT reading the group or
  invoking the evaluator; (2) only when no valid hint remains, it resolves
  the Author by exact name then the shared unambiguous matcher, reads the
  live complete group, and invokes the same extracted pure complete-group
  evaluator used by ordinary road reconciliation; then it either writes all
  successful Author/name/link-task/Author-route and Work settlement effects
  and commits, or returns Deferred/Rejected and rolls back. No nested transaction and no public AuthorService call occurs
  inside it. Competing writers queue before the authoritative reads; the
  decision cannot become stale before the writes. Pure title parsing and
  file read/stat/hash happen before the transaction so it remains DB-only.

  The ordinary `settle` path is a backstop: after removing the
  ManualImport unattached arm and
  `commit_unattached_import_review`, a minimum-only ManualImport
  request whose shared reconciliation is Review or AutoMerge with 2+ members
  returns Deferred before `commit_settlement`; it never parks or
  consumes an absorption list. This backstop does not replace the production
  coordinator's Author atomicity.

  **Per-item behavior and order.** Before the write transaction the title
  must parse to a non-empty identity main (blank, whitespace, punctuation-
  and marker-only fail), author must be nonblank, and owned-file evidence
  must be readable. Handler dedup still derives an optional hint from the
  request-start Work snapshot using only title+author. Under the reserved
  live transaction:

  1. A still-valid exact-text hint attaches and the file imports; the
     evaluator is not consulted. A stored exact-title twin therefore always
     attaches even when a grey sibling (e.g. the same main title with a
     subtitle) also exists in the library — the recovery § 3 promises
     depends on this rule.
  2. With no valid hint, an empty group or candidate text distinction creates
     the zero-route Work and imports.
  3. With no valid hint, a live group of EXACTLY ONE member whose pair with
     the candidate is text-certain attaches and imports, including item 2 of
     the same new book in one batch.
  4. With no valid hint, two or more text-certain members defer (no
     absorption).
  5. With no valid hint, an audited distinction in the group or any
     non-certain pair, including a one-sided subtitle tail, defers (no
     GroupIdentity card). A distinguished work reached by an exact-text hint
     attaches under rule 1 — the flag gates the evaluator's Review, never
     the hint (recovery § 3 promises depends on this too).

  A Deferred/Rejected/invalid item has `status=Failed` and leaves no
  Author create/update/adoption, name variant, link task, Author route, Work,
  review card, identity audit, generation, LibraryItem, or file write. Its
  sole persistent effect is the ordinary unattached operational
  failure-history event written after the transaction; it is not identity
  state. Batch 200/per-item independence remains unchanged.

  **Exact defer copy.** Group members are ordered by stored display title
  casefold, stored author casefold, then Work id (id is only a tie-breaker and
  is not displayed). For one member the per-item `error` is exactly:

  `Not imported: you already have "<title>" by <author>. Choose Edit title and author for this row. To add this file to that book, enter exactly "<title>" and "<author>", then retry. To add it as a different book, enter a different main title (a different subtitle alone is not enough).`

  For two or more members, form `<book-list>` by rendering every
  ordered member as `(N) "<stored title>" by <stored author>` with
  one-based `N`, joined by `; `. The per-item error is
  exactly:

  `Not imported: this title matches multiple existing books: <book-list>. Manual import cannot choose among them. Choose Edit title and author for this row and enter a different main title to create a different book (a different subtitle alone is not enough).`

  Neither string has `work creation failed:` or `conflict:`
  wrappers. The plural form deliberately promises no arbitrary target attach.

  **Operable screen recovery.** Each import row gains an “Edit title and
  author” frontend control, prefilled from the effective parsed/match values.
  Saving a manual edit requires both fields, marks a local manual override,
  clears the visible selected match/existing-work badge and prior failed
  result, and makes `handleImport` send those values while omitting
  every match-derived provider field. It uses the existing
  `ImportItem.title/author` and optional key fields; no wire shape
  changes. From a singular defer, entering the stored pair yields the
  guaranteed text attach; a different main title yields create. Provider
  search remains a separate provider-evidence action and is not described as
  this recovery.

- **REQ-005** — User dismissal is durable against equivalent machine
  proposals (AUD-P1-7b / IA-F04; U5). The keyed kinds in this wave are:
  `GroupIdentity` — (user, sorted cohort Work ids, normalized
  proposed main/subtitle/volume + primary Author, empty proposal for a heal
  card, sorted route provider/kind/value identities, sorted merge choices);
  `PendingRoute` — (user, Work, provider, kind, trimmed value),
  excluding route owner; `EditionEvidence` — (user, Edition id),
  independent of Work generation and evidence ids. Canonical JSON and a
  `key_version=1` are shared by pending reuse, Dismiss, historical
  adoption, suppression, and revocation.

  A new migration creates an internal dismissal ledger with one row per
  (user, kind, key_version, canonical key), latest dismissal timestamp/source
  card, nullable revoked timestamp/reason, and indexed Work-membership rows
  for GroupIdentity/PendingRoute revocation. Source card ids are informational,
  not foreign keys. For a keyed kind, the real user Dismiss transaction
  cancels the selected row and every same-user pending row with the same key,
  writes one ReviewActor dismissal audit naming the selected card and the
  equivalent ids it closed, and upserts/reactivates the ledger atomically.
  Unkeyed kinds still cancel only the selected row. The three current machine
  cancellation sites, and U7's duplicate cleanup below, write no ledger row.

  The U7 helper returns exactly `Minted`, `ReusedPending`,
  or `SuppressedByDismissal` inside the caller's write transaction.
  Suppression is decided before settlement mutation. A proposal is suppressible
  only when it contains no genuine explicit identity choice and comes from:

  | Proposal seat | Suppressed observable |
  |---|---|
  | ListImport automatic/row-confirm without an explicit identity-candidate pick, AuthorMonitor, SeriesMonitor, or ReadarrImport | Road `Deferred { reason: DeferReason("standing dismissal") }`; no Work/card/settlement audit/notification. List row is `add_failed`; AuthorMonitor reports `works_added=false, notifications_created=false`; SeriesMonitor neither creates nor links/counts; the live Readarr workflow records its “identity did not settle” per-book error, maps no Work, and imports no file. |
  | Generic settlement reached by `EnrichmentPass`, `ManualRefresh`, or `ConvergenceVisit` | `Deferred { reason: DeferReason("standing dismissal") }`; anchor title/routes/generation unchanged; no settlement audit/card/notification. Add-background reports its non-settled branch; manual refresh returns its ordinary refresh response without identity application; a real convergence chase records one card/miss pass and remains incomplete. |
  | Captured-route proposal handoff (site 2) | Suppressed proposals are skipped; if all are suppressed, `Ok(None)` (a proposal-less pass), no card/notification; a real chase still records card/miss once. |
  | Startup title-policy heal | No card/audit/notification; the collision remains blocked and the existing marker rule remains unset until the cohort changes or is otherwise repaired. |
  | `apply_evidence` / `apply_work_evidence` contradictory EditionEvidence | The contradictory proposal is ignored and the existing Edition is returned unchanged as the normal successful outcome; no card, evidence mutation, audit, notification, or import failure. |

  Intent, not `DoorKind`, controls bypass. Valid DirectAdd and
  minimum/manual candidate choices bypass. ListImport bypasses only when its
  validated `user_choice` is the REQ-018 explicit candidate pick;
  merely selecting/confirming a list row is not enough and producers must not
  synthesize `ExplicitCreate` solely because a row is anchorless.
  `WorkUpdateRekey`, `ManualWorkMerge`, and
  `AffirmPendingRoute` always bypass and mint/resolve exactly as
  before. A differing semantic key mints. Explicit actions bypass but do not
  generally erase the user's standing “do not ask the machine again” choice.

  **Future revocation is explicit.** After activation, a successful certified
  identity edit and a successful PendingRoute Affirm revoke every active
  GroupIdentity/PendingRoute ledger key containing that Work in the same
  transaction as the user action, whether or not the edit changes the key.
  A failed/stale action revokes nothing. No action in this wave revokes an
  EditionEvidence tombstone; its future continuation must define that policy.
  Title/author update, merge, resolution of another kind, DirectAdd, and
  ListImport choice are not revocations (they still bypass where applicable).
  No card is reopened; after revoke the next same-key machine proposal mints.

  **Upgrade adoption and the irrecoverable edit-timing policy.** One
  marker-gated activation pass examines cancelled GroupIdentity,
  PendingRoute, and EditionEvidence rows. It adopts a row only when a matching
  `review-dismissal` actor parses as `ReviewActor`, its
  payload/key parses, and every key Work/Edition still exists. Equivalent
  rows adopt only the latest user dismissal by `resolved_at`, then card id.
  Only a parsed `review-resolution` event later than that dismissal blocks
  adoption, and only when its actor is a ReviewActor and its command is
  PendingRoute `Affirm` on a Work in the key.

  The pass intentionally ignores `identity_routes.user_confirmed`
  and `observed_at`: ST-019 proves they cannot distinguish
  certified edit, explicit DirectAdd/ListImport, PendingRoute Affirm, or a
  later machine refresh. Therefore both “edit before dismissal, machine
  re-observation after” and the irrecoverable “dismissal then certified edit
  before this upgrade” adopt the dismissal. This conservative one-time policy
  favors the recorded Dismiss; explicit user doors still bypass it, and every
  post-upgrade certified edit has an exact transactional revoke. No spec
  claims exact reconstruction from unavailable history.

  Malformed actor/payload rows are skipped and logged without aborting;
  machine cancels adopt nothing; a missing member adopts nothing; no card row
  is rewritten/deleted. The pass is idempotent, and any failure leaves its
  marker unset for retry.

- **REQ-007** — Card hygiene (IA-F05 / IA-F09; U7; IA-F08 handled by
  REQ-001). Every runtime mint site after U2 (ST-007 sites 1, 2, 4, 5)
  invokes one transactional mint/reuse/suppress authority and passes the
  proposal's exact origin plus validated explicit-choice fact. Cutover staging
  site 6 is excluded: it runs pre-serve and is wave 3.

  Pending reuse keys are the REQ-005 keys: PendingRoute owner-free,
  GroupIdentity insight-92, EditionEvidence (user, edition) independent of
  generation. Concurrent equivalent proposals serialize to one card. If
  pre-U7 duplicate pending rows already exist, the helper reuses the oldest
  and machine-cancels later equivalents with a distinct duplicate-cleanup
  audit (never a ReviewActor dismissal and never a tombstone). A
  notification is emitted exactly once, in the mint transaction, only for a
  newly minted card still pending when the operation returns and safe to act
  on — in this wave exactly PendingRoute. No notification for reuse,
  suppression, GroupIdentity, EditionEvidence/refused kinds, or inline cards
  resolved in their minting request.

  A refused card remains visible and is labelled “not yet actionable —
  <kind>; handled by <owner feature>”; Resolve is absent and Dismiss remains.
  EditionEvidence adds “Dismiss prevents this evidence question from returning”
  because U5 makes that statement true for both producers. GroupIdentity keeps
  its existing silent list presence and actions; U7 does not newly advertise it.

  `notes` is removed from the handler and TypeScript conflict request
  types. Old HTTP clients may still send it; default unknown-field behavior
  ignores it and persists nothing. The cutover action-file parser rejects a
  `notes` key recursively at any object nesting with
  `InvalidActionFile` and no write. Every other unknown key keeps
  ST-004's location-specific result, including ContributorOrder string-valued
  `order`/`primary`.

## 3. UI/Interface Design

No request/response shape changes beyond REQ-007's removal of `notes`.

1. Typed refused cards show “not yet actionable — <kind>; handled by <owner
   feature>” and only Dismiss. EditionEvidence additionally says its Dismiss
   prevents the same evidence question returning.
2. Every conflict-page Resolve button and Dismiss renders the server's
   kind-bearing error message, not a fixed toast.
3. `ResolveIdentityConflictRequest` loses `notes`.
4. ManualImportPage adds an inline “Edit title and author” control with two
   existing-value inputs and Cancel/Save. A saved override visually replaces
   the provider match, clears its provider keys/existing-work badge and failed
   result, and is what the existing DTO submits. The singular and plural
   messages are exactly REQ-003's grammar. Search remains separately labelled
   as provider search, never “choose this existing Work.”

No mockup is required; these are additions to existing rows/cards.

## 4. Non-Requirements

- **Wave B** remains design-first and unopened here: v2 REQ-002
  (GroupIdentity + FieldResolution/parent-child continuation), REQ-004
  (versioned full-loser archive), REQ-006 (one conflict authority, legacy→typed
  mapping, all-door audited Reject, producer conflict flags), and REQ-008
  (per-work GenerationClaims + versioned CLI). It also owns an explicit
  manual-import existing-Work wire choice, replacement of U2's temporary grey/
  multi-member defer, and GroupIdentity notifications after its continuation
  is safe.
- No frozen-scalar rewiring (fix-wave 2), cutover staging change (wave 3),
  handoff/matching rewrite (wave 4), Readarr feature, AUD-P1-20 repair,
  identity-edit route registration, or GroupIdentity action/archive repair.
- `ImportIdentity` continuation remains Readarr-owned; after U2 no
  production producer exists.
- `EditionEvidence` continuation remains its own design feature
  because current cards carry empty `evidence_ids`. This wave defines
  only pending reuse, durable Dismiss, safe machine suppression, refusal/copy,
  and no notification. Its future feature defines a revocation.
- Unproduced ContributorOrder, FieldResolution, MigrationRepair, and
  InvariantRepair stay refused with their producer features.
- No exact reconstruction of a pre-upgrade certified-edit timestamp is
  claimed; ST-019 makes it unavailable. The deterministic adoption policy in
  REQ-005 is the complete migration contract.
- No new HTTP/CLI field, flag, or action shape; no legacy conflict migration;
  no card kind; no new file-materialization road.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | HTTP status for no-continuation kind | resolved | 409 with kind-bearing `ContinuationUnavailable`; new named road/CLI variants; CLI stderr + exit 2. |
| Q-002 | How ImportIdentity re-imports a private parked file | withdrawn | Manual import no longer parks; Readarr's feature owns its private state/capability design. |
| Q-003 | Legacy grey list/store registration or migration | deferred | Wave B owns one conflict authority and legacy→typed migration. |
| Q-004 | Persist or remove resolution `notes` | resolved | Remove from HTTP types (old field ignored); recursively reject in CLI action files; preserve every other unknown-key baseline. |
| Q-005 | Whole-batch 4xx or per-item invalid minimum | resolved | Existing batch 200 and per-item failure; authoritative title/file preflight before U2's write transaction. |
| Q-006 | Refuse IdentityConflict now or keep no-op resolution | resolved | Refuse on every real legacy/typed conflict action after scope/existence but before action-specific validation; show the server reason. |
| Q-007 | Notify every card or actionable cards only | resolved | Only a newly minted, still-pending safe kind; PendingRoute alone in wave A. |
| Q-008 | Manual grey/absorption after unattached-card removal | resolved | **PO 2026-08-23: DEFER.** Exact singular/plural message; no card/state, including all Author-side effects; only ordinary failure history. U2's transaction makes this atomic. |
| Q-009 | Adopt pre-upgrade dismissals and how to identify later revokes | resolved | Adopt user dismissals for all three keyed kinds. Only a later parsed user PendingRoute Affirm is historically unambiguous. Ignore mutable route timestamps; conservatively adopt the irrecoverable pre-upgrade certified-edit ordering. Future edit/Affirm revokes are transactional. |
| Q-010 | Non-`notes` CLI unknowns | resolved | Preserve each value-shape result exactly; objects cannot replace string AuthorRefs. |
| Q-011 | Which proposals does a tombstone suppress | resolved | Equivalent machine intent only. A genuine REQ-018 choice and inline origins bypass; ListImport is classified from validated choice, not origin/interaction/click. |
| Q-012 | Notify GroupIdentity in wave A | resolved | No; its actions remain AUD-P0-2/3. |
| Q-013 | Where does manual defer execute | resolved | New internal method on the already-injected road trait; one repository `BEGIN IMMEDIATE` transaction; shared pure group evaluator; ordinary road backstop. |
| Q-014 | How can the displayed import recovery be performed | resolved | Frontend-only row title/author editor using the existing DTO; manual override omits match-derived provider keys; AC starts from the displayed failure. |
| Q-015 | Is EditionEvidence Dismiss durable without its continuation | resolved | Yes, by (user, edition); both machine writers return the unchanged Edition on suppression; no revoke until that continuation's design. |
| Q-016 | Multi-member defer grammar/target | resolved | Stable numbered semicolon list of every book and no arbitrary attach promise; use the row editor with a different main title. |

## 6. Acceptance Criteria

- [ ] **AC-001 (REQ-001) — every ingress and every conflict action.**
  Through the real router/auth, each refused kind on a pending typed card
  returns 409 naming the kind through the typed card route and legacy alias;
  card remains
  pending, generation/audit unchanged, and the handler seam exposes
  `ContinuationUnavailable { kind }`. Unproduced kinds use a stated
  serialized compatibility fixture. CLI drives the same cards and returns
  the named CLI variant, kind-bearing stderr, exit 2, and no writes.

  Seed an authorized open legacy conflict through its production writer, list
  it through `GET /identity-conflict`, and independently click Keep
  Existing, Treat as Separate, Use New Match, Combine Both, and Dismiss on
  fresh copies in production `ConflictCard`. Each request returns
  the same captured 409 naming `IdentityConflict` even though Treat
  as Separate sends no `winningWorkId`; visible text contains the
  server message, row remains actionable/open, no typed card/audit/work
  invalidation/success toast. Repeat with a typed compatibility card.
  Unknown/foreign/closed ids are 404. Mapper tests pin proposal invalidation
  409 and mismatch 400.

  Update, merge-with-choices, and affirm each run through their registered
  router paths and are byte-identical in mutation/audit/response to pre-U1.
  Both GroupIdentity actions are pinned as known wave-B defects, not certified
  correct; PendingRoute is pinned unchanged.

- [ ] **AC-003 (REQ-003) — production door, transaction, UI, and boundary.**
  All cases use admin auth, real router, real SQLite, real U2 coordinator,
  real import workflow, and before/after snapshots of Authors, variants,
  link tasks, Author routes, Works, cards, identity audits, generations,
  LibraryItems, and source/target files.

  1. Valid minimum-only title+author+owned file creates a zero-route Work and
     imports with manual post-steps; no card.
  2. Mixed [valid, blank-title, valid, punctuation-only-title, valid,
     blank-author, valid-but-unreadable-file] returns 200 with
     Imported/Failed/Imported/Failed/Imported/Failed/Failed. Each failed item
     has no state delta except exactly one failure-history event; valid items
     commit regardless of position.
  3. Exact-text request-start dedup attaches; two files of one new book in one
     batch create one Work then the second live-group attaches; no card.
     Hint-first with a grey sibling: the library holds `Book` and
     `Book: Tail` by the same author (legal under `idx_works_identity_v2` —
     subtitle is in the key); a minimum-only import titled exactly `Book`
     attaches to `Book`, imports, writes no card, and emits NEITHER defer
     sentence.
  4. One-member grey subtitle case Deferred: `error` exactly equals REQ-003
     singular text (no wrapper), no Author exact-hit update/re-arm and, in a
     second case, no unambiguous-match adoption; existing Work row
     byte-identical. Audited distinction, split by hint: a distinguished
     `Book` (`text_distinction != "common"`) plus a minimum-only import
     titled exactly `Book` is an exact-text hint and ATTACHES with the same
     non-defer assertions as case 3 (recovery depends on the stored pair
     attaching); the evaluator's distinction-Review branch is covered with
     NO hint — the same distinguished `Book` plus an import titled
     `Book: Tail` (same main, no exact match) → Deferred with the singular
     sentence and the same no-write assertions.
  5. Two-member text-certain group is a constructed-state test because
     `idx_works_identity_v2` forbids it in activated production. Drive
     the real coordinator over that fixture; exact error equals the plural
     grammar with stable ordering and both books, with no absorption/state.
  6. Deterministic interleaving: request A acquires U2's BEGIN IMMEDIATE and
     pauses after authoritative Author/group reads for `Book`; start
     request B for `Book: Tail` and prove it waits at begin. Release A
     to create/commit; B then reads the committed group and defers before its
     first write. B leaves only its failure-history event. Reverse scheduling
     in a second test and assert whichever request defers has no residue. No
     test-only lock replaces the production transaction.
  7. Call ordinary `settle` directly as a compatibility/backstop
     fixture with an existing Author and grey, then 2+ reconciliation: each is
     Deferred before `commit_settlement`, no card/absorption/audit.
  8. Starting from the visible singular failure in production
     `ManualImportPage`, click Edit title and author, enter the stored
     pair, save, retry through the production client: the same file attaches,
     no provider id/card. Repeat from a fresh defer with a different main
     title: new Work/import, no card. Assert the request JSON shape is unchanged
     and every match-derived key is absent. Search-select remains a provider
     item and is not used.
  9. A provider-bearing ISBN case records pre-U2 result/cards/rows and is
     byte-identical after U2. The unattached-import repository operation and
     its sole road arm no longer exist.

- [ ] **AC-005 (REQ-005) — keys, intent, every origin, revocation, upgrade.**
  For GroupIdentity, PendingRoute, and EditionEvidence, mint through the real
  production write path (using the stated callerless `apply_evidence`
  compatibility fixture), Dismiss through the real typed route, and assert
  cancellation + actor audit + active version-1 ledger key are one transaction,
  then replay. Seed
  two pre-U7 equivalent EditionEvidence pending rows first: helper reuse keeps
  the oldest and machine-cancels the later row without a tombstone; separately,
  Dismiss with duplicates present cancels every equivalent pending row.

  **Origin matrix:** run every row below after an equivalent standing
  dismissal. For every machine row, common assertions are no new
  card/notification and no proposal mutation/audit/generation. The explicit
  ListImport-choice row is the control: it bypasses suppression and receives
  the ordinary safe PendingRoute mint/notification behavior.

  | Origin | Door and exact observable |
  |---|---|
  | ListImport without genuine identity choice | Production confirm row returns status `add_failed` and message `identity review required: Deferred { reason: DeferReason("standing dismissal") }`; no Work. |
  | ListImport with genuine explicit candidate choice | Validated road/service compatibility fixture with a PendingRoute proposal (the current list-confirm wire carries row selection, not a per-book candidate pick): the helper bypasses the tombstone and mints the PendingRoute card with its one normal notification. The fixture adds no production route and never reaches GroupIdentity; merely Add/Review/row selection takes the prior row. |
  | AuthorMonitor | Production workflow returns `works_added=false, notifications_created=false`; no WorkAutoAdded notification. |
  | SeriesMonitor | Production workflow logs non-settled, created/linked counters and series membership unchanged. |
  | ReadarrImport | Live Readarr workflow constructs the road request and calls `submit_readarr_identity`; for fixture title `Suppressed Book`, its per-book outcome has no Work and error `Work 'Suppressed Book': identity did not settle (Deferred { reason: DeferReason("standing dismissal") })`, so no mapped file imports. |
  | EnrichmentPass | Production add-background handoff: a generic GroupIdentity suppression follows the non-settled/false branch; an all-suppressed route-proposal handoff is `Ok(None)` and the adapter's successful no-op branch. |
  | ManualRefresh | Registered refresh route returns its ordinary refresh response; identity proposal is Deferred or all-suppressed `None`, anchor unchanged. |
  | ConvergenceVisit | Registered Retry-All workflow leaves Work incomplete; a real fired chase records exactly one CardOrMiss pass for Deferred or all-suppressed `None`. |
  | Captured-route handoff | At site 2, one suppressed proposal plus one changed-key proposal skips the first and mints the second; all suppressed returns `None`. |
  | Startup heal | Second boot after dismiss writes no card/audit and leaves marker unset while collision remains; changed cohort key mints. |
  | Edition `apply_evidence` | Same contradiction returns existing Edition unchanged as success, no card. Drive the repository method as a compatibility fixture: the live tree has no caller of this method, while it remains a runtime-capable mint writer. |
  | Edition `apply_work_evidence` | Same, including live manual-import caller: import does not fail merely because dismissed evidence recurs. |

  **User bypass:** after equivalent tombstones, DirectAdd and ManualImport
  explicit choices bypass; the validated explicit ListImport row above
  bypasses; update, merge, and affirm registered routes mint/resolve
  byte-identically. Changed elements mint; PendingRoute owner is not an
  element; another user is unaffected.

  **Future revoke:** after Dismiss, call
  `WorkService::commit_identity_edit` (production writer; its HTTP
  handlers are unregistered) on a member and, separately, registered
  affirm-pending-route. Successful transaction marks matching active
  Group/Pending ledger rows revoked and next same-key machine proposal mints.
  A forced stale/failure rolls back both mutation and revoke. DirectAdd,
  ListImport choice, title update, merge, and a refused EditionEvidence
  resolution do not revoke. EditionEvidence has no revoke and remains
  suppressed.

  **Machine cancellation:** satisfied-route, dedup-heal, sweep-heal, and U7
  duplicate-cleanup cancellations followed by the same proposal mint normally;
  no ledger row.

  **Upgrade fixture:** before activation create (u1) user-dismissed
  GroupIdentity, (u2) PendingRoute, (u3) EditionEvidence from
  `apply_evidence`, (u4) EditionEvidence from
  `apply_work_evidence`, (u5) edit before dismissal followed by a
  later machine re-observation, (u6) dismissal followed by a certified edit,
  (u7) dismissal followed by later user PendingRoute Affirm, (u8) dismissal
  followed by DirectAdd and separately ListImport UserChoice route,
  (u9/u10/u11) each machine cancellation, (u12) two equivalent user
  dismissals, (u13) multi-work GroupIdentity whose non-anchor member is later
  deleted, (u14) later historical no-op EditionEvidence review-resolution,
  (u15) malformed actor/payload. After activation u1-u6, u8, u12-latest and
  u14 adopt; u7 and machine/malformed/missing-member rows do not. Replaying
  real producers suppresses adopted keys (including both Edition writers)
  and mints non-adopted keys. u5 proves refreshed `observed_at` does
  not cancel Dismiss; u6 pins the conservative irrecoverable policy. No card
  row is rewritten/deleted, skips are logged, second start adds nothing, and
  failpoint rollback leaves marker unset for successful retry.

- [ ] **AC-007 (REQ-007) — one helper, notifications, UI, codec.**
  Instrument the one helper and drive every runtime site: generic
  PendingRoute/GroupIdentity settlement; captured-route PendingRoute;
  `apply_evidence` and `apply_work_evidence`
  EditionEvidence; startup GroupIdentity heal. Two simultaneous same-key
  calls serialize to one card. Preexisting equivalent pending duplicates
  collapse to the oldest with no tombstone/notification. First pending mint:
  exactly one notification
  for PendingRoute and zero for GroupIdentity/EditionEvidence. Reuse:
  same card, no notification. User Dismiss then replay: helper returns
  Suppressed and AC-005's exact producer result; EditionEvidence cannot
  reappear. Inline update/merge/affirm emit none.

  The review page renders EditionEvidence “not yet actionable,” owner text,
  durable-Dismiss explanation, no Resolve, and Dismiss. After both evidence
  writers replay, the card stays absent. Direct-add grey and heal
  GroupIdentity cards remain unnotified.

  HTTP `notes` is accepted from an old client as an ignored unknown
  and persists nowhere; current Rust/TypeScript types omit it. Through the real
  CLI command: ordinary struct action inner unrelated unknown accepted;
  second top-level unknown invalid; nested/top-level `notes` invalid.
  ContributorOrder compatibility card: unrelated command-payload and
  partition-entry unknowns parse then reach
  `ContinuationUnavailable`; `notes` at either location is
  `InvalidActionFile`; object-valued order element/primary and a
  second top-level key remain invalid. Every invalid case writes nothing.
