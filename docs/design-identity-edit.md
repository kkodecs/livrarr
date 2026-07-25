# Design: identity-edit — user-editable identity on the Book Information tab (r4)

Status: DRAFT r4 (2026-07-24). r1-r3 were rejected
(`build/reviews/identity-edit/design-r{1,2,3}-codex.md` plus the corresponding
Anthropic/Google verdicts). r4 preserves r3's product surface but replaces its incomplete
process-local identity lock with a durable `works.identity_generation` compare-and-swap,
moves ledger completion out of SQL into a normalizing startup Rust pass, completes the HC
preview and provider type seam, and folds every r3 transaction, tenancy, error, badge,
pending-row, DTO, cache-key, and snapshot-cap finding. Feature state:
`~/Projects/kk-build/build/state/identity-edit.yaml`. Process tier remains lightened
(PO-approved): this design note plus both-family review is the gate; there is no IR/arch
gate. Migration 076 is only an index replacement plus one coordination column — the
startup backfill is Rust, and there is no spine/entity change.

## What this is

The work page's Book Information tab shows identity as read-only rows
(`frontend/src/pages/work-detail/components/BookInformationTab.tsx:60-64`). A work stuck on
a wrong identifier has no user exit except delete-and-re-add. This feature gives the user a
**preview-confirm** edit: paste an identifier (or provider URL), see exactly which book it
resolves to, certify it, and the system commits it as final — reconciling the work's other
identifiers against the certified record. It also lets the user **clear** a wrong
identifier they can't replace.

Product grounding: user selection is final and tops the identity confidence hierarchy
(`ARCHITECTURE.md` Part 1). Machine contradictions against a User-set anchor are already
silently dropped (`crates/livrarr-db/src/sqlite_work_identity.rs:322-340`), so a certified
edit is durable against provider churn.

## PO-settled decisions folded here (binding; ratified 2026-07-24)

- **ISBN doctrine:** permanent as a provider access key, transient as identification. Any
  identification use resolves into work keys / user confirmation or wears Provisional.
- **UI:** ISBN leaves the Identity section (input-only: search and fix-match accept pasted
  ISBNs/URLs). ASIN stays visible. Identity section = badge + GR/OL/HC rows + ASIN.
- **Edit behavior:** per-slot edit with preview-confirm; user-confirmed = absolute; clear a
  slot is in scope; a work-key edit reconciles siblings against the certified record and
  drops non-agreeing ones (re-chaseable); a user ISBN is edition evidence and never drops
  work keys; cross-work collision → 409 naming the owning work, merge offered.
- **Uniqueness:** drop per-user uniqueness for `isbn_13` AND `asin`; keep it for the three
  work keys. (Ground-truth note: 044 already made confirmed-anchor uniqueness per-user
  across all anchor types; 076 narrows that live constraint to work keys, so same-user
  bridge sharing is the only uniqueness-policy delta — see ground truth 6.)
- GB text-search promotion and its constraints are the **GB text-first follow-up unit**,
  not this feature (see Out of scope).

## Ground truth (re-verified against source; r2/r3 errors corrected)

1. **The affirm door is the handler template** (`crates/livrarr-handlers/src/work.rs:1023-1107`):
   ownership via `work_service().get` → 404; settled-slot 409 for guesses; atomic
   `confirm_anchor_and_recompute_badge`; `identity_resolved` history; fire-and-forget
   `refresh(Interactive)`. `settled_anchor_types`
   (`crates/livrarr-handlers/src/work.rs:938-959`) treats a populated
   `works.*` column as settled even with no ledger row — column-only legacy works are real
   and affect MORE than the no-op branch (see ground truth 6b and §Migration).
2. **`confirm_anchor_in_tx` is the anchor-write chokepoint**
   (`crates/livrarr-db/src/sqlite_work_identity.rs:20-89`): rejects empty; canonical
   validation for isbn13/gr_key/asin; ol/hc accept any non-empty string
   (`crates/livrarr-db/src/sqlite_work_identity.rs:33-39`); upserts
   the confirmed ledger row; syncs the matching `works.*` column.
3. **`supersede_anchor` bypasses that chokepoint** (raw insert,
   `crates/livrarr-db/src/sqlite_work_identity.rs:113-222`), runs its own transaction,
   recomputes no badge, and
   has zero callers beyond its own impl (LSP reference check, 2026-07-24). This design
   deletes it (trait + impl + stubs) rather than giving it a caller.
4. **Badge derivation today** (`crates/livrarr-db/src/sqlite_identity_conflict.rs:486-531`): any
   open conflict → Conflict; else confirmed work key → Confirmed; else confirmed isbn/asin
   → Provisional; else Pending. It reads **ledger rows only**, so a column-only value is
   invisible; r4 replaces this with the shared validated ledger∪column projection rather
   than assuming the backfill can eliminate every column-only row.
5. **Conflict store**: migration 052 rebuilt `work_identity_conflicts` with no CHECK
   constraints (`crates/livrarr-db/migrations/052_federate_identity_conflicts.sql:13-26`).
   Kinds implicate only
   ol/gr/hc work keys plus QuorumTie
   (`crates/livrarr-db/src/sqlite_work_identity.rs:300-320`,
   `crates/livrarr-db/src/sqlite_identity_conflict.rs:470-482`); no isbn/asin kinds exist.
   Conflict resolution
   and review-apply both use first-write conditional claims (`status='open'` /
   `needs_review`) — the race-safe pattern this design's commit now mirrors.
   **Any open conflict blocks identity re-chase everywhere**: `settle_identity` treats
   Conflict as terminal, and the convergence selector never selects Conflict works — so
   "cleared and re-chaseable" is FALSE while any conflict is open (r2's clear section
   over-promised; corrected in §Clear).
6. **Uniqueness today — the r2 P2-1 migration reading was WRONG (empirically re-verified
   2026-07-24):** migration 041 initially created
   `uniq_user_confirmed_ol_anchor` on **(anchor_type, anchor_value) with NO user column**,
   a global unique index despite its "same user" comment
   (`crates/livrarr-db/migrations/041_anchor_user_uniqueness.sql:1-5`). Migration 042
   then explicitly **dropped that index**
   (`crates/livrarr-db/migrations/042_fix_anchor_user_scope.sql:1-8`). Migration 044
   added and backfilled `work_identity_anchors.user_id`
   (`crates/livrarr-db/migrations/044_anchor_per_user_uniqueness.sql:1-9`) and recreated
   the now-absent same-name index on
   **(user_id, anchor_type, anchor_value) WHERE confidence='confirmed'**
   (`crates/livrarr-db/migrations/044_anchor_per_user_uniqueness.sql:11-14`). Thus the
   LIVE constraint since 044 is per-user and covers ALL anchor types: cross-user sharing
   is already legal; within one user, duplicate work keys and duplicate bridges are both
   rejected. Migration 076 changes only the latter scope by freeing same-user
   `isbn_13`/`asin` sharing while retaining per-user uniqueness for OL/GR/HC work keys.
   Per-work "one confirmed row per type"
   (`crates/livrarr-db/migrations/039_work_identity_anchors.sql:17-19`) is separate and
   stays.
   **6b. The ledger is INCOMPLETE**: works columns predate it (HC/ISBN/ASIN since
   migration 001; `crates/livrarr-db/migrations/001_initial_schema.sql:70-73`), the only
   SQL backfill is OL
   (`crates/livrarr-db/migrations/043_backfill_ol_anchors.sql:1-10`), the
   GR backfill method has no production
   caller, and HC/ISBN/ASIN were never backfilled. Ledger-only machinery
   (`find_work_by_anchor`
   `crates/livrarr-db/src/sqlite_work_identity.rs:782-802`, badge derivation) cannot see
   those values (codex r2 P1, verified).
