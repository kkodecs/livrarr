---
feature: "identity-conflict-authority"
stage: spec
status: frozen
version: 12
req_ids: [REQ-002, REQ-004, REQ-006]
---

# Spec: identity-conflict-authority (audit fix-wave 1b — the review surface, remaining half)

**Review gate overridden by the PO on 2026-08-28 (2026-08-28T23:19:20Z, `override_log`): v12 is the spec the architecture stage builds from.** Round 1 verdicts stand as filed (openai FAIL p0=2 p1=7; xai FAIL p0=0 p1=3); every finding is folded below; no round 2 was run.

v12 (2026-08-28) folds review round 10 — the first round on the WHAT-level
rewrite, and the first this feature ever filed into the gate (xAI FAIL
p0=0 p1=3 p2=2 p3=3; OpenAI FAIL p0=2 p1=7 p2=2 p3=1; every finding verified
at source before it was folded; both families' suggested fixes were the
starting point). What changed: the cohort is the card's `work_ids` plus its
subject, never a member list (XAI-R10-001); a **Route ownership** rule
restores v10's third-owner refusal for every cell that applies a proposal,
attaches a route or absorbs (OAI-R10-003); `Incoming × DifferentFromAll`
leaves the new book user-distinguished (OAI-R10-004); undo keeps each moved
file's recorded location truthful until the file step moves it, so the
merge's own moves are never a mismatch (XAI-R10-002); the file move-back is
promised as no-clobber (before or during the move), never-the-only-copy,
truthful-record-or-named-mismatch and per-file report including leftover
duplicates, over both books' files — beyond what the forward code gives
today, which ST-049 now states accurately (OAI-R10-001, XAI-R10-004,
XAI-R10-008); adopted legacy route-conflicts carry an explicit unknown
contradicted id, their user dismissals are history with no standing
dismissal, their Keep/Treat-as-Separate resolve that card only, and
unreadable rows are history-only while readable non-keyable rows are
actionable — the neighbouring cells of the Q-025 fold (OAI-R10-002, -006,
-007); ST-048 names all three supersede-with-successor writers (XAI-R10-005,
OAI-R10-010); ST-005's migration filename was wrong and is fixed
(XAI-R10-007, OAI-R10-011); the acceptance criteria are table-driven over
every 5×2 cell and cohort size and every 5×4 cell, with AC-030–AC-032 added
for route ownership, survivor deletion / the resolving card, and
exact-question suppression (XAI-R10-003, OAI-R10-005, -008, -009); and the
mechanism leftovers both families listed are rewritten as behaviour or moved
to the design inputs (XAI-R10-006, OAI-R10-012). Two checks were run on
this text and their results are stated, not claimed: a **citation audit**
(script: every `path:line[-line]` in the file, shorthand basenames resolved
by a fixed alias table, migrations by number) — 120 occurrences, 118
unique, 0 missing files, 0 out-of-range endpoints; and a **mechanism-term
sweep by stem** (41 patterns including claim/claims/claimed, seal/seals,
token, guard, marker, transaction, symbol, grep, the D/B names and the
phrases both families quoted) — hits remain only in this header's own list,
in System Truths and Prior Art (facts and pointers), in §4's exclusions,
§5's "design inputs" pointers and §7's unit names; §2 has one ("a lifecycle
event", plain English) and §6 two ("marker": the heal's and the cutover's
completion flags, observable state).

