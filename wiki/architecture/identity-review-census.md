# Identity review cards — where they are minted, and every door into the one continuation

Enumerated from the live tree at `8353c0bc` (2026-08-23) for the identity-review-fixes
spec (ST-007 / ST-009 / ST-011 there hold the `file:line` detail). Read this before
writing any spec, packet or test that says "every card", "every door" or "manual import
attaches". Two review rounds failed because the census below was sampled instead of
enumerated. **Code is authoritative; re-enumerate after any change to these files.**

## The nine kinds and who mints them

`ReviewKind` (`crates/livrarr-domain/src/identity_layer/shared.rs`) has nine variants.
Production writes exactly four. Enumerate by the STORAGE TARGET, not the type names:
a python walk of `crates/` for `INSERT INTO identity_review_cards` finds **seven
statements at six sites** (the code-index was ~300 lines stale in `identity_layer.rs`
the day this was written — walk the tree, don't trust the index for a census).

| # | Site | Kind(s) | Runtime? | Notes |
|---|------|---------|----------|-------|
| 1 | `crates/livrarr-db/src/identity_layer.rs` `commit_settlement` (generic settlement cards) ← road `settle` (`crates/livrarr-metadata/src/identity_road.rs`) | PendingRoute, GroupIdentity | yes | reuse = insight-92 key for GroupIdentity |
| 2 | `commit_pending_route_review` ← `apply_captured_route_handoff_authority` (captured-route handoff / search fallback) | PendingRoute | yes | the only site that already pairs card + notification and returns on a pending reuse; key = (provider, kind, trimmed value), owner excluded |
| 3 | `commit_unattached_import_review` ← the ManualImport arm in `settle` (its ONLY caller) | ImportIdentity | yes | removed by identity-review-fixes REQ-003; after that nothing mints ImportIdentity |
| 4 | `apply_evidence` (generation bound 0) and `apply_work_evidence` (generation = the Work's current) — the live manual-import door reaches the second via `crates/livrarr-server/src/state.rs` | EditionEvidence | yes | empty `evidence_ids`; NO pending-card check → duplicates accumulate |
| 5 | `crates/livrarr-db/src/pool.rs` `heal_identity_title_policy`, run at every server start (`main.rs`) until its marker is current | GroupIdentity (no proposal) | yes (boot) | guarded only by "no equivalent PENDING row" → re-mints a card the user cancelled |
| 6 | `stage_legacy_identity_rows` (pre-activation cutover staging) | GroupIdentity | no (cutover) | wave-3 territory |

No production producer: IdentityConflict, FieldResolution, ContributorOrder,
MigrationRepair, InvariantRepair. (`plan_readarr_identity` in
`crates/livrarr-server/src/readarr_import_workflow.rs` names two of them and has no caller.)

## Every door into the one continuation (`resolve_review`)

Eight, by LSP references plus the CLI's direct call:

1. `/identity-review-card/{id}/resolve` — typed (`crates/livrarr-handlers/src/identity_layer.rs`)
2. `/identity-review/{work_id}/resolve` — legacy alias, same handler
3. `/identity-conflict/{id}/resolve` and 4. `/identity-conflict/{id}/dismiss`
   (`identity_conflicts.rs`) — load a TYPED `IdentityConflict` card by `conflict_id`,
   while the page's list/detail read the LEGACY `work_identity_conflicts` store
   (`crates/livrarr-server/src/services/identity_conflict_service.rs`). Nothing mints the
   typed kind, so every Resolve/Dismiss on a listed row is a 404 today.
5. Work title/author update (`crates/livrarr-handlers/src/work.rs` `update`) — mints a
   GroupIdentity card through `WorkUpdateRekey` and resolves it `DifferentFromAll` in the
   same request (AUD-P1-20: a "user decision" the user never saw)
6. merge with choices (`merge`) — `ManualWorkMerge`, resolved `AttachOrMerge` inline
7. affirm pending anchor (`affirm_pending_anchor`) — `AffirmPendingRoute`, resolved `Affirm` inline
8. cutover CLI `identity-cutover resolve` (`crates/livrarr-server/src/identity_layer.rs`,
   builds an `IdentityRoadServiceImpl` and calls `resolve_review`; errors → stderr, exit 2)

Doors 5–7 only ever submit GroupIdentity / PendingRoute for a card minted in the same
request — a notification rule keyed on "minted" would alert on already-resolved cards.

## The GroupIdentity continuation is known-broken (AUD-P0-2) until wave B

`DifferentFromAll` with a parked proposal runs `UPDATE works SET title=…, author_id=… WHERE
id = <the card's work>` — it overwrites the established work with the proposal.
`AttachOrMerge` absorbs with no archive. Any change that routes a NEW flow into a
GroupIdentity card (e.g. letting minimum-only manual import reach the road's group
reconciliation) hands the user buttons that corrupt identity. Wave B fixes the
continuation; until then a bounded fix must defer such cases, not park them.

## Manual import: what the door actually feeds the matcher

`find_existing_work` → `work_dedup::find_matching_work` with `ProviderKeys { ol_key, ..Default }`
— the item's ISBN / ASIN / GR / HC keys never reach the matcher (no HC slot exists), so
only the OL-key arm and the text tier (exact main title + author agreement) can fire. The
work list is loaded ONCE per request, so item N cannot dedup-attach to a work item 1
created — it reaches the road and AutoMerges onto it (text-certain pair). A one-sided
subtitle tail is grey at the dedup tier ONLY (never absorbed there — `identity_absorb_match`
compares the raw incoming string against `Work.title`); at the road's group reconciliation it
is text-CERTAIN, because `candidate_core` splits the incoming title into its identity tuple
and `evaluate_match` compares tuple mains only (`crates/livrarr-domain/src/identity_layer/services.rs:639-640`
at `c71af24a`) — so a lone sibling is ATTACHED and a sibling inside a 2+ cohort is ABSORBED
(observed live 2026-08-24; spec identity-review-fixes v11 ST-011). The earlier claim on this
page that the road "parks" it was wrong (corrected 2026-08-24).

## Cancellation is four sites; only one is the user (verified 2026-08-23)

`status='cancelled'` is written at FOUR sites: the user's Dismiss
(`crates/livrarr-db/src/identity_layer.rs:1116-1170` — the only one whose
`review-dismissal` audit row carries a `ReviewActor` JSON actor), a satisfied
pending-route proposal (`:2732-2750`, actor `identity-engine`), the
dedup-residue heal (`:4948-4975`, actor `identity-dedup-residue-heal`), and
the sweep heal (`crates/livrarr-db/src/pool.rs:1729-1745`, actor
`identity-sweep-heal`). "Cancelled" therefore never implies a user decision.
Nothing in the tree sets a card back to `pending`. A card's `(user_id,
work_id)` FK is ON DELETE CASCADE with `foreign_keys=ON` — a card whose own
work is deleted vanishes with it.

## `user_confirmed=1` is NOT identity-edit-only, and `observed_at` refreshes

Three writers stamp `user_confirmed` routes: the certified identity edit
(`sqlite_work_identity.rs` → `sync_identity_edit_route:1957`), PendingRoute
Affirm, and any DirectAdd/ListImport settlement whose bundle carries a
`user_choice` (`identity_road.rs:958-989` — `RouteProvenance::UserChoice` ⇒
`user_confirmed`). And the settlement route upsert
(`identity_layer.rs:2790-2839`) preserves the bit and UserChoice/OwnedFile
provenance but REPLACES `observed_at` on every machine re-observation. So
"a user-confirmed route observed after time T" proves nothing about user
activity after T. The identity edit writes NO `identity_audit_events` row;
its durable trace is the route row + generation bump only, and its HTTP
handlers (preview/commit/clear, `work.rs:1870,1934,1992`) are NOT in the
router (IA-C15) — the production entry is `WorkService::commit_identity_edit`.

## The road-service trait is three methods; the door cannot reach the impl

`IdentityRoadService` = `settle` / `resolve_review` /
`apply_captured_route_handoff` (`identity_layer/services.rs:324+`).
`reconcile_complete_group` is an inherent method on the impl in
livrarr-metadata and `authority_certain` is crate-private
(`identity_road.rs:1087-1098`) — a handler (compile wall: domain, http,
matching, jobs only) cannot call either. Any door-side decision that needs
the group rule needs a trait surface, never a re-implementation.

## The import screen has no title editor

The parsed row's title text is a BUTTON that opens the OL search
(`ManualImportPage.tsx:567-576`); `correctedMatch` is set only by
search-select (`:297-311`), which submits the candidate's provider ids (a
provider-identity item, a different door-matrix cell) and DROPS the
`existingWorkId` the search computed. There is no on-screen way to type a
title/author into the import request today, although the wire carries both.