7. **Dead-end machinery**: rows per (work, type)
   (`crates/livrarr-db/src/sqlite_work_identity.rs:935-957`); per-slot and whole-work
   deletes exist (`crates/livrarr-db/src/sqlite_work_identity.rs:986-1007`).
   `chaseable_anchor_types` skips a
   type at ≥3 attempts (`crates/livrarr-metadata/src/work_service.rs:194-220`). Refresh
   clears dead-ends ONLY when Interactive AND NotFound
   (`crates/livrarr-metadata/src/work_service.rs:1615-1628`); refresh refuses enrichment
   while Pending/Conflict/NeedsReview
   (`crates/livrarr-metadata/src/work_service.rs:1672-1675`); refresh fetches live
   (`Freshness::Bypass`, `crates/livrarr-metadata/src/work_service.rs:1695`).
8. **Concurrency reality (codex r2/r3 P1, verified):** `merge_generation` CAS protects ONLY
   the enrichment field merge (`crates/livrarr-db/src/sqlite_work.rs:1189-1240`). Identity
   completion has NO generation guard: `merge_missing_anchors` re-reads confirmed types
   and confirms every currently-missing type in a loop of separate transactions
   (`crates/livrarr-db/src/sqlite_work_identity.rs:224-271`). `refresh` reads before
   taking its lock
   (`crates/livrarr-metadata/src/work_service.rs:1569-1578`), while convergence, add
   completion, the add-time/mid-enrichment settle legs, and retry-incomplete all call
   `settle_identity` without a common lock
   (`crates/livrarr-metadata/src/convergence_service.rs:49-119,262-310`;
   `crates/livrarr-metadata/src/work_service.rs:1094-1147,2189-2240,2488-2528`). A
   resolver in flight across an edit
   can therefore re-confirm dropped old-book keys into emptied slots. r4 does not try to
   repair that topology with another lock.
9. **Enrichment fetch primitive**: `fetch_by_anchor` accepts exactly GB←Isbn13, GR←GrKey,
   HC←HcKey|Isbn13, OL←OlKey|Isbn13, Audnexus/Audible←Asin
   (`crates/livrarr-external-data/src/provider_client.rs:84-118`), returns
   `NormalizedWorkDetail` (`crates/livrarr-external-data/src/types.rs:9-37`), and emits a
   call record per real HTTP. **But the client registry is NOT reachable from
   `EnrichmentWorkflow`** (codex r2 P1, verified): the trait exposes only
   enrich/reset/inject (`crates/livrarr-domain/src/services/enrichment.rs:64-100`), and
   the concrete clients live in the private queue internals. A preview fetch needs a new
   method on the `ProviderQueue` trait threaded up (§Preview seam).
10. **`external_ids` has NO live behavioral reader** (full main-tree enumeration:
    merge-repoint write `crates/livrarr-db/src/pool.rs:664-678`; append-only upsert
    `crates/livrarr-db/src/sqlite_external_id.rs:57-61`; the dedup readers
    `work_exists_by_isbn_13/_10`
    (`crates/livrarr-db/src/sqlite_list_import.rs:278-303`) have zero production callers;
    `list_external_ids`
    callers are tests only). The edit door does not touch the table; the hygiene unit
    deletes the dead readers.
11. **History**: `identity_resolved(work_id, work_title, action, identity)` — fields are
    `work_title`/`action`/`identity`, no `cause`
    (`crates/livrarr-domain/src/history_events.rs:546-562`). The History tab summarizer
    renders only `work_title` for these rows today
    (`frontend/src/pages/work-detail/components/HistoryTab.tsx:89-96`).
    `contract-work-history.yaml:69` REQUIRES any new user-initiated identity door to add
    its `identityResolved` writer AND door-inventory row (here +
    `ir-v2-work-history.yaml:567-577`) in the same change (verified real).
12. **Paste gap is real**: `lookup_term_to_seed` recognizes only a literal `isbn:` prefix
    (`crates/livrarr-domain/src/seed.rs:288-310`). `normalize_isbn13` accepts
    hyphens/spaces and folds ISBN-10→13
    (`crates/livrarr-domain/src/normalization.rs:24-96`); `normalize_asin`
    treats ANY 10-char alphanumeric as an ASIN — including a checksum-invalid 10-DIGIT
    string (`crates/livrarr-domain/src/normalization.rs:98-143`) — and
    `normalize_gr_key` takes any leading digit run
    (`crates/livrarr-domain/src/normalization.rs:146-160`). So naive type-detection
    misroutes a valid 10-digit GR key to ASIN
    (codex r2 P2, verified); the classification table below is ordered to prevent that.
13. **Frontend plumbing**: work detail polls only while the last response said
    `enriching=true` (`frontend/src/pages/work-detail/WorkDetailPage.tsx:52-77`); the
    detail query key uses the STRING route param (`["work", id]`,
    `frontend/src/pages/work-detail/WorkDetailPage.tsx:47-55`) while children
    receive numeric `work.id` — invalidation must use `String(workId)` (the pending-anchor
    precedent does exactly that, `frontend/src/components/PendingAnchorBanner.tsx:40-53`);
    the banner omits
    `hc_work` because the key is an internal id with no public page
    (`frontend/src/components/PendingAnchorBanner.tsx:58-59`). Routes live at
    `crates/livrarr-server/src/router.rs:289-293`.
    **The frontend HAS a test harness — r2's contrary claim was false (codex r2 P1,
    verified):** vitest + Playwright scripts (`frontend/package.json:6-16`) and a
    component-test precedent
    (`frontend/src/pages/activity/history/HistoryPage.test.tsx:1-51`).
14. **add_fast bridge dedup is first-hit-wins** today
    (`crates/livrarr-metadata/src/work_service.rs:902-968`, loop at
    `crates/livrarr-metadata/src/work_service.rs:930`); after the bridge branch it falls
    through to `try_dedup_by_normalized`
    (`crates/livrarr-metadata/src/work_service.rs:971-985`) — the
    abstention AC must account for that fallthrough (codex r2 P2). Work-key add dedup
    (`crates/livrarr-metadata/src/work_service.rs:606-640`) uses `find_work_by_anchor`
    for ol/gr/hc only.

## Design

### Slot roster

| Slot | Identity row | Editable | Clearable | Canonical input |
|---|---|---|---|---|
| `gr_work` | yes | yes | yes | digits (any length); or goodreads.com `/book/show/<id>[-slug]` URL |
| `ol_work` | yes | yes | yes | `OL<digits>W`; or openlibrary.org `/works/OL…W` URL. `OL…M` (edition) rejected with a "that's an edition id" message |
| `hc_work` | yes (display) | **no** | yes | — (internal numeric id; no public page a user could obtain or verify — same rationale as `frontend/src/components/PendingAnchorBanner.tsx:58-59`) |
| `asin` | yes | yes | yes | 10-char alnum **containing at least one letter** (bare form); or amazon `/dp/<asin>`, `/gp/product/<asin>` URL |
| `isbn_13` | **no row** (read-only "ISBN-13" row + clear (×) move to the Details section) | via fix-match paste only | yes | ISBN-13 or ISBN-10 (hyphens/spaces fine) via `normalize_isbn13` |

Edit affordances: a pencil on each editable identity row (slot-scoped), plus one **Fix
match** button that accepts any pasted identifier/URL. Both open the same preview-confirm
modal.

### Input classification (one new pure authority)

`livrarr-domain/src/identity_edit.rs::classify_identifier_input(input, slot_hint) ->
Result<(AnchorType, String), ClassifyError>` — pure, table-tested, the ONE place paste
parsing lives.

