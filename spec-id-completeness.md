---
feature: id-completeness
stage: spec
status: draft
version: 2
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009]
---

# Spec: id-completeness (unified identity path — wiring + safe ID harvest)

> **Scope marker.** Make every book carry as many *trustworthy* cross-provider IDs as possible — because
> enrichment is anchor-gated, so a missing ID is a provider that cannot contribute (§0b ST-003). Three moves:
> (1) route every identity-resolving door **and** the background loop through the one engine
> `settle_identity`; (2) harvest the IDs each work is missing — auto-attaching IDs linked by a **hard
> cross-reference**, and **holding fuzzy guesses** (the audiobook is the poster child) for a one-click user
> affirm so a wrong guess never displays; (3) drive the background top-up off the convergence loop.
>
> **Builds on:** the engine (`spec-unified-identity-path`, done), the door-wiring trace (`design-uip-wiring`),
> and the convergence loop's selection/pacing (`spec-convergence-unified-path` / `design-convergence-selection-fix`).
> **Supersedes** `design-uip-wiring` §4 (the Sprint-E Confirmed-gate). **Corrected per cross-family review R1
> (2026-06-23)** — see `design-uip-id-completeness` §9. **REQ-009 amended per architecture review R-3
> (2026-06-23, PO):** dead-end suppression is threshold-based (mark unobtainable after 3 failed tries),
> because the resolver exposes no per-anchor no-candidates signal — a timeout and a true miss are
> indistinguishable.

## 0a. Design Principles

Choices committed to. If a requirement conflicts, the principle wins.

- **One identity road.** Every present and future door delegates the *entire* identity decision to
  `settle_identity` — no door re-implements "resolve, then stamp the badge." Removes the duplicated wrappers.
- **IDs are the lever on metadata.** Enrichment fetches **only** by stored anchor; a missing anchor = that
  provider contributes nothing. Maximizing trustworthy IDs is the goal (overrides speed-first re-chase gating).
