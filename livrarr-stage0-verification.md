# Livrarr — Stage 0 Verification Work-Order
### CC investigation (READ-ONLY)

**Mode.** Read-only. **No code changes, no builds.** Confirm or refute each lead against the actual repo with `file:line` evidence, and size the stage it gates.

**Rules.**
- **Refutations are the highest-value output** — they revise the build plan *before* it anchors work. A wrong lead is a good find; say so plainly.
- **Do not rationalize a lead into truth.** Quote the code as it is. If reality differs from the lead, report reality.
- If a lead is only partly right, mark PARTIAL and explain the gap.

**Why.** Stage 0 of `livrarr-build-plan.md` gates every later stage. The plan and the reconciliation (`livrarr-architecture-review-delta.md`) rest on memory-based leads; this pass turns them into confirmed facts.

**Output format — one entry per item:**
- **Verdict:** CONFIRMED / REFUTED / PARTIAL
- **Evidence:** `file:line` + a one-line quote or paraphrase of the actual code
- **Consequence:** what it unblocks, or what in the build plan changes if refuted

---

## V1 — Identity-write funnel  *(gates P2 · Stage 1d · Stage 3)*
- **Lead:** anchor writes exist in `sqlite_work_identity.rs` (supersede/confirm), but a single funnel is unconfirmed.
- **Confirm/refute:** Is there exactly one path that writes or mutates a work's anchors (ISBN/ASIN/external IDs)? **List every call site** that writes anchors.
- **If multiple paths:** that's the gap — Stage 1d (identity-write gate) must introduce the single funnel; estimate its size.
- **Stakes:** P2 ("a failed/unavailable source never writes identity") is unenforceable without one gate.

## V2 — `tag_convergence` write-race  *(gates Stage 4)*
- **Lead:** the list asserted a 60s `tag_convergence` write-race with the reconciler; this may already be per-work serialized.
- **Confirm/refute:** When `tag_convergence` writes `Work` rows, is it serialized per-work (lock / queue / single writer), or can a reconciler write race it? Trace the write path and any locking.
- **Consequence:** serialized → Stage 4 needs only **diff-before-write**; not serialized → Stage 4 adds a **per-work write lock**. Decides 3b's scope.

## V3 — Two-clock separation  *(confirms the #4 reversal + the ledger model)*
- **Lead:** `transport_cache` (cached payloads, transport efficiency) and `provider_retry_state.next_attempt_at` (the retry/outcome clock) are distinct mechanisms with distinct lifetimes.
- **Confirm/refute:** Are they separate, with separate lifetimes? Does anything currently conflate cache-expiry with retry-eligibility?
- **Consequence:** confirms the single-scheduler model (ledger = `next_attempt_at`; cache separate). If entangled, the Stage-2 ledger work is larger.

## V4 — Provider-logic map  *(sizes Stage 2)*
- **Lead:** provider logic is scattered (`goodreads.rs` ~60KB, `google_books.rs` ~43KB) across `provider_queue` / `provider_client` / `provider_policy` / `transport_cache`.
- **Map — for each provider, where does each concern live:** (a) HTTP/transport, (b) rate-limit/queue, (c) caching, (d) translation/normalization, (e) query-building. Mark which are **already shared-per-source** vs **per-provider-duplicated**.
- **Consequence:** sizes the Stage-2 adapter / shared-infra split, and reveals how much shared infra already exists (the loop/breaker may need less new code than assumed).

## V5 — Priority models  *(sizes Stages 1 & 5)*
- **Lead:** language priority already exists as static `english()` / `foreign()` models selected at merge start (`enrichment/lib.rs:239-315`); a per-category authority structure exists.
- **Confirm:** the models, the selection point, and the category structure (Content / Description / Cover / Audio?). Is media-type already a discriminator anywhere?
- **Consequence:** confirms #6's right-sized approach (static models, **no** DB table) and sizes the Stage-5 `cjk()`/`korean()` model addition.

## V6 — Scheduler primitives  *(sizes Stage 4)*
- **Lead:** `list_works_due_for_retry`, `converge_identity_pending`, `next_attempt_at` exist; the reconciler (`enrichment_retry_tick`) was deleted and has no live caller.
- **Confirm:** these primitives exist and are wired (or unwired). Any live caller of the loop today? What sets `next_attempt_at` currently (backoff? `source_empty`?)?
- **Consequence:** sizes Stage 4 — how much is "wire up existing primitives" vs new.

## V7 — Multi-user substrate  *(confirms D2)*
- **Lead:** `library_items` UNIQUE includes `user_id`; provenance is keyed by `user_id`. The works/anchor catalog's sharing scope is the open question.
- **Confirm:** Is library membership already user-scoped? Is the works/anchor catalog global or per-user today? Are override/lock rows user-scoped or global?
- **Consequence:** confirms D2 (shared catalog + per-user override layer, nullable `user_id`) and tells us whether override rows need a `user_id` column added.

---

## After the pass
- Update the build plan's Stage-0 checkboxes and the delta's "verify" flags to confirmed/refuted.
- **Any REFUTED lead → revise its stage before building.**
- Net: every stage is sized; the two gating facts (V1, V2) are settled; Stage 1 can open.