Slot-free (Fix match) order — chosen so the normalizer overlaps of ground truth 12 cannot
misroute:
1. Provider URL forms (host-recognized, key segment extracted, then slot-normalized).
2. `normalize_isbn13` on the separator-stripped value (10/13 length, checksum) → `isbn_13`.
3. All-digits (any length, checksum failed or non-10/13 length) → `gr_work` via
   `normalize_gr_key`. A 10-digit checksum-invalid value is a GR key here, NEVER an ASIN —
   bare ASIN classification requires ≥1 letter.
4. `OL\d+W` → `ol_work`; `OL\d+M` → typed edition-key error. Checked BEFORE the ASIN
   shape: a 10-character OL key (e.g. `OL1234567W`) is alphanumeric-with-letters and
   would otherwise be swallowed by the ASIN branch.
5. 10-char alphanumeric with ≥1 letter → `normalize_asin` → `asin` (an ISBN-10-shaped
   value with a trailing X folds to `isbn_13` per AsinNorm).
6. Anything else → typed error.

Slot-hinted (row pencil): only that slot's forms are accepted. A value that classifies to
a DIFFERENT slot → typed 422 "that looks like a {other} identifier — use Fix match"
(never a silent slot switch; this resolves r2's ASIN-row/ISBN-10 contradiction — the fold
to `isbn_13` happens only on the slot-free road).

The search box reuses the authority: `lookup_term_to_seed` gains the bare-ISBN branch
(classify step 2 only — URLs and other forms stay fix-match affordances).

### Preview seam (specified at the layer that owns clients)

New method on the **`ProviderQueue` trait** (livrarr-enrichment), implemented by
`DefaultProviderQueue` over its private client registry:

`preview_fetch(provider, query: AnchorQuery, language, priority) -> PreviewFetchOutcome`

- The queue-layer outcome may carry `NormalizedWorkDetail`: both types live above the
  domain leaf, and the current queue trait already lives beside provider payloads
  (`crates/livrarr-enrichment/src/lib.rs:200-214`). It calls the named client's
  `fetch_by_anchor` directly (ground truth 9): NO
  provider_response_cache read or write (that cache's single seam stays
  `dispatch_enrichment`, insight 68), NO `provider_retry_state` writes, no budget
  bookkeeping. Call records emit as usual at the client wrapper (truthful HTTP).
- The domain leaf does **not** name that provider type. Define
  `livrarr-domain::services::IdentityPreviewRecord` (title, author, year, language,
  cover URL, and canonical identity fields) plus a domain preview-outcome enum. Thread:
  `ProviderQueue::preview_fetch` → new `EnrichmentService` method →
  `EnrichmentWorkflow::fetch_anchor_preview`, whose return value is the domain record.
  `EnrichmentWorkflowImpl` in livrarr-metadata maps `NormalizedWorkDetail` into it; that
  adapter already converts enrichment-layer results into domain results
  (`crates/livrarr-metadata/src/enrichment_workflow_service.rs:48-87`), and
  livrarr-domain has no workspace-crate dependency it could use to name the external
  payload (`crates/livrarr-domain/Cargo.toml:1-20`). Stub impls in
  `livrarr-behavioral`, `ResetOnlyEnrichmentWorkflow`, and the door-gate stub are
  scripted or panic-if-called as appropriate — trait+impl+every stub, insight 7.
- Ordered fallback lives in `WorkService::preview_identity_edit` (one `preview_fetch` per
  leg): gr→Goodreads; ol→OpenLibrary; asin→Audnexus then Audible; isbn→Google Books when
  configured (ISBN-echo-verified) then OpenLibrary; **hc→Hardcover with
  `AnchorQuery::HcKey`**. The source accepts that pairing and implements the by-key fetch
  (`crates/livrarr-external-data/src/provider_client.rs:185-201,645-659`). Rides the
  process-global outbound queue at Interactive priority like any other client call.

### Preview (phase 1 of 2)

`POST /work/{id}/identity/preview` body `{ "input": string, "slot": string|null }`.

1. Ownership: `work_service().get` → 404. Then one user-scoped repository snapshot reads
   `identity_generation`, the validated ledger∪column five-slot projection, and open
   conflicts coherently; every assessment below uses that basis. A later generation
   read must never be paired with these earlier slot values. Classify + normalize → 422
   typed. `hc_work` → 400.
2. Fetch the certified record for the submitted value (seam above). Provider failure →
   200 with `resolved: null` + reason; nothing is certifiable; NO snapshot stored.
3. **Sibling assessment (work-key slots):** for each OTHER work-key slot with a value
   (ledger-confirmed OR column-only — the union, per ground truth 6b), fetch its payload
   and compute the verdicts. **The keep bar is proven agreement** (codex r2 P1 — the r2
   "not-Disagree" bar was weaker than the identity authority and treated absence of proof
   as agreement): keep iff `title_verdict` is Same — or Grey{OneSidedSubtitle} with an
   agreeing hard identifier (the ratified AC-004 hatch, via `title_id_trust`) — AND
   `author_verdict` is Agree. Everything else — uncorroborated Grey, VetoVolume,
   Different, author Abstain/Grey/Disagree, or a FAILED sibling fetch — is NOT proven and
   goes to the drop set, labeled with its cause (`disagrees` / `unproven` /
   `unverifiable`). HC follows the same rule on success/failure, with one deliberate
   distinction: `NotConfigured` means **keep**, because an unconfigured Hardcover client
   contributes no payload to enrichment (`crates/livrarr-enrichment/src/lib.rs:719-726`);
   `NotFound`, retryable outage, and permanent fetch failure are unproven and drop.
   Dropped ≠ destroyed: cleared and re-chaseable; convergence re-finds and the quorum
   re-validates. The preview shows every planned drop to the user's face before they
   certify. (This supersedes r2's "conservative keep" — both reviewers independently
   showed kept-but-unproven siblings re-poisoning enrichment, agy P1-1 / codex P1-4.)
   Bridges (ISBN/ASIN) are assessed **informationally only**: a disagreeing bridge warns
   ("your stored ISBN resolves to a different book — consider fixing or clearing it") and
   is NEVER auto-dropped (ratified; the Details row carries its one-click clear).
4. **Collision check (work-key slots):** `user_id` is an explicit input. Ledger
   `find_work_by_anchor` UNION a **same-user-filtered** works-column scan (column-only
   owners must be visible, ground truth 6b) — if another work owned by this user has the
   value, preview carries `{owning_work_id, owning_work_title}`, UI offers Merge works
   (live `WorkService::merge_works`), Confirm is disabled, and no snapshot is stored.
   The existing ledger lookup already joins `works` on `w.user_id`
   (`crates/livrarr-db/src/sqlite_work_identity.rs:782-801`); the new column half obeys
   the same explicit-user invariant (`crates/livrarr-db/src/lib.rs:4-5`). Another user's
   id/title can never be returned. Same-user bridge duplicates become legal post-076 and
   produce an informational same-user list only; cross-user sharing was already legal
   under 044.
