# Design Direction: ID Completeness via the Unified Identity Path (revised)

**Status:** direction for review — NO code. Consolidates the 2026-06-23 PO discussion, grounded against
source this session.
**Relationship to prior artifacts:** absorbs the wiring mechanics of `design-uip-wiring.md` (4 insertion
points, legacy deletion — still valid) and **supersedes its §4 "Confirmed-gate" decision** (we now *do*
chase IDs on Confirmed works — §5B). Builds the background-loop selection/pacing from
`design-convergence-selection-fix.md` (still valid) with one selector change (§5D).

---

## 0. The reframe (PO, 2026-06-23)

**ID completeness is a first-class objective: more cross-provider IDs ⇒ better metadata, mechanically.**
Enrichment fetches **only by stored ID** — the fetch vocabulary `AnchorQuery` admits no title/author, so
there is no text-search fallback (`crates/livrarr-domain/src/lib.rs:1332`); a provider with no stored ID for
a work is skipped with a `SkippedNoAnchor` record and contributes nothing
(`crates/livrarr-enrichment/src/provider_queue.rs:440`, comment: *"anchor acquisition is the identity
track's job"*). So acquiring IDs is **the** lever on metadata quality. This **overrides** the earlier
speed-first stance (skip re-chasing Confirmed works).

## 1. What the current plan actually builds — grounded gap analysis

- **One identification pass already harvests across providers.** `resolve` fans out to the eligible sources
  in parallel and `project_cluster` merges *every agreeing provider's* IDs onto the winner — not just the
  confirming one (`english_identity_resolver.rs:553`). Max fan-out = 6 sources (`select_providers:204`):
  OpenLibrary, Hardcover, Google Books *(if API key)*, Goodreads *(if LLM)*, Audible, Audnexus
  *(background tier only)*.
- **But the gap splits hard by path:**
  - **Interactive add (search → pick):** the seed is `user_confirmed=true` (`livrarr-handlers/src/work.rs:211`)
    and carries the picked source's work key, which trips the **zero-network trust shortcut**
    (`english_identity_resolver.rs:~74`) → **no fan-out** → the work keeps **only the pick's IDs**, and
    systematically lacks the audiobook ASIN. *(An ISBN/ASIN-only pick with no work key does still fan out —
    `work.rs:195` comment.)*
  - **Automated paths** (Readarr, series monitor, list import, refresh of non-Confirmed): `user_confirmed=false`
    → full fan-out. Gap = whoever didn't answer that second.
  - **No second pass** fills missed IDs anywhere.
- **Consequence:** the books the user personally searches for and adds are the **most ID-starved** — the
  inverse of what we want.

## 2. The trust boundary (PO insight, 2026-06-23)

The user verified **only the one pick**. Auto-harvesting the rest re-introduces the project's recurring
**wrong-book / wrong-edition** failure (different translation, abridged vs. unabridged audiobook, omnibus) —
guards exist *because* it has bitten before (AC-020 ISBN-collision guard; the C1 "different work-keys veto a
fuzzy-title merge" fix, commit `74bec92`). And the affirm-gap **persists regardless of when** we harvest: a
machine-linked ID is unaffirmed whether fetched at add-time or in the background. **So we cannot silently
auto-attach everything.**

## 3. The resolution — safe vs. guessed

Split the harvest by *how the ID was matched*, not by *when*:

- **Safe → auto-attach.** IDs linked by a **shared hard identifier** (the work already carries an
  ISBN/ASIN/work-key, and another source is queried *by that exact ID*). Same book by construction
  (the rare reused-ISBN case is what the AC-020 collision-guard catches).
- **Guessed → hold for affirm.** IDs from a **title/author match with no hard bridge** — the audiobook is
  the poster child (an ebook pick has no ASIN, so Audible must be title-searched). This is a genuine guess.
- **This already matches the resolver's own philosophy:** when it lands a title/author match with nothing
  hard to anchor it, it **refuses to auto-commit** — it downgrades to "needs confirmation"
  (`english_identity_resolver.rs:~190`, the Tier-B rule). We extend that same rule to the top-up.

## 4. Provenance — already modeled, not used honestly (grounded)

The data model **already carries the tags the split needs** — we don't invent provenance, we make the
writers tell the truth.

- `work_identity_anchors` (migration `039_work_identity_anchors.sql`) stores per ID:
  - **`setter`** ∈ `user | auto_isbn | auto_search | import | redirect` (`AnchorSetter`, `identity.rs:47`).
    *Already names exactly: user-pick vs hard-ID match vs fuzzy-search match.*
  - **`confidence`** ∈ `confirmed | pending | superseded` (`AnchorConfidence`, `identity.rs:39`).
    *Already has a `pending` slot for "suggested, not yet affirmed."*
- **But the writers are loose:**
  - `merge_missing_anchors` (the resolver's harvest) stamps **everything `Import`**
    (`sqlite_work_identity.rs:~225`).
  - the add path collapses everything non-user to **`AutoSearch`** via a lossy catch-all
    (`work_service.rs:619,811` — `_ => AutoSearch`; `AutoIsbn` appears mainly in tests).
  - `confirm_anchor` **always** writes `confidence='confirmed'` (`sqlite_work_identity.rs:9-116`) — **nothing
    writes a real `pending` anchor** *(CONFIRMED, R1: the only `pending` writer `set_identity_pending` writes
    an empty-string sentinel, `sqlite_work_identity.rs:355` — see §9)*.
- **So today a fuzzy audiobook guess and a hand-picked book are indistinguishable in the data.** The fix is
  to populate the existing fields honestly: hard match → `auto_isbn` (auto-trust), fuzzy → `auto_search` +
  `confidence='pending'` (hold for affirm).

## 5. The design (revised)

- **A. Wiring (unchanged).** Every door + the background loop route through `settle_identity`; 4 insertion
  points + legacy deletion per `design-uip-wiring.md` §3/§5.
- **B. Confirmed-gate REVERSED (supersedes wiring §4).** Refresh **does** re-chase a Confirmed work's
  **missing** IDs — gated not by "is it Confirmed" but by "are any IDs missing AND not dead-ended" (C).
- **C. Re-add the two smarts the engine dropped vs. `complete_anchors`** (which the wiring deletes):
  (1) **skip when no IDs are missing** (`complete_anchors` short-circuits on `missing.is_empty()` —
  `async_resolver.rs:~127`; `settle_identity` does not); (2) **dead-end suppression** — don't re-hammer an
  unfindable ID every pass (e.g. an ASIN for an ebook-only title), reusing the existing per-provider
  retry-suppression `complete_anchors` already honored. **These are the interim safety for aggressive
  chasing; the full throttle/quota is still Part 2 (deferred, §7).**
- **D. Background top-up via the convergence loop.** The loop is the off-critical-path vehicle. **Widen its
  selector** to also catch "Confirmed but missing harvestable IDs" — today it selects only identity-`Pending`
  OR enrichment-incomplete works (`design-convergence-selection-fix.md` §2), so a Confirmed-but-ID-short work
  is invisible to it. **Two guards from review R1:** (1) the widened selector MUST **exclude** works that
  already hold an outstanding `pending` guess for the missing ID — otherwise the loop re-chases and re-guesses
  them every tick, the M9 indefinite-loop violation (Google R-003; cf. the `identity_status='pending'` sweep
  `list_identity_pending_works`, `sqlite_work.rs:819`). (2) Widening to the Confirmed backlog enlarges the
  worst-case volume the convergence pre-activation safeguard (`spec-convergence-unified-path.md` REQ-007) must
  bound — because nearly every interactively-added book is Confirmed-but-ID-short (§1), the backlog can be the
  whole library; the REQ-007 volume calc MUST be **redone for the widened set** and gate activation. The
  loop's bounded-batch + conservative cadence drain it slowly, so this is a go/no-go calc, not a blocker
  (both families P0/P1).
- **E. Provenance-honest tagging + the safety mechanism (corrected per review R1).** A **new** repository
  writer `record_pending_anchor` persists a guessed (fuzzy) anchor to `work_identity_anchors` ONLY — real
  value, `setter='auto_search'`, `confidence='pending'` — and does **NOT** write the denormalized `works.*`
  column. Load-bearing: enrichment derives its fetch IDs **solely** from `works.*` (`derive_anchor_query`,
  verified `provider_queue.rs:69`), so a pending guess is **invisible to enrichment by construction** — no
  wrong cover/description can be fetched or shown. A **hard-ID** match uses the existing `confirm_anchor`
  (writes `confidence='confirmed'` + syncs `works.*`), so it is used immediately. **Affirm** = promote the
  pending row via `confirm_anchor`, which then syncs `works.*` and unlocks enrichment. The UI reads pending
  guesses from `work_identity_anchors` (a new read path — `works.*` cannot represent them). *(R1, both
  families P0: the original §5E was unsafe — it never prevented a pending ID from reaching `works.*`.)*
- **F. Affirm surface — RESOLVED (PO 2026-06-23): option (a), the gentle prompt.** A book with a pending
  guessed ID shows a subtle non-blocking "we think this is the audiobook — confirm?" affordance. The ID is
  held unused until affirmed (E); the prompt is how the user grants the affirm that unlocks its metadata.

**Net flow:** interactive add stays instant (trust shortcut preserved) → background loop tops up the missing
IDs → hard-linked ones attach automatically → fuzzy ones (the audiobook) wait, pending, for a one-click
affirm before their metadata is used.

## 6. Resolved decision (PO, 2026-06-23)

**F — the affirm surface: option (a), the gentle prompt.** A book with a pending guessed ID shows a subtle,
non-blocking "we think this is the audiobook — confirm?" affordance (not a modal). Rationale: without it, a
correctly-guessed audiobook's metadata sits empty until the user happens to go looking; the prompt surfaces
the value while the held-unused-until-affirmed rule (§5E) keeps a wrong guess from ever displaying.

Open sub-questions for the spec: exact placement (work detail vs. a review queue), and whether multiple
pending guesses batch into one affordance.

## 7. Scope note (what grew, what stays deferred)

This grew from "wire the engine" to "wire it **and make ID-completeness honest**." It pulls the a6
*anchor-first* ambition forward in spirit (maximize *safe* IDs), and pulls **one** Part-2 element forward —
**dead-end suppression** (the interim backoff `complete_anchors` already had). It does **not** pull the rest
of Part 2 forward: the global rate cap, daily-quota counter, and per-instance→global GR pacing stay deferred.

## 8. To verify before/at spec

- **(P1)** The ≈80% claim that nothing writes `confidence='pending'` — enumerate **all** writers of
  `work_identity_anchors.confidence`.
- Whether the in-memory anchor read-model exposes `setter`/`confidence` to the harvest-decision logic (needed
  to *act* on provenance, not just store it).
- Exact production write-sites of `auto_isbn` (saw the add path produce `user`/`auto_search`; `auto_isbn`
  appeared mainly in tests — confirm whether any live path tags a hard match).
- That widening the convergence selector to "Confirmed + ID-incomplete" does not balloon the per-tick batch
  beyond the GB daily-quota safeguard (`spec-convergence-unified-path.md` REQ-007). **[Now a required
  go/no-go activation gate — §5D guard 2.]**

## 9. Review round 1 — findings & resolutions (cross-family, 2026-06-23)

Both reviewers (Gemini + Codex) returned **FAIL** with convergent findings; each was verified against source
this session. The direction holds; the implementation is corrected above. This section supersedes the
confidence hedges in §4/§8 for these items.

- **R1 (P0, both families) — safety leak [RESOLVED §5E].** Enrichment reads IDs only from denormalized
  `works.*`, which has no confidence concept (`derive_anchor_query`, verified `provider_queue.rs:69`); a
  pending guess synced there would be enriched and displayed. Fix: pending guesses live in
  `work_identity_anchors` only, never `works.*` — safe by construction.
- **R2 (P1/P2, both) — no real `pending` writer [CONFIRMED + RESOLVED §5E].** The only `pending` writer,
  `set_identity_pending`, writes an empty-string sentinel, not a real anchor value (verified
  `sqlite_work_identity.rs:355`); `confirm_anchor` always writes `confirmed` + syncs `works.*` (`:9-116`).
  §4's ≈80% claim is now **confirmed**. Fix: add `record_pending_anchor`.
- **R3 (P1, Google) — affirm-hold would loop [RESOLVED §5D guard 1].** A selectable work holding a pending
  guess would be re-chased forever (M9). Fix: exclude outstanding-pending-guess works from the selector.
- **R4 (P0/P1, both) — selector widening vs GB quota [GATED §5D guard 2].** Redo the REQ-007 pre-activation
  volume calc for the widened Confirmed-backlog set; it gates activation.

**Net: 3 internal fixes + 1 pre-activation gate. No new PO decision; the (a)-prompt direction (§6) stands.**
