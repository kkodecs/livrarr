# Livrarr — Stage 0 Verification RESULTS

Companion to `livrarr-stage0-verification.md` (the work-order). Records the verdicts of the
read-only Stage-0 audit so they survive outside the session transcript.

**Provenance.** Verdicts were delivered in-session only (transcript
`f76261a4-2451-4e09-8c05-3c98fa9e57d9`, synthesis turn 2026-06-15T20:23:44Z) and recovered here
2026-06-16. Confidence is **not uniform** — record it honestly:

- **V1, V2** — orchestrator re-read the source directly. High.
- **V6** — re-verified against live code this session (2026-06-16): `converge_identity_pending`
  (`async_resolver.rs`) exists with **zero referencing symbols**; `enrichment_retry_tick` is absent
  (find_symbol empty). Matches the in-session finding. High.
- **V3, V4, V5, V7** — subagent-cited file:line, consistent with insights 50/51/54, but **not each
  re-read** by the orchestrator. Treat the `file:line` refs as leads to confirm before building on them.

---

## V1 — Identity-write funnel  *(gates P2 · Stage 1d · Stage 3)*
- **Verdict:** REFUTED — there is **no single funnel**.
- **Evidence:** ≥5 anchor-write paths. The hot path: `create_work` (`sqlite_work.rs:1219`)
  `INSERT INTO works (… ol_key, gr_key, … isbn_13, asin …)` writes anchor columns directly with **no
  `confirm_anchor` and no `work_identity_anchors` ledger row** — same class as F1. Only
  `create_work_with_anchor` (`sqlite_work.rs:1273`) adds the ledger write, and only for OL.
  `external_ids` has no production writer (REQ-007 removed it — matches insights 50/51).
- **Consequence:** Stage 1d is real work — funnel ~3–4 sites; Path B (`create_work`, the hot creation
  path) is highest-risk. **P2 ("a failed/unavailable source never writes identity") is unenforceable
  until this funnel exists.**

## V2 — `tag_convergence` write-race  *(gates Stage 4)*
- **Verdict:** REFUTED — no race.
- **Evidence:** `tag_convergence` does **not** write `Work` rows — only
  `update_library_item_tag_status(item.id, …)` (`library_items`, per-item) + the file
  (`tag_convergence.rs:114–116`). The reconciler writes `series`/`series_members`. The row sets are
  disjoint.
- **Consequence:** Stage 4 shrinks — **no per-work write lock needed**; diff-before-write already
  exists (`merge_generation`).

## V3 — Two-clock separation  *(confirms the #4 reversal + the ledger model)*
- **Verdict:** CONFIRMED — separate, zero coupling.
- **Evidence:** `transport_cache` (in-mem, 300s `Instant` TTL, consume-once;
  `transport_cache.rs:29`, set `main.rs:219–220` `Duration::from_secs(300)`) vs `next_attempt_at`
  (persisted UTC backoff clock; struct `livrarr-db/src/lib.rs:2081`, written `record_will_retry`
  `sqlite_retry_state.rs:126–139`, read `list_works_due_for_retry` `sqlite_retry_state.rs:291–293`
  and suppression filter `work_service.rs:1322`). No shared TTL constant, data source, or decision path.
- **Consequence:** the single-scheduler model is safe; the Stage-2 ledger is scoped to
  `provider_retry_state` only.

## V4 — Provider-logic map  *(sizes Stage 2)*
- **Verdict:** (map — no CONFIRMED/REFUTED label; treated as an enumeration task).
- **Evidence:** Queue / breaker / rate-limit / retry are **already fully shared**
  (`DefaultProviderQueue`); anchor-derivation is shared. Per-provider-duplicated = HTTP boilerplate,
  normalization (genuinely per-provider), query-building; caching exists only on Audnexus.
- **Consequence:** the adapter split needs **less new code than the file sizes imply** — breaker/limiter
  = zero new code per provider.

## V5 — Priority models  *(sizes Stages 1 & 5)*
- **Verdict:** CONFIRMED (media-type sub-claim REFUTED).
- **Evidence:** static `english()`/`foreign()` `PriorityModel`, 4 categories
  (Content / Description / Cover / Audio), selected via `for_language` at merge
  (lead cited `enrichment/lib.rs:239–315`). **No media-type discriminator exists** anywhere.
- **Consequence:** `cjk()`/`korean()` is a clean add, **no DB table**. If audiobooks ever need a
  different priority, that is net-new work.

## V6 — Scheduler primitives  *(sizes Stage 4)*
- **Verdict:** EXIST + UNWIRED (built and tested, **zero live callers**).
- **Evidence:** `list_works_due_for_retry` (trait `livrarr-db/src/lib.rs:2176`, impl
  `sqlite_retry_state.rs:281`) and `converge_identity_pending` (`async_resolver.rs`) are fully built —
  callers exist **only in tests**. `enrichment_retry_tick` is **truly deleted** (no
  `crates/livrarr-server/src/jobs/enrichment.rs`). `next_attempt_at` IS actively written by backoff
  (`provider_queue.rs:689,703,807`, `audible.rs:111,158,201`, `google_books.rs:215+`,
  `provider_client.rs:348+`). *Re-verified live 2026-06-16: `converge_identity_pending` has zero
  referencing symbols; `enrichment_retry_tick` symbol absent.*
- **Consequence:** the retry/convergence loop is **~50–80 lines of wiring, not new logic**. This is the
  parked convergence of insight 54 — and the subject of the **owed V6 convergence decision** (restore a
  background convergence tick vs. make manual-refresh semantics explicit).

## V7 — Multi-user substrate  *(confirms D2)*
- **Verdict:** REFUTED (the lead's premise — the catalog is **per-user today, not global**).
- **Evidence:** `works.user_id NOT NULL` (`001_initial_schema.sql:53`); `library_items` UNIQUE is
  `(user_id, root_folder_id, path)` (`001_initial_schema.sql:109–120`); `work_identity_anchors` gained
  `user_id` in `044_anchor_per_user_uniqueness.sql`. Override tables mostly carry `user_id`; the lone
  exception is `work_metadata_provenance` (PK `(work_id, field)` only, no `user_id`; migration 028).
- **Consequence:** D2 "shared catalog + per-user override layer" is **NOT** "add `user_id` to overrides"
  — it is a **core-table migration** (reshape `works`/anchors/`external_ids` from per-user to shared).
  Much bigger than the lead assumed.

---

## Net
- **The two gating facts are settled, both REFUTED:** V1 (no identity-write funnel — Stage 1d is real
  work, P2 blocked until done) and V2 (no `tag_convergence` race — Stage 4 shrinks).
- **Smaller-than-feared:** V4 (shared infra already exists), V5 (static model add, no table),
  V6 (~50–80 lines of wiring).
- **Bigger-than-assumed:** V7 (shared catalog = core-table migration, not an override-column add).
- **Owed decision:** V6 convergence — background tick vs. explicit manual-refresh semantics. Blocks the
  V1/V6/V7 architecture intervention (the "B" fork).