5. Response: resolved record (title, author, year, language, cover_url, canonical value,
   slot), per-sibling assessments with causes, bridge warnings, collision info, an
   open-conflict warning where relevant ("enrichment stays paused until the conflict is
   reviewed"), and — when certifiable — an opaque **`preview_id`**.
6. **Snapshot store** (single-use intent token): server-side bounded map on
   `WorkServiceImpl` keyed by random `preview_id`, holding user_id, work_id, slot,
   canonical value, the observed **`identity_generation`**, the five effective slot
   values for UI display, the computed drop set with causes, and expiry. The generation,
   not a later read/compare of the slot set, is commit staleness authority. Bounds:
   per-user cap 4, global cap 64, TTL 10 min. A user's fifth live preview evicts only
   that user's oldest; expired entries are removed first. If the global cap is still
   full of live entries after those two operations and this user has no own entry to
   replace, reject the new preview with retryable `503 preview_capacity` and
   `Retry-After`, **never evict another tenant's token**. The process has one shared live
   work service (`crates/livrarr-server/src/state.rs:147-152`), so this saturation rule
   is load-bearing. The map lock is never held across provider awaits; only the
   pre-fetch basis plus computed assessments are assembled and inserted after fetches.
   Process-local by design (restart → redo preview).

### Commit (phase 2)

`PUT /work/{id}/identity/{slot}` body `{ "preview_id": string }` → 200 with the standard
`WorkDetailResponse`.

Handler: ownership → 404; slot whitelist → 400; **consume the snapshot atomically**
(remove-on-read; matching user+work+slot required) → missing/expired/already-used → 409
`preview_required`. The true-no-op check below uses one current repository snapshot; a
generation mismatch is stale, never a no-op. A real edit calls the repository with plain
data only:

`apply_identity_edit(work_id, user_id, slot, new_value, expected_generation, drop_slots)`

No preview-cache/service struct crosses the repository boundary.

0. **Generation CAS is the transaction's FIRST statement:**
   `UPDATE works SET identity_generation = identity_generation + 1 WHERE id = ? AND
   user_id = ? AND identity_generation = ?`. Zero rows is typed `StalePreview` → 409
   `preview_required`; there is no read-then-compare window. This is the same
   write-claim shape the current review and conflict paths use
   (`crates/livrarr-db/src/sqlite_work_identity.rs:605-625`;
   `crates/livrarr-db/src/sqlite_identity_conflict.rs:230-248`), now against durable
   identity state rather than one conflict row.
1. Work-key slots: in-tx, user-filtered collision re-check over the validated
   ledger∪column projection → typed `Collision{owning_work_id}` → 409 with same-user
   details + merge offer. The per-user work-key unique index is the race backstop. The
   edit transaction keeps `sqlx::Error` typed through rollback; on
   `DatabaseError::is_unique_violation()` (the codebase idiom is
   `crates/livrarr-db/src/sqlite_common.rs:10-20`), it re-runs the same user-filtered
   owner lookup and returns `Collision`. A same-work/per-type violation with no other
   owner is an internal invariant error, not a fabricated collision.
2. Current confirmed row for the slot (value ≠ new): mark
   `confidence='superseded', superseded_by=<new>`. Column-only current value: no row to
   supersede; old value still captured for history.
3. **`confirm_anchor_in_tx(slot, new_value, AnchorSetter::User)`** — validation,
   generation bump, anchor upsert, and column sync at the existing single anchor-write
   chokepoint (`crates/livrarr-db/src/sqlite_work_identity.rs:11-88`).
   `supersede_anchor` is deleted, not reused (ground truth 3). Its edit-transaction error
   becomes DB-local `IdentityTxError::{InvalidValue, Sqlx(sqlx::Error)}` and stays typed
   until step 1's constraint classification; legacy repository wrappers map it outward
   only after their transaction ends. The current `WorkIdentityError::Db(String)` erasure
   at `crates/livrarr-db/src/sqlite_work_identity.rs:51-68` is not used inside this
   transaction.
4. Close superseded disputes: same-slot open conflicts → `status='resolved'`,
   `resolved_at=now`, `resolution_notes='superseded by user identity edit'`
   (`resolution_action` NULL — no schema change, 052 dropped the CHECKs). Work-key
   commits also close open **QuorumTie** conflicts (a user-certified work key IS the
   work-level tie-break). Other slots' kind conflicts stay open. Closing is what
   disarms a stale "Use New Match" replay — resolution claims `status='open'` and loses.
5. Sibling drops: exactly the snapshot's drop set (server-computed; the generation claim
   guarantees no identity writer changed it since preview): per slot — mark its confirmed
   row `superseded, superseded_by=NULL`, NULL its `works.*` column, DELETE **all pending
   rows** for that slot, and DELETE its dead-end row. Bridges never enter a drop set.
   Pending deletion is required for re-chase: `chaseable_anchor_types` rejects a missing
   slot while any pending row exists
   (`crates/livrarr-metadata/src/work_service.rs:189-219`).
6. DELETE every pending row and the dead-end row for the **edited** slot too. This makes
   a replaced pending guess neither visible nor later affirmable; pending rows otherwise
   survive beside confirmed values (`crates/livrarr-db/src/sqlite_work_identity.rs:899-932`).
7. `merge_generation + 1` — the ENRICHMENT CAS: an in-flight old-anchor field merge fails
   and reports Superseded (`crates/livrarr-db/src/sqlite_work.rs:1189-1240`).
8. `derive_badge_in_tx` over the validated ledger∪column projection → write
   `identity_status`.
9. Commit; any failure rolls the whole edit back (repo-level tests pin atomicity).

Post-tx: eagerly remove all remaining process-local snapshots for this work (the durable
generation already makes them stale; removal only frees capacity); history
`identity_resolved(work_id, &work.title, "edit", "{slot}: {old|'(empty)'} → {new}")`;
fire-and-forget `refresh(user, work, Interactive)` (door→road, insight 46); respond 200.

**True no-op:** after consuming the token, a single current repository snapshot must show
the expected generation, the same canonical slot value, a confirmed row for it with
`setter='user'`, an agreeing works column, an **empty computed drop set**, no implicated
open conflict (same-slot, plus QuorumTie for a work-key edit), no dead-end row for the
edited slot, and a stored badge equal to the current union derivation. Only then return
200 with no write/history/refresh. Any failed predicate runs the full commit: in
particular a disagreeing sibling shown for drop, conflict to close, dead end to clear,
badge drift, machine setter, column drift, or column-only value is not a no-op. The
existing consistency scanner names the column/ledger drift states
(`crates/livrarr-db/src/sqlite_work_identity.rs:738-779`).

### Durable identity generation (new, load-bearing)

Migration 076 adds `works.identity_generation INTEGER NOT NULL DEFAULT 0`. This is a
coordination column in the same class as `merge_generation` (introduced by the one-column
migration at `crates/livrarr-db/migrations/030_add_merge_generation.sql:1-3`), not a new
entity, relationship, or spine change. Every committed identity mutation advances it.
No identity lock is added or repurposed.

#### Claims and delayed completion

- `confirm_anchor_in_tx` advances the generation before its anchor upsert/column sync;
  composite transactions may already have won a first-statement claim, so more than one
  increment in one transaction is valid. `raise_identity_conflict` likewise advances it
  in the same transaction as conflict insertion/dedup and badge update; today those
  operations already share one transaction
  (`crates/livrarr-db/src/sqlite_work_identity.rs:392-463`). Edit and clear have the
  explicit claims described here.
- The adjacent raw status arms do not remain loopholes. Any `identity_status` mutation
  outside those transactions advances `identity_generation` in the same SQL statement.
  In particular, delayed NotFound/status conclusions take an expected generation and
  no-op on mismatch; create-time `set_identity_pending` claims before its pending-row +
  OL-column mutation (`crates/livrarr-db/src/sqlite_work_identity.rs:466-510`); and
  manual-refresh recovery
  bumps the identity generation when its CASE can recover NotFound
  (`crates/livrarr-db/src/sqlite_work.rs:1337-1364`) before refresh performs the coherent
  pre-resolve re-read.
- Each resolver road obtains `(Work, identity_generation)` from **one repository read**
  immediately before the provider await and passes `expected_generation` into
  `settle_identity`. A separate work read followed by a generation read is forbidden:
  an edit between them could pair stale anchors with a fresh generation.
