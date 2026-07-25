# Design — identity-edit fix unit (r1)

**Status:** draft, awaiting PO go + cross-family review. No code written.
**Base:** `a7f03540` (merge of `race/fable`). **Contract:** `docs/design-identity-edit.md` (r4).
**Origin:** blinded cross-family review, `/mnt/opt/livrarr-review/FINDINGS-B.md` +
`FINDINGS-SUMMARY.md`. The reviewer chose this entry and listed six repairs; none of the
four contest entries was merge-ready.

Every defect below was re-verified against the merged tree at the cited `path:line`
before being written down. The reviewer's own citations pointed into `B/tree/...`; those
are not repeated here.

---

## What shipped and what did not

The feature's central mechanism **is** in place: a work carries an `identity_generation`
(migration `076_anchor_uniqueness_identity_generation.sql:12`); a resolver reads
`(Work, identity_generation)` in one transaction *before* provider I/O
(`crates/livrarr-identity/src/async_resolver.rs:132`) and submits one claimed completion
*after* it (`crates/livrarr-identity/src/async_resolver.rs:259`). A lost claim returns
`Superseded` and writes nothing. All six resolver roads converge on that call, and
`tests/behavioral/test_identity_edit_race_repro.rs` passes on this base.

What did not ship is **uniform** application of that mechanism. Three identity writers
still decide from one point in time and write at another, and four ordinary correctness
defects ride along. The fix unit closes both sets.

---

## F1 — two terminal writers decide before they claim

### F1a. Convergence can park a work the user just corrected

`crates/livrarr-metadata/src/convergence_service.rs:50` reads the work with `svc.get(...)`,
which yields no generation. The Pending→NeedsReview arm at
`crates/livrarr-metadata/src/convergence_service.rs:92-96` then calls
`svc.db.set_needs_review(work_id)` unconditionally.

*Failure:* convergence reads a Pending work at generation 7 and decides its attempt budget
is spent. Before the write, the user commits a valid identity edit — generation 8,
status Confirmed. The old arm sets NeedsReview anyway and parks the corrected work.

**Fix.** Read `(work, generation)` via `get_work_with_identity_generation` at the top of
the pass and route the terminal write through the existing claimed path
(`complete_anchors` with `target_badge: Some(NeedsReview)`), so a lost claim is a no-op.
No new repository surface — `complete_anchors` already carries a `target_badge`
(used at `crates/livrarr-metadata/src/work_service.rs:1208-1211`).

### F1b. The delayed NotFound conclusion observes its generation too late

`complete_add` *does* claim: `crates/livrarr-metadata/src/work_service.rs:1192-1197` reads
`(work, generation)` and `:1203-1212` writes `NotFound` under it. But that read happens
**after** enrichment has already returned — `identity_not_found` was decided upstream. The
CAS therefore claims a generation the decision never saw.

*Failure:* add enrichment starts against an anchorless work and blocks. The user certifies
a valid identity mid-wait. Enrichment returns NotFound, reads the *new* generation,
claims it successfully, and stamps NotFound over the user's correction.

**Fix.** Move the `(work, generation)` read to before enrichment is dispatched and thread
that generation into the completion. Same shape as the resolver already uses. This is an
observation-point move, not a new mechanism.

---

## F2 — add/adopt preflight is three transactions with no claim

`preflight_and_merge_anchors` (`crates/livrarr-metadata/src/work_service.rs:288-310`) runs
`detect_conflicting_anchors` (`:294`), then a loop of `raise_identity_conflict` (`:299-304`)
each in its own transaction, then `merge_missing_anchors` (`:305-308`) in a third. No
generation is carried through any of it.

*Failure:* adopt preflight finds two gaps and one conflict against generation 12. A user
edit lands mid-loop. The preflight raises a stale conflict and confirms one or both old
keys over the newer identity, in separate commits. An error partway leaves a partial
preflight committed.

**Fix.** Collapse to one claimed transaction: read `(work, generation)` before detection,
then submit conflicts + gap fills as a single `complete_anchors` call (it already accepts
`conflicts` and `merge_anchors`). Removes the partial-commit window as a side effect of
removing the multi-transaction shape — not a separate change.

---

## F3 — startup backfill cannot see an owner that exists only in the ledger

`crates/livrarr-db/src/sqlite_work_identity.rs:2132-2140` scans `works` with
`WHERE ol_key IS NOT NULL OR gr_key IS NOT NULL OR ...`, so an all-NULL work is absent from
the result. `user_of` is built from that result (`:2155-2156`), and `owner_of` only records
a confirmed row when `user_of.get(w)` succeeds (`:2159-2163`). A work that owns a confirmed
ledger row but has all five legacy columns NULL is therefore invisible during winner
selection.

*Failure:* work 2 owns GR `123` in a confirmed ledger row with all legacy columns NULL;
work 1 has legacy `gr_key=123` and no ledger row. Backfill omits work 2, elects work 1, and
inserts a second same-user confirmed owner. The unique index
(`crates/livrarr-db/migrations/076_anchor_uniqueness_identity_generation.sql:5-8`) rejects
it and **startup fails**.

**Fix.** Take CC's approach — verified, not assumed. `race/cc`'s
`crates/livrarr-db/src/work_identity_ledger_backfill.rs:222-224` resolves the owner by
querying `work_identity_anchors` directly on `(user_id, anchor_type, anchor_value)` instead
of building a map off column-populated works. `work_identity_anchors` carries `user_id`
(same migration, `:6`), so the join through `works` that produced this bug is unnecessary.

