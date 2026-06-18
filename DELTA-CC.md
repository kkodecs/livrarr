# DELTA-CC — TARGET-ARCH.md vs. actual livrarr code

Independently derived from `~/tmp/TARGET-ARCH.md` + the source tree only (no other plan/audit/delta/wiki doc read for *deriving* verdicts; a few wiki/spec lines appear only as corroboration of code findings). Five sections diffed in parallel by sub-agents, then the load-bearing claims re-read from source by the main loop before being asserted here.

**Headline:** The code is the *same architecture viewed through a self-hoster's pragmatism* — internal integer keys, a deterministic priority merge, derived status. Three findings are real defects/gaps worth flagging: **(1)** automatic background convergence was *deleted* and replaced by a manual sweep (§4.3), **(2)** `source_empty` is terminal so a work a provider lacks today is never auto-retried (§4.2), **(3)** the `cleared` user-override release flag is honored for covers but silently ignored for text fields (§3.4a). The rest of the divergences are deliberate, defensible scale-downs — and several TARGET decisions are over-built for a single-user SQLite app.

**Confidence:** High on the three flagged defects and on D0.1/§1/§4.3 (re-read from source: file:line snippets below). Medium on the *absence* claims (must/cannot-link store, drift detection, corroboration, blocking step) — proving a negative; sub-agents searched but absence is never as certain as presence.

---

## Verdict table