- All post-await resolver writes collapse into two repository completion primitives.
  `merge_missing_anchors(user_id, work_id, expected_generation, incoming, target_badge)`
  becomes one transaction whose **first statement** is the conditional generation bump;
  only after winning does it inspect missing types and call `confirm_anchor_in_tx` inside
  that same transaction. This replaces today's loop of separate `confirm_anchor`
  transactions (`crates/livrarr-db/src/sqlite_work_identity.rs:224-271`). The sibling
  `complete_anchors(..., expected_generation, completion)` legs apply pending guesses,
  NeedsReview candidates/status, badge-only outcomes, and conflict raises under the same
  first-statement claim. The current separate post-resolve setters/pending writes at
  `crates/livrarr-identity/src/async_resolver.rs:159-285` are folded into those
  transactions. Add/adopt's current detect→raise loop→merge sequence
  (`crates/livrarr-metadata/src/work_service.rs:275-301`) likewise submits one
  expected-generation completion, so it cannot raise a stale conflict and then perform
  a separately raced gap-fill.
- A zero-row completion claim returns `IdentityCompletionOutcome::Superseded`: no anchor,
  pending row, review state, conflict, badge, or dead-end mutation from that stale
  resolution is written. The caller discards that resolver output and re-reads the work
  before enrichment; convergence also skips its old `before_missing` dead-end accounting
  on Superseded rather than applying it after the edit. Current refresh/convergence
  already have post-settle re-read sites
  (`crates/livrarr-metadata/src/work_service.rs:1651-1654`;
  `crates/livrarr-metadata/src/convergence_service.rs:119-120`). No write transaction is
  held over provider I/O, consistent with the transaction rule at
  `build/foundation/error-handling-policy.md:118-120`.

#### Writer coverage by construction

| Identity writer | Existing site | r4 coverage mode |
|---|---|---|
| refresh settle | `crates/livrarr-metadata/src/work_service.rs:1569-1649` | coherent generation read + expected-generation carry into completion |
| `complete_add` settle | `crates/livrarr-metadata/src/work_service.rs:1094-1147` | expected-generation carry |
| add-time anchorless settle | `crates/livrarr-metadata/src/work_service.rs:2189-2240` | expected-generation carry |
| mid-enrichment anchor completion | `crates/livrarr-metadata/src/work_service.rs:2488-2528` | expected-generation carry |
| `retry_all_incomplete` settle | `crates/livrarr-metadata/src/convergence_service.rs:262-310` | re-read coherent generation; expected-generation carry (never the enumerated stale `Work`) |
| convergence settle and its Pending→NeedsReview arm | `crates/livrarr-metadata/src/convergence_service.rs:49-119` | expected-generation carry through `merge_missing_anchors`/`complete_anchors` |
| delayed NotFound/other raw status conclusion | `crates/livrarr-metadata/src/work_service.rs:1163-1189`; `crates/livrarr-db/src/sqlite_work.rs:603-620` | expected-generation conditional status write |
| create-time Pending/status initialization | `crates/livrarr-metadata/src/work_service.rs:1055-1073,2324-2349` | first-statement generation claim/bump before status or pending-row mutation |
| manual-refresh NotFound recovery | `crates/livrarr-db/src/sqlite_work.rs:1337-1364` | same-statement generation bump, then coherent re-read before resolve |
| conflict raise from any completion/add preflight | `crates/livrarr-db/src/sqlite_work_identity.rs:392-463` | expected-generation completion claim, then chokepoint bump |
| pending affirm | `crates/livrarr-handlers/src/work.rs:1032-1079` | pending value+generation read together; `confirm_anchor_and_recompute_badge` first-statement conditional claim, then chokepoint bump |
| review apply **and adjacent dismiss** | `crates/livrarr-db/src/sqlite_work_identity.rs:593-717` | candidate+generation read together; first-statement conditional generation claim before the existing parked-state claim |
| conflict resolution **and adjacent dismiss** | `crates/livrarr-db/src/sqlite_identity_conflict.rs:214-248,401-445` | conflict+generation read together; first-statement conditional generation claim before the existing open-status claim |
| identity edit | new transaction | first-statement conditional claim using preview generation |
| clear | new transaction | first-statement user-scoped generation claim; it clears the then-current slot |
| `merge_works` | `crates/livrarr-db/src/sqlite_work.rs:751-905` | first statement bumps both same-user works in one `UPDATE ... id IN (...)`; require two rows before repoint/delete |
| direct confirmation on a newly created work | `crates/livrarr-metadata/src/work_service.rs:856-879` | `confirm_anchor_in_tx` chokepoint bump |

The race rule is symmetric. Delayed writer first → generation advances → an older preview
commit loses its CAS. Edit/clear first → every delayed completion carrying the old
generation returns Superseded. A conflict raised after preview advances the generation
even when no slot changes, so the edit cannot silently resolve a conflict the preview
never showed. Affirm/review/conflict actions use their own first-statement claims, so
whichever transaction loses cannot apply a stale value after waiting. `merge_works`
advances both rows and eagerly removes both works' local snapshots; surviving tokens
would fail durably even if that cleanup were missed.

### Clear

`DELETE /work/{id}/identity/{slot}` (all five slots) → 200 `WorkDetailResponse`; 404 when
the slot is empty. **Empty means no confirmed row, no nonempty column, and no pending
row**; historical superseded rows do not make a slot nonempty. A pending-only slot
therefore returns 200 and is cleared, not 404. No preview (client-side confirm dialog).

The transaction's first statement is a user-scoped `identity_generation` bump, which
claims the then-current slot against delayed completions. It then supersedes the
confirmed row (`superseded_by=NULL`) if present, NULLs the column, deletes **every
pending row** for the slot, deletes its dead-end row, bumps `merge_generation`, and
recomputes the badge from the validated ledger∪column projection. If the slot proves
empty after the claim, the transaction rolls back (including the bump) and returns 404.
It eagerly consumes/invalidates all of the work's preview snapshots; the generation is
the durable backstop. Open conflicts remain untouched — a conflict card may hold the
right candidate; the Review surface owns it. This same pending-row rule is applied to
edit's sibling drops and edited slot, not only DELETE.

**Honest re-chase contract (codex r2 P1 — r2 over-promised):** with the dead-end gone the
slot is chaseable **only while no conflict is open** — Conflict is terminal to both
`settle_identity` and the convergence selector (ground truth 5). The response carries
computed `WorkDetailResponse.parked_by_conflicts=true` (serialized
`parkedByConflicts`) when opens exist, and the UI says "cleared — re-matching is paused
until the open conflict is reviewed." No-conflict clears, including a
latent-pending-only clear, spawn refresh and become chaseable because neither the pending
filter nor the dead-end filter remains. History:
`("clear", "{slot}: {old-or-pending} → (cleared)")`.

### Migration 076 + startup ledger completion

`076_anchor_uniqueness_identity_generation.sql` contains **only** these schema operations,
in this order:

```sql
-- 1. Drop 044's live per-user all-anchor index (same name as 041's dropped predecessor).
DROP INDEX IF EXISTS uniq_user_confirmed_ol_anchor;

-- 2. Recreate per-user uniqueness for WORK KEYS only (the ratified scope).
CREATE UNIQUE INDEX IF NOT EXISTS uniq_user_confirmed_work_anchor
    ON work_identity_anchors(user_id, anchor_type, anchor_value)
    WHERE confidence = 'confirmed'
      AND anchor_type IN ('ol_work', 'gr_work', 'hc_work');

-- 3. Durable coordination for identity writers; same class as merge_generation.
ALTER TABLE works
    ADD COLUMN identity_generation INTEGER NOT NULL DEFAULT 0;
```

There are no backfill inserts and no `_livrarr_meta` marker writes in migration 076.
Migrations 041/042/044 are immutable; 039's per-work confirmed-type index remains
(`crates/livrarr-db/migrations/039_work_identity_anchors.sql:16-19`). The index swap
preserves 044's existing per-user uniqueness for work keys while removing only its
same-user cross-work restriction on `isbn_13`/`asin`; cross-user sharing requires no
repair because 044 already permits it
(`crates/livrarr-db/migrations/042_fix_anchor_user_scope.sql:1-8`;
`crates/livrarr-db/migrations/044_anchor_per_user_uniqueness.sql:11-14`).