v11 (2026-08-28) was the **rewrite to the WHAT level**, on the PO's call the
same day ("stick to the process"): v1–v10 had grown from 19K to 152K
characters by absorbing mechanism — lock protocols, journal formats,
migration order, token rules — that a spec cannot check and that nine
review rounds could only push around. Everything this spec promises is a
behaviour a user or a test can observe; every mechanism v10 carried is
preserved, not deleted, for the design stage in
`build/design/design-inputs-identity-conflict-authority.md`, with the v10
text itself snapshotted byte-identical at
`build/reviews/identity-conflict-authority/spec-identity-conflict-authority-v10.md`
(v1–v9 and v11 beside it, both families' r1–r10 reviews beside them). Two
decisions of 2026-08-28 are folded: undo moves files back **simply** (§2
REQ-004, Q-010, Q-026), and the anchors ledger is **not** append-only on this
tree, so every adopted legacy route-conflict is non-keyable (ST-048, Q-025 —
both r9 reviews reproduced it; writers re-read at source). The remaining r9
findings are mechanism findings and travel to the design inputs with their
ids. `verify.py spec` passes on v11 and v12; no earlier revision was run
through it.

Wave 1a (`identity-review-fixes`, closed `507f2ac2`) made the review surface
*honest*. This spec is the repair it deliberately deferred: the two
GroupIdentity actions, reversible absorption, one conflict authority.
Numbering continues the audit's wave-B plan (v2 REQ-002 / REQ-004 / REQ-006).

**PO calls of record:** full undo, not forensic-only; one design in build
units; undo data lives as long as the surviving Work and cascades with it;
"different from all" REFUSES rather than creating a near-duplicate when the
proposal is not certainly distinct; conflicts live in the review-card system;
one complete record per merge; machine conflict cards are
dismissal-ledger-keyed; undo moves files back the same simple way the merge
moves them forward and reports each file it leaves (2026-08-28); merges stay
pairwise with an elected survivor; a pre-feature dismissal whose question
cannot be told apart stops being honoured (2026-08-27); `identity_conflicts_v2`
stays cutover-only.

## 0a. Design Principles

- **Data safety outranks tidiness** (`PRINCIPLES.md` conflict ladder, rung 1).
  Where "migrate" and "drop" compete, migrate. Where an effect of a merge
  cannot be classified, refuse the merge rather than guess.
- **Operations must be recoverable** (`PRINCIPLES.md` §6). Merging is the one
  identity operation with no way back. Undo is the requirement, and undo is
  only real if it covers the *whole* operation.
- **One authority per concern** (`PRINCIPLES.md` §1). Three conflict stores is
  the bug. Three absorption implementations is the same bug. One rule decides
  which provider ids survive a merge; one authority performs every Work
  deletion.
- **One canonical key.** Reuse, duplicate cleanup, Dismiss, suppression and
  revocation share one key definition (wave 1a). A second key lets a dismissed
  question return under another name.
- **User intent is final** (`ARCHITECTURE.md` Part 1). "This is a different
  book" must not destroy a book. "Merge these" must not discard what the user
  merged in, and must let the user say which book survives. A standing
  dismissal is honoured where its question is provably the same, and never
  stretched over a different one.
- **Uncertainty is visible, not silent.** An action that cannot be performed
  safely refuses and says why (wave 1a's pattern). Uncertain is not
  different: an unsure comparison refuses, it never creates.
- **Decisions at decision time** (insight 91). Every action a user takes on a
  card or a merge preview is checked against exactly what the user was shown;
  a change to any of it since is a refusal, never a silent overwrite.
- **Reuse the road's certainty rule where it already runs.** The same-book
  decision is made inside the identity road, never copied into a handler
  (insight 98's split-authority shape).

## 0b. System Truths

Read from source by tree walk at `507f2ac2` (never the index — the census
page's standing rule); `.claude/worktrees/agent-a141be70bbf7f9932/` is a
stale full source copy inside the repo and is excluded from every count.
Schema facts (ST-005, ST-017, ST-030, ST-037, ST-042, ST-048) were produced
by applying all 85 migration files (`001`–`086`, no `037`) to a fresh SQLite
3.45.1 with `foreign_keys=ON` and querying the live schema; both r9
reviewers reproduced them the same way. Every `file:line` below passed the
v12 citation audit (header); ST-048's writers and ST-049 were read at
source for v11 and v12.

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | `052_federate_identity_conflicts.sql`; `crates/livrarr-domain/src/identity.rs:469-524`; `crates/livrarr-domain/src/identity_layer/services.rs:179-182`; `crates/livrarr-db/src/identity_layer.rs:1793-1838`; `crates/livrarr-handlers/src/identity_conflicts.rs:259-282` | The conflict surface is THREE stores. (1) `work_identity_conflicts`, rebuilt by 052 without SQL enum checks; live columns `id, user_id, existing_work_id, kind, incoming_payload_json, raised_at, raised_by, raised_source_path, status, resolved_at, resolution_action, resolution_notes`; `existing_work_id → works ON DELETE CASCADE`; Rust vocabulary: five kinds (`IncomingDifferentOlKey/GrKey/HcKey`, `OlRedirectCollision`, `QuorumTie`), three statuses, nine sources, four actions (`KeepExisting, AcceptSeparate, ReplaceAnchor, Merge`); this is what the page lists. (2) `identity_conflicts_v2` (082): typed, `current_work_id` CASCADE, `expected_generation`. (3) `identity_review_cards` kind `IdentityConflict` holding only `{ conflict_id, work_id }`; nothing mints it at runtime; the handler probes the typed card and the open legacy row, then refuses through `require_continuation`. | Saying "the typed store" without saying which; leaving two actionable stores after this feature. | high |
| ST-002 | `crates/livrarr-db/src/identity_layer.rs:6436-6488`, `:2360`, `:3905-3923`; `crates/livrarr-domain/src/identity_layer/services.rs:570` | `identity_conflicts_v2` has one INSERT producer (`stage_route`, pre-activation cutover staging: a pending row only when a staged route's value is already active on another Work) and two UPDATE writers (`resolve_conflict_atomically`, a repository method with no handler, road or job caller; `absorb_work_into`, which repoints `current_work_id`). No page lists it. | Treating v2 rows as a runtime authority; promoting or deleting them in this wave. | high |
| ST-003 | `082_identity_layer_foundation.sql:196-203` | `identity_route_archives` has columns `user_id, provider, kind, route_value, former_owner_type, former_owner_id, reason, audit_id, archived_at` — no owner, state, provenance, `user_confirmed` or `observed_at` — and zero writers. | Using it to restore a route row. | high |
| ST-004 | `082:209-216`; `crates/livrarr-db/src/identity_layer.rs:3946-3963` | `identity_merge_archives` is `id, user_id, winner_work_id, loser_work_id, preserved_fields, audit_id, archived_at`; its only FK is `user_id → users`; `preserved_fields` holds the literal `'all-dependent-rows'`, one row per absorbed loser, written immediately before `DELETE FROM works`. | Reading an existing row as restorable; a Work FK on the existing columns. | high |
| ST-005 | `crates/livrarr-db/src/identity_layer.rs:3684-3964` (`:3813-3842`, `:3526-3535`, `:3579-3605`); applied indexes of `identity_routes`; `crates/livrarr-db/migrations/083_identity_layer_cutover_support.sql:5-11` | Absorption mutates the winner's routes in place (`user_confirmed = MAX(...)`, provenance `MergeCoalesced`), deletes the loser's duplicates, repoints non-duplicates with provenance overwritten, and repoints a loser Work-scoped route whose id differs from the winner's same-provider route as a **second active id** — nothing checks the contradiction. `identity_routes` has NO unique index: all three indexes (`idx_identity_routes_owner`, `idx_identity_routes_typed_lookup`, `idx_identity_routes_one_active_owner`) report `unique = 0`; `insert_route` keys its existence check on the full `(provider, kind, provider_scoped_id)` of the destination Work only, so a second Work-scoped id of the same provider — or an id another Work holds — inserts freely. | Relying on storage to stop two active same-provider ids or a route owned by a third Work; assuming a unique triple (the r5 xAI claim was wrong). | high |
| ST-006 | `crates/livrarr-db/src/identity_layer.rs:1948`, `:1961-2044` | "Different from all" with a proposal runs `UPDATE works SET title, subtitle, identity_volume, normalized_*, author_name, author_id, primary_author_id, normalized_author, identity_title_provenance, text_distinction WHERE id = <the card's work>`, then merges contributors, inserts the proposed routes and recomputes status — all onto the established Work. | Calling today's continuation correct. | high |
| ST-007 | `crates/livrarr-db/src/identity_layer.rs:2058-2072` | "Attach or merge" refuses any survivor other than the card's own Work, then applies the merge field choices and absorbs every other member; `proposed_identity` is never read. | A survivor election through today's arm; assuming the proposal is applied. | high |
| ST-008 | `crates/livrarr-db/src/identity_layer.rs:2045-2056`; `crates/livrarr-db/src/pool.rs:1696-1699` | A "different from all" on a card with no proposal writes `text_distinction = 'different:review:<card id>'` on the card's Work, then claims, audits and resolves. Boot-heal cards carry no proposal. | Describing that arm as "does nothing". | high |
| ST-009 | `tests/behavioral/test_irf_u1_refusal.rs:1851-1870`, `:1944`, `:2015`; `test_ilr_contracts.rs:9167`; `test_irf_u5_durable_dismissal.rs:4878-4882`; `build/reviews/identity-review-fixes/CORRECTION-JUDGE-CONTEST-U5R1.md` §2 | Settlement and continuation EACH claim a generation, by design: an update is one settle plus one continuation, the continuation expects `generation + 1`, the end state is `+ 2`; a committed settlement claim surviving a failed continuation is pinned as correct. | Deferring the settlement claim; "fixing" the two-step protocol. | high |
| ST-010 | `crates/livrarr-db/src/identity_layer.rs:717-753`, `:2145-2158`, `:5626-5636`, `:5854-5865`; `crates/livrarr-db/src/pool.rs:1725-1747`, `:1776-1808`; `services.rs:171-172` | Generation claims around absorption differ per caller: settlement AutoMerge claims the survivor before absorbing; the review continuation after; the dedup heal bumps unconditionally after; the article heal claims each fold winner after all folds (a tuple-renormalizing UPDATE or a bare compare-and-increment, both against the pre-open value). No caller claims the loser; `absorb_work_into` itself claims nothing. | Assuming the loser is protected against a concurrent edit; assuming one claim shape. | high |
| ST-011 | `crates/livrarr-handlers/src/types/work.rs:388`; `crates/livrarr-db/src/sqlite_work_identity.rs:1210`; `crates/livrarr-handlers/src/work.rs:2066`, `:2079` | `parked_by_conflicts` is hardcoded `false` in the DTO; the only computed value is a clear-operation result used for its re-chase decision and never projected. | Treating the DTO field as real. | high |
| ST-012 | `crates/livrarr-db/src/identity_layer.rs:416-527` (SQL `:488`), `:343-357` | `mint_reuse_or_suppress_review_card_in_tx` owns the only runtime `INSERT INTO identity_review_cards`, the reuse keys, oldest-wins duplicate cleanup, the dismissal-suppression preflight and the single `identityReviewNeeded` rule; its proposal carries a `site` and an `origin`. | A second INSERT; a card kind minted outside it. | high |
| ST-013 | `crates/livrarr-domain/src/identity_layer/dismissal.rs:16-46`; `crates/livrarr-db/src/sqlite_work_identity.rs:2338`; `crates/livrarr-db/src/identity_layer.rs:2198-2207` | `ReviewDismissalKeyV1` has `GroupIdentity { user_id, work_ids, proposed, merge_choices }`, `PendingRoute`, `EditionEvidence` — no `IdentityConflict`, no `IdentityPark`, no proposal role. Exactly two doors revoke: the certified identity edit and a PendingRoute Affirm. | Keying a conflict or park card with the V1 enum; inferring a role from a V1 row. | high |
| ST-014 | `crates/livrarr-domain/src/identity_layer/services.rs:354-384`; `crates/livrarr-domain/src/identity_layer/reconciliation.rs:71-81`; `crates/livrarr-metadata/src/identity_road.rs:385` | The road trait is four methods (`settle`, `resolve_review`, `settle_manual_import_minimum`, `apply_captured_route_handoff`); `authority_certain` is `pub fn` in domain; `reconcile_complete_group` is an inherent method on the metadata impl, behind the compile wall for handlers — and `resolve_review` runs on that same impl. | A handler-side copy of the same-book rule. | high |
| ST-015 | `.claude/worktrees/agent-a141be70bbf7f9932/` | A stale agent worktree inside the repo holds a full source copy. | Any enumeration that does not exclude it (counts double). | high |
| ST-016 | `crates/livrarr-domain/src/identity_layer/services.rs:187-191`; `crates/livrarr-metadata/src/identity_road.rs:183-188`, `:190-207`, `:208-220`, `:262-288`; `crates/livrarr-handlers/src/work.rs:841-874`; `identity_road.rs:865-905` | `SettlementReviewCard::GroupIdentity { work_ids, proposed_identity, merge_choices }` has no role, and four road arms mint it: the edit door (origin `WorkUpdateRekey`, which settles and resolves `DifferentFromAll` in the same request), the merge door (`ManualWorkMerge`, proposal = survivor ∪ loser routes), the engine-Review arm, and the complete-group Review arm that binds the subject to the first broad-group candidate when none was supplied. The origin exists only transiently; the card row and the settlement audit carry none. | Recovering a role from payload shape; treating the four arms as one. | high |
| ST-017 | Applied schema (`PRAGMA foreign_key_list` per table); `crates/livrarr-db/src/identity_layer.rs:3706-3721`, `:3756-3780`, `:3846-3897`, `:3905-3944`, `:3958-3963`; callers `:811-818`, `:2070-2072`, `:5619-5624`, `:5854-5865` (via `pool.rs:1606-1614`) | **27** live tables hold a direct FK to `works` (CASCADE unless marked): `author_link_candidates` (SET NULL), `author_link_key_attempts`, `author_provider_routes` (SET NULL), `bookmarks`, `editions`, `external_ids`, `grabs`, `history` (SET NULL), `identity_conflicts_v2`, `identity_provider_attempts`, `identity_review_cards`, `identity_round15_gr_cover_reselect_queue`, `identity_routes` (two columns), `import_intents`, `library_items`, `machine_subtitle_projections`, `provider_retry_state`, `work_anchor_dead_ends`, `work_contributors`, `work_cover_selections`, `work_field_dissents`, `work_identity_anchors`, `work_identity_conflicts`, `work_identity_review_candidates`, `work_metadata_provenance`, `work_relationships` (`from_work_id` only), `work_subjects`. Absorption also writes three tables with no `works` FK (`identity_audit_events` repointed; `work_contributor_roles` copied; `work_default_editions` deleted then re-inserted), repoints `identity_review_cards` of every status, mutates `work_relationships` on both ends, and ends with `DELETE FROM works`, which cascades or nulls everything in the list. Four callers absorb: settlement AutoMerge, the review continuation, the dedup-residue heal and the article-duplicate heal. | A snapshot of "the loser row plus its routes" as a complete record (the r1 P0); an effect inventory from migration text. | high |
| ST-018 | `crates/livrarr-handlers/src/work.rs:1163-1169`; `crates/livrarr-server/src/import_service.rs:496-533`, `:612-614`, `:628-647`, `:649-658` | The merge door moves files **after** the transaction commits, best-effort, over every item the survivor then owns; failures become response warnings, never a rollback; nothing records the transitions. | Treating the file move as part of the transaction; assuming a record of what moved. | high |
| ST-019 | `crates/livrarr-server/src/router.rs:563-571`; `crates/livrarr-handlers/src/identity_review.rs:84-111`, `:184-223` | `GET /identity-review` lists Works parked `NeedsReview` with their persisted candidates; the resolve route is wired to the typed-card handler; `identity_review::dismiss` has no route. The grey-park list has no exit (IA-F02). | Calling the park list actionable today. | high |
| ST-020 | `crates/livrarr-db/src/sqlite_work_identity.rs:2385-2415` (`:2377`, `:2408`; callers `:375`, `:1440`); `crates/livrarr-db/src/sqlite_identity_conflict.rs:68`; `crates/livrarr-db/src/pool.rs:2777` (inside a test at `:2723`) | One live legacy-conflict producer, `raise_identity_conflict_in_tx`, inserts after a dedup select and then writes the frozen pre-F2 column `works.identity_status = 'conflict'`; `create_identity_conflict` has no caller. | A second producer; keeping the frozen-status write. | high |
| ST-021 | `crates/livrarr-handlers/src/work.rs:1075-1083`, `:1121-1124`, `:1163-1179`; `crates/livrarr-metadata/src/identity_road.rs:250-256` | The manual merge is pairwise by construction (`(id, loser_id)`; empty choices park the card and return 202; the response returns the path Work). The road's AutoMerge arm absorbs **every** other broad-group member. | An N-way user merge; assuming machine merges are pairwise. | high |
| ST-022 | `git grep -n work_identity_conflicts -- crates/` (tests and migrations excluded; both r4 reviewers reproduced 18): `identity_layer.rs:3133`, `:5760`; `pool.rs:655`; `sqlite_identity_conflict.rs:53-56`, `:89`, `:119`, `:146`, `:172`, `:196`, `:218`, `:294`, `:506`, `:581`; `sqlite_work_identity.rs:322`, `:1260`, `:2263`, `:2377`, `:2390` | The legacy conflict store has eighteen runtime SQL statements in four files: cutover readiness, the orphan-dependency tuple, the backfill's repoint, two dormant helpers, list/get/resolve/dismiss/badge, re-raise suppression, the identity-edit basis and close, the producer's dedup select and insert. | A cutover that replaces fewer than all eighteen. | high |
| ST-023 | `crates/livrarr-db/src/sqlite_identity_conflict.rs:321-338`; `crates/livrarr-handlers/src/identity_conflicts.rs:167-175` | Legacy `AcceptSeparate` and `KeepExisting` are one arm (the same re-stamp); the typed adapter maps `AcceptSeparate` to `DifferentWork` and requires `winningWorkId`, which the page never sends. | Requiring a winning id for Treat-as-Separate. | high |
| ST-024 | `crates/livrarr-identity/src/async_resolver.rs:54-92`; `crates/livrarr-db/src/sqlite_identity_conflict.rs:544-551`, `:377-389` | `QuorumTie` conflicts can be route-less (raised only when no OL/GR/HC key exists); their `ReplaceAnchor` is gap-fill with no supersede. | Implicating an existing route for `QuorumTie`. | high |
| ST-025 | migration `068` (`candidates_json`); `crates/livrarr-handlers/src/identity_review.rs:14-36`, `:145-148`, `:32-34` | Grey parks are candidate lists, not Work cohorts; resolve applies the chosen candidate's anchors to the parked Work; a candidate's `existing_work_id` is informational. | An absorb through the park exit. | high |
| ST-026 | `crates/livrarr-domain/src/identity_layer/reconciliation.rs:209-252` | The complete-group evaluator strips routes, records one pairwise-certain entry per member, and returns `Create` only for a pre-distinguished or empty group. | Reusing it unchanged as the three-valued "is this proposal a different book" rule. | high |
| ST-027 | `crates/livrarr-handlers/src/work.rs:1121-1124`; `POST /identity-review-card/{id}/dismiss` | The typed-card Dismiss accepts any pending card, including a parked merge card, and writes a V1 `GroupIdentity` ledger row for it; such rows exist in deployed ledgers. | Assuming every V1 group row is a creation-door question. | high |
| ST-028 | ST-004; `crates/livrarr-db/src/pool.rs:22` | Existing archive rows can be orphans (no Work FK) under `foreign_keys=ON`. | A retention rule that assumes every existing row names a live Work. | high |
| ST-029 | `crates/livrarr-db/src/identity_layer.rs:702-940` (settlement: claim `:717-753`, absorb `:811-818`, writes after `:819-933`); `:1840-2231` (continuation: absorb `:2070-2072`, claim `:2145-2158`, audit and card `:2173-2197`); `:5490-5712` (dedup heal: many pairs per transaction, collapse `:5658-5697`, marker `:5699-5707`); `crates/livrarr-db/src/pool.rs:1390-1845` (article heal: all folds first, cohort card mints `:1686-1723`, winner claims `:1725-1828`, marker `:1830-1834`) | Every absorbing caller writes member-affecting rows **before and after** the absorb inside the same transaction, and the two boot heals run **several** operations per transaction with writes after the last fold that belong to no one operation (card mints, an equivalent-card collapse, a marker). A record kept only inside `absorb_work_into` misses those writes. | Recording a merge from inside the absorber alone; treating a heal transaction as one operation. | high |
| ST-030 | Applied schema, recursive FK walk from `works`; trigger `078:200-218`; `082:83`, `:174-175`, `:202-203` | The rows a merge can reach are a **37-table closure** (`works` included; nine tables only through a direct table: `audiobook_chapters`, `kash_links`, `playback_progress`, `cross_format_state`, `edition_cover_candidates`, `work_default_editions`, `work_contributor_roles`, `author_link_candidate_alternate_name_evidence`, `author_name_variants`), one trigger on `works` itself (`author_link_work_evidence_changed`, writing `author_link_progress`), and Work-pointer columns with no FK: `identity_audit_events.work_id`, `identity_merge_archives.winner_work_id/loser_work_id`, `identity_review_dismissal_works.work_id`, `provider_call_records.work_id`, `work_contributor_roles.work_id`, `work_default_editions.work_id`, `work_relationships.target_work_id`, `identity_conflicts_v2.proposed_owner_id` (Work or Edition by type), `identity_route_archives.former_owner_id`, and `notifications.data` JSON. | "Every row that references the book" derived from FK columns alone or from a name pattern. | high |
| ST-031 | `crates/livrarr-db/src/identity_layer.rs:401-415`, `:4465-4472`, `:500-514`; migration `086` (`source_card_id`) | Inline mints (`WorkUpdateRekey`, `ManualWorkMerge`, `AffirmPendingRoute`) bypass the dismissal ledger; every other origin consults it. No mint persists its origin, so a V1 group row's question (creation-door incoming, machine re-identification, parked manual merge, engine creation park) cannot be told apart. | Honouring an ambiguous V1 group dismissal; inferring its role. | high |
| ST-032 | `crates/livrarr-domain/src/identity.rs:147-150` | Grey-park candidate ids are consume-once cache handles. | A dismissal key over candidate ids. | high |
| ST-033 | `crates/livrarr-server/src/import_service.rs:669-690`; `crates/livrarr-library/src/lib.rs:122-142`; `crates/livrarr-db/src/sqlite_library_item.rs:316-336` | A cross-device reorganization move is copy-then-remove (`atomic_copy` streams into a temp file, then the source is removed); the path-record write is an unconditional `UPDATE library_items SET path = ? WHERE id = ? AND user_id = ?`. | Assuming the forward move is atomic across devices or that the path write is compare-and-set. | high |
| ST-034 | `frontend/src/pages/review/ReviewPage.tsx:56-71`, `:153-197`; `crates/livrarr-handlers/src/identity_conflicts.rs:47-56` | The conflict page has four buttons (`keep_existing`, `accept_separate` "Treat as Separate", `replace_anchor`, `merge`) plus Dismiss, and sends `{ action }` and nothing else; `surviving_routes`, `target_edition` and `winning_work_id` are never sent. | An action that needs a field the page cannot send. | high |
| ST-035 | `crates/livrarr-db/src/identity_layer.rs:4338-4361`; `identity_road.rs:216`, `:268-288` | A GroupIdentity card's cohort is its `work_ids` plus its subject (`work_ids.first()`, else the settlement's Work), and the four mint arms give it four shapes: `[W]` (edit), `[S, L]` (merge), `[W]` or **empty** (engine-Review), the broad-group candidates (complete-group Review). | One cohort shape; a cohort read from `work_ids` alone. | high |
| ST-036 | `crates/livrarr-metadata/src/identity_road.rs:236-256`; wave 1a's coordinator; `crates/livrarr-db/src/identity_layer.rs:755-786` | No production settlement creates a Work and absorbs in the same commit, but the settlement repository allows the combination structurally. | Assuming the combination cannot occur. | high |
| ST-037 | migration `086` | The dismissal ledger has no foreign keys and versions its keys (`UNIQUE(user_id, kind, key_version, canonical_key)`; `identity_review_dismissal_works` has no Work FK). | A ledger row cascading with a Work. | high |
| ST-038 | `git grep 'DELETE FROM works' -- crates/`: `identity_layer.rs:3958`; `sqlite_work.rs:895` (`merge_works`, `:751-911`, reached only by `WorkService::merge_works`, `work_service.rs:2222-2305`, which has no production caller — the HTTP door calls `preview_merge_works` only, `handlers/work.rs:1064`; in-file callers under `#[cfg(test)]` at `:1808`); `pool.rs:783` (`backfill_normalized_identity`, `:417-827`); `sqlite_work.rs:740-748`; `sqlite_import.rs:167-176`; `sqlite_list_import.rs:224-235` | The tree holds THREE absorption implementations (the live absorber, the dead `merge_works` chain, the dead pre-038 backfill) and six `DELETE FROM works` statements; none of the plain deletes cancels a pending card. | Leaving a second absorption implementation; a delete door that leaves pending cards naming a dead Work. | high |
| ST-039 | `crates/livrarr-db/src/sqlite_work_identity.rs:273-299`, `:2390-2394` | A legacy key conflict is raised against the Work's ONE confirmed anchor of that type, and the row persists only the incoming payload — never the contradicted value. | Recovering the contradicted id from the row. | high |
| ST-040 | `crates/livrarr-domain/src/identity_layer/reconciliation.rs:71-81`; `crates/livrarr-domain/src/identity_matching.rs:88-96` | `authority_certain` = title `Same` ∧ author `Agree` ∧ id not `WorkKeyContradiction`; `Grey` titles (`NearMain`, `SubtitleDisagreement`, `VolumeAsymmetry`, `OneSidedSubtitle`) are close but not certain; `Different`/`VetoVolume` are hard negatives; `AuthorVerdict` is `Agree | Grey | Disagree | Abstain`. | A two-valued same/different rule; treating Grey as different. | high |
| ST-041 | `crates/livrarr-db/src/identity_layer.rs:4559-4603`; `notifications` schema | Notifications point at cards through untyped JSON (`ref_key = identity-review-card:<id>`, `data.cardId/workId`), with no FK to Works or cards; the only card notification is `identityReviewNeeded` for a minted non-inline PendingRoute card. | A notification left pointing at a withdrawn card. | high |
| ST-042 | `082:183-194`; `crates/livrarr-domain/src/identity_layer/services.rs:178-192` | A card has one relational Work column; its cohort exists only inside the serialized payload; `identity_review_cards.id` is `INTEGER PRIMARY KEY AUTOINCREMENT`, so an id is never reused at or below the table's high-water mark even after a delete. | Finding "every card naming a book" by the relational column alone; using the clock as a card-ordering authority. | high |
| ST-043 | `crates/livrarr-handlers/src/identity_layer.rs:102-108`; `crates/livrarr-domain/src/identity_layer/review.rs:72-121`; `crates/livrarr-handlers/src/types/work.rs:553-587`; `crates/livrarr-domain/src/services/work.rs:224-247`; `crates/livrarr-metadata/src/work_service.rs:2185-2219`; `handlers/work.rs:1132-1138` | No decision proof exists on any wire: a card response carries one `expectedGeneration`; every resolution command carries one `expected_generation`; the conflict request is `{ action }`; the merge preview/execute carry no generation and no snapshot; a merge choice is `KeepSurvivor | TakeLoser` per field (side-relative), preview takes the pair positionally, and the door resolves with the path Work. | Assuming the merge preview and execute are bound to each other today. | high |
| ST-044 | Reproduced on SQLite 3.45.1 (r5 xAI fixes pass; r9 both families) | Subqueries are prohibited in CHECK constraints (a naive `CHECK (work_id > 0)` checks nothing about tenancy); `BEFORE INSERT OR UPDATE` is a syntax error (one trigger per event); `ON DELETE SET NULL` fires the child table's BEFORE UPDATE trigger. | A tenant rule as a CHECK; a combined-event trigger; a tenant trigger that rejects NULL. | high |
| ST-045 | `crates/livrarr-db/src/pool.rs:1627-1666`, `:1686-1723`, `:1725-1828`, `:1830-1834` | The article heal mints its cohort cards from the post-fold graph, after all folds and before the winner claims, and writes its marker only when no key is blocked. | Attributing those cards to one fold. | high |
| ST-046 | `git grep backfill_normalized_identity -- crates/ tests/`: `pool.rs:436` (definition), `:256`, `:2346`, `sqlite_work.rs:833`, `038:2` (docs), `pool.rs:2450-3140` (own tests); `main.rs:771-858`; `pool.rs:279`, `:701`; `sqlite_work.rs:835`; `tests/behavioral/test_author_link_credit_gate.rs:1466`, `test_author_link_db.rs:194`, `test_author_link_role_gate.rs:608` | The pre-038 backfill has no production caller (startup never runs it; `idx_works_identity` is created only there and dropped at cutover); `merge_user_identity_state` is called only from that backfill and from the dead `merge_works` — so no reachable merge preserves a loser's user-confirmed legacy anchors today. Three behavioral fixtures call the backfill as test setup. | Converting the dead chains instead of deleting them; deleting them without re-basing the three fixtures. | high |
| ST-047 | `git grep -e worksMerged -e works_merged -- crates/`: definitions/docs at `sqlite_history.rs:46,68`, `entities.rs:169`, `history_events.rs:452-462`; the sole writer `work_service.rs:2307-2319` (dead chain); `handlers/work.rs:1075-1181` | No live absorption writes a `worksMerged` history event; a user has no history anchor for a merge today. | Assuming an existing event undo could attach to. | high |
| ST-048 | Applied `039_work_identity_anchors.sql` (PK `(work_id, anchor_type, anchor_value)`; `confidence IN ('confirmed','pending','superseded')`; `setter`, `set_at`, `superseded_by`; `uniq_primary_confirmed_anchor` on `(work_id, anchor_type) WHERE confirmed`; `work_id → works ON DELETE CASCADE`), `044` (`user_id`). Writers re-read for v11/v12: `crates/livrarr-db/src/sqlite_work_identity.rs:137-145` (`confirm_anchor_in_tx`: `ON CONFLICT DO UPDATE` rewrites `confidence`, `setter`, `set_at`, clears `superseded_by`), `crates/livrarr-db/src/sqlite_identity_conflict.rs:328-330` (KeepExisting/AcceptSeparate restamp `setter`/`set_at` of the confirmed row), `sqlite_work_identity.rs:1843-1844` (`delete_slot_residue` deletes pending rows); supersede-with-successor writers: `sqlite_work_identity.rs:2214-2217` (the identity edit), `sqlite_identity_conflict.rs:350-357` (legacy `ReplaceAnchor`), `:411-414` (legacy `Merge`). r10 census, both families, 79 hits classified: also `identity_layer.rs:2115-2119` (PendingRoute affirm upsert), `sqlite_work_identity.rs:412-417`, `:864-870`, `:1400-1406` (pending upserts rewrite in place), `:1157-1159`, `:2281-2283` (supersede with no successor), the dead chain `pool.rs:330-371`, `:718-731`, and every `DELETE FROM works` cascading the table. | The anchors ledger is **not** append-only: its primary key IS the value, so a re-confirm of the same value must rewrite the row; production writers restamp `set_at`, rewrite superseded rows back to pending, supersede without a successor, delete pending rows, and every Work delete cascades the history. Three writers do record a successor (the identity edit, legacy Replace, legacy Merge); the other classes are enough to make the confirmed value at a past instant NOT reconstructible. | Keying an adopted legacy route-conflict from the ledger; substituting the Work's current anchor for the one it was raised against; any "design will prove append-only" hedge. | high |
| ST-049 | `crates/livrarr-server/src/import_service.rs:496-533` (`reorganize_work_files`), `:568-728` (`MoveMethod` + `reorganize_items`), `:731-799` (`revert_physical_move`); `crates/livrarr-library/src/lib.rs:122-140` (`atomic_copy`); `crates/livrarr-handlers/src/work.rs:1163-1179`; read 2026-08-28 | The forward file reorganization the merge door runs after commit, **on the surviving book only**, over every item that book then owns (loser-origin items, and survivor-origin items whose canonical place changed): computes each item's canonical path from the book's author and title through the import path builder; skips items already there; heals a stale path record when the file already sits at the target (no file operation); checks the destination and, if occupied, leaves the item in place with a warning — but the check and the move are two steps, and the cross-device `atomic_copy` persists a temp file with **replace** semantics, so a file created at the destination between the check and the move IS overwritten; renames, or copies-then-removes across devices — a failed source removal after a successful copy leaves a duplicate that is only **logged**, the item is recorded at the target and no warning is returned; if the path-record write fails the move is reverted best-effort, and when that revert fails too the returned warning names BOTH locations (record at the old path, file at the new) and the next reorganization of that book heals it; returns one warning per item it could not move or record; retags the items it moved. Best-effort and not crash-safe. | Promising atomic no-clobber, guaranteed reversion or "always tracked" from this code as it stands; designing a second move rule; describing the forward move as transactional or crash-safe. | high |

## 0c. Prior Art

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | `wiki/architecture/identity-review-census.md` (re-enumerated at `cd506d15`) + `wiki/insights.md` insight 101 | The one mint authority, the eight doors into the one continuation, the five cancellation writers, the durable Dismiss and its two revocation doors, and the standing warning that the two GroupIdentity buttons corrupt identity until this wave. Every "every card / every door" claim here derives from it. |
| PA-002 | `docs/identity-lifecycle-audit-2026-08.md` + `build/audit-identity-lifecycle/FINDINGS.md` | AUD-P0-2 (IA-C03: "different from all" overwrites), AUD-P0-3 (IA-C04: the loser's concurrent edit is erased), AUD-P1-7 (IA-B03 + IA-F01/F02/F06/F11: the conflict surface and the park list) are this feature's defects. AUD-P1-15 (IA-F03) is not closed here; AUD-P1-20 is accepted for now (§4). |
| PA-003 | `spec-identity-review-fixes.md` v12 §7b | The deferral list this feature collects: the two actions, absorption without archive, one conflict authority, the multi-work generation claim. |
| PA-004 | `build/design/map-identity-conflict-authority.md` | The settled behaviour map of the review surface (informative). |
| PA-005 | `wiki/insights.md` 59, 91, 92, 97, 98, 99 | One matching authority and the contradiction veto (59); decision-time generation (91); semantic pending-card key (92); the Goodreads id taxonomy every route write must follow (97); the split-authority failure shape (98); "is this seam wired" is a caller scan, never test-greenness (99). |
| PA-006 | `~/Projects/kk-build/wiki/lessons/schema-inventory-apply-not-grep.md`, `enumerate-to-the-artifact-boundary.md`, `fold-sweep-must-be-mechanical.md` | Every schema count here is from an applied database; every write list runs to the transaction's commit; every fold is swept by script. |
| PA-007 | `build/reviews/identity-conflict-authority/spec-identity-conflict-authority-v10.md` + `build/design/design-inputs-identity-conflict-authority.md` | The mechanism work of v3–v10 (the decision-token contract, the effect manifest and retention table, the one route plan, the file-transition protocol, the four-phase undo, the journal envelope, the migration internals, the completeness oracle) — carried to the design stage as inputs with the open r9 findings, not re-derived and not deleted. |
| PA-008 | `build/reviews/identity-conflict-authority/review-spec-{openai,xai}-r1..r9.md` | What the nine rounds settled at the WHAT level: a snapshot cannot restore what absorption destroys (r1 P0 → every affected row is recorded); an unsure comparison never creates (r4/r5 → three-valued distinctness); every merge is previewed under the chosen survivor and no provider id is retired unshown (r8); the ledger cannot key adopted conflicts (r9 → Q-025 as the rule). |

## 1. Problem Statement

1. **A review action destroys the book it was asked about** (ST-006, ST-016).
   Answering "this is a different book" writes the incoming book's title,
   author and provider links onto the established book.
2. **Merging discards what you merged in, forces the survivor, and cannot be
   undone** (ST-007, ST-004, ST-005, ST-017, ST-018, ST-010, ST-029, ST-038).
   The survivor is forced to the card's own book, the proposal is never
   applied, the absorbed book is deleted with a sentinel string as its only
   record, a concurrent edit to it is erased without a conflict, two
   contradicting provider ids can end up both active, and no history event
   says a merge happened.
3. **The conflict page cannot act on what it shows, machine conflicts are
   invisible, and the grey-park list has no exit** (ST-001, ST-011, ST-019,
   ST-020, ST-022, ST-023).

## 2. Requirements

### Shared definitions

- **Cohort** of a GroupIdentity card = every Work in the card's `work_ids`
  plus the card's subject Work, deduplicated (ST-035, ST-042). The proposal
  is identity content — a title, an author, routes (ST-016) — never a list
  of members. Mint, deletion and undo all use this one notion of cohort.
- **Route ownership.** A provider id that is active on a book outside the
  action's pair — the survivor and, for an absorbing action, the absorbed
  member — is never taken from that book. Every action that applies a
  proposal, attaches routes or absorbs a book refuses **before any write**
  when such an id is in its way, naming the id, the book that holds it and
  the recourse (the pairwise merge, or the identity edit).
- **Absorption operation** = one survivor absorbing one loser set, all or
  nothing, whoever started it: a user merge, a machine AutoMerge (one
  operation, several losers), or a boot heal (one operation per anchor/orphan
  pair or per winner component — several in one run, ST-029). Undo works
  per operation.
- **The stale-decision rule.** Every action a user takes on a card, and every
  merge execute, is checked against exactly what that user was shown when
  they decided: the card and its version, every cohort member and its
  identity generation, and — for a merge — the elected survivor, both books'
  field values, the displayed field outcome and every provider id the
  preview said would be retired. If any of it changed since, the action is
  refused with the existing stale-decision 409 and **no write**; a later read
  gives the user a fresh view. Item counts, grab counts and the monitor union
  are informational and never part of the check.
- **Retirement.** Only a user's **shown** survivor election ever retires a
  provider id (a route the user had confirmed included, and named as such);
  a machine never picks between two contradicting ids — it refuses and parks
  the question for the user.

- **REQ-002** — The two GroupIdentity actions act on the right book.

  **Roles.** Every GroupIdentity card records whose identity its proposal is,
  at every producer (ST-016; the boot heal; cutover staging):
  `ReIdentify { work, proposer: User | Machine }` (the proposal is the new
  identity of the subject: the title/author edit door, the engine-Review
  arm, the complete-group Review arm when a subject was supplied);
  `Incoming { add_source }` (a book not yet created, colliding with the
  cohort: the complete-group Review arm when it bound the subject itself);
  `MergeMembers` (the merge door); `None` (no proposal: the boot heal and
  cutover staging); and *Unknown* for any card minted before this feature.
  The continuation branches on the stored role only — never on payload
  shape (Q-009). An Unknown-role card refuses both actions by name and stays
  dismissable **cancel-only**: cancelled with a user audit, no standing
  dismissal recorded, nothing suppressed, and a later role-known card for the
  same books is minted unsuppressed.

  **Pair rule.** Every action that absorbs requires a cohort of exactly two
  (Q-011, ST-021); a larger cohort can only take `DifferentFromAll`. Machine
  AutoMerge is not a user action and keeps absorbing every certain member as
  one operation — but a same-provider id contradiction anywhere in it refuses
  the whole component and parks the cohort as a GroupIdentity card instead.

  **Three-valued distinctness** (Q-004, ST-040): a proposal against a member
  is *certainly the same* (`authority_certain`), *certainly different* (title
  `Different` or `VetoVolume`, or author `Disagree`, or a same-provider
  work-key contradiction), or *unsure* (everything else, every Grey title
  cause included). Evaluated at the generations the user was shown, by the
  road's own rule (§0a). Unsure never creates.

  **The matrix (5 roles × 2 actions):**

  | Role | `DifferentFromAll` | `AttachOrMerge { survivor }` |
  |---|---|---|
  | `ReIdentify { W }` | apply the proposal to W: a re-key from OL `A` to OL `B` leaves exactly `B` active (today `B` lands beside `A`, ST-005) and stamps W's distinctness; a proposal id active on another book refuses, naming it (Route ownership); every other member's identity and routes untouched | one member: **refuse** (pair rule). Two members: apply the proposal to the elected survivor the same way — it ends holding exactly the proposal's ids in the slots the proposal fills — and absorb the other, as one operation. Larger: **refuse** |
  | `Incoming { add_source }` | if any member is certainly the same → **refuse**, naming it and offering attach/merge; if any member is unsure → **refuse**, naming it and the reason; only when **every** member is certainly different → **create** the Work from the proposal, all or nothing: its routes attached (a proposal route active on any other book refuses the whole create, naming that book — Route ownership), one `added` history event carrying `add_source`, and the new book **recorded as user-distinguished** from every reviewed member, so no machine pass later merges them (a user merge stays available); every member untouched | one member: attach the proposal's routes to it ("the incoming *is* this book"), no absorb, identity fields untouched; a proposal id contradicting the member's live same-provider id, or held by another book, refuses naming both ids and the recourse. Two members: the elected survivor gets the proposal's routes and the other member's routes, then absorbs the other. Larger: **refuse** (pair rule) |
  | `MergeMembers` | **refuse** by name | one member: **refuse** (pair rule). Two members: the field choices are applied on the elected survivor, the other member's routes come across, then absorb. Larger: **refuse** |
  | `None` | stamp the subject's distinctness (`different:review:<card>`, ST-008), resolve; nothing else changes | one member: **refuse** (pair rule). Two members: absorb the other into the elected survivor with the survivor's fields. Larger: **refuse** |
  | *Unknown* | **refuse** by name | **refuse** by name |

  "Untouched" means identity fields and routes; generation, audit and card
  state do change (ST-009). A refusal leaves the card pending and names the
  recourse. Every cell runs under the stale-decision rule: a change to any
  cohort member since the user's read is a 409 with no write.

  **Survivor election and preview.** Every merge — from the merge page or
  from a two-book card of any known role — lets the user choose the survivor
  (either member), then previews under that choice: both sides; for each
  conflicting field, which book's value is kept (a choice names the **book**,
  never a side, so no election can invert it — a choice naming a book outside
  the pair is a 400); every provider id the choice retires (provider, kind,
  id, whether the user had confirmed it, and why); and what happens to files.
  Nothing merges without that preview; execute applies exactly what was
  previewed and retires exactly the ids it listed; any change to what the
  preview showed — a member's identity, a field value (one-sided values the
  merge adopts without a choice included), a route, the card's version — is
  the stale-decision 409 with no write. Two pending cards over the same pair
  can never act on each other's preview. No card action other than a
  previewed two-member `AttachOrMerge` ever absorbs a book.

  **The resolving card.** The card whose action performed the merge stays
  `resolved` and follows the elected survivor; every other pending card that
  named the absorbed book is withdrawn (REQ-006).

- **REQ-004** — Every absorption is recorded completely and is reversible.

  **One implementation.** After this feature exactly one way to fold a Work
  into another exists (ST-038, ST-046): the dead `merge_works` chain, the
  dead pre-038 backfill and the helper only they share are **removed**, not
  converted (Q-016, Q-022), and the three behavioral fixtures that used the
  backfill as setup are re-based on what production actually runs. That no
  second fold path and no Work deletion outside the delete authority
  (REQ-006) can return is a build-time check derived from ST-038's and
  ST-046's Forbids at the architecture stage (the System-Truth → contract
  bridge), not a runtime behaviour.

  **Every absorption is recorded — user or machine, including both boot
  heals — as one complete record per operation** (Q-006), kept as long as
  the surviving Work exists and cascading with it (Q-003, Q-024). The record
  covers every row the operation changed on either side (ST-017, ST-029,
  ST-030): the survivor's own changes, the absorbed book, its routes
  (including the ones coalesced or retired), every dependent row, the cards
  it withdrew and their notifications, the legacy conflict rows it closed,
  the cutover-staging pointers it moved, and the generations of every member
  before and after. An operation whose effects cannot all be classified
  **refuses to commit** rather than recording part of them. History is never
  deleted by undo: the audit rows, the history events — including one
  `worksMerged` event on the survivor that every operation now writes
  (ST-047) — and the resolving card are kept; undo only restores the pointers
  on them.

  **Concurrency.** Every cohort member is checked against the generation the
  user was shown (a machine caller: the generation it read for that
  operation); a concurrent change to any member collides — the
  stale-decision 409, nothing written — and is never erased (AUD-P0-3). The
  two-step settle-plus-continuation protocol is preserved (ST-009).

  **Undo** restores every affected row of the operation exactly — with one
  stated exception: for each file the merge moved, the item's recorded
  location stays truthful (where the file actually is) until the file step
  moves it back, so the merge's own file moves are never a mismatch — or
  refuses whole naming the first mismatch (a change made after the merge by
  anything other than the merge's own file step) and writes nothing.
  Concretely: the
  absorbed book comes back with its identity, routes (user-confirmed bits
  included), dependent rows, and its own earlier merge records; the survivor's
  rows return to their pre-merge values; the ids the merge retired are active
  again; the withdrawn cards and their notifications come back; a closed
  legacy conflict row or a terminalized staging row is the open question
  again. Cards minted **after** the merge on the merged graph — by a later
  settlement, or by a heal — are withdrawn, and the next pass re-mints
  whatever still holds. Both books' generations advance on undo, so a decision
  taken before the merge and one taken after it are both stale afterwards
  (Q-008). Undo is idempotent: an already-undone record reports success with
  no writes; a partly-completed undo resumes where it stopped and is shown as
  incomplete until it finishes. Undo refuses only for a pre-feature record
  (version 0), a missing or foreign record, or a mismatch.

  **Nested and sibling undo** (Q-008, Q-024). Undoing a later merge that
  absorbed the survivor of an earlier one brings the earlier record back so
  it can be undone too; undoing a later merge onto the same survivor keeps the
  earlier one undoable; an unrelated change to the book between two merges
  makes the earlier undo refuse honestly rather than silently absorb the
  change.

  **A machine merge, undone, stays undone.** Undo of an AutoMerge or a heal
  records the user's decision that the books are distinct (ST-008's
  distinctness mark on each restored book), so no machine path re-merges
  them; a user merge remains
  available (Q-020).

  **Files** (Q-010, Q-026; PO 2026-08-28). Only the merge moves files
  (ST-018): best-effort, after commit, every item the surviving book then
  owns — the absorbed book's items, and the survivor's own whose place
  changed. Undo, after the database restore commits, moves each file the
  merge moved back to its own book's folder the same simple way, under four
  promises — the first three go beyond what the forward code gives today
  (ST-049), and the design is free to give the forward move the same ones:
  (1) **no clobber** — a destination occupied before *or during* the move is
  left byte-identical and the item is reported, never overwritten; (2)
  **never the only copy** — no step, and no failed step, destroys the last
  copy of an item's file; a successful move may remove its source only once
  the destination holds the file; (3) **a truthful record or a named
  mismatch** — after every step the item's record says where its file is, or
  the response names both locations (record and file), the undo stays
  incomplete until a retry repairs it, and the app never reports such an
  item as in order; (4) **per-file report** — every file left in place, every
  leftover duplicate and every mismatch is named with its reason in the undo
  response and wherever the merge is reported. The database restore never
  waits on the file step and is never held hostage to it. An undo whose file
  step is incomplete is reported as such, and a later undo request resumes
  it. No crash-proof or lock-protected file protocol is built in this feature
  (§4).

  **Sibling operations.** Undoing one operation of a heal that ran several in
  one run leaves the others exactly as they were.

- **REQ-006** — One conflict authority for every conflict a user is shown.

  **The authority is the review-card system** (Q-005): a conflict card
  carries the full conflict — kind, incoming payload, source, when and where
  it was raised, the book it was raised on, **the exact provider id it was
  raised against** for a card raised after this feature by a kind that
  implicates one (`QuorumTie` implicates none, ST-024; an adopted legacy row
  carries an explicit *unknown* — below), and for history its status,
  action, notes and timestamps —
  so listing, detail, history, validation and every action read the card
  alone. The two-field pointer card (ST-001) is superseded. Every conflict
  ever presented to a user — the legacy store's rows and every runtime
  producer — is a card; after the cutover the legacy table is read-only
  history; that nothing in the app still reads or writes it — except the
  one history reader — is a build-time check derived from ST-022's Forbids
  at the architecture stage. `identity_conflicts_v2` stays
  **non-authoritative cutover staging** (Q-021): after the cutover its
  pending rows affect no badge, list, detail, suppression, decision or
  resolver; this wave preserves them and counts them, and touches them only
  when a lifecycle event forces it — a pending row whose book is deleted, or
  which becomes self-referential after a merge, is closed into a retained
  diagnostic that undo reopens, and every question is counted once (Q-023).
  Promoting or deleting those rows is wave 3's.

  **Machine conflicts are visible and durable.** Every runtime conflict
  becomes a card at the moment it is raised, never separately (ST-012), and the
  frozen `identity_status = 'conflict'` write stops (ST-020).
  Machine conflict cards are dismissal-keyed (Q-007) so an equivalent one
  cannot come back after a user dismissed it.

  **Card history survives its book.** A pending card always names a live
  book of its user, and every book its cohort names is live and of its user
  (checked at mint; existing pending cards are censused once at the first
  unit, and one that names a deleted, foreign or unreadable member becomes
  non-actionable history with a diagnostic). A resolved or cancelled card
  survives its book's deletion as history.

  **One delete authority.** Every deletion of a Work — the user door, import
  cleanup, list-import undo and absorption (ST-038) — withdraws every pending
  card whose subject or cohort names it (machine actor, audited, never a user
  decision), closes its open legacy conflicts as "work deleted" until the
  cutover (a machine close, never honoured as a user dismissal), closes its
  pending staging rows into retained diagnostics, and only then deletes;
  inside an absorption every one of those is part of the record and comes
  back on undo. Absorption never repoints another book's cards onto the
  survivor (ST-017's card repoint stops); the resolving card is the one
  exception (REQ-002).

  **Lossless legacy mapping** (ST-001): five kinds map to five, nine sources
  to nine, notes and timestamps verbatim; `open` → pending card; `resolved`
  → resolved history card with the recorded action; `dismissed` → cancelled
  card — **plus** a standing dismissal only when the row is keyable (a
  `QuorumTie` row, or a row whose question this feature can identify) and
  the dismissal was a user's on a live book; a dismissed adopted
  route-conflict (non-keyable, below) and a machine close ("work deleted")
  yield history only, each counted separately. Two classes of adopted rows
  are told apart everywhere — in the mapping, on the page, in the counts and
  in the tests: a row whose book is missing or belongs to another user, or
  whose payload cannot be read, is **history only** — no card to act on, no
  buttons, a diagnostic, nothing fabricated; a readable route-conflict row
  that merely cannot prove its contradicted id is **actionable but
  non-keyable**. Every class is counted and reported; none blocks the
  cutover. The cutover runs once, is all-or-nothing, and a second run
  changes nothing; it guarantees exactly one card per legacy row (a row that
  already has pointer cards ends with exactly one; extras and pointer cards
  with no row are cancelled with a diagnostic), and it cannot strand an undo:
  a merge recorded before the cutover that closed a legacy row is still
  undoable afterwards — the reopened question appears as a pending card, and
  the legacy table stays byte-stable.

  **Adopted conflicts are non-keyable** (ST-039, ST-048; Q-025 is the rule).
  A newly raised conflict card persists the exact provider id it was raised
  against and keeps the full action matrix. A legacy row never stored that id
  and the ledger cannot prove it, so every adopted `IncomingDifferent*` /
  `OlRedirectCollision` row is **non-keyable**: its card carries an explicit
  *unknown* contradicted id; its Dismiss is cancel-only (no standing
  dismissal, nothing suppressed); `KeepExisting` and `AcceptSeparate`
  resolve **that card only** — no future suppression, because the question
  cannot be identified, so a later collision of the same incoming with the
  book is a new card; `ReplaceAnchor` / `Merge` refuse, naming the identity
  edit, which shows the user the current ids and is the way to a durable
  outcome. The book's *current* id is never substituted for the one the
  conflict was raised against — that would key a question never asked.
  Adopted `QuorumTie` rows are keyable as before (no contradicted id by
  kind).

  **Action matrix — every cell** (each under the stale-decision rule, each
  writing one resolution audit; only `ReplaceAnchor` and `Merge` change
  routes; `KeepExisting`, `AcceptSeparate` and Dismiss leave the route graph
  byte-identical):

  | Kind | `KeepExisting` | `AcceptSeparate` ("Treat as Separate") | `ReplaceAnchor` | `Merge` |
  |---|---|---|---|---|
  | `IncomingDifferentOlKey` | card resolved; routes untouched; on a keyable card the same question does not re-raise (on an adopted non-keyable card, that card only) | as `KeepExisting` with intent `accept_separate`; the incoming is not created (§4); needs no winning id (ST-023) | incoming OL key present, else refuse; the contradicted route is retired and the incoming OL route attached as user-confirmed; an incoming route active on another book refuses, naming it (Route ownership) | `ReplaceAnchor` **plus** gap-fill: every other present incoming route the book lacks is attached as user-confirmed (one active on another book refuses the whole action, naming it); nothing else retired |
  | `IncomingDifferentGrKey` | same | same | same, for the incoming Goodreads route (edition-homed when it is a Book id — insight 97); exactly the one contradicted route retired | same |
  | `IncomingDifferentHcKey` | same | same | same, for the incoming HC route | same |
  | `OlRedirectCollision` | same | same | retire the contradicted old OL route, attach the redirected id as user-confirmed; an adopted row → the identity-edit refusal | identical to `ReplaceAnchor` plus gap-fill — two active OL Work routes are never created |
  | `QuorumTie` | same | same | gap-fill only (ST-024): attach every present incoming route the book lacks, as user-confirmed; nothing retired; zero present keys → refuse | identical to `ReplaceAnchor` for this kind |

  `Dismiss` (all kinds): a keyed standing dismissal for a keyable card
  (cancelled, ledger row, user audit, 204); cancel-only for any card whose
  question cannot be keyed (an adopted route-conflict card, an Unknown-role
  group card): cancelled, user audit, no ledger row, nothing suppressed. On
  an adopted non-keyable card the matrix reduces to `KeepExisting`,
  `AcceptSeparate` and that Dismiss; `ReplaceAnchor`/`Merge` refuse naming
  the identity edit. A readable payload whose incoming key is absent
  likewise refuses `ReplaceAnchor`/`Merge` and leaves the other three
  available; an unreadable payload is never an actionable card (history
  only, above). A refusal names every book that owns a route in the way and
  both contradicting ids, before any write. A book's other same-provider
  edition routes are never implicated.

  **Request shape.** Every conflict action carries the action and the
  decision proof the user was shown (the stale-decision rule);
  `surviving_routes`, `target_edition` and `winning_work_id` are removed
  (ST-034); extra fields from old clients are ignored and change no outcome.

  **One definition of "the same question", versioned** (ST-013, ST-027,
  ST-032; Q-012). It grows to carry a group card's role, the conflict card (book, kind,
  the incoming ids, the contradicted id or its explicit absence, the
  normalized title tuple, author, year — so the same incoming colliding with
  `A` and later with `B` on one book is two questions), and the park card
  (book plus the candidates' identity content, never their consume-once
  ids); one definition serves reuse, duplicate cleanup, Dismiss, suppression,
  both revocation doors and the cutover. **V1 compatibility** (PO 2026-08-27):
  a pre-feature group dismissal keeps its force only where its shape proves
  its role (a `proposed = None` row matches `None`-role questions); every
  other V1 group row is inert — retained, matching nothing — because its
  question cannot be told apart (ST-031). Cost, stated: such a question may
  appear once more; its re-dismissal writes the role-bearing key. V1
  `PendingRoute` and `EditionEvidence` rows stay live. The migration counts
  and logs the inert rows.

  **The grey-park list gets an exit** (Q-013; ST-019, ST-025): a new card
  kind `IdentityPark` — one card per parked book carrying its persisted
  candidate list; every parked book has exactly one from the moment it is
  parked, and existing parks get theirs at upgrade — with
  `ChooseCandidate` (today's apply-candidate, under the stale-decision rule;
  a candidate's `existing_work_id` stays informational, never an absorb) and
  `DismissPark` (today's revert-to-pending, as a keyed dismissal). The park
  list shows these cards; the typed card routes act on them; the candidates
  table becomes history.

  **`parked_by_conflicts` carries its real value** (ST-011): a read-time
  projection from pending conflict cards, feeding detail, list and the clear
  door.

## 3. UI/Interface Design

Fixed here; detailed at the design stage:

- Every actionable card and the merge preview carry a decision proof; the
  page echoes it with every action. A stale decision is shown as "this
  changed while you were deciding — reload", never as success.
- Every merge — from the merge page or from a two-book review card — lets
  the user **choose the survivor** (either direction), previews under that
  choice, names for each conflicting field the book whose value is kept,
  **lists every provider identity the choice retires** (naming the ones the
  user had confirmed), and says what the choice means for files; nothing
  merges without that preview.
- Undo is offered **where the merge is reported** (the merge response and
  the book's history, which carry the record id), states what it will move
  back, lists the pending questions it will withdraw, and its response lists
  every file left in place with the reason; an incomplete undo is shown as
  such until it completes.
- A refusal states the reason in the user's terms and names the recourse
  (wave 1a's `ConflictCard` precedent); "we are not sure these are different
  books" is a refusal, never a silent create; "two of these books carry
  different <provider> ids" is a parked question, never a silent pick.
- The conflict page's four buttons and Dismiss map one-to-one to REQ-006's
  matrix; Treat-as-Separate asks for nothing beyond the decision proof
  (ST-034). The park list keeps today's choose-a-candidate shape.

## 4. Non-Requirements

- Fix-wave 2, wave 3 (including `identity_conflicts_v2`: its staging
  producer, the promotion or deletion of its pending rows — this wave's
  lifecycle-triggered closes are the stated exception — the unique
  route-ownership index, the version-guard bump), wave 4 (handoffs, ledgers,
  matching; entry A; the latent two-sided subtitle disagreement).
- **A crash-proof or lock-protected file move-back** (PO 2026-08-28, Q-026):
  no transition journal, per-file claims, undo phases or startup
  reconciliation for files. The accepted residual: a crash between a
  move-back and its record write can leave a file under the other book's
  folder with the record naming the old place until the next reorganization
  of that book heals it (ST-049's heal branch); the response names both
  locations (REQ-004 Files). Not accepted, and promised in REQ-004: an
  overwrite, the loss of an item's only copy, or a record reported as in
  order when it is not. A later feature may add the protocol; the v10 design
  of it is preserved in the design inputs.
- **AUD-P1-20** — the title/author edit auto-resolves "different from all"
  inline: accepted for now, pinned as `ReIdentify { proposer: User }` (and
  now a working re-key); wave 4 owns removing it.
- **AUD-P1-15's "2 of 9 kinds" half**: after this feature the UI acts on
  GroupIdentity, PendingRoute, IdentityConflict and IdentityPark;
  EditionEvidence and the five never-produced kinds stay refused.
- Creating the **incoming** of a conflict as a new Work from the conflict
  page.
- N-way *user* merges. `EditionEvidence` continuation/revocation.
  `FieldResolution` / parent-child. v2 REQ-008's CLI. The two-step
  generation protocol (ST-009). Automatic retry of a failed forward file
  move. A machine tie-break between two absorbed members' same-provider ids
  (the merge refuses and parks). A pair-scoped machine-merge veto (Q-020
  keeps ST-008's marker). Running or converting the pre-038 backfill
  (deleted, ST-046; a pre-038 upgrade is wave 3's cutover hardening,
  insight 100).
- No drop of any table; no timer-based cleanup.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Full undo or forensic archive only | resolved | **PO 2026-08-26: full undo.** |
| Q-002 | One design or split | resolved | **PO 2026-08-26: one design**, staged in build units (§7). |
| Q-003 | Undo retention | resolved | **PO 2026-08-26:** the record lives as long as the surviving Work and cascades with it. |
| Q-004 | Uncertain proposal on "different from all" | resolved | **PO 2026-08-26:** refuse; three-valued distinctness — only *certainly different* creates (REQ-002, ST-040). |
| Q-005 | The one conflict authority | resolved | **PO 2026-08-26: the review-card system.** |
| Q-006 | Where the record lives | resolved, narrowed | **PO:** one complete record per merge, in the merge record store. Schema changes are limited to what retention (Q-003), card history (Q-015) and the legacy rebuild need; the design decides the shapes. |
| Q-007 | Machine conflict cards ledger-keyed | resolved | **PO 2026-08-26: yes.** |
| Q-008 | Undo's generation for the restored Works | resolved (PM) | Both books' generations advance on undo so decisions taken before and after the merge are both stale; the survivor's expected generation is the one the merge left, as adjusted by any later undo that touched it; an unrelated change in between makes the undo refuse. Mechanism: design inputs. |
| Q-009 | Whose identity is a proposal | resolved (PM) | persisted role at every producer; *Unknown* → refuse. |
| Q-010 | Does undo move files back | resolved | **PO 2026-08-27: yes**, guarded per file, warn on each left. **PO 2026-08-28: by the same rule the merge moves them forward — simply** (REQ-004 Files, Q-026). |
| Q-011 | Merge cohort size | resolved | **PO 2026-08-27: pairwise** with survivor election for user actions. |
| Q-012 | Dismissal key V1 → V2 | resolved | **PO 2026-08-27: stop honouring a dismissal whose question cannot be told apart.** Only `proposed = None` V1 group rows keep a role; all other V1 group rows are inert (REQ-006). |
| Q-013 | Grey-park exit | resolved | `IdentityPark`; key over candidate identity content. |
| Q-014 | Journal placement and enforcement | resolved (PM) | Behaviour: every absorption is recorded whole or refuses to commit; the record is per operation and covers every affected row (REQ-004). Mechanism: design inputs. |
| Q-015 | Card history vs Work deletion | resolved (PM) | Behaviour: a pending card always names a live book of its user; resolved/cancelled cards survive their book as history; one delete authority withdraws pending cards (REQ-006). Mechanism (FK shape, the tenant triggers, ST-044): design inputs. |
| Q-016 | Three absorption implementations | resolved (PM) | one: the dead `merge_works` chain, the dead pre-038 backfill (ST-046) and their shared helper are removed; nothing is converted; a build-time check derived from ST-038/ST-046 pins it. |
| Q-017 | Notifications under undo | resolved (PM) | notifications are restored with the cards they name; never a dangling actionable notification after undo. |
| Q-018 | Two absorbed members, different same-provider Work ids, survivor has none | resolved (PM, v7; reviewers split) | **refuse the component before any write; park for the user** — not a tie-break (insight 98's park-never-fold, insight 59's contradiction veto). PO may flip; cost: the user resolves it pairwise through the merge door. |
| Q-019 | Merge field choices under survivor election | resolved (PM, v7) | a choice names the **book whose value is kept**, never a side; the preview under the election binds it and execute echoes it. |
| Q-020 | What an undo of a *machine* merge means for the next machine pass | resolved (PM, v8; reviewers split) | the user's decision that the books are distinct: ST-008's marker on every restored book, so no machine path re-merges them; a user merge remains available. Cost, stated: the restored book thereafter parks for review instead of auto-merging with a future certain duplicate. PO may flip. |
| Q-021 | Pending `identity_conflicts_v2` rows vs "one authority" | resolved (PM, v7; exception stated v10) | the invariant covers every conflict ever shown to a user; v2 is non-authoritative staging, counted and reported at the cutover, wave 3's — except the two lifecycle-triggered closes (book deleted; self-conflict after a merge), which this wave owns, each counted once. |
| Q-022 | The pre-038 backfill as a fifth caller | resolved (PM, v8) | it has **no production caller** (ST-046) — deleted, not converted; a pre-038 upgrade is wave 3's cutover hardening (insight 100). |
| Q-023 | Pending `identity_conflicts_v2` rows vs Work deletion or absorption before the cutover | resolved (PM, v9) | a row whose book is deleted, or which becomes self-referential after a merge, is closed into a retained diagnostic that undo reopens; a row pointing at a third book stays pending; the cutover counts every question once. |
| Q-024 | An `undone` prior record whose survivor is later absorbed as a loser | resolved (PM, v9; PO may flip) | its record cascades with the survivor, intentionally (retention is the survivor's lifetime), and the cascade is part of the new merge's record, so undoing the new merge brings it back. Cost, stated: undo history of an already-undone merge is gone once its survivor is merged away, unless that merge is undone. |
| Q-025 | Adopted legacy conflicts whose contradicted id cannot be proved | resolved (PM v10; **the rule as of v11**, PO accepted in substance 2026-08-28) | the ledger is not append-only (ST-048), so **every** adopted route-conflict is non-keyable: Keep / Treat-as-Separate / Dismiss stay on the page; Replace / Merge go through the identity edit, which shows the current ids. Newly raised cards persist the contradicted id and keep the full matrix. Keyability for adopted rows would need a new immutable anchor-event history whose deployed provenance is itself proved — not this wave. |
| Q-026 | Crash-proof file move-back inside this feature | resolved | **PO 2026-08-28: no** — undo moves files back the same simple way the merge moves them forward, per-file report; the crash residual (wrong folder until the next heal, both locations named) is accepted; the no-clobber, only-copy and truthful-record promises (REQ-004 Files) are not that protocol and are in scope; the v10 protocol is preserved in the design inputs for a later feature. |

## 6. Acceptance Criteria

Each criterion exercises the **real** production entry path (`CLAUDE.md`
Code Stage Gate) and pins the **named observable**, never a whole state
snapshot. "No write" means the affected tables' row counts and the named
rows' values are unchanged; "409" is the stale-decision or refusal response
with no write. Tree references are at `507f2ac2`. Where a criterion says
"table-driven", one test row exists per cell named.

- [ ] **AC-001** (REQ-002): the 5×2 matrix, table-driven, every cell over its legal cohort sizes, through `resolve_review` (the title/author edit door, `handlers/work.rs:841-874`, and the merge door are two producers of the cards): `ReIdentify × DifferentFromAll` on W (OL `A`) with a proposal carrying OL `B` → `A` retired, `B` the only active OL Work route on W, W's title/author as proposed, every other member's identity fields and routes byte-identical (today's tree attaches `B` beside `A`); `ReIdentify × AttachOrMerge` with one member → 409 (pair rule), with two members and the *other* member elected → that member ends holding exactly `B` (no `A`) and W is absorbed, with three → 409; `Incoming × DifferentFromAll` → AC-002; `Incoming × AttachOrMerge` with one member → AC-003, with two → the elected survivor holds the proposal's routes and the other member's and the other is absorbed, with three → 409; `MergeMembers × DifferentFromAll` → 409 by name; `MergeMembers × AttachOrMerge` with one → 409, with two → the field choices applied on the elected survivor and the other absorbed, with three → 409; `None × DifferentFromAll` on a boot-heal card (no proposal) → the subject's `text_distinction` reads `different:review:<card>`, the card is resolved, and no other row changed; `None × AttachOrMerge` with one → 409, with two → the other absorbed into the elected survivor with the survivor's fields, with three → 409; *Unknown* × both → 409 by name. Every 409 above leaves the card pending with no write.
- [ ] **AC-002** (REQ-002): `Incoming × DifferentFromAll`: an *unsure* member (a Grey title cause) → 409 naming it and the reason, no Work created; a *certainly same* member → 409 naming it and offering attach/merge; three members of which two are certainly different and one certainly same → 409 (every member is checked before a create); every member certainly different → exactly one new Work with the proposal's routes, one `added` history event carrying the card's `add_source`, every member untouched — and the new book is **user-distinguished**: a machine pass that would otherwise call it certainly the same as a reviewed member leaves both books separate (parked or untouched, never absorbed), while a user merge of the two succeeds.
- [ ] **AC-003** (REQ-002): a one-member `Incoming` attach → the proposal's routes on the member, identity fields untouched, no absorb; a one-member attach whose proposal id contradicts the member's live same-provider id → 409 naming both ids and the identity edit, no write.
- [ ] **AC-004** (REQ-002): both actions on an Unknown-role card → 409 naming the reason, card still pending, no write; Dismiss on it cancels the card with a user audit, writes no ledger row and fabricates no role, and a later role-known card for the same books is minted unsuppressed.
- [ ] **AC-005** (REQ-002): for each of `ReIdentify`, `Incoming`, `MergeMembers` and `None`, a two-member card previewed under **both** elections lists every provider id the election retires (provider, kind, id, user-confirmed flag, reason); `AttachOrMerge` executed under one election absorbs the other member into exactly that survivor, retires exactly the listed ids, and — for `ReIdentify` and `Incoming` — leaves the survivor holding exactly the proposal's ids in the slots the proposal fills; executed with the other election's preview, with no preview, or after a member, a field value, a route or the card's version changed → 409, no write.
- [ ] **AC-006** (REQ-002): merge door: preview S←L with S holding OL `B` and L holding user-confirmed OL `A` → exactly one retirement listed (OL, `A`, user-confirmed); execute retires only `A`; the same pair previewed with L elected lists `B`'s retirement instead; preview with conflicting series values, choose L's value, execute with the choice pointing at S → 409 before any write; a choice naming a Work outside the pair → 400.
- [ ] **AC-007** (REQ-002): the stale-decision rule: client A reads a two-member card, client B changes one member, client A submits → 409, card still pending, no write; an older decision does not fail merely because another client read the card; after a preview, adding a loser item and flipping a monitor flag does **not** stale the decision, while editing a one-sided series value on either book does.
- [ ] **AC-008** (REQ-002, REQ-006): machine AutoMerge with a route-less survivor and losers holding distinct OL `A`/`B`, and the settlement, article-heal and dedup-heal doors each with the survivor holding OL `B` and the loser OL `A` → every door refuses atomically: both Works unchanged, no record written, the AutoMerge cohort parked as a GroupIdentity card, the heal component reported unfolded; the same pair through the merge door with S elected succeeds with `A` listed and retired.
- [ ] **AC-009** (REQ-004): after the first unit, the three behavioral fixtures that used the pre-038 backfill as setup keep their assertions and pass against production startup alone; the absence of a second fold path and of any Work deletion outside the delete authority is a build-time check derived from ST-038's and ST-046's Forbids at the architecture stage; the first-unit-only refusal is AC-029.
- [ ] **AC-010** (REQ-004): for each of the four absorbing doors — the merge door, a settlement AutoMerge, the dedup heal with two pairs in one run, the article heal with two winner components in one run — with member-owned rows seeded in every one of the 37 closure tables on both sides plus a third-Work relationship row, a pending side-effect card and its notification, an open and a closed legacy conflict row of the loser, and a pending staging row naming both members: the door commits with one record per operation and one `worksMerged` event on the survivor carrying the record id; undo of the first operation returns every affected row to its pre-merge value except both books' generations (advanced), the restored loser's distinctness marker on the machine doors, history rows (kept, pointers restored), and — on the merge door with one moved file — the item's recorded location, which after the database restore still names where the file actually is (AC-017); the sibling operation is untouched; the marker and the heal's post-fold writes are outside the record; the record reads `undone`.
- [ ] **AC-011** (REQ-004): a merge whose effects include a row the classification does not cover (a test-only table with a `works` FK, a test-only trigger on a closure table, a Work-pointer column the registry does not know) refuses to commit, naming it; once classified, it commits and undoes.
- [ ] **AC-012** (REQ-004): the loser is edited between the user's read and the merge → 409, nothing absorbed, no record; the same for an edit to the survivor.
- [ ] **AC-013** (REQ-004): undo after a merge when a restored row changed since (the survivor's title edited) → refuse whole naming that row, nothing restored, the record still `active`; the merge's own file moves (an item's path changed by the merge's file step) are never such a change; the identical undo with no interference completes; a second undo of an `undone` record reports success with no writes.
- [ ] **AC-014** (REQ-004): decisions read before and after a merge are both stale after its undo; fresh reads show both books' generations advanced; cards minted on the merged graph after the merge (a later settlement's card on the survivor; the article heal's post-fold cohort card) are cancelled by the undo while a genuinely pre-merge card of a member is restored and stays pending.
- [ ] **AC-015** (REQ-004): nested A←B then C←A, undo C←A → A and the A←B record are back and undo A←B then completes; the same with an unrelated generation bump of A between the two merges → undo C←A restores A and undo A←B refuses naming the generation; siblings A←B then A←C, undo A←C → undo A←B then completes; A←B, undo, then C←B commits and undoes with the undone A←B record untouched; A←B, undo, then C←A → the undone A←B record cascades with A, is part of C←A's record, and returns when C←A is undone.
- [ ] **AC-016** (REQ-004): after undoing a machine merge A←B, presenting B with a certainly-same C parks a review instead of auto-merging (Q-020's stated cost), and a user merge of B and C succeeds.
- [ ] **AC-017** (REQ-004): merge door, one file moved into the survivor's folder (record updated), then undo → 200, the file is back at the restored book's canonical path, the record agrees, no file listed; a merge whose field choices change the survivor's title (so its own files moved forward too) → undo moves the survivor's files back as well; a foreign file at the destination **before** the move-back → it stays byte-identical, the merged-in file stays where the merge put it, its record still points at it, and the response names it with the reason; a foreign file created at the destination **after** the occupancy check and before the move → the same outcome, never an overwrite; a cross-device (copy-then-remove) move undoes the same way, and a copy whose source removal fails leaves the leftover duplicate **named in the response** with the record pointing at the destination; a path-record write that fails together with its revert → the response names both locations, the undo is reported incomplete, and a retry repairs the record or completes the move; in none of these does any step destroy the only copy of an item's file.
- [ ] **AC-018** (REQ-004): an undo interrupted after the database restore and before the file step, resumed → the file step completes and the record reads `undone`; until then the merge's history shows the undo as incomplete.
- [ ] **AC-019** (REQ-006): every runtime conflict producer yields a conflict card at the moment it raises the conflict (a raise with no card, or a card with no raise, never occurs), carrying the exact contradicted route; the frozen `identity_status = 'conflict'` write no longer occurs; a newly raised card resolved with `ReplaceAnchor` retires exactly that route; an adopted card's contradicted route reads *unknown*, never the current anchor.
- [ ] **AC-020** (REQ-006): the 5×4 conflict matrix, table-driven through the production conflict door, each cell asserting the exact route delta, card state, audit, intent and generation: `KeepExisting` on all five kinds → route graph byte-identical, card resolved, and on a keyable card the same question raised again is suppressed; `AcceptSeparate` on all five → the same with intent `accept_separate` and no winning id sent; `ReplaceAnchor` on `IncomingDifferentOl/Gr/HcKey` → exactly the contradicted route retired and the incoming route attached user-confirmed (the Goodreads one edition-homed; on a Work with two Goodreads edition routes exactly the contradicted one retired), nothing else; on `OlRedirectCollision` → the old OL route retired and the redirected id attached, never two active OL Work routes; on `QuorumTie` → every present incoming route the book lacks attached, nothing retired, and zero present keys → refuse; `Merge` on `IncomingDifferentOl/Gr/HcKey` → the `ReplaceAnchor` delta plus every other present incoming route the book lacks attached user-confirmed (an Incoming OL card with an extra present GR → GR attached, only the OL retired), nothing else retired; on `OlRedirectCollision` → identical to its `ReplaceAnchor` plus gap-fill; on `QuorumTie` → identical to its `ReplaceAnchor`; and a request carrying the removed legacy fields produces the same outcome as one without them.
- [ ] **AC-021** (REQ-006): cutover fixtures: open, resolved and dismissed legacy rows of every kind and every one of the nine sources, a cross-tenant row, an orphan row (book deleted between the first unit and the cutover), an unreadable row, and one keyable open row seeded with zero, one and two pointer cards → the marker is set with the per-class counts; exactly one pending card per open row and per key; resolved rows are resolved history; notes and timestamps are verbatim on every card; a user's dismissal of a live book's keyable row yields a cancelled card plus a standing dismissal; a user's dismissal of an adopted route-conflict row yields a cancelled card and **no** standing dismissal, counted separately; a "work deleted" close yields history and **no** standing dismissal, and a later genuine raise of the same incoming against the restored book is not suppressed; cross-tenant, orphan and unreadable rows are history only with a diagnostic, no pending card and no actions; a readable adopted route-conflict row yields exactly one actionable card with `KeepExisting`, `AcceptSeparate` and Dismiss; extra pointer cards are cancelled with a diagnostic; a second run of the cutover changes nothing (every table byte-identical).
- [ ] **AC-022** (REQ-006): adopted route-conflict rows seeded across the reachable ledger histories (a re-confirm restamp, a pending upsert, a cleared slot, a deleted pending row, a Work-cascade) → none is keyed, the current anchor is never substituted, the report states the non-keyable count; on such a card Keep / Treat-as-Separate / Dismiss work, Dismiss writes no ledger row, and Replace / Merge → the refusal naming the identity edit with every route unchanged; raise and dismiss C-versus-A, edit the Work A→B, then cut over → the adopted row is never keyed C-versus-B and never suppresses a later real C-versus-B card; Keep or Treat-as-Separate on an adopted card, then edit the Work A→B, then raise C-versus-B → the new card is not suppressed.
- [ ] **AC-023** (REQ-006): a book with an open legacy conflict is absorbed before the cutover; the cutover runs; undo → exactly one pending card names the restored book and the original question, the legacy table receives no post-cutover write, and no cancelled adoption card suppresses it.
- [ ] **AC-024** (REQ-006): a pending staging row with current = S and proposed = L, then L absorbed into S → the row is no longer pending but one counted retained diagnostic carrying the original pair, and undo restores the pending pair; a staging row whose book is deleted through each non-absorption door yields one counted diagnostic; a row pointing at a third book stays pending; the cutover report counts diagnostics and remaining pending rows once each; after the cutover a seeded pending staging row changes nothing on the review badge, the review list, the conflict list and detail, the suppression of a new equivalent card, any resolution, and the resolver — each checked.
- [ ] **AC-025** (REQ-006): each of the four delete doors, deleting B where a pending card scoped to A names `[A, B]` (the set-based doors with B inside a larger set) → the card is cancelled with a machine audit before the delete and no pending card names a dead Work afterwards; deleting a book sets its resolved/cancelled cards' Work pointer to NULL and they remain readable as history; a pending card that at the first unit names an already-deleted or another user's book is non-actionable history and no projection names that book.
- [ ] **AC-026** (REQ-006): after the cutover, every conflict door (list, detail, the four actions, Dismiss), every absorption and every Work deletion leaves the legacy table byte-identical; that no production statement other than the one history reader names the table is a build-time check derived from ST-022's Forbids at the architecture stage.
- [ ] **AC-027** (REQ-006): the same incoming route colliding first with existing `A` and later with existing `B` on one book produces two different dismissal keys, and dismissing the `A` question does not suppress the `B` question; a V1 group dismissal with `proposed = None` still suppresses a `None`-role question, while every other V1 group row suppresses nothing and is counted as inert.
- [ ] **AC-028** (REQ-006): every parked book has exactly one `IdentityPark` card; `ChooseCandidate` applies the chosen candidate's anchors under the stale-decision rule; `DismissPark` reverts the book to pending and a later equivalent park is suppressed; the park list shows the cards and the candidates table is no longer read for decisions; `parked_by_conflicts` on a book with a pending conflict card reads `true` in detail and list and drives the clear door.
- [ ] **AC-029** (REQ-004, REQ-006): a build carrying the first unit's schema and contracts without the second unit's recording refuses every absorption door by name with zero writes; the first releasable prefix (both units) serves merges only with recording and the one-id-per-provider rule active; a release/upgrade test per supported unit prefix boots the exact nonempty upgrade fixture through the full production boot seam and fails closed on any other combination.
- [ ] **AC-030** (REQ-002, REQ-006): Route ownership: for `ReIdentify × DifferentFromAll`, `ReIdentify × AttachOrMerge`, one- and two-member `Incoming × AttachOrMerge`, `Incoming × DifferentFromAll` (create), and conflict `ReplaceAnchor` / `Merge` gap-fill, a proposed or incoming route that is active on a third Work → 409 naming that route, that Work and the recourse; all three Works, their routes, the card and every generation unchanged.
- [ ] **AC-031** (REQ-004, REQ-002): deleting the surviving Work through the user door cascades its merge records while an unrelated Work's records remain; a merge whose card subject is the absorbed member (the other member elected survivor) leaves that card `resolved` and pointing at the elected survivor.
- [ ] **AC-032** (REQ-006): Q-007: dismiss a newly raised machine conflict card; the exact same question raised again by the machine mints no new pending card (suppressed); the same incoming against a different contradicted id mints a new card.

## 7. Build order

One design; the PO's "one design in units" (Q-002). Units land and verify
separately; the first two ship in **one** release.

| Unit | Delivers (behaviour) | Why here |
|---|---|---|
| **B0 — shared contracts, schema, dead surface** | the record store's retention and the card/legacy-table history shapes (Q-003, Q-015); the one delete authority; the decision proof on every card read, the merge preview and every action; roles stamped at every producer; the versioned dismissal key with the V1 rule; the conflict-card payload; the `IdentityPark` kind (refused until B4); deletion of the dead chains with the three fixtures re-based; the census of existing pending cards; **and a gate that makes a B0-only build refuse every absorption by name** | every shared representation lands once; B0 alone is not releasable and the gate makes that checkable rather than trusted |
| **B1 — the recorded operation** | every absorption — user, machine, both heals — is recorded whole or refuses; the one-id-per-provider rule in every operation (a machine contradiction refuses and parks; a user pair without a preview refuses like a machine); cards withdrawn instead of repointed; every operation writes its `worksMerged` event | a deliberate behaviour change; after it no absorption commits unrecorded and no door leaves two active same-provider ids |
| **B2 — the two actions** | the 5×2 matrix; roles, three-valued distinctness, the pair rule; survivor election and the preview for the merge door and every two-book card; Work-keyed field choices; the stale-decision rule end to end | the data-corruption fix |
| **B3 — undo** | database restore with nested/sibling undo and the machine-merge distinctness marker; the withdrawal of post-merge cards; the file move-back by the forward rule; the undo door and its surfacing | needs B1's record and B2's settled shape |
| **B4 — conflict adoption** | the cutover (lossless mapping, one card per row, undo not stranded, the staging count); the action matrix with adopted rows non-keyable; `parked_by_conflicts`; the `IdentityPark` producer and continuation; the post-cutover guard | largest migration risk; safe last because B1 records legacy-row closes and B0 removed both cascades |