- **Trust is tiered, never silent.** A user's pick is verified. An ID obtained via a **hard cross-reference**
  the work already holds (shared ISBN/ASIN/work-key) is as trustworthy as the pick — auto-attach. An ID from
  a **fuzzy title/author match** is a guess — held unused and surfaced for affirm; never silently trusted
  (mirrors the resolver's existing refusal to auto-commit a no-hard-id match, ST-006).
- **Safe by construction.** A guessed (`pending`) ID lives **only** in the anchor ledger, never the
  denormalized `works.*` columns enrichment reads — so a wrong guess can never be fetched or displayed
  (the R1 P0 correction).
- **Monotonic, no-clobber, one-way.** Harvest only *fills missing* IDs and *raises* trust; it never
  overwrites a confirmed ID, never downgrades, and writes no enrichment payload during the identity step.
- **Bounded before throttle.** The background top-up is paced (bounded batch + conservative cadence) and its
  worst-case volume is computed and **gated** before activation — the only interim protection until Part 2's
  quota layer (deferred, §4).

## 0b. System Truths

Facts about the codebase the feature must conform to. Each source is a line range read this session.

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | `crates/livrarr-identity/src/async_resolver.rs:293` (`settle_identity`); `find_referencing_symbols` → empty | The one identity engine — resolves, writes the badge + merges anchors itself, returns an audit report. **Zero callers** — unwired. | Adding another resolve→badge wrapper; assuming any door reaches it today. | High |
| ST-002 | `settle_identity` body `async_resolver.rs:293-421` (read) | Terminal short-circuit covers **only** `Conflict`/`NotFound`/`NeedsReview`. **`Confirmed`/`Provisional` do NOT short-circuit** — they fall through to a full `resolver.resolve()` fan-out, then no-op the badge. | Wiring refresh to call it unconditionally (re-introduces the ~2s/work cost Sprint-E removed — insight 55). | High |
| ST-003 | `derive_anchor_query` `crates/livrarr-enrichment/src/provider_queue.rs:69`; `AnchorQuery` `crates/livrarr-domain/src/lib.rs:1332`; the no-anchor skip in `dispatch_enrichment` (read `:440-661`) | Enrichment fetches **only** by the work's denormalized `works.*` anchors (`isbn_13`/`gr_key`/`hc_key`/`ol_key`/`asin`); `AnchorQuery` admits no title/author (no text fallback); a provider with no stored anchor gets `SkippedNoAnchor`+`NotFound`, no fetch. | Assuming a missing ID can be back-filled by enrichment; writing a *pending* ID into `works.*` (it WOULD be enriched). | High |
| ST-004 | migration `crates/livrarr-db/migrations/039_work_identity_anchors.sql`; `AnchorSetter` `crates/livrarr-domain/src/identity.rs:47`; `AnchorConfidence` `:39` | The anchor ledger `work_identity_anchors` already carries `setter ∈ {user, auto_isbn, auto_search, import, redirect}` and `confidence ∈ {confirmed, pending, superseded}` — the vocabulary already distinguishes user-pick / hard-match / fuzzy-match / not-yet-confirmed. | Inventing a new provenance model; claiming the distinction must be built from scratch. | High |
| ST-005 | `confirm_anchor` `crates/livrarr-db/src/sqlite_work_identity.rs:9-116`; `merge_missing_anchors` `:203-236`; `set_identity_pending` `:355-400`; add path `crates/livrarr-metadata/src/work_service.rs:619,811` | The writers do **not** use the vocabulary honestly: `confirm_anchor` always writes `confidence='confirmed'` **and syncs the `works.*` column**; `merge_missing_anchors` stamps `setter='import'`; the add path collapses non-user to `auto_search`; the **only** `pending` writer `set_identity_pending` writes an **empty-string sentinel**, not a real anchor value. **Nothing stores a real `pending` guess today.** | Assuming a pending-guess writer exists; assuming any confirmed-anchor write skips the `works.*` sync. | High |
| ST-006 | `resolve` `crates/livrarr-identity/src/english_identity_resolver.rs` (~line 74, read); `work.rs:211` (`user_confirmed=true`); `select_providers` `english_identity_resolver.rs:204` | A `user_confirmed` seed carrying a work anchor (ol/gr/hc) **skips the provider fan-out entirely** (zero-network trust shortcut). So interactive add-from-search harvests **only the pick's IDs** (no audiobook ASIN); automated paths (`user_confirmed=false`) fan out to up to 6 sources. The resolver also refuses to auto-commit a no-hard-id fuzzy match (downgrades to NeedsConfirmation). | Assuming interactive adds already harvest broadly; making the safe/fuzzy split a new concept (the resolver already embodies it). | High |
| ST-007 | `design-convergence-selection-fix.md` §2; `converge_identity_pending`/`list_works_due_for_retry` orphaned (refs empty) | The background convergence loop is **not built**. Its designed selector picks identity-`Pending` **OR** enrichment-incomplete works by a `works.next_convergence_at` clock, bounded batch + cadence. | Treating the loop as existing; rebuilding its selection/pacing here (this feature widens + guards it). | High |
| ST-008 | `list_identity_pending_works` `crates/livrarr-db/src/sqlite_work.rs:819` (`WHERE identity_status='pending'`) | The existing identity-pending sweep selects by status — a work parked `pending` is re-selected every cycle. | Parking an awaiting-affirm work as `pending` (it would loop — M9). | High |
| ST-009 | `refresh` `crates/livrarr-metadata/src/work_service.rs:1286` calls `reset_for_manual_refresh` (`:1299`, deletes `provider_retry_state`); the function's own comment (`:1317-1321`) states provider suppression does **not** survive a manual refresh; the `suppressed` list is then built from `list_retry_states` (`:1326`) — now empty. | A manual refresh wipes `provider_retry_state` **before** the anchor chase reads it, so refresh-path provider-suppression is always empty; `provider_retry_state` is the rate-limit clock, not a durable dead-end store. | Reusing `provider_retry_state` for cross-refresh dead-end suppression (REQ-002/009 use a durable marker instead). | High |

## 1. Problem Statement

Books carry too few cross-provider IDs, so enrichment is starved: each provider can only fetch by *its own*
ID, and a work missing that ID gets nothing from it (ST-003). The shortfall is worst exactly where it hurts
most — the books a user **adds from search**: that path trusts the pick and skips the harvest entirely
(ST-006), so the work keeps only the picked source's IDs and reliably lacks the audiobook. Nothing makes a
second pass. And the engine that should own identity is unwired (ST-001).

The obvious fix — harvest everything automatically — is **unsafe**: auto-linking the wrong edition or
audiobook is this project's recurring wrong-book failure (AC-020 guard; the C1 veto, `74bec92`). The user
verified only the one pick; the rest are machine-inferred.

This feature resolves the tension: wire the **one engine** everywhere, **auto-attach** the IDs that link by a
hard cross-reference (safe), and **hold fuzzy guesses** for a one-click affirm (the audiobook), with the
guess stored where enrichment can't see it until the user says yes.

## 2. Requirements

- **REQ-001 — One identity road (wiring).** Every identity-*resolving* door — add-from-search, manual add,
  per-file import, Readarr import, list import, series monitor — and the background loop MUST route their
  identity decision through `settle_identity` (mode per door: interactive vs background; source per door).
  The duplicated resolve→badge wrappers (`ensure_identity_and_enrichment`'s leg, `complete_anchors`,
  `retry_all_incomplete`'s direct resolve) are removed. **Author monitor is the deliberate exception** — it
  *asserts* a hard OL key from an authoritative source rather than resolving, so it does not fan out
  (routing it would be a wasteful no-op per work, ST-002/ST-006).

- **REQ-002 — Confirmed-chase, durably suppressed (supersedes wiring §4 + the Sprint-E `!=Confirmed` gate,
  ST-002).** Refresh re-chases a work's **missing** IDs, **including on `Confirmed` works** — gated to:
  **skip when no IDs are missing**, and **skip IDs marked unobtainable** (REQ-009). The dead-end protection
  MUST be the **durable** unobtainable marker (REQ-009), **NOT** `provider_retry_state`, which a manual
  refresh deletes before the chase reads it (ST-009) — provider-suppression cannot survive refresh. A manual
  refresh is user-initiated, so it MAY re-attempt an unobtainable ID (an explicit "try again"); the
  unbounded-churn protection that matters runs on the background loop (REQ-006/009). The "skip when nothing
  missing" smart is also re-added at the caller (`settle_identity` lacks it, ST-002).

- **REQ-003 — Safe ID harvest (auto-attach).** An ID obtained by querying another source via a **hard
  cross-reference the work already holds** (shared ISBN/ASIN/work-key — an exact match, not a guess) MUST be
  attached as `confirmed` (via `confirm_anchor`, syncing `works.*`) and is used by enrichment immediately.

- **REQ-004 — Guessed ID hold (the safety property).** An ID obtained **only** by a fuzzy title/author match
  (no hard bridge) MUST be stored by a **new** repository writer `record_pending_anchor` as
  `confidence='pending'`, `setter='auto_search'`, in `work_identity_anchors` **only** — and MUST NOT write
  the denormalized `works.*` column. Because enrichment reads `works.*` (ST-003), a pending guess is
  therefore **never fetched or displayed** until affirmed. No live writer does this today (ST-005).

- **REQ-005 — Affirm promotes.** A user affirm MUST promote a pending guess via `confirm_anchor`
  (`confidence='confirmed'` + `works.*` sync), which unlocks its enrichment. The UI MUST surface pending
  guesses (read from `work_identity_anchors` — a new read path; `works.*` cannot represent them) as a
  **gentle, non-blocking** "we think this is the audiobook — confirm?" affordance (PO decision (a),
  2026-06-23). No modal; the work is fully usable without affirming.

- **REQ-006 — Background top-up.** The convergence loop's selector (ST-007) MUST be **widened** to also select
  `Confirmed`-but-ID-incomplete works (today: identity-`Pending` OR enrichment-incomplete only). It MUST
  **exclude** (a) works that already hold an outstanding `pending` guess for the missing ID — otherwise the
  loop re-chases and re-guesses them every tick (the M9 indefinite-loop violation, ST-008); and (b) works
  whose **every** remaining missing ID is marked **unobtainable** (REQ-009) — so the loop converges instead
  of churning a dead-end forever. Convergence routes only through the one pipeline (no side-door writes).

- **REQ-007 — Pre-activation volume gate.** Before the widened top-up is enabled on a live library, the
  worst-case daily provider-call volume for the `Confirmed` backlog — which is nearly the whole library,
  since interactive adds are ID-starved (ST-006) — MUST be computed and shown to stay within Google Books'
  daily quota, with the bounded-batch + conservative cadence as the drain; and the live DB MUST be
  snapshotted before first activation. (Extends `spec-convergence-unified-path` REQ-007 to the widened set.)

- **REQ-008 — Monotonic, no-clobber, identity-only.** The harvest MUST only fill *missing* IDs and *raise*
  trust; it MUST NOT overwrite a confirmed ID, downgrade a badge, or write any enrichment/metadata payload
  during the identity step (inherits the engine's contract).

- **REQ-009 — Durable dead-end suppression (threshold-based; amended per review R-3, PO 2026-06-23).** The
  resolver cannot distinguish a genuine dead-end from a transient outage — a provider timeout and a true
  "no such record" both collapse to the same empty result, and no per-anchor no-candidates signal is
  exposed. So a missing ID is marked **unobtainable** after it **fails 3 consecutive background top-up
  attempts** (threshold configurable via TOML; default 3). The count is a **durable** per-`(work,
  anchor_type)` marker that survives a routine refresh's state reset (ST-009 — so NOT
  `provider_retry_state`); a successful harvest clears it. The background top-up (REQ-006) MUST respect it:
  it does not re-chase an unobtainable anchor, and a work whose **every** remaining missing ID is
  unobtainable (or held as a pending guess) falls out of the loop's selection — the loop converges, with no
  indefinite churn (M9) and no wasted Google Books volume. A persistent outage that fails 3× is therefore
  treated as a **clearable** dead-end — accepted (the cost of a false dead-end is one click): the marker MAY
  be cleared by an explicit user "try again" (manual refresh), which re-attempts it.

## 3. UI/Interface Design

The only UI is the **gentle affirm prompt** (REQ-005): a non-blocking affordance on a book that holds a
pending guess, reading the guess from `work_identity_anchors`, offering one-click confirm. Exact placement
(work-detail inline vs. a small review queue) and whether multiple pending guesses batch into one affordance
are design-stage questions (§5 Q1). Everything else is backend.

## 4. Non-Requirements

- **Part 2 — throttle / quota governance.** No daily-quota counter, no global rate cap, no consolidation of
  the fragmented rate-limit locations. Separate later feature; REQ-007's volume gate is the interim guard.
- **The convergence loop's core selection/pacing.** Built by `spec-convergence-unified-path` /
  `design-convergence-selection-fix`; this feature **widens** the selector and **adds guards**, it does not
  rebuild the clock/batch/cadence.
- **Full "harvest every ID up front at add-time" (the a6 anchor-first phase).** Interactive add stays
  instant (the trust shortcut is preserved, ST-006); the harvest happens on the **background top-up** and on
  refresh, not by making interactive add fan out. The exhaustive add-time harvest remains a possible later
  unit.
- **The resolver's matching logic.** Used as-is (provider fan-out, quorum, hard-vs-fuzzy distinction); not
  changed.
- **mp3-format audiobook specifics.** Out of scope (m4b matters; mp3 does not).
- **UI surfacing of identity states other than the pending-guess affirm** (Pending/NeedsReview/Conflict
  surfacing is a separate concern).

## 5. Open Questions

| ID | Question | Resolve at |
|----|----------|-----------|
| Q1 | Affirm-prompt placement (work detail vs review queue) + whether multiple pending guesses batch. | Design |
| Q2 | How "fuzzy vs hard" is detected **at harvest time** so REQ-003/004 route correctly — the resolver knows (hard id vs title-match); confirm the signal is exposed to the harvest decision. | Architecture |
| Q3 | The exact REQ-007 worst-case volume numbers for the live library, vs the GB daily quota. | Pre-activation (blocking) |
| Q4 | Does the in-memory anchor read-model expose `setter`/`confidence` to the harvest logic (needed to *act* on provenance, not just store it)? | Architecture |
| Q5 | Per-door interactive-vs-background mode + `ConflictSource` for REQ-001 (the engine's `ConflictSource` enum already has the 7 door variants; the loop + manual-retry need new variants). | Architecture |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Each resolving door, driven through its **real entry path** (no injected identity
  state), settles identity **via `settle_identity`** with the expected `(mode, source)` — not a legacy
  writer. (Faithful door→engine test — the metadata-refactor lesson, insight 46.)
- [ ] **AC-002** (REQ-001): Author monitor adds a work **without** a resolver fan-out (the negative test that
  locks the exception).
- [ ] **AC-003** (REQ-002): Refresh of a **fully-anchored** `Confirmed` work triggers **no** resolver
  fan-out (Sprint-E no-regression); a `Confirmed` work **missing** an obtainable ID does re-chase it.
- [ ] **AC-004** (REQ-002/006/009): After a missing ID **fails 3 consecutive background attempts** and is
  marked **unobtainable** (a durable count that survives a refresh's state reset — NOT
  `provider_retry_state`), the **background loop** does not re-chase it on later ticks, and a work whose only
  missing IDs are all unobtainable is **not re-selected** (the loop converges). *(A manual refresh MAY still
  re-attempt — user-initiated.)*
- [ ] **AC-005** (REQ-003): An ID linked by a shared hard identifier is attached `confirmed` and enrichment
  uses it on the same pass.
- [ ] **AC-006** (REQ-004 — the safety test): A **pending** guessed ASIN produces `SkippedNoAnchor` /
  `NotFound` for Audible/Audnexus (no fetch, no display) and is absent from `works.asin`, until affirmed.
- [ ] **AC-007** (REQ-004): `record_pending_anchor` writes `work_identity_anchors` (`confidence='pending'`,
  `setter='auto_search'`, real value) and leaves `works.*` untouched.
- [ ] **AC-008** (REQ-005): A user affirm promotes the pending row to `confirmed`, syncs `works.*`, and
  enrichment then fires for that provider.
- [ ] **AC-009** (REQ-005): The UI lists a work's pending guesses (read from the ledger) and exposes the
  one-click confirm; the work is fully usable with the guess unaffirmed.
- [ ] **AC-010** (REQ-006): The widened loop selects a `Confirmed`-but-ID-incomplete work; it does **not**
  re-select a work that already holds an outstanding pending guess (no loop).
- [ ] **AC-011** (REQ-007): The chosen batch + cadence are documented with the computed worst-case daily
  Google Books call count (incl. the Confirmed-backlog drain) shown ≤ the GB daily quota; a DB snapshot is
  taken before first activation.
- [ ] **AC-012** (REQ-008): During any identity harvest, no enrichment/metadata payload is written and no
  confirmed anchor or badge is overwritten/downgraded.