After all migrations and the version gate, startup runs
`backfill_work_identity_ledger(pool)` before services/jobs are constructed, alongside the
existing Rust backfills (`crates/livrarr-server/src/main.rs:628-660`):

No `identity_generation` bump is needed in this pass: it runs single-threaded with
respect to identity writers in the exclusive pre-service startup sequence, before
AppState or jobs exist
(`crates/livrarr-server/src/main.rs:61-68,628-660`). Moving this pass to an online job
would invalidate that assumption and require the normal generation claim/bump protocol.

1. Read `_livrarr_meta['work_identity_ledger_backfill_complete']`; `"1"` is an
   idempotent early return.
2. Begin one transaction and scan populated `works.{ol_key,gr_key,hc_key,isbn_13,asin}`
   in `(user_id,id,slot)` order. Apply the same domain normalizers used by the write
   chokepoint. GR/ISBN/ASIN must canonicalize; OL/HC follow the current nonempty contract
   (`crates/livrarr-db/src/sqlite_work_identity.rs:27-41`). A valid noncanonical value
   (for example slug-form GR or ISBN-10) is rewritten to canonical form in the column and
   inserted as `confidence='confirmed', setter='import'` only when that work lacks a
   confirmed row of the type. This is Rust because the existing contract explicitly says
   SQL cannot call the GR normalizer
   (`crates/livrarr-domain/src/services/work_identity.rs:183-188`).
3. An invalid value is quarantined in place: keep the raw column so the user can see and
   clear it, create no ledger row, and warn with work id/slot (not the full value).
   A pre-existing ledger/column disagreement is likewise not overwritten; warn and leave
   it for the existing consistency surface.
4. Group valid work-key columns by `(user,type,canonical_value)`. If the group already
   has one confirmed ledger owner, preserve that owner and leave every other member
   canonical column-only; do not silently transfer an existing identity assertion merely
   because its work id is higher. If the group has no confirmed owner, its lowest work id
   deterministically gets the ledger row and later members stay canonical column-only.
   A member whose work already has a different confirmed value for that type is the
   ledger/column disagreement from step 3, not an owner candidate. Bridges insert per
   work because 076 removes their same-user cross-work uniqueness.
5. Write `work_identity_ledger_backfill_complete=1` as the last statement and commit.
   Any database/storage error rolls back rows and marker together and fails startup. This
   follows the existing atomic marker-last pattern
   (`crates/livrarr-db/src/pool.rs:400-434,789-804`); invalid user data is a quarantined
   row, not a pass failure.

All read-side identity unions (preview, collision, no-op, and badge) share one projection.
A “populated column” in that projection means a nonempty value accepted/canonicalized by
the same normalizers; quarantined invalid raw values remain clearable but do not earn a
badge or collide. `derive_badge_in_tx` checks open conflicts first, then
`confirmed-ledger ∪ valid-columns`: work key → Confirmed, bridge → Provisional, neither →
Pending. This keeps intentional duplicate losers truthful on every edit/clear recompute
without inventing a ledger owner.

### add_fast multi-bridge abstention (required by 076)

The bridge-dedup block (`crates/livrarr-metadata/src/work_service.rs:902-968`) collects
ALL verdict-eligible hits:
exactly one → adopt (unchanged); two or more → abstain from bridge dedup and fall through
to the EXISTING normalized-title dedup/create
(`crates/livrarr-metadata/src/work_service.rs:971-985`) — the AC pins the abstention
and the fallthrough separately (codex r2 P2: "creates a third work" was only true when
normalized dedup also misses).

### API error contract (concrete; codex r2 P1 / agy r2 P2)

`ApiError::Conflict` gains structured details, and `ServiceUnavailable` gains an
optional retry detail/header; the serialized envelope
(`crates/livrarr-handlers/src/types/api_error.rs:366-387`) adds an optional `details`
object:
`{ "code": "preview_required" | "anchor_collision" | "preview_capacity" |
"pending_anchor_stale" | "identity_review_stale" | "identity_conflict_stale",
"owningWorkId"?: number, "owningWorkTitle"?: string }`. The frontend
`ApiErrorResponse`/`ApiError` types and client normalizer
(`frontend/src/api/client.ts:35-67`) retain `details` instead of discarding it.

The DB implementation keeps a private typed `sqlx::Error` through the edit transaction
and converts only at its boundary to exhaustive `IdentityEditError` variants:
`InvalidValue` → 422 (classify errors and the chokepoint; never 500);
`StalePreview`/missing-used-expired token → 409 `preview_required`;
same-user `Collision{owner}` → 409 `anchor_collision` + id/title;
initial ownership miss → 404; snapshot saturation → 503 `preview_capacity` +
`Retry-After`; exhausted `SQLITE_BUSY` and `SQLITE_FULL`/`SQLITE_IOERR`/equivalent
storage failures → 503. Other invariant/corruption failures remain 500 with no internal
details. The 503 split follows the approved taxonomy
(`build/foundation/error-handling-policy.md:86-111`), correcting the current generic
DB-I/O→500 collapse (`crates/livrarr-handlers/src/types/api_error.rs:564-571`). Route
tests assert exact status, envelope, details, and retry header; the unique-race repo test
uses two connections and lets the competing owner commit after preview preflight. A
focused DB-unit case also feeds a real unique violation through the private typed
transaction mapper so the index-backstop branch cannot rot behind the normal in-tx
owner recheck.

The retrofitted interactive doors surface a lost first-statement generation claim as
typed `WorkIdentityError::StaleIdentity` (affirm/review) or
`ConflictError::StaleIdentity` (conflict resolve/dismiss) through their existing
repository/service boundary, then map it contextually at the handler. It is never erased
into `Db(String)` or returned as 500:

| Door whose generation claim lost | HTTP response and caller recovery |
|---|---|
| pending-anchor affirm | `409` envelope `{status:409,error:"conflict",message:"identity changed; reload pending anchors",details:{code:"pending_anchor_stale"}}`; refetch work detail + pending anchors |
| review candidate apply | `409` envelope `{status:409,error:"conflict",message:"identity changed; reload review candidates",details:{code:"identity_review_stale"}}`; refetch the review list/candidates |
| review dismiss | same `409 identity_review_stale` envelope and review refetch |
| conflict resolve | `409` envelope `{status:409,error:"conflict",message:"identity changed; reload identity conflicts",details:{code:"identity_conflict_stale"}}`; refetch conflict detail/list + work detail |
| conflict dismiss | same `409 identity_conflict_stale` envelope and conflict/work refetch |

These mappings extend the current handler boundaries: affirm currently maps its
transaction result before history/refresh
(`crates/livrarr-handlers/src/work.rs:1070-1106`), review apply/dismiss already map
`NotParked` to 409 (`crates/livrarr-handlers/src/identity_review.rs:142-202`), and
conflict resolve/dismiss already flow through the shared `ConflictError` mapping
(`crates/livrarr-handlers/src/identity_conflicts.rs:119-147`;
`crates/livrarr-handlers/src/types/api_error.rs:335-353`). Adjacent outcomes do not
change: ownership or missing resources remain 404; affirm's already-settled precheck,
review `NotParked`, and conflict `AlreadyResolved` remain their existing 409 conflicts.
The three new stable codes mean only “the resource was current at the door read, but a
different identity mutation won the generation claim.”

`WorkDetailResponse` gains nonoptional camel-case `parkedByConflicts: bool`, computed on
**every** shared work-detail mapping/return, and the frontend work type gains the same
field. The current DTO ends at `identity_status`/`enriching` with no such home
(`crates/livrarr-handlers/src/types/work.rs:173-230`). It is true exactly when the work
is parked by open identity conflicts, computed in the shared mapper from the persisted
Conflict badge (the conflict transactions and `derive_badge_in_tx` maintain that
invariant); edit/clear return the same DTO as ordinary GET.

