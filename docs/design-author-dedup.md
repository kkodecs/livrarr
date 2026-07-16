# Design: author-dedup (bugfix)

**Date:** 2026-07-11 · **Type:** bugfix (design → red tests → code; no spec/IR ceremony per process-level rule) · **Trigger:** PO-reported duplicate author entries from spelling variants ("WEB Griffin"/"W.E.B. Griffin", "JK Rowling"/"J.K. Rowling", "Robert Anson Heinlein"/"Robert A. Heinlein").

Every code claim sampled against main working tree on 2026-07-11. Annotations: **NEW** / **EXISTING** (verified).

## 0. Problem + verified root cause

Author creation dedups by exact string only: `find_author_by_name` runs `LOWER(TRIM(name)) = LOWER(TRIM(?))` (EXISTING `crates/livrarr-db/src/sqlite_author.rs:214-216`) behind `find_or_create_author` (EXISTING `crates/livrarr-metadata/src/work_service.rs:2026-2056`, normalization = bare `.to_lowercase()`), the shared chokepoint for 5 of 6 author-creating doors. The standalone author-add door (`crates/livrarr-metadata/src/author_service.rs:62-118`) uses the same lookup; Readarr import (`crates/livrarr-server/src/readarr_import_workflow.rs:1035-1039`) uses its own token-key exact match. `authors` has **no** unique constraint on name (`migrations/001_initial_schema.sql:38-49`), and its `ol_key`/`gr_key`/`hc_key` columns are never consulted for lookup (no `find_author_by_*_key` exists on `AuthorDb`, EXISTING `crates/livrarr-db/src/lib.rs:563-601`).

Live damage (dev DB, verified 2026-07-11): exactly 3 duplicate clusters = the PO's 3 examples (author ids 14/15, 46/47, 38/39; six rows; Griffin id=39 carries 0 works). OL assigned different ol_keys to the same person in all 3 pairs — key anchoring alone prevents zero of the three.

Door-coverage note `[REV codex R-1, refuted-with-citation]`: `SecondaryApiImpl::add` (`crates/livrarr-server/src/api_secondary_impl.rs:52`) also runs find-then-create, but the file is a test-harness implementation by its own header (`:1` "Secondary API implementations for testing") and `SecondaryApiImpl` has zero non-test constructors (verified by reference search: only `tests/behavioral/test_api_secondary.rs` and `tests/implementation/test_impl_secondary.rs`). It is not a production door; it stays exact-match so the API-mapping tests it serves keep their semantics. Out of gate scope, deliberately.

**Source-verified correction to the research report:** `canonical_author_key` (EXISTING `identity_matching.rs:445-457`) tokenizes "JK Rowling" → `"jk rowling"` but "J.K. Rowling" → `"j k rowling"` — different strings. A blunt normalize-at-create catches only the Griffin shape (1 of 3). The verdict layer is the necessary core: `full_name_match` + `given_token_compatible` (EXISTING `identity_matching.rs:769-798` — initials match on first char, surplus given tokens never block) return a match for all 3 example pairs.

## 1. Data-flow contract

**Create flow (all author-creating doors):**
```
candidate author_name
  → clean_author                          EXISTING (title_cleanup.rs:98-110; unchanged)
  → exact match (LOWER/TRIM SQL)          EXISTING fast path (unchanged; catches self-matches, e.g. monitors reusing stored names)
  → miss: unambiguous_author_match        NEW (U-1) over the user's author names
      exactly one compatible author  → ADOPT (return its id; fill survivor's missing ol_key from candidate — never overwrite)
      zero / several / grey / unusable → CREATE new row (today's behavior)
```