| Decision | Verdict | Evidence | Note |
|---|---|---|---|
| **D0.1** internal opaque ID; external IDs as aliases | DIVERGENCE / DELIBERATE | `pub type WorkId = i64` (domain/lib.rs:21); `id INTEGER PRIMARY KEY AUTOINCREMENT` (migrations/001:52) | Key is system-minted & opaque ✓ but it's an i64, not a ULID; external IDs live in **3** overlapping stores not one alias set |
| **D0.2** evidence layer + recomputable resolved projection | DIVERGENCE / DELIBERATE | `work_metadata_provenance` PK `(work_id, field)`, upsert one row/field (sqlite_provenance.rs:114) | No append-mostly observation log w/ confidence+observed_at; "current winner" + per-merge dissent log instead |
| **§1.1** Work/Edition/File three-level grain | DIVERGENCE / DELIBERATE | Only `Work` (domain/lib.rs:392) + `LibraryItem` (lib.rs:495); no Edition anywhere | Two levels; edition fields flattened onto Work |
| **§1.1** Edition as a persisted entity | GAP | No `editions` table, `Edition` struct, or `EditionId` type in any crate | "Release" in BIG7 ≠ Edition (Release is transient search-result, never stored) |
| **§1.2** media_type on the Edition | DIVERGENCE / DELIBERATE | `media_type` on `LibraryItem` (lib.rs:495); `CHECK(... IN ('ebook','audiobook'))` (migrations/001:115) | Work carries `monitor_ebook/_audiobook` flags + dual cover slots instead |
| **§1.2** field placement (Work vs Edition split) | DIVERGENCE / DELIBERATE | All title/publisher/isbn/asin/narrator/duration/page_count on `Work` (lib.rs:397-419) | No structural enforcement that ebook-only works null the audio fields |
| **§1.1** File identity = content hash + path | DIVERGENCE | `UNIQUE(user_id, root_folder_id, path)` (migrations/001:118); no content-hash PK | `KashLink.epub_hash` exists but only for cross-format resume, not file identity |
| **§1.2** user-facing record is a runtime projection | DIVERGENCE / DELIBERATE | `WorkDetailView { work, library_items, .. }` (services/work.rs:96) returns stored rows | Merge happens at write-time → Work row *is* the resolved value |
| **§2.1** signal hierarchy (Tier A/B/C) | DIVERGENCE / DELIBERATE | `has_work_anchor()` (english_identity_resolver.rs:427); anchor>bridge demotion (lines 191-196) | Binary anchor/bridge split, not named A/B/C; Tier-C page-count signal absent |
| **§2.2** linkage pipeline normalize→**block**→score→decide | PARTIAL + GAP (block) | `find_matching_work` linear scan over `&[Work]` (work_dedup.rs:53) | No blocking/candidate index; provider fan-out + linear library scan instead |
| **§2.2/2.3** two thresholds + human-review band | DIVERGENCE / DELIBERATE | `should_auto_confirm`: `conf>=High && score>=0.90` else NeedsConfirmation (matching/lib.rs:177-185) | Two-state (confirm/ask), not three-band; `0.90` hardcoded, some thresholds in `ResolverConfig` |
| **§2.4** must/cannot-link constraint store | GAP | conflict `resolve()` writes action label only, no side-effects (identity_conflict_service.rs:70) | No merge executor (tombstone/repoint), no split, no pairwise constraint table |
| **§2.4** identity corrections survive re-processing | DIVERGENCE / BUG | `converge_identity_pending` re-runs resolve from seed (async_resolver.rs:26), no constraint memory | A user split could be re-merged on the next resolve; mitigated only because convergence is now manual (§4.3) |
| **§3.1** deterministic `resolve()` precedence | DIVERGENCE / DELIBERATE | "the merge is purely deterministic — ZERO LLM" (enrichment/lib.rs:425-428); first-non-null wins (lib.rs:859) | Was LLM-driven, deliberately converted to pure priority list; `new_with_llm` stub kept for call sites |
| **§3.2** per-field authority map | DIVERGENCE / DELIBERATE | `FieldCategory{Content,Description,Cover,Audio}` → 4 priority lists (lib.rs:498-519) | Per-*category*, not per-field; audio vs bibliographic split achieved, finer grain not |
| **§3.3** absence ≠ null observation | MATCH | no-winner branch keeps last-known-good, deletes provenance, writes no null (lib.rs:880-895) | `non_blank()` treats `Some("")` as absent |
| **§3.1/3.5** provenance retained | MATCH | `SetFieldProvenanceRequest` per winning field (lib.rs:869-876); `work_field_dissents` logs losers | `enrichment_source` work-level summary string too |
| **§3.4** user override sticky (protected) | MATCH | `fp.setter==User → keep current, continue` (enrichment/lib.rs:838-844) | Re-enrichment never overwrites a user-set field |
| **§3.4** override `cleared` release honored | DIVERGENCE / BUG | text path checks setter only (lib.rs:840); cover path checks `&& !fp.cleared` (lib.rs:949) | `cleared=true` releases a cover lock but NOT a text-field lock — asymmetric, latent bug |
| **§3.4** drift detection (strong source disagrees → prompt) | GAP | dissent reasons are PayloadMismatch/FieldConflict/LanguageIncompatible only (sqlite_field_dissents.rs:8-14) | Provider value contradicting a user edit is dropped silently |
| **§3.1** corroboration (N-agree → confidence) | GAP | strict positional priority; no reliability×recency×agreement anywhere | Raw material (dissents) stored but never scored |
| **§4.1** "incomplete" derived not stored | DIVERGENCE / DELIBERATE | `IdentityStatus` stored but always written from `derived_identity_status()` (identity.rs:318-329) | Derived badge + `EnrichmentStatus{Unenriched/Enriched/Thin/Failed}`; no config required-field gate, no τ_commit |
| **§4.2** attempt ledger w/ outcome classes | PARTIAL MATCH | `provider_retry_state` per (work,provider); `OutcomeClass{Success,NotFound,NotConfigured,WillRetry,PermanentFailure,Conflict,Suppressed}` (domain/lib.rs:1286) | Ledger exists; `not_attempted` = absent row (implicit) |
| **§4.2** source_empty ≠ source_unavailable retry posture | DIVERGENCE / BUG | `NotFound` ∈ `is_phase2_terminal()` (domain/lib.rs:1356-1365) → never auto-retried | TARGET wants source_empty retried slow-cadence; code makes it terminal until manual reset |
| **§4.3** automatic background reconciler ("no manual step") | GAP | recurring job **deleted**, replaced by user-triggered `retry_all_incomplete` (services/work.rs:310-316); wired to `POST` route only (router.rs:238) | The single biggest divergence — convergence is now manual, breaking TARGET's core "without manual intervention" guarantee |
| **§4.4** zero-source / manual-entry first-class | PARTIAL MATCH | anchorless `Pending{seed_anchors:None}` supported (identity.rs:283-288) | Stable i64 id (not ULID); dedup/alias-acquisition-later depends on the absent auto-convergence (§4.3) |

---

## Per-decision detail

### §0 — Two load-bearing decisions