### Frontend

- `BookInformationTab`: ISBN row leaves Identity (read-only ISBN-13 row + clear (×) in
  Details); pencil on GR/OL/ASIN rows; HC row clear-only (×); **Fix match** button opens
  the modal slot-free. Modal states: input → previewing → certifiable (book card +
  per-sibling keep/drop chips with causes + bridge warnings) | collision (Merge works
  handoff) | unresolvable (provider down / invalid) → confirming → done/error.
- Invalidations on commit or clear preserve each mounted query's actual key type:
  `["work", String(workId)]`, `["works"]`, and
  `["work", String(workId), "pending-anchors"]` use strings
  (`frontend/src/pages/work-detail/WorkDetailPage.tsx:47-55`;
  `frontend/src/components/PendingAnchorBanner.tsx:40-53`), while History uses
  **numeric** `["history", workId]`
  (`frontend/src/pages/work-detail/components/HistoryTab.tsx:27-37`).
- **Bounded post-save poll** (agy r2 P2 tightened): after the 200 (which already carries
  the updated rows), poll work detail at 1.5s for at most 6 probes, stopping early when
  `enriching` is observed true (hand off to the existing poll machinery,
  `frontend/src/pages/work-detail/WorkDetailPage.tsx:52-77`) or when the response's
  `identity_status` is one that blocks
  enrichment (conflict/needs-review — nothing to wait for).
- History tab: `summarizeHistoryData` renders identityResolved rows as
  `"{action}: {identity}"` before the `work_title` fallback; version-skew `??` fallbacks
  stay.
- **Component tests are required, not optional** (codex r2 P1; harness verified real,
  ground truth 13): vitest component tests for the modal state machine (preview →
  certify, drop-chip rendering, collision→merge handoff, unresolvable, 409
  preview_required recovery, and retryable preview-capacity), the exact mixed-type
  invalidation keys, `parkedByConflicts`, the History summary line, and the bounded poll
  handoff; plus ONE Playwright happy path (edit a GR key end-to-end against a stubbed
  backend). Live validation remains the final gate but is no longer the only frontend
  evidence.

### History door inventory (required)