**Merge flow (NEW, one DB transaction; ordering is load-bearing — series has `ON DELETE CASCADE` on author_id and `UNIQUE(user_id, author_id, gr_key)`, `migrations/023_series_monitoring.sql:6,13`):**
```
(user, survivor_id, loser_id)
  1 validate: both exist, same user, survivor ≠ loser        → DbError otherwise
  2 works:  UPDATE works SET author_id=survivor, author_name=survivor.name,
            merge_generation=merge_generation+1 WHERE author_id=loser
            (normalized_author UNTOUCHED — D-3)
            [REV codex R-12: the generation bump makes tag convergence re-sync
            file tags to the new author spelling — the job selects items with
            tagged_at_generation < works.merge_generation; without the bump,
            already-tagged files would keep the loser's spelling forever,
            violating the DB↔file metadata coherence principle]
  3 series: per loser series row:
       survivor has same gr_key → FOLD: repoint works.series_id loser-series→survivor-series;
                                  OR loser row's monitor_ebook/monitor_audiobook into the
                                  survivor row; series monitor_language := survivor's ∥ loser's,
                                  then the monitored-series⇒language invariant is enforced here
                                  (this write bypasses upsert_series/update_series_flags where it
                                  normally lives): merged row monitored with language NULL → "en"
                                  [REV codex R-6 — series.monitor_language is real, migration 063]
                                  (work_count: survivor's kept — count IS roster size and heals
                                  on the next roster save, insight 62/ST-007);
                                  series_roster [REV codex R-10 — migration 062, PK series_id,
                                  CASCADE on series delete]: survivor series lacks a roster row →
                                  repoint the loser's (UPDATE series_roster SET series_id=survivor);
                                  survivor has one → keep it (loser's cascades away with its row);
                                  work-flag propagation [REV codex R-14 — mirrors the two EXISTING
                                  write paths]: if the OR changed the survivor row's flags,
                                  propagate the final values to ALL works linked to the survivor
                                  series (update_series_flags semantics: series flags stamp linked
                                  works); if unchanged, stamp ONLY the repointed loser works with
                                  the survivor's current flags (link_work_to_series semantics:
                                  entering a series stamps the work);
                                  DELETE loser series row
       else                     → MOVE: UPDATE series SET author_id=survivor
  4 caches: DELETE loser rows in author_series_cache + author_bibliography (refetchable; heal on next open)
  5 author fields [REV codex R-2 + gemini R-3, convergent]: merge author-owned state onto survivor —
       monotonic, computed in Rust from the two loaded rows:
         monitored, monitor_new_items    := survivor OR loser
         monitor_since                   := MIN(non-null)   (earlier bound monitors more, capturing both intents)
         sort_name, import_id, ol/gr/hc  := survivor's when non-null, else loser's (never overwrite)
         monitor_language                := survivor's ∥ loser's; then the monitored⇒language
                                            invariant (insight 53) is enforced HERE too since this is a
                                            new write path bypassing the update_author guard: if merged
                                            monitored=true and language still NULL →
                                            seed::dominant_language(survivor's works post-step-2) else "en"
                                            — the ONE shared default rule (insight 53), computable here
                                            because the works are already reassigned in this transaction
                                            [REV gemini r2 R-4: upgraded from a bare "en" to the governing
                                            rule; the backstop remains monitored-conditioned — an
                                            unmonitored merged row keeps NULL language]
  6 DELETE loser author row (step 3 handled EVERY loser series row — fold or move —
    so no series row remains for this delete's CASCADE to touch; the only other
    inbound reference, works.author_id, was repointed in step 2)
  → AuthorMergeReport { works_moved, series_moved, series_folded }
```
Invariant: after commit, zero rows reference the loser; work/series counts are preserved (moved or folded, never dropped); no monitoring the user had enabled — author-level or series-level — is lost. Naive loser-delete is forbidden: step 3's repoint-first prevents both the series CASCADE wipe and `works.series_id`'s `ON DELETE SET NULL` (`migrations/023:24`) silently unlinking works.

Why the MOVE arm cannot violate `UNIQUE(user_id, author_id, gr_key)` `[REV gemini R-1 crash claim, refuted-with-reasoning]`: the arm split is evaluated per loser row against the survivor's rows, and the loser's own rows are already gr_key-distinct under the same constraint — so a row reaches MOVE only when the survivor holds no row with that gr_key, and no two moved rows can share one. The genuine gap gemini's finding surfaced is the FOLD arm's flag loss, folded above.

