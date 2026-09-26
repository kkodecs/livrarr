---
feature: card-edits-lift
stage: spec
status: draft
version: 5
type: bugfix
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005]
---

# Bug Spec: card-edits-lift

## Executive summary

Two things the user cannot do today come back, one defect is fixed, and one
card answer is removed. A user can again change a book's title or author, and
can answer "different book" on a review card. Answering "different book" must never copy the card's proposed
title, author or provider ids onto an existing book, which the old code did (defect
D1). The card's "same book / merge" answer is removed for good: cards offer
"Different book" and "Dismiss". Manual merging, automatic merging and the two
startup clean-ups stay off. There is no migration.

- **Main risk for the next stage.** Today the title/author edit applies the user's
  new values *by answering "different book" on its own one-book card*. That is the
  same code path as D1. Fixing D1 without giving the edit another way to apply its
  values would break the edit. The acceptance criteria pin both, so a design that
  fixes one by breaking the other fails its tests ([ST-004](#0b-system-truths)).
- **Checked, with controls.** The edit's card carries no provider ids
  (ST-003, `CONTROLS.log` C1). Writing the user's own title and author onto the
  user's own book is the intended edit.
- **Saving the edit dialog must save everything the user changed.** The dialog
  sends title, author, series and monitor flags together. The current title/author
  path returns before saving series and monitor changes (ST-011). A successful Save
  must persist every submitted change. An author-only edit must keep the stored
  subtitle and volume (ST-012).
- **Duplicate edits are refused (PO decision, 2026-09-25).** An edit that would
  give a book the exact title, subtitle, volume and author of another of the
  user's books is refused before any write, with a clear 409 message that the
  dialog shows (REQ-002, AC-023).
- **Added by the PO (2026-09-24): the review card shows what the book is being
  compared with**, at least the proposed title and the proposed author's name. The
  card data already holds the proposed title; it holds only the author's id
  (ST-013). Display only.
- **Four points for the PO stay parked, none blocking** ([§5](#5-open-questions)).
  The most important: re-adding a book after answering "different book" may just
  produce another card instead of creating the book (inferred, not tested).

Sections: [what is broken](#1-problem-statement), [requirements](#2-requirements),
[the card after the fix](#3-uiinterface-design), [acceptance criteria](#6-acceptance-criteria).

## Revision

v2, 2026-09-24: folds Astra R1 (R-1..R-7); adds card comparison display (PO approved).
v1 is kept at `build/reviews/card-edits-lift/spec/fold-r1/spec-card-edits-lift-v1.md`.

v3, 2026-09-25: REQ-002 permits a distinction marker only when the edit would otherwise duplicate another Work (PM decision on build/BLOCKED.md, option A-prime).
v2 is kept at `build/reviews/card-edits-lift/build/spec-card-edits-lift-v2.md`.

v4, 2026-09-25: collision edits are refused (PO decision 2026-09-25, option B); replaces v3's distinction-marker rule. Adds ST-014 and AC-023.
v3 is kept at `build/reviews/card-edits-lift/build/spec-card-edits-lift-v3.md`.

v5, 2026-09-25: an author edit applies the typed spelling by renaming the resolved Author (PO decision after live check).
v4 is kept at `build/reviews/card-edits-lift/fix-r2/spec-card-edits-lift-v4.md`.

## 0a. Design Principles

- Never rewrite an existing book from a review answer the user did not mean as
  "change this book". "Different book" means *keep them apart*, nothing more.
- A user edit is the user's own statement about the user's own book. It applies
  in full or not at all, and it leaves no question behind for the user.
- Merging stays refused, explicitly and before any write. Refusals keep the
  existing refusal class; nothing new is invented.
- Keep the change to what the PO approved on 2026-09-24. No migration.

## 0b. System Truths

No external provider, indexer or download client is touched. The rows below are
facts about the current code on `main` at `a31fc998` that the design must respect.

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | `crates/livrarr-db/src/identity_layer.rs:2004-2041`, `:2042-2065` (read this session) | "Different book" on a card with a proposal writes the proposal's title, subtitle and author onto the card's anchor Work, sets its contributor, and inserts the proposal's provider ids as active routes (D1). Without a proposal it only records the distinction: `:2101-2112` sets `works.text_distinction` to `different:review:<card id>`. | Treating the with-proposal branch as the correct "different book" behaviour. | high |
| ST-002 | `crates/livrarr-db/src/identity_layer.rs:1904-1910`, `:1945-1947`, `:2198-2214`; `crates/livrarr-metadata/src/identity_road.rs:851-856` | A GroupIdentity card's actionable generation is the anchor Work's *current* generation. The HTTP list and the command-line ShowReview return that value (`identity_layer.rs:1711-1716`, `:1686-1693`). The command-line ListReviews does not: it returns the card's stored mint generation (`crates/livrarr-server/src/identity_layer.rs:769-772`). Resolve refuses with "stale identity generation" when the request's generation differs, then advances the anchor's generation by one. Only the anchor's generation is checked. | Checking the card's mint-time generation (insight 91). | high |
| ST-003 | `crates/livrarr-handlers/src/work.rs:1138-1157`; `crates/livrarr-metadata/src/identity_road.rs:142-152`, `:212-218`, `:1099-1128`; `crates/livrarr-domain/src/identity_layer/dismissal.rs:181-204`. Controls: `build/reviews/card-edits-lift/spec/CONTROLS.log` C1a–C1e | The title/author edit's card is a one-book GroupIdentity card whose proposal is the requested title and author with **no provider ids**. The edit door passes an empty provider list; card routes are built only from that list; a reused pending card must match on routes too. | Asserting provider ids can reach an existing Work through the edit. | high |
| ST-004 | `crates/livrarr-handlers/src/work.rs:1157-1171`; ST-001 | The edit's second step, resolving its own card with "different book", writes the new title and completes the title/author pair, through the D1 write. The first step has already written the author (ST-005). | A D1 fix that silently disables the edit, or an edit fix that keeps D1. | high |
| ST-005 | `crates/livrarr-metadata/src/identity_road.rs:333-339`, `:379-385`; `crates/livrarr-db/src/identity_layer.rs:722-736`; CONTROLS C7, C8 | The edit runs as two separate commits. The first commits the card, advances the generation and already writes the **requested author** but keeps the old title. The second ("different book") writes the title. If the second fails, the Work keeps the new author and old title, and the card stays pending. | Pinning only the happy path. | high (code read; failure path not exercised) |
| ST-006 | `crates/livrarr-domain/src/identity_layer/services.rs:655-672` | Every stored title is split: text after the first colon becomes the subtitle; trailing parentheticals leave the main title. An edit to "A: B" stores title "A", subtitle "B". | Asserting the stored title equals a submitted string that contains a colon. | high |
| ST-007 | `crates/livrarr-db/migrations/082_identity_layer_foundation.sql:193`; `crates/livrarr-db/src/sqlite_work.rs:880-889`; CONTROLS C3 | Deleting a Work deletes the cards anchored on it. A card that lists the deleted Work as another member stays pending, still listing it. | Assuming a deleted duplicate cleans up its cards. | high |
| ST-008 | `crates/livrarr-db/src/identity_layer.rs:4182-4187`; CONTROLS C5 | "Different book" does not re-check that the card's other Works or proposed provider ids still exist or belong to the group; only the merge answer does. | Relying on proposal re-validation for "different book". | high |
| ST-009 | `crates/livrarr-domain/src/identity_layer/services.rs:56-57`, `:629-632`; `crates/livrarr-handlers/src/identity_layer.rs:137-140`; `crates/livrarr-handlers/src/types/api_error.rs:453` | The containment refusal is HTTP 409 with reason "Merging is currently unavailable.", raised before generation or action checks. The command-line resolve maps it to a generic database-class error (`crates/livrarr-server/src/identity_layer.rs:971-992`). | Inventing a new refusal class for the merge answer. | high |
| ST-010 | `crates/livrarr-domain/src/identity_layer/reconciliation.rs:246-252` | When the incoming candidate carries no distinction of its own, a group that contains a Work with a recorded distinction is sent to review, not auto-attached. A candidate that carries its own distinction is created instead (`:249-250`). | Assuming "different book" makes a later add of the same title create a new Work (see Q-002). | medium (read, not run) |
| ST-011 | `frontend/src/pages/work-detail/components/EditModal.tsx:58-66`, `:49-53`; `crates/livrarr-handlers/src/work.rs:1112-1118`, `:1176`, `:1179-1193`; containment diff `cc949b35` | The edit dialog always sends title, author, series name, series position and both monitor flags, then reports success and closes. The title/author path returns before the ordinary update that saves series and monitor fields. Only the containment code, which drops unchanged title/author, lets an ordinary Save reach that update; it did not exist before containment. | A Save that reports success but drops a submitted field; losing the unchanged-field handling when the switch goes. | high (code read, not run) |
| ST-012 | `crates/livrarr-handlers/src/work.rs:1120`; `crates/livrarr-metadata/src/identity_road.rs:919-927`; `crates/livrarr-domain/src/identity_layer/services.rs:646-672`; `crates/livrarr-db/src/identity_layer.rs:2017-2031` | An author-only edit sends the stored display title back through the title split (ST-006) without the stored subtitle or volume. The "different book" write then stores the split's empty subtitle and volume. A Work "Alpha", subtitle "Beta", volume 2 would lose both. | Fixtures with plain titles only for author edits. | high (code trace, not run) |
| ST-013 | `crates/livrarr-domain/src/identity_layer/services.rs:267-277`; `crates/livrarr-domain/src/identity_layer/matching.rs:78-82`; `crates/livrarr-domain/src/identity_layer/title.rs:14-22`; `frontend/src/types/api.ts:477-480` | The card list returns each GroupIdentity card's stored proposal: its title (main, subtitle, volume), the proposed author's **id** and its provider routes. It carries no author name. The frontend types the proposal as `unknown`. | Assuming the author name is already on the card; a new card kind for display. | high |
| ST-014 | `crates/livrarr-db/src/identity_layer.rs:3089-3092`, `:750`, `:3228-3230`; `crates/livrarr-metadata/src/identity_road.rs:1309`; `crates/livrarr-handlers/src/work.rs:886`; `crates/livrarr-metadata/src/author_service.rs:92-117` | Production has a unique index on (user, normalized main, subtitle, volume, primary author, distinction). A settlement that breaks it maps to a generic server error (500) today. The author name lookup the edit uses writes even when the author exists: it updates the author row and re-arms its link task. | A duplicate edit surfacing as a 500; a "before any write" refusal placed after the author lookup. | high (code read) |

## 0c. Prior Art

Searched `git grep -il 'GroupIdentity\|DifferentFromAll\|merge' docs/ wiki/` (74 files;
most "merge" hits concern metadata merging) and narrowed to GroupIdentity /
DifferentFromAll / "different book" (17 files). Rows with bearing:

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | `wiki/decisions/merge-undo-rewrite-dropped.md` (amended 2026-09-24) | Scope source: fix D1, edits back on, "different book" back on, merge answer removed, machine merging and clean-ups off, no migration. |
| PA-002 | `spec-merge-containment.md` REQ-001–REQ-003, ST-002 | The contract this feature narrows. ST-002: one-member GroupIdentity actions can rewrite the existing book; REQ-002 refused edits before creating a card. |
| PA-003 | `build/reviews/identity-conflict-authority/assessment-20260923/ASSESSMENT.md` §1 | Names D1 and the claim that the edit only reaches D1 (~85%). Re-derived here on `main` (ST-001, ST-003). |
| PA-004 | `wiki/architecture/identity-review-census.md:93-99`, `:182-192` | Lists the three review doors (two HTTP aliases, CLI) and the edit door that "resolves it DifferentFromAll in the same request"; records D1 as known-broken. |
| PA-005 | `docs/identity-lifecycle-audit-2026-08.md:103-104` (AUD-P0-2) | Original finding: "DifferentFromAll edits the wrong work". |
| PA-006 | `wiki/domain/work.md:85-87` | Documents applying a one-Work proposal to the existing Work as *intended* for the add-dedup review. After this fix nothing applies such a proposal (Q-003). |
| PA-007 | `wiki/insights/history-and-review.md` insights 91 and 101 | Generation rule (ST-002) and "refuse by name, before any write" (ST-009). |

## 1. Problem Statement

**What is broken.** Three things, all behind one switch since 2026-09-07
(`crates/livrarr-domain/src/identity_layer/services.rs:53`):

1. *Changing a book's title or author.* `PUT /api/v1/work/{id}` with a changed
   title or author returns 409 "Merging is currently unavailable."
   (`crates/livrarr-handlers/src/work.rs:1093-1111`). The edit dialog makes both
   fields read-only (`frontend/src/pages/work-detail/components/EditModal.tsx:77`, `:83`, `:86`).
2. *Answering a review card.* Every GroupIdentity answer is refused with the same
   409 (`crates/livrarr-domain/src/identity_layer/services.rs:629-632`). The card
   shows only Dismiss and the unavailable text (`frontend/src/pages/review/ReviewPage.tsx:187-192`).
3. *Defect D1, masked by the switch.* Before containment, "different book" on a
   card with a proposal overwrote the existing book (ST-001).

**Reproduction (real doors).**
- Edit: create a Work "Alpha" by "Ann"; `PUT /api/v1/work/{id}` `{"title":"Beta"}`.
  Expected 200 and title "Beta". Observed 409, title "Alpha".
- D1: a Work W "Alpha" by "Ann" with provider id P1, and a pending GroupIdentity
  card anchored on W whose proposal is "Beta" by "Bob" with provider id P2.
  `POST /api/v1/identity-review-card/{card}/resolve` with the different-book action
  and W's listed generation. Expected: 200, W still "Alpha" by "Ann" with P1 only,
  card resolved. Observed on `main`: 409 (the switch). With the switch removed
  (code read, not run): the request reaches the write in ST-001, so W becomes
  "Beta" by "Bob" and gains P2. A reproduction test is therefore red for two
  reasons. It must assert both the
  success *and* the unchanged W, so it stays red if only the switch is removed.
- Mixed Save (code read, not run): with the switch removed, a dialog Save that
  changes the title and a monitor flag returns 200 but saves only the title. If
  the unchanged-field handling also goes, a Save that changes only a monitor flag
  loses it too, because the dialog always sends title and author (ST-011).

## 2. Requirements

- **REQ-001**: "Different book" on any GroupIdentity card (one book or many; with or
  without a proposal) never writes the proposal's title, subtitle, author or
  provider ids onto any existing Work. It records the distinction on the card's
  anchor Work the same way the no-proposal answer does today (ST-001) and
  resolves the card. The only permitted durable changes are: the anchor's
  recorded distinction, one increment of the anchor's generation, the card's
  resolution, and one resolution audit row. Every other stored field of every
  Work, all contributors and all provider routes stay as they were. It is
  accepted only with the generation the user saw when deciding (ST-002). It is
  available at all three review doors (both HTTP aliases and the command-line
  resolve).
- **REQ-002**: A user can change a Work's title, author, or both through
  `PUT /api/v1/work/{id}`, alone or together with the other fields the edit dialog
  sends (series name, series position, both monitor flags).
  - *Whole Save.* A successful Save persists every submitted change in that
    request. It never reports success while dropping one. If a submitted change
    cannot be saved, the whole request is refused before any write.
  - *Permitted changes.* The requested values plus their bookkeeping: the
    normalized title and author fields, the author reference and primary
    contributor (the Author comes from the existing name lookup: an exact or
    unambiguous name match, else a new Author record,
    `crates/livrarr-metadata/src/author_service.rs:75-182`), the title
    provenance, the identity generation, the identity status as recomputed from
    the unchanged routes, and audit or history rows recording the edit. The
    recorded distinction, provider routes and all other stored fields stay as
    they were. When only the author changes, the stored title, subtitle and
    volume stay as they were (ST-012). A changed title is split as every title
    is (ST-006). When the submitted author name resolves to an existing Author
    stored under another spelling, that Author is renamed to the submitted
    spelling once the edit has settled, so every Work by that Author shows it
    and no second Author is created.
  - *Duplicate identity.* An edit that would give the Work the exact identity
    (title, subtitle, volume and author) of another of the same user's Works is
    refused before any write, with 409 and the message "Another book already has
    this title and author." The Work is unchanged; no card, no marker, nothing
    absorbed. The check is in the backend. The database's uniqueness rule
    (ST-014) stays as a backstop and never surfaces as a server error for this
    case. The edit dialog shows the message (§3).
  - *No leftover question.* No pending review card is left for the user.
  - *Failure.* A failed edit leaves no durable identity change attributable to
    it, and no card it created stays pending. Values written by another
    successful writer are never undone. A refusal because the Work's identity
    changed underneath the edit returns 409; so does a duplicate-identity
    refusal. Storage and rollback faults keep their existing server error class,
    and nothing from the failed edit persists.
  - Submitting unchanged title/author with other field edits keeps working (ST-011).
- **REQ-003**: The "same book / merge" answer is gone from the card. A request for
  it at any review door is refused with the existing refusal (ST-009) before any
  write. The card stays pending. Works, routes, cards, audit rows and files are
  unchanged. Pending cards minted with a merge proposal stay visible and dismissible.
- **REQ-004**: Still refused, unchanged: manual merge preview and execute, automatic
  absorption of existing Works, and the two startup clean-ups.
- **REQ-005**: The review card and the edit dialog match §3. A GroupIdentity card
  shows what its book is being compared with: the proposed title (with subtitle
  and volume when present) and the proposed author's name when that Author record
  exists. A card without a proposal shows no comparison. Display only: no new card
  kind and no change to what a card stores. The card data carries the author's id,
  not the name (ST-013); how the name reaches the page is a design choice.

**What happens to each use of the switch** (10 uses; Serena and text search agree, CONTROLS C2):

| Use | Behaviour behind it | After this feature |
|-----|---------------------|--------------------|
| `crates/livrarr-domain/src/identity_layer/services.rs:630` | All GroupIdentity answers (road) | "Different book" available; merge answer refused |
| `crates/livrarr-db/src/identity_layer.rs:1922-1926` | All GroupIdentity answers (direct repository) | Same split |
| `crates/livrarr-handlers/src/work.rs:1093-1116` | Title/author edit | Available |
| `crates/livrarr-metadata/src/identity_road.rs:118-125` | Edit card and manual merge card creation | Edit available; manual merge refused |
| `crates/livrarr-metadata/src/identity_road.rs:523-528` | Automatic merge of several Works becomes a card | Stays: automatic absorption refused |
| `crates/livrarr-metadata/src/work_service.rs:1852-1856` | Manual merge preview | Stays refused |
| `crates/livrarr-db/src/identity_layer.rs:548-552` | Settlement that absorbs Works | Stays refused |
| `crates/livrarr-db/src/identity_layer.rs:3587-3589` | Absorbing one Work into another | Stays refused |
| `crates/livrarr-db/src/pool.rs:817-819` | Startup title-policy clean-up | Stays off |
| `crates/livrarr-db/src/identity_layer.rs:5393-5395` | Startup duplicate-residue clean-up | Stays off |

**Affected existing REQ-IDs** (`spec-merge-containment.md`): REQ-001 narrows
(GroupIdentity "different book" no longer refused; merge still refused); REQ-002's
edit refusal is lifted; REQ-003 narrows (a "Different book" control returns; merge
controls stay absent). Its AC-001, AC-002 and AC-004 change the same way. REQ-004
is unaffected. Active containment tests that pin refusal of edits or of every
GroupIdentity answer will contradict this spec; the test stage must list them by
name (not enumerated here).

**Suspended tests** (27, `suspended-contracts.json`; classified by name and by
which card answer or door each test's helper function names, found by text search;
bodies not reviewed for what they assert; method in `NOTES.md`):
- *Edit or "different book" (candidates to un-suspend; any that assert the proposal
  is written onto the existing Work must be re-pinned to REQ-001):*
  `door_gate_work_update_rekey_mints_then_resolves`,
  `work_update_identity_edit_mints_then_resolves_group_card`,
  `work_update_missing_resolved_or_generation_stale_card_fails_closed`,
  `direct_add_dedup_review_reuses_existing_work`,
  `http_and_cli_resolve_share_exact_continuation`,
  `resolve_every_review_kind_through_http_and_cli_same_graph`.
- *Mixed or unclear (the helper serves both edit and merge, or the search found
  no answer; inspect, then split or re-pin):*
  `registered_update_merge_and_affirm_bypass_exact_tombstones_byte_identically`,
  `registered_inline_doors_mint_then_resolve_and_emit_zero_notifications`,
  `all_non_revoking_user_actions_preserve_tombstones_and_reopen_no_card`,
  `interactive_review_requires_freshly_minted_card`,
  `interactive_card_origination_is_one_commit_then_typed_resolution`,
  `http_review_all_kinds_map_bad_conflict_notfound_internal`,
  `identity_review_card_list_resolve_and_dismiss_are_complete_http_paths`.
- *Merging or startup clean-up (stay suspended):*
  `door_gate_manual_merge_mints_then_resolves`,
  `manual_merge_preview_then_post_mints_then_resolves_group_card`,
  `manual_merge_missing_resolved_or_generation_stale_card_fails_closed`,
  `manual_merge_field_review_and_file_warning_do_not_escape_atomic_identity_commit`,
  `group_identity_card_revalidates_after_settlement_and_invalidates_specifically`,
  `existing_provider_titles_heal_once_with_generation_audit`,
  `title_policy_heal_collision_parks_group_review_and_retries_marker`,
  `article_variant_heal_folds_one_sided_volume_and_preserves_routes`,
  `article_variant_heal_parks_work_key_contradiction_with_current_card`,
  `startup_heal_folds_dedup_orphan_and_cleans_duplicate_group_cards`,
  `startup_heal_keeps_one_equivalent_pending_group_card`,
  `dedup_residue_machine_cancellation_writes_no_tombstone`,
  `startup_heal_suppresses_on_second_boot_but_changed_cohort_key_mints`,
  `second_boot_reuses_startup_heal_card_and_changed_cohort_members_mint`.

## 3. UI/Interface Design

**Review page, GroupIdentity card.** The card keeps its book link, author line and
Dismiss button (`frontend/src/pages/review/ReviewPage.tsx:161-185`). It gains one
action button, **"Different book"**. Like "Link it", it carries a short help tip,
for example "Keep these as separate books. Nothing about your books is changed."
It sends the different-book answer with the generation shown on the card. On
success the card leaves the list. On a refusal the user sees the error message.
- *Comparison line (PO approved 2026-09-24).* Under the book and author, the card
  shows what the book is being compared with, for example "Compared with: Beta by
  Bob". It uses the proposed title (with subtitle and volume when present) and the
  proposed author's name when that Author record exists (REQ-005). A card without
  a proposal shows no comparison line. The other books in a many-book card need not
  be named.
- *Merge removal.* No "Same book", "Merge" or "Confirm Merge" control on any card,
  including cards minted with a merge proposal. The sentence "Merging is currently
  unavailable. This question remains unresolved…" is removed from the card, because
  the card now has a working answer. Before containment the card's only button was
  "Confirm Merge" (`cc949b35^` ReviewPage, CONTROLS C4). It is not restored.
- *Card whose other book was deleted.* The card still shows, for the remaining
  book, with "Different book" and Dismiss. It must not state a book count that
  includes deleted books (today's copy counts the stored list, `ReviewPage.tsx:191`).
  The simplest compliant copy states no count. If the card's own book was deleted,
  the card is gone (ST-007).

**Edit dialog.** Title and author are editable again; the "temporarily
unavailable" note (`EditModal.tsx:86`) is removed. A Save reports success only
when every change in it was saved (REQ-002); otherwise it shows the refusal and
stays open. A duplicate-identity refusal shows its message, "Another book already
has this title and author.", in the dialog. No other change.

**Work page.** Unchanged: no Merge Duplicate control; the existing note stays (Q-004).

## 4. Non-Requirements

- No migration and no schema change. Existing pending cards stay as stored.
- No manual merge in any form (preview, execute, UI), no fix for D2/D3/D4, no undo,
  no machine merging, no startup clean-ups.
- "Different book" does not create a Work from the card's proposal.
- No change to machine settlement, card minting, dismissal rules, or existing
  Works' audiobook flag.
- No new review card kinds and no change to what a card stores. The only new
  display is the comparison line (REQ-005).

## 5. Open Questions

None blocks this spec. Each parked item goes to the project to-do list as unapproved unless the PO rules.

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | The card shows only its own book, not what it is compared with. "Different book" is hard to judge blind. | resolved — PO approved 2026-09-24 | Now REQ-005, §3 comparison line, AC-021. |
| Q-002 | After "different book" on a card minted by adding a book, the added book is not created (PO accepted: "the user adds it again"). ST-010 suggests re-adding the same title and author parks another card instead. | parked — needs a probe, not blocking | Inferred, about 65%. Disproved by a re-add that creates the Work. Outside this scope. |
| Q-003 | With D1 fixed, no answer applies a one-book proposal. Before containment the card's single button did (PA-006), for example accepting a matched add's new provider id. | parked — consequence of the PO decision | Consistent with the 2026-09-24 decision. `wiki/domain/work.md:85-87` needs updating at close. |
| Q-004 | The Work page says "Merging is currently unavailable." (`frontend/src/pages/work-detail/WorkDetailPage.tsx:174`); merging is now dropped for good. | parked — wording, out of scope | Unchanged here. |
| Q-005 | The delete-Work action the PO chose instead of merging also tries to delete that Work's files (`crates/livrarr-metadata/src/work_service.rs:1566-1567`). Deleting a duplicate that holds the only copy of a file may lose or orphan it. | parked — outside scope | Not verified at runtime. Flagged because the decision relies on this action. |

## 6. Acceptance Criteria

All through the real router with authentication, the real SQLite writer, or the rendered page.

- [ ] **AC-001** (REQ-001): One-book card on W with a proposal whose title, author and provider id differ from W. Different-book with W's listed generation, via both HTTP aliases (separate runs), returns 200. W's title, subtitle, volume, author, contributors and active provider ids are unchanged. The only durable changes are the ones REQ-001 permits: W's recorded distinction names this card, W's generation is one higher, the card is resolved (no longer listed), and one resolution audit row exists. No Work is created and no other Work changes.
- [ ] **AC-002** (REQ-001): Card on W1 and W2 with a proposal: same request. No member's title, subtitle, author or provider ids change; the card is resolved.
- [ ] **AC-003** (REQ-001): Card without a proposal: exactly AC-001's permitted changes (distinction, one generation increment, card resolution, one audit row) and no others.
- [ ] **AC-004** (REQ-001): The anchor's identity changes after the card is listed. Different-book with the old generation returns 409 "stale identity generation"; no Work, route, card or audit row changes; the card is still listed and dismissible. A card minted at an older generation succeeds with the currently listed generation.
- [ ] **AC-005** (REQ-001): The card's other book is deleted: the card is still listed and different-book succeeds with AC-002's invariants on the remaining book. The anchor is deleted: the card is not listed and resolve returns 404.
- [ ] **AC-006** (REQ-001): Command-line door, on a card minted at generation g whose anchor is now at g+1. The command-line ShowReview reports g+1. Resolve with g+1 and the different-book action gives AC-001's result. Resolve with g is refused as stale and writes nothing (ST-002: ListReviews still shows g).
- [ ] **AC-007** (REQ-002): `PUT /api/v1/work/{id}` with a new plain title returns 200. The response and a later GET show the new title; author and provider ids are unchanged; the pending-card list has no new card.
- [ ] **AC-008** (REQ-002): Author-only edit on a Work stored as title "Alpha", subtitle "Beta", volume 2, with a provider id. Run it two ways: (a) an API request that omits the title; (b) a rendered-dialog Save with the title unchanged. In both, the Work's author is the one the name lookup resolves for the submitted name, and its display name and stored author reference agree. Title "Alpha", subtitle "Beta", volume 2 and the provider id are unchanged in storage and in a later GET. No pending card exists. A title-and-author edit applies both (ST-006 for titles with a colon).
- [ ] **AC-009** (REQ-002): Unchanged title and author submitted together with a monitor-flag change: the flag is saved, identity is untouched, no card.
- [ ] **AC-020** (REQ-002): Mixed Save through the rendered dialog: change the title, the series name and one monitor flag in one submission. The response is 200, and a later GET shows all three changes. An unchanged-identity fixture does not satisfy this AC.
- [ ] **AC-010** (REQ-002): Injected fault, no rival writer. On the real authenticated edit request and SQLite writer, a storage fault is injected after an identity write has run but before commit. The response is the existing server error class (5xx). Title, subtitle, volume and author are as before. The edit left no durable identity change and no pending card.
- [ ] **AC-022** (REQ-002): Stale claim. After edit A has read the Work and before its write is claimed, writer B commits "Gamma" by "Cara". The ordering is controlled, not timed. Edit A returns 409. The Work shows "Gamma" by "Cara". Neither A's requested values nor any card A created persists. With no rival writer, the same edit succeeds.
- [ ] **AC-023** (REQ-002): Duplicate identity, on the production-shaped database (ST-014). The user has Work X "Alpha" by "Ann" and Work Y "Beta" by "Bob". `PUT /api/v1/work/{X}` with title "Beta" and author "Bob" returns 409 with the message "Another book already has this title and author.". X and Y are unchanged, no card exists, no Work is absorbed, and no Author record is created or changed. Positive control: the same title with a different author succeeds.
- [ ] **AC-011** (REQ-002): Empty or null title/author still returns 422 (`crates/livrarr-handlers/src/work.rs:1048-1091`).
- [ ] **AC-012** (REQ-003): The merge answer on a pending card, via both HTTP aliases, returns 409 "Merging is currently unavailable.". Works, routes, cards, audit rows and files are unchanged, and the card stays listed. The command-line resolve refuses it with no write.
- [ ] **AC-013** (REQ-003): A pending card minted with merge choices is listed; Dismiss returns 204 and removes it from the list.
- [ ] **AC-014** (REQ-004): Merge preview returns 400 and merge execute returns 409, both with the merging-unavailable message. No card is minted; Works, routes, cards, history and files are unchanged.
- [ ] **AC-015** (REQ-004): A settlement whose group holds two or more existing Works absorbs nothing, leaves a pending card and keeps source files. A direct settlement that names absorbed Works is refused with no partial write.
- [ ] **AC-016** (REQ-004): Both startup clean-ups write nothing and advance no completion marker.
- [ ] **AC-017** (REQ-005): Rendered review page: a GroupIdentity card shows "Different book" and "Dismiss". There is no merge, same-book or confirm control, no "Merging is currently unavailable" sentence on the card, and no count that includes a deleted book. Clicking "Different book" sends the different-book answer with the listed generation.
- [ ] **AC-021** (REQ-005): Rendered review page, fed a list response the real route produces: a card on Work "Alpha" by "Ann" whose proposal is "Beta" by "Bob" shows both "Alpha" by "Ann" and the comparison "Beta" by "Bob". If the proposed author's record no longer exists, the proposed title still shows. A card without a proposal shows no comparison line.
- [ ] **AC-018** (REQ-005): Rendered edit dialog: title and author are editable, the "temporarily unavailable" note is gone, and saving calls the edit route. The Work page still has no Merge Duplicate control.
- [ ] **AC-019** (REQ-004): No migration: `crates/livrarr-db/migrations/` is unchanged (last file `090_add_metadata_preservation.sql`).