`contract-work-history.yaml:69` binds: the edit and clear doors each add their
`identityResolved` writer row to the door inventory in
`contract-work-history.yaml:69` AND `ir-v2-work-history.yaml:753-757` in the same change
(affirm's writer is the template, `ir-v2-work-history.yaml:567-577`).

## Residuals (accepted, with reasons)

- **Stale enrichment values from the old identity persist until the post-edit refresh
  completes.** Covers ride the write-gate comparator; the user cover picker is the
  recovery. Accepted since r1.
- **A kept disagreeing bridge keeps feeding the old book to ISBN-driven providers** until
  the user fixes or clears it (ratified: bridges are never auto-dropped). Mitigation is
  visibility: the preview warns by name; the Details ISBN row carries the one-click clear.
- **Dropped-but-correct siblings** (provider outage at preview time forces a drop under
  the proven-agreement bar): recoverable by design — cleared slots are re-chased and
  re-validated by convergence; the preview said it would happen. The alternative (keep
  unproven) was shown by both reviewers to re-poison enrichment.
- **Editing slot A while slot B has an open kind-conflict:** commit succeeds, badge stays
  Conflict, enrichment and re-chase stay gated (ground truth 5). Preview warns; Review is
  the exit.
- **Same-user legacy duplicate work keys** (rare, pre-ledger): backfill preserves an
  existing confirmed owner, or chooses the lowest work id when no owner exists; every
  loser stays column-only and visible to the union reads. A certify against a loser's
  value 409s to the ledger owner (merge is the offered cure).
- **Invalid legacy identifiers remain as quarantined columns.** They get no ledger row,
  badge, or collision authority, but remain visible/clearable so startup never destroys
  user data it cannot normalize. One conservative incoherence is accepted:
  `settled_anchor_types` treats every nonempty column as settled
  (`crates/livrarr-handlers/src/work.rs:933-958`), whereas the new validated projection
  excludes quarantined-invalid values. Thus a same-slot pending guess is hidden from the
  pending list and a hand-crafted affirm is rejected while the invalid column remains
  (`crates/livrarr-handlers/src/work.rs:1005-1012,1052-1058`). Clearing the invalid slot
  deletes that latent pending row and makes the slot re-chaseable; silently affirming
  over visible invalid data is intentionally not allowed.
- **Preview snapshots are process-local** — restart → 409 preview_required → redo. Two
  quick previews cost two provider fetch rounds; acceptable for a rare admin action.
- **A wrong-but-plausible certify** (user confirms a lookalike): recoverable — re-edit or
  clear; user setter means the system won't fight them, by design.

## Out of scope (queued follow-up units)

- **GB text-first unit** (constraints from the confer, binding there): typed text-query
  provider variant (AnchorQuery stays anchor-only), text-keyed cache fingerprint +
  invalidation, lower field-trust tier for text-picked edition fields, quoted queries,
  top-10, result-side language verify, title-only retry on "no acceptable candidate", no
  query negatives.
- **Hygiene unit**: delete `work_exists_by_isbn_13/_10`, stale ListImportPage copy,
  Readarr bare-ISBN gate. (`supersede_anchor` deletion happens HERE, not deferred.)
- Edition-selection flow; DB-level machine-path OL/HC canonical validation.

## Acceptance criteria / test plan (red-first, real door)

Route-level tests drive the real router + auth middleware + real `SqliteDb`. New file
`tests/behavioral/test_identity_edit.rs` — `[[test]]` registration + `git add -f` are ONE
change (insight 65). Repo-level tests sanctioned for tx atomicity/CAS. Frontend: vitest
component tests + one Playwright path (see §Frontend — no longer manual-only).

Preview & classification:
- AC-1 pasted GR URL preview → 200; resolved record carries provider title/author;
  canonical bare numeric key; `preview_id` present.
- AC-2 classification table (table-driven, pure): GR URL, OL URL+bare, `OL…M` rejection,
  ISBN-13/10 with separators, ISBN-10-with-X fold, amazon URL, bare B0-ASIN, **10-digit
  checksum-invalid → gr_work (never asin)**, slot-hinted cross-type rejection (ASIN row +
  ISBN-10 paste → 422 naming fix-match), garbage → error.
- AC-3 collision preview (ledger owner) → collision block, owning id+title, NO
  preview_id. Same-user column-only legacy owner (backfill loser fixture) → same block.
  A different user holding the same work key neither blocks preview/commit nor leaks
  id/title. Race path uses two DB connections and a barrier: preview preflight passes,
  the other connection commits the same-user unique claim before the edit transaction,
  then commit returns 409 `anchor_collision` naming that owner — never 500. A focused
  repo case routes a real `is_unique_violation()` through the backstop mapper and reaches
  the same result, rather than relying only on the explicit in-tx recheck.
- AC-4 sibling assessment: proven-agree kept; uncorroborated-Grey, author-Abstain, and
  fetch-failure siblings ALL in the drop set with causes; disagreeing ISBN → warning
  only, never in the drop set. HC fixtures cover agreement→keep, disagreement→drop,
  NotFound/outage→drop-unproven, and NotConfigured→keep.
- AC-5 all providers failing → 200, `resolved: null`, no preview_id; commit without a
  valid preview_id → 409 `preview_required`.

Commit:
- AC-6 empty `gr_work` slot commit → 200; confirmed row setter=user; column synced; badge
  Pending→Confirmed; response carries both.
- AC-7 settled-slot overwrite → old row superseded(superseded_by=new), new confirmed row,
  column=new — one tx (repo-level).
- AC-8 snapshot drop set applied exactly: dropped sibling → superseded(NULL) + column
  NULL + **all same-slot pending rows deleted** + dead-end deleted; kept sibling
  untouched; bridges untouched. After commit the dropped slot is chaseable and its stale
  pending guess cannot be affirmed.
- AC-9 **single-use + generation CAS**: (a) commit twice with one preview_id → second
  409; (b) two previews/two commits → the first increments generation and the second
  returns 409 preview_required; (c) preview → clear → delayed commit → 409; (d) a
  background confirmed/pending/review writer after preview → 409; (e) conflict raised
  after preview with no slot change → commit 409 and leaves that conflict open. Assert
  the commit's first SQL mutation is the conditional generation UPDATE and a zero-row
  claim leaves every edit table untouched.
- AC-10 **writer-coverage race matrix** (repo/service-level barriers, no lock
  assumptions): for refresh settle, `complete_add`, add-time anchorless settle,
  mid-enrichment completion, retry-incomplete, and convergence, capture generation G,
  block after resolve, commit edit/clear, release → completion returns Superseded and
  writes no old anchor, pending guess, review state, conflict, or badge. Reverse order →
  completion advances generation and the older edit returns 409. Separate two-order
  cases pin pending affirm, review apply/dismiss, conflict resolve/dismiss, and
  `merge_works`: the losing first-statement claim makes zero identity mutations and
  merge advances/invalidates both work generations. Delayed NotFound/status writes lose
  after an edit, while manual-refresh NotFound recovery bumps generation and then passes
  the post-reset generation—not the pre-reset value—to its resolver. At the real route
  boundary, force the generation loss after each door's coherent read and assert the
  exact envelope: affirm → 409 `pending_anchor_stale`; review apply and dismiss → 409
  `identity_review_stale`; conflict resolve and dismiss → 409
  `identity_conflict_stale`. Each loser emits no success-only history/refresh side
  effect. Adjacent cases retain their prior contract: missing/foreign resource → 404,
  already-settled affirm / `NotParked` / `AlreadyResolved` → 409 without being mislabeled
  as a generation loss.
- AC-11 conflicts: same-slot open conflict → resolved with the superseded-by-edit note;
  `apply_conflict_resolution` on it now fails its open-claim; QuorumTie + work-key
  commit → closed; other-slot kind conflict → open, badge Conflict, refresh skips
  enrichment.
- AC-12 invalid values: route 422 (bad checksum, OL…M, garbage) AND repo-level
  `InvalidAnchorValue` from the chokepoint maps to 422 (never 500); forced mid-tx
  failure rolls back generation+supersede+column+badge together. Typed injected
  SQLITE_BUSY-exhausted and SQLITE_FULL/IOERR cases map to exact 503 envelopes; an
  unrelated invariant failure remains 500 without internals.
- AC-13 merge_generation: pre-edit generation, edit, then `apply_enrichment_merge` with
  the stale expected generation → Superseded.
- AC-14 **no-op/reconciliation matrix:** same value + ledger-confirmed user setter +
  agreeing column + empty drop set + no implicated conflict + no edited-slot dead end +
  correct badge → 200 with zero DB write/history/refresh. Each predicate gets a negative:
  disagreeing sibling in drop set → full commit drops it; same-slot conflict and
  work-key QuorumTie → full commit closes it; dead-end → full commit deletes it; badge
  drift → recompute; machine setter → stamp user; column drift → repair both stores;
  column-only value → create ledger row. A generation change before the no-op snapshot
  returns 409, never a stale 200.
- AC-15 ISBN commit via fix-match: isbn_13 updated, badge Provisional when no work keys,
  work keys untouched; two works sharing the ISBN both commit cleanly post-076.
- AC-16 clear: populated slot → superseded row, NULL column, **all slot pending rows**
  and dead-end deleted, badge recomputed; truly empty (no confirmed/column/pending) →
  404. A pending-only latent fixture → 200, rows deleted, later pending endpoint empty,
  stale affirm 404/409, and no-conflict slot chaseable after refresh. Same-slot open
  conflict → 200 + `parkedByConflicts:true` on the standard DTO, conflict remains open,
  no re-chase; no-conflict clear → `parkedByConflicts:false`, refresh spawned and slot
  chaseable.
- AC-17 cross-user work → 404 no mutation; unauthenticated → 401 (real middleware).
- AC-18 unknown slot → 400; `hc_work` PUT → 400; `hc_work` DELETE with value → 200.
- AC-19 history: edit → one identityResolved, `action=="edit"`, identity carries old AND
  new; clear → `action=="clear"`; no-op → none.
- AC-20 pending guesses for the edited slot (both the new value and a different latent
  value) are deleted in the edit transaction, absent from pending-anchors, and cannot be
  affirmed; this is distinct from AC-8's sibling-drop fixture.
- AC-21 **migration 076 on a 075-shaped fixture DB** (codex r2 P2): resulting index
  inspected via sqlite_master — per-user + work-keys-only predicate; same-user work-key
  duplicate REJECTED and cross-user same work key ACCEPTED as the pre-existing 044
  invariants; same-user shared bridge ACCEPTED as 076's uniqueness delta.
  `PRAGMA table_info(works)` shows
  `identity_generation` INTEGER NOT NULL default 0. The migration contains no ledger
  INSERT/UPDATE and 041/042/044 checksums are unchanged.
- AC-22 add_fast: two verdict-eligible same-bridge works → bridge branch ABSTAINS
  (pinned directly), then (a) normalized-title match present → adopts THAT work,
  (b) no normalized match → creates new. Exactly one eligible hit → adopts (unchanged).
  Bare-ISBN search term seeds the bridge; `isbn:` unchanged.
- AC-23 **startup Rust backfill + union truth** (075 fixture migrated through 076):
  canonical OL/HC/GR/ISBN/ASIN columns gain confirmed `setter=import` rows; slug-form GR
  and ISBN-10 normalize in Rust before column+ledger write. Malformed GR, checksum-invalid
  ISBN, and invalid ASIN keep their raw columns, get no ledger row/badge authority, and
  warn. For same-user duplicate work keys, a fixture with no pre-existing ledger owner
  deterministically gives the lowest id the row, while a fixture with an existing
  higher-id confirmed owner preserves that owner; every loser column remains. Recompute
  badge after both an edit and a clear on a loser and assert its remaining valid column
  still earns Confirmed. A quarantined-invalid GR column plus a latent pending GR guess
  is hidden by the pending list and rejects direct affirm until clear; clear removes both
  column and pending row, after which the slot is chaseable. The pre-service pass leaves
  `identity_generation=0` (no online writer can race it). Marker is last and atomic:
  injected mid-pass failure rolls back rows+marker; rerun succeeds; completed rerun is
  byte-identical/no-op.
- AC-24 snapshot store: per-user cap evicts own oldest (5th preview), other users'
  snapshots untouched (tenant isolation); TTL expiry → 409; own eviction → 409. Fill all
  64 slots with live entries from other users, then preview as a new user → 503
  `preview_capacity` + retry signal and all 64 existing tokens still commit-capable.
- AC-25 crate/repository boundary: `cargo check` proves livrarr-domain's
  `EnrichmentWorkflow` returns domain `IdentityPreviewRecord` and never names
  `NormalizedWorkDetail`; the metadata adapter mapping and every stub compile. The
  `apply_identity_edit` repository contract accepts only
  `(work_id,user_id,slot,new_value,expected_generation,drop_slots)`, not a preview
  snapshot/cache type.
- FE (vitest): modal machine states incl. 409 recovery and 503 capacity retry;
  invalidation asserts string detail/pending keys **and numeric History key** in both edit
  and clear flows; all work-detail responses type/render `parkedByConflicts`; History
  summary line; poll stops at 6 probes / on enriching-true handoff / on blocking
  identity_status.
- FE (Playwright): one happy path — open modal, paste GR URL, preview, certify, see the
  row update.