Why step 2 cannot violate the works unique index `[REV gemini r2 R-1, refuted-with-live-schema]`: the ONLY unique index on works is `idx_works_identity ON works(user_id, normalized_title, normalized_author)` (verified against the live schema's sqlite_master; no unique index involves `author_id` or `series_id`). Step 2 updates `author_id` and the display `author_name` only — `normalized_*` are untouched (D-3), so every row's unique tuple is unchanged. Same listing refutes the r2 R-2 claim of a series-dependent works constraint; the FOLD arm's repoint-THEN-delete sequencing was already explicit in step 3.

**Cleanup (one-time, after deploy):** merge the 3 live clusters through the new endpoint. DB snapshot already taken: `testdata/livrarr.db.pre-author-dedup-20260711`.

## 2. Unit map

| Unit | What | Files |
|---|---|---|
| U-1 | `unambiguous_author_match` pure fn + unit tests | `crates/livrarr-domain/src/identity_matching.rs` |
| U-2 | Create-gate wiring at all three lookup sites | `crates/livrarr-metadata/src/work_service.rs`, `crates/livrarr-metadata/src/author_service.rs`, `crates/livrarr-server/src/readarr_import_workflow.rs` |
| U-3 | `merge_authors`: `AuthorDb` method (SQL) + `AuthorService::merge` + `POST /author/{id}/merge` handler `[REV codex R-11 — singular, matches the route family]` | `crates/livrarr-db/src/{lib,sqlite_author}.rs`, `crates/livrarr-domain/src/services/author.rs`, `crates/livrarr-metadata/src/author_service.rs`, `crates/livrarr-handlers/src/author.rs` |
| U-4 | Cleanup run + verification queries | live dev instance (no code) |

No migration. No new crates. No frontend (D-6).

## 3. Unit designs

### 3.1 U-1 — the matching-authority function (NEW, `identity_matching.rs`)

```rust
/// Exactly-one-unambiguous-match author adoption gate (author-dedup).
/// Some(i) iff `candidate` adoption-matches stored[i] and NOTHING else.
pub fn unambiguous_author_match(candidate: &str, stored: &[String]) -> Option<usize>
```
Pseudocode: canonicalize candidate via `canonical_author_name` (EXISTING; `None` → return `None`); canonicalize each stored name; collect indices where the ADOPTION MATCH below holds; return `Some(i)` iff exactly one index matches. The exactly-one rule is `author_verdict`'s own unambiguity principle (EXISTING `identity_matching.rs:258-273`) specialized to the 1×N adopt shape and returning WHICH row matched. All matching stays in the ONE authority file (insight 59); call sites consume the `Option`.

**Adoption match — deliberately TIGHTER than `full_name_match` `[REV gemini R-2]`.** The existing zip semantics let a lone initial adopt across unchecked surplus given names ("J. Rowling" would silently adopt onto "Jane Joanne Rowling" — a hidden false merge, worse than a visible split, and author adoption carries no title/bridge corroboration the way work matching does). Rules, given equal canonical surnames (unequal → no match; one side given-less while the other carries given names → no match, EXISTING grey rule; both given-less → match):

1. **Equal given-token counts** → pairwise `given_token_compatible` (EXISTING semantics: initial matches first char, full words exact). Covers Heinlein ("robert anson" / "robert a") and Griffin ("w e b" both sides).
2. **Unequal counts, glued-initials interpretation:** EITHER side — candidate or stored, fully symmetric `[REV gemini r3 R-2, clarified]` — consisting of ONE all-alphabetic 2–4 char token may be read as a run of single-char initials ("jk" → j,k); if that reading equalizes the counts and every resulting pair is first-char-equal with the other side's initials, match. Covers Rowling ("jk" / "j k") in both directions. "jk" vs "joanne" fails (j,k vs j — counts still differ) → creates separately, mergeable later (documented limitation, unchanged from v1). `[REV gemini r4 R-3, kept-with-consistency]`: the exploded initials may match FULL given names first-char-wise ("jk" / "john ken") — deliberately identical to rule 1's existing initial-compatibility for the spaced form ("j k" / "john ken" already matches under `given_token_compatible`); a stricter glued form would treat "J.K." and "JK" differently, which is this bug's exact class.
3. **Unequal counts otherwise:** match ONLY when every zipped pair is an exact multi-char word equality — surplus given names are tolerated solely beyond an exact full-word prefix ("robert" / "robert a" adopts; "j" / "jane joanne" does NOT — the lone initial never spans unchecked names). `[REV gemini r2 R-3, kept-with-reasoning]`: blocking the exact-prefix shape would recreate visible dupes for the commonest provider form (bare names without middle initials — GB/embedded tags routinely omit them), the exactly-one guard already refuses when a second same-surname author exists in the library, industry precedent (Readarr) adopts bare forms, and a wrong adoption is recoverable via U-3 merge. Documented residual, PO-visible in D-2.

Consequences pinned by unit tests: the 3 PO pairs match — each asserted in BOTH candidate/stored directions `[REV gemini r3 R-2]`; "J. Rowling" vs stored "Jane Joanne Rowling" → `None` (rule 3); "Robert Heinlein" vs stored "Robert A. Heinlein" → adopts (rule 3, exact prefix); "J. Smith" against a library holding both "John Smith" and "Jane Smith" → `None` (exactly-one rule); "JK Rowling" vs stored "Joanne Rowling" → `None`; surname-only candidate vs given-named stored → `None`; empty/garbage candidate → `None`. Residual accepted: "J. Smith" adopts onto the library's ONLY "John Smith" (equal counts, rule 1) — standard in-library bibliographic assumption; the moment a second compatible author exists, the exactly-one rule refuses.

### 3.2 U-2 — create-gate wiring (three sites, same shape)

- `find_or_create_author` (`work_service.rs:2026`): on exact-miss, `self.db.list_authors(user_id)` (EXISTING `AuthorDb::list_authors`) → `unambiguous_author_match(cleaned_author, names)` → `Some(i)` → return `(false, Some(authors[i].id))`, and when the candidate carries `author_ol_key` and the adopted row's `ol_key` is NULL, fill it via `update_author` (EXISTING; mirrors the author-add door's key-fill on name-hit, `author_service.rs:77-114`). `None` → `create_author` exactly as today.
- `AuthorServiceImpl::add` (`author_service.rs:62-118`): same gate between its exact-match hit-arm and its create-arm. `[REV codex R-13]`: both the exact-hit arm AND the new adopt arm switch to the monotonic key policy — set `ol_key` only when the stored row's is NULL, never overwrite a populated `ol_key`/`gr_key`/`hc_key` from a spelling variant. The EXISTING hit-arm behavior (passing `req.ol_key` through `update_author`, which replaces on `Some`) is a corruption path for exactly this bug class, since OL assigns the same person different keys (§0); it changes to fill-if-missing here.
- Readarr `process_authors` (`readarr_import_workflow.rs:1035-1041`): replace the `identity_key("", name).1` equality filter with `unambiguous_author_match(name, &existing_names)` — its surrounding exactly-one-adopt / else-create logic is ALREADY the same semantics (EXISTING `matches.len() == 1` arm) and stays. `[REV codex R-7]`: the EXISTING loop takes ONE `list_authors` snapshot before iterating (`:1011-1015`) and never appends creations — two spelling variants arriving in the same import batch would both miss it and double-create. The wiring therefore appends each NEWLY-CREATED author to the batch-local list before the next row, so in-batch variants adopt. `[REV codex R-9]`: adopted authors are NOT appended (they are already in the snapshot — re-appending would double-list the same id, and a later variant matching both entries would trip the exactly-one rule into the create arm); belt-and-braces, the exactly-one rule at this door counts DISTINCT author ids among matches, never raw list entries.