**D0.1 (internal opaque key; externals as aliases) — DIVERGENCE / DELIBERATE.**
`WorkId` is `i64` backed by `INTEGER PRIMARY KEY AUTOINCREMENT` (domain/lib.rs:21, migrations/001_initial_schema.sql:52). The key *is* system-minted and opaque — no external ID is the PK — which honors D0.1's substance. Two deviations: (a) it's an autoincrement integer, not a ULID; (b) external IDs are spread across **three** stores rather than one alias set: denormalized scalar columns on `works` (`ol_key, hardcover_id, isbn_13, asin, gr_key`, migrations/001:69-73), a `work_identity_anchors` table (work-grain anchors with confidence/supersession, migration 039), and a typed `external_ids` table (`ExternalIdType` variants incl. ISBN10/13, ASIN, OL work/edition, GR, HC, GB volume; domain/lib.rs:1435-1443). Pragmatic: the scalar columns let nearly every API path filter by ISBN/ASIN without a join; `external_ids` carries the long tail. ULID vs i64 is irrelevant for a single-user embedded DB.

**D0.2 (evidence layer + recomputable projection) — DIVERGENCE / DELIBERATE.**
The append-mostly observation layer `(entity, field, source, value, confidence, observed_at)` does **not** exist. `work_metadata_provenance` keys on `(work_id, field)` and *upserts* the current winner (sqlite_provenance.rs:114-123) — it records *who owns each field now*, with no confidence column and no history of prior values. The closest thing to an audit trail is `work_field_dissents` (migration 060): append-only per merge generation, but it logs the *loser* of each pass, not every observation ever seen. The resolved side is real and genuinely deterministic — `DefaultMergeEngine::merge_impl` (enrichment/lib.rs:703) recomputes one chosen value per field from `current_provenance + provider_results + priority_model`, "ZERO LLM" (lib.rs:425-428). But the projection is **not freely rebuildable offline**: the `works` row is also the store of user-owned values, and no raw provider payload history is retained (only the latest payload per provider in `provider_retry_state.normalized_payload_json`, for replay). So you cannot wipe-and-rebuild without re-fetching every provider.

### §1 — Data model

Two persisted levels: `Work` (domain/lib.rs:392) and `LibraryItem` (lib.rs:495). **No Edition entity exists** — confirmed across all crates (GAP). Every "edition field" rides on `Work`: `title` (lib.rs:397), `publisher` (412), `publish_date` (413), `isbn_13` (417), `asin` (418), `narrator` (419), `duration_seconds` (416), `page_count` (415), plus dual covers `cover_url`/`audiobook_cover_url` (435-444). `media_type` lives on the file (`LibraryItem.media_type`, lib.rs:495; CHECK constraint migrations/001:115), with `Work.monitor_ebook/monitor_audiobook` flags carrying the dual-format intent at work grain. File identity is path-based (`UNIQUE(user_id, root_folder_id, path)`, migrations/001:118), not content-hash. The user-facing record `WorkDetailView` (services/work.rs:96) is a two-table bundle of stored rows, not a runtime field-merge — the merge already ran at enrichment write-time. Note for cross-check: BIG7's "Release" is a transient indexer result (never persisted), not the TARGET's Edition.

### §2 — Identity resolution

A working **binary** hierarchy exists but not the named A/B/C tiers. Work anchors (`ol_key/gr_key/hc_key`) outrank ISBN/ASIN "bridges": `has_work_anchor()` (english_identity_resolver.rs:427), and a quorum winner resting on no hard ID is demoted `Resolved→NeedsConfirmation` (lines 191-196). The TARGET's Tier-C signal set is only partially present — `m4_scoring.rs` weights title 0.45 / author 0.40 / year 0.10 / series 0.05 (lines 23-36); **page-count proximity and explicit ambiguity-class penalties are absent**. The **blocking step is a GAP**: `work_dedup::find_matching_work` linearly scans an in-memory `&[Work]` (work_dedup.rs:53-105) — fine at solo-library scale, O(n²) at TARGET's implied scale. Decision is **two-state**, not the three-band commit/review/create: `should_auto_confirm` = `confidence>=High && score>=0.90` else NeedsConfirmation (matching/lib.rs:177-185); `0.90` is a hardcoded literal, while `confirm_title_jaccard=0.75` lives in `ResolverConfig` — thresholds are split between config and constants.