Adopt the direct-ledger-query owner lookup only. Porting CC's full 537-line module is out of
scope — it is a larger restructure than this defect needs, and CC's tree was not selected.

*Also worth taking from that module:* it reads the existing confirmed row **before**
rewriting the legacy column (`work_identity_ledger_backfill.rs:166-192`), so a pre-existing
ledger/column disagreement is left untouched. That ordering is the P2 the reviewer found in
opus. Fable's backfill should be checked for the same ordering while F3 is open.

---

## F4 — pending rows are not treated as state

### F4a. Clearing a slot whose only content is an empty-string pending row 404s

`crates/livrarr-db/src/sqlite_work_identity.rs:1131` builds `old_value` with
`pending.iter().find(|v| !v.is_empty())`, so a pending row with `anchor_value = ''` is
filtered out; `:1132-1135` then returns `EmptySlot` (404) and the row survives. The comment
at `:1125-1127` says "no pending row" makes a slot empty — the code implements "no
*nonempty* pending row". The design defines the presence of any pending row as state.

**Fix.** Presence-based, not value-based: a slot is empty only when there is no confirmed
row, no nonempty column, and no pending row at all. Cleanup and the generation advance then
run as specified.

### F4b. A same-value commit no-ops while a stale pending row still needs deleting

`is_true_no_op` (`crates/livrarr-metadata/src/work_service.rs:2253-2258`) tests confirmed
value, column agreement, drop set, implicated conflicts, dead-end and badge coherence — but
never inspects the submitted slot's pending rows.

*Failure:* GR `123` is user-confirmed and mirrored in the column, and a stale pending GR row
also exists. The user previews and commits `123`. The service returns `changed:false`
without entering the transaction, leaving the pending row AC-20 requires it to delete.

**Fix.** Add "no pending rows in the submitted slot" to the `is_true_no_op` conjunction.

---

## F5 — two contained correctness defects

### F5a. A full preview store rejects a user who has an evictable token

`crates/livrarr-metadata/src/work_service.rs:2526` evicts the requesting user's oldest token
only once they hold `PREVIEW_PER_USER_CAP`. The global-capacity rejection at `:2531-2535`
runs regardless.

*Failure:* the store holds 64 live tokens — one the requesting user's, 63 other users'. The
user requests a second preview. No eviction fires (they are under the per-user cap), so they
get 503 while holding a token that the design says to replace first.

**Fix.** When the store is globally full, evict the requesting user's oldest token if they
hold any, and only reject when they hold none.

### F5b. URL classification matches provider domains anywhere in the input

`url_segment` (`crates/livrarr-domain/src/identity_edit.rs:194-198`) lowercases the whole
input and tests `lower.contains(host)` — a substring test over the entire string, not the
parsed host.

*Failure:* `https://evil.example/?next=goodreads.com/book/show/12345` classifies as
Goodreads `12345`. Low severity — the user is pasting their own URL, so this is a
misclassification rather than a privilege boundary — but it certifies an identity from a
URL that was never a Goodreads URL.

**Fix.** Parse the URL and match the host structurally (exact host or a dot-suffix of the
provider domain), then take the path segment from the parsed path only. Reject non-http(s)
schemes.

---

## F6 — frontend coverage is absent in all four entries

No identity-edit test or spec file exists under `frontend/` on any contest entry. The
contract requires component coverage for the edit modal plus a Playwright GR-edit happy
path. This is single-authored work, independent of F1–F5, and is the one item the contest
tells us nothing about.

---

## Sequencing

F1–F3 share one shape — read `(work, generation)` at the decision point, write through one
claimed `complete_anchors` — so they are one coherent block and should land together.
F4 and F5 are independent and can land in any order. F6 is parallel to everything.

```
Block 1 (concurrency):  F1a → F1b → F2 → F3      one reviewer, one packet
Block 2 (correctness):  F4a, F4b, F5a, F5b       independent
Block 3 (frontend):     F6                        parallel
```

Recommended: Block 1 first. It is the block where a wrong fix is expensive and where the
existing `complete_anchors` surface already carries everything needed, so the change is
threading, not invention.

## Test plan

Red-first, real doors, per `CLAUDE.md` "Tests drive the real door". Each defect above states
a concrete failure scenario; each becomes one behavioral test that fails on `a7f03540` and
passes after its fix. Pattern to copy: `tests/behavioral/test_identity_edit_race_repro.rs`
parks a resolver via `StubProviderClient::with_delay` and proceeds on `call_count() > 0`,
never a timer — no injected state, no toy router.

F3 additionally needs a startup test that seeds a ledger-only owner and asserts the pass
completes without violating `uniq_user_confirmed_work_anchor`.

## Out of scope

- The four deferred test clusters (AC-24 saturation, AC-4 OL/HC fixture arms, AC-12
  BUSY/FULL injection) — post-race single-authored work, tracked separately.
- The `crates/livrarr-http/src/outbound_queue.rs` dispatcher watchdog. It arrived with this
  merge, three of four entries wrote it independently, and it is proven by experiment
  (Arm A hung 7 min; Arm B completed in 36.82 s, single variable). It is a `livrarr-http`
  resilience fix, not identity-edit, and wants its own decision — **PO call open**.
- Any change to the r4 contract. Defects are measured against it, not the other way round.