Load ceiling `[REV gemini R-4, refuted-with-numbers]`: `list_authors` is one indexed per-user SELECT and the match is pure in-memory string work per stored author; it runs only on exact-miss (a new-to-library author), never on the hot self-match path. At 10k authors this is single-digit milliseconds of tokenization on a path whose surrounding add flow budgets 3 s for its phase-1 cover alone — an SQL surname pre-filter would require a new normalized-surname column + index (schema change) for no observable win at real library sizes (dev library: 47 authors). If libraries ever reach the size where this matters, the follow-up is a stored canonical-surname column; deliberately out of this bugfix.

### 3.3 U-3 — merge

- `AuthorDb::merge_authors(user_id, survivor_id, loser_id) -> Result<AuthorMergeReport, DbError>` (NEW; all SQL in livrarr-db, one transaction, step order per §1). `AuthorMergeReport { works_moved: u64, series_moved: u64, series_folded: u64 }` lives in livrarr-domain (trait-signature type safety, insight 9e).
- `AuthorService::merge(user_id, survivor_id, loser_id)` (NEW trait method beside `delete`, `services/author.rs:95-136`): validate-and-delegate; no business logic beyond mapping `DbError`.
- Handler `POST /author/{survivor_id}/merge` body `{ "loser_id": N }` in `crates/livrarr-handlers/src/author.rs` following that module's conventions (narrow `Has*` bound, validate → call trait → map result; insight 9). Returns the report JSON. `[REV codex R-8]`: SINGULAR `/author/...` — the existing route family is `/author`, `/author/{id}`, `/author/{id}/bibliography` (`router.rs:299-310`); the plural form in v1 was a convention break.
- The existing standalone `delete_author` path is untouched.