**Must/cannot-link store — GAP, with a latent correctness risk.** `IdentityConflictService::resolve()` (identity_conflict_service.rs:70) persists the chosen `ConflictResolutionAction` (KeepExisting/AcceptSeparate/ReplaceAnchor/Merge) as a label and stops — **no merge executor** (no tombstone, no `library_items.work_id` repoint, no alias B→A), no split, no pairwise constraint table. Re-resolution (`converge_identity_pending`, async_resolver.rs:26) rebuilds from the seed with no memory of prior user splits, so in principle it could re-merge a user-separated work. In practice the blast radius is small *today* only because auto-convergence was removed (§4.3) — re-resolution runs only on manual trigger.

### §3 — Enrichment

The merge is a **deterministic priority walk**, deliberately de-LLM'd (enrichment/lib.rs:425-428); first provider in the field's category list with a non-null value wins (lib.rs:859). Authority is **per-category, not per-field**: `FieldCategory{Content,Description,Cover,Audio}` → four `PriorityModel` lists (lib.rs:498-519); English audio = `[Audible,Audnexus,HC]`, content/desc/cover = `[HC,GR,Readarr,OL,Audible]`. So narrator/duration rank differently from ISBN/publisher (TARGET's intent ✓) but ISBN and Title share one ordering (finer grain ✗). `provider_policy` table snapshots exist but drive *dispatch selection*, not merge order (the merge hardcodes `PriorityModel::english/foreign`).

Three sub-decisions are clean MATCH: **absence≠null** (no-winner keeps last-known-good and deletes provenance rather than writing null, lib.rs:880-895; `non_blank` guards empty strings), **provenance retained** (per-field rows + dissent log, lib.rs:869-876), and **override stickiness** (`fp.setter==User → keep, continue`, lib.rs:838-844).

Two are defects/gaps. **`cleared` flag — DIVERGENCE/BUG:** the text-field user skip checks `setter==User` only (lib.rs:840), while the cover path checks `setter==User && !fp.cleared` (lib.rs:949). A user who clears a text override stays locked out of provider updates; clearing a cover override works. Asymmetric, almost certainly unintended. **Drift detection — GAP:** dissent reasons are only PayloadMismatch/FieldConflict/LanguageIncompatible (sqlite_field_dissents.rs:8-14); a strong source contradicting a *user* value is dropped silently, never surfaced. **Corroboration — GAP:** strict positional priority; three agreeing sources carry exactly the confidence of one.

### §4 — Recovery & convergence

**Derived incompleteness — DIVERGENCE/DELIBERATE.** `IdentityStatus` (`Pending|Confirmed|Provisional|Conflict|NeedsReview|NotFound`) is stored but always written from `derived_identity_status()`, which reads the anchor set (identity.rs:318-329) — a materialized projection, not free-floating state. Completeness uses `EnrichmentStatus{Unenriched,Enriched,Thin,Failed}` plus per-provider outcome rows; there is **no config required-field set and no τ_commit** — confidence is effectively binary (anchored or not).

**Attempt ledger — PARTIAL MATCH, with a retry-posture BUG.** `provider_retry_state` tracks per `(work_id, provider)`: `last_outcome, attempts, next_attempt_at, suppressed_passes` (sqlite_retry_state.rs). `OutcomeClass` has 7 variants (domain/lib.rs:1286). The empty-vs-unavailable *states* exist (`NotFound` vs `WillRetry`), satisfying the letter of §4.2 — **but** `NotFound ∈ is_phase2_terminal()` (domain/lib.rs:1356-1365), so a "source has no record of this work today" outcome is terminal and never auto-retried. TARGET explicitly wants `source_empty` retried on a slow cadence (long-tail content appears over time); the code requires a manual reset. `not_attempted` is modeled implicitly as an absent row.

**Background reconciler — GAP (the headline).** The recurring convergence job was **deleted**, not merely missing: `WorkService::retry_all_incomplete` is documented as "the convergence the deleted background job used to do… Replaces the removed `enrichment_retry_tick`" (services/work.rs:310-316), and it's wired to a `POST` route + handler only (router.rs:238; handlers/work.rs:676), with the behavioral test stating it "replaces the deleted recurring background [job]" (test_s6_retry_all_incomplete.rs:3). `JobRunner::start()` registers exactly 7 jobs — download_poller, session_cleanup, author_monitor, state_map_cleanup, rss_sync, tag_convergence, call_record_retention (jobs/mod.rs:80-130) — **no enrichment/identity convergence tick.** The DB building block `list_works_due_for_retry` (sqlite_retry_state.rs:281) has no live caller (tests only). So a work that can't resolve on first contact does **not** improve on its own — it improves only when the user clicks "retry incomplete." This directly contradicts TARGET §4.3's core promise.

**Zero-source manual entry — PARTIAL MATCH.** Anchorless works are first-class at add time: `IdentityState::Pending{ seed_anchors: None, .. }` is supported and tested (identity.rs:283-288). Stable internal id (i64, not ULID). The two follow-on guarantees — "participates in dedup later" and "acquires aliases later" — are architecturally possible but depend on the auto-convergence that was removed (§4.3), so in practice an identifier-less work acquires anchors only via manual retry.

---

## Where the TARGET is wrong / naive / over-built for this system

The TARGET is a clean, multi-tenant/enterprise entity-resolution design. Several decisions are genuinely over-built for one user with a few thousand books on SQLite:

1. **Separate Edition entity (§1.1).** Over-built. A solo user typically owns one ebook + one audiobook per title. Flattening edition fields onto `Work` + two `monitor_*` flags + two cover slots covers the dual-format case without a join on every read or a "which edition owns this field" arbitration layer. Readarr does the same. The code's choice is the right scale-down.

2. **Append-mostly observation log with per-value confidence (D0.2).** Over-built. This is event-sourcing for metadata — full historical replay and "every value ever told" querying. For this scale it costs storage + a confidence subsystem to deliver query power nobody will use. The shipped "current-winner provenance + per-generation dissent log" keeps the operationally useful outputs (who set this, what lost, why) without the warehouse.

3. **Per-field authority map (§3.2).** Over-built at full grain — 24 `WorkField` × 7 providers = a dense table no self-hoster will tune. The 4-category model captures the one real distinction (audio vs bibliographic) and stops.

4. **Three-band decision + human-review queue (§2.2/2.3).** Naive for this user. A solo librarian wants the book to appear or not — managing a queue of ambiguous candidates is friction, not value. Two-state (auto-confirm or ask-once) fits better. *However*, the TARGET is right that thresholds should be config; the hardcoded `0.90` (matching/lib.rs:185) is the one place the code under-delivers on its own pragmatism.

5. **Must/cannot-link constrained-clustering store (§2.4).** Half right. The TARGET correctly identifies that user merge/split decisions must survive re-processing — and the code genuinely lacks a merge *executor*, which is a real gap. But the *mechanism* (a pairwise-constraint store feeding a constrained-clustering linker) is academic overkill here. The right fix is "execute the merge as a durable anchor write + repoint, and don't undo it," not a constraint-satisfaction layer.

6. **Drift detection (§3.4) and corroboration scoring (§3.1).** Both low-value friction for this user. The user deliberately set the value; periodically prompting "an API disagrees" is annoyance. N-sources-agree confidence is a metadata-vendor feature; the priority list already encodes the substantive "trust HC over OL" judgment.

7. **`source_empty` slow-cadence retry (§4.2).** The TARGET is arguably *wrong* here, or at least in tension with another constraint: auto-retrying every provider that lacks a book risks hammering hostile sources for content they genuinely don't have. The code's terminal-`NotFound` is defensible. The real gap is the missing `WillRetry` convergence loop, not the `NotFound` policy.

8. **Where the TARGET is simply right and the code is wrong:** the **automatic convergence guarantee (§4.3)**. "A work that can't be resolved on first contact must improve later without manual intervention" is a reasonable, scale-independent product promise — and the code deleted exactly the mechanism that delivered it, leaving a manual button. The `cleared`-flag text/cover asymmetry (§3.4) and the un-executed merge action (§2.4) are also straightforwardly defects, not defensible scale-downs.

---

### Method / falsifiability note

Verdicts on D0.1, §1 grain, §3.4 cleared-flag, §4.2 NotFound-terminal, and §4.3 convergence-removal were re-read from source by the main loop (snippets above are from those reads). The *absence* verdicts (must/cannot-link executor, drift detection, corroboration, blocking index) rest on sub-agent symbol searches — they would be falsified by a caller/table I didn't find; medium confidence. If one matters for a decision, grep for: a `library_items` work_id repoint on conflict-merge, a `must_link`/`cannot_link` table, a `notification` emitted on provider-vs-user disagreement, or any timer registering `retry_all_incomplete`/`converge_identity_pending`.