### 3.4 U-4 — cleanup (live data, PO-visible)

Re-verify the 3 clusters against the live DB immediately before running (ids/works may have drifted since research). Survivor policy (D-5): most works, tie → most external keys, tie → oldest id. On today's data: Rowling keep 14 (2 works) drop 15; Griffin keep 38 (19 works) drop 39 (0 works); Heinlein keep 47 (has gr_key) drop 46. Execute via the new endpoint; verify: duplicate-cluster query returns zero, per-cluster works sum preserved, loser ids gone, survivors carry unioned keys. Snapshot exists (§1).

## 4. Decisions

- **D-1 — ONE matching authority.** The gate is a pure fn in `identity_matching.rs`; the three doors consume it. No door-local comparison logic. (The third parallel comparator, `livrarr_matching::author_similarity` used by the GR auto-link at ≥0.90, is out of scope here — noted, not touched.)
- **D-2 — adopt only on exactly-one unambiguous ADOPTION match (§3.1 rules — tighter than `full_name_match`).** Grey shapes (shared surname, ambiguous initials, multi-match, initial-spanning-surplus-names), Abstain shapes (unusable name), and Disagree all CREATE a separate row — grey never absorbs (insight 59 philosophy); the recovery for a wrong split is the U-3 merge, mirroring works' visible-duplicate + merge-works pattern. The false-merge residual is narrowed `[REV gemini R-2]` to the equal-count single-initial case ("J. Smith" adopted onto the library's only "John Smith") — accepted: standard in-library bibliographic assumption, and the exactly-one rule refuses the moment a second compatible author exists.
- **D-3 — merge rewrites `works.author_name` to the survivor's name but never touches `works.normalized_author`.** The display string must stop showing the loser's spelling (the user-visible bug); the normalized column is part of the stored identity key, managed by the generation-marker backfill (insight 59) and load-bearing for work-dedup — rewriting it here could collide with the works unique index and silently change dedup identity. Divergence between the two is pre-existing and unchanged by this feature. `[REV codex r8 R-15, refuted-as-pre-existing]`: yes, a future re-add of a merged work under the survivor spelling can miss `find_by_normalized_match` and create a duplicate WORK row — but that exact miss happens identically WITHOUT this merge (the two spellings always produced different work identity keys; `identity_key`'s own doc declares it deliberately blunter than `author_verdict` for precisely this variant class), so the author merge neither causes nor worsens it. The add path's anchor dedup and verdict-gated bridge dedup remain the stronger layers for real re-adds. Durable convergence already has an owner: the `identity_key_generation` startup backfill recomputes stored keys from the live `author_name` (which this merge sets to the survivor spelling) with its own unique-violation skip handling — a synchronous in-merge rekey would reimplement that machinery minus its safeguards and could force work-merge decisions inside an author-merge transaction. Work-identity rekeying stays that machinery's job; explicitly out of this bugfix.
- **D-4 — no DB unique backstop.** Initial-compatibility is pairwise and non-transitive ("J. Rowling" matches both "Joanne Rowling" and "Jane Rowling", which don't match each other) — no canonical key can express the relation, so no unique index can enforce it. The residual byte-identical concurrent-insert race stays theoretical (single-admin usage) and is documented here deliberately.
- **D-5 — cleanup survivor policy** as in §3.4.
- **D-6 — no frontend in this bugfix.** Endpoint + service only; a UI merge affordance is a separate PO call.
- **D-7 — merge preserves every user-set monitoring intent `[REV codex R-2 + gemini R-1/R-3 + codex R-6]`.** ONE monotonic policy at both levels. Author level: booleans OR, `monitor_since` MIN, `monitor_language` coalesce + monitored⇒language "en" backstop (§1 step 5). Series level (FOLD arm): monitor booleans OR'd, `monitor_language` coalesced survivor-first, same monitored⇒language "en" backstop — explicitly re-enforced in the merge because this write path bypasses `upsert_series`/`update_series_flags` where the invariant guard normally lives. The MOVE arm carries the loser row intact (its language travels with it). `series.name` needs no rewrite `[REV gemini R-5, refuted]` — it is the series title from the GR roster (e.g. a series page name), not a denormalized author name; `works.author_name` is the only denormalized author-name column in play (§0 schema survey).

## 5. TDD directives (Codex authors; red before implementation)

- **U-1 unit (in `identity_matching.rs` tests):** the 3 PO pairs adopt (`Some`, right index); John+Jane ambiguity → `None`; equal-count initial compatibility adopts ("Robert Anson Heinlein" stored, "Robert A. Heinlein" candidate AND the reverse); exact-prefix surplus adopts ("Robert Heinlein" candidate vs "Robert A. Heinlein" stored, both directions); lone initial never spans surplus names ("J. Rowling" vs "Jane Joanne Rowling" → `None`, both directions) `[REV gemini R-2]`; "JK" vs "Joanne" → `None`; surname-only vs given-named → `None`; garbage/empty candidate → `None`.
- **U-2 behavioral (through the REAL doors — no injected state):** add a work whose seed author is a spelling variant of an existing author → same `author_id`, authors row-count unchanged, missing ol_key filled and never overwritten; the standalone author-add door (`AuthorServiceImpl::add`) covered for BOTH exact-hit and adoption arms with a request carrying a CONFLICTING ol_key → stored key preserved `[REV codex R-13]`; variant with two compatible stored authors → NEW row (count +1); exact-match fast path unchanged (monitor-style reuse of the stored spelling → no gate involvement observable, same id); Readarr batch with TWO in-batch spelling variants of an author absent from the DB → ONE Livrarr author, both Readarr ids mapped to it `[REV codex R-7]`; Readarr batch where the FIRST row adopts a pre-existing author and a LATER row is another spelling variant → still adopts the same author, never creates `[REV codex R-9]`.
- **U-3 behavioral:** full merge on a fixture with works + one colliding series (same gr_key on both authors) + one loser-only series + bibliography/cache rows → works repointed AND renamed to survivor spelling; colliding series folded with `works.series_id` repointed (no work unlinked), the loser row's series monitor flags OR'd into the survivor row `[REV gemini R-1]`, the loser-only `monitor_language` carried onto the survivor, and an OR-monitored fold with both languages NULL ending at "en" (invariant) `[REV codex R-6]`; non-colliding series moved (and the move must succeed while the survivor holds series rows with OTHER gr_keys — pins the no-collision reasoning); loser caches gone; survivor's NULL keys filled, populated keys NOT overwritten; loser row gone; report counts exact. Author-field policy fixtures `[REV codex R-2]`: loser is the ONLY monitored row (with monitor_since + monitor_language set) → survivor ends monitored carrying both; survivor-monitored + loser-unmonitored → unchanged; both monitored with different monitor_since → MIN wins; merged-monitored with both languages NULL → "en" backstop (invariant). Error arms: survivor==loser, cross-user, missing ids. Repeat-merge of the same pair → NotFound error, first merge's state intact. Route-level test `[REV codex R-8]`: `POST /author/{id}/merge` exercised through the real router — auth context, body shape, and the report JSON pinned. Roster fixtures `[REV codex R-10]`: colliding-series fold with loser-only roster → roster repointed to the survivor series; survivor-only and both-present → survivor's roster kept (loser's gone with its row). Work-flag fixtures `[REV codex R-14]`: fold where the loser series was monitored and the survivor was not → survivor-series works (pre-existing AND repointed) end with the merged flags; fold with no flag change → repointed works stamped with the survivor's flags, pre-existing survivor works untouched. Tag-sync fixture `[REV codex R-12]`: a merged work with a synced library item (tagged_at_generation == old merge_generation) becomes eligible for tag convergence after the merge (generation bumped), so the file's author tag converges to the survivor spelling.
- Every test red on current tree for its semantic reason (gate absent / method absent), never a compile error — stubs land first (trait methods + `todo!()`, same as U-A/U-B1 pattern).

## 6. Compliance + landmines

- No SQL outside livrarr-db; handler stays validate→trait→map (insight 9/9b); `AuthorMergeReport` in livrarr-domain (9e); no new dependencies; no migration; zero network anywhere in this feature (no OL/UA surface).
- Series CASCADE + works.series_id SET-NULL are THE landmines — §1 step order and the folding rule exist for them; tests pin both.
- Real-data cleanup runs only after the suite is green, against re-verified ids, with the snapshot already on disk (ops rule, insight 49).
