---
feature: unified-identity-path
stage: spec
status: draft
version: 4
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008]
---

# Spec: unified-identity-path

> **Scope marker — the engine only.** This feature builds *one complete "identify a book" path*: a single
> process that takes a work and a caller mode and settles the work's identity to a final, correct state.
> **Explicitly deferred (separate features):** (a) wiring the existing/future entry doors (Add, manual import,
> refresh, manual Retry, background convergence) to call this path; (b) the background *selection + pacing*
> that decides which works to converge and when. This spec defines the path; it does not move any caller onto
> it yet. (PO decision, 2026-06-20: "write the one identity path to rule them all… we'll worry about the
> wiring later.")

## 0a. Design Principles

Choices committed to. If a requirement conflicts, the principle wins.

- **One identity authority.** There is exactly one path that settles a work's identity badge. It is built so
  every present and future caller can delegate the *entire* identity decision to it — no caller need
  re-implement "resolve, then stamp the badge." (Removes today's three duplicated copies — §0b ST-003.)
- **Same destination, tiered patience.** For the same inputs, the path reaches the **same identity** no matter
  who calls it (the identity is caller-independent). The only caller-visible knob is *patience*: an
  interactive caller (a person is waiting) and a background caller (unattended) differ **only** in how a
  *non-resolving* verdict is handled — never in the identity reached for a resolvable work.
- **Terminal honesty.** A genuine dead-end — an ambiguous `NeedsConfirmation` (only a human can pick) or a
  `Conflict` — becomes a surfaced, terminal state, never silent limbo (M9). A *transient* outage is **not** a
  dead-end: it stays an eligible `Pending` and retries. (Bounding the retries of a perpetually-unresolved work
  — so "no indefinite loop" holds end-to-end — is the deferred background-pacing layer, §4, not this engine.)
- **Monotonic, no-clobber, one-way.** The path only fills *missing* anchors and *raises* the badge. It never
  clobbers an established anchor, never downgrades a settled badge, never re-litigates a user-resolved
  terminal, and never writes enrichment/metadata payloads (identity → enrichment is one-way).
- **Idempotent.** Running the path on an already-settled work is a safe no-op for identity (no downgrade, no
  re-open, no duplicate anchors).

## 0b. System Truths

Facts about the codebase the path must conform to. Each Source is a line range read this session.

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | `crates/livrarr-domain/src/lib.rs:98-128` (`IdentityStatus`, read this session) | The stored identity badge has **six** values: `Pending` (default, no confident identity), `Confirmed` (a work anchor OL/GR/HC), `Provisional` (an ISBN/ASIN bridge, no work anchor; enriches, upgrades to Confirmed later), `Conflict` (open anchor dispute — terminal until the user resolves), `NeedsReview` (a non-interactive path exhausted resolution — surfaced), `NotFound` (legacy — see ST-004; rejected every payload; terminal until manual refresh). | Inventing a new identity state, or treating `Conflict`/`NotFound`/`NeedsReview` as non-terminal. | High |
| ST-002 | `crates/livrarr-domain/src/identity.rs:224-248` (`Resolution`), `:341-348` (`PendingReason`), `:394-402` (`IdentityConflictKind`); `crates/livrarr-identity/src/english_identity_resolver.rs:154-167` (no-responders → `Unresolved{NoCandidates}`), `:188-197` (Tier-B guess → `NeedsConfirmation`), `:353-371` (`run_quorum` → `QuorumTie`, incl. ties among *anchored* clusters); `crates/livrarr-metadata/src/work_service.rs:932-965` (legacy flatten); `crates/livrarr-db/src/sqlite_work_identity.rs:244-283` (established-anchor contradiction check); `crates/livrarr-identity/src/async_resolver.rs:293-298` (`is_terminal_pending`, the buggy legacy helper) | `resolver.resolve` returns one of **four** verdicts: `Resolved{identity}` (a work anchor or a hard-id bridge fixed the identity), `NeedsConfirmation{candidates}` (candidates exist but **no resolving hard id** — ambiguous; only a human can pick), `Conflict{..}` (quorum tie / conflicting same-kind anchor), `Unresolved{reason,captured}` (no provider responded / a provider abstained — **transient**; the resolver's own comment: "converges on a later pass"). **Every `Unresolved` reason is transient/retryable**: the resolver's *only* self-produced reason is `NoCandidates` (no responders, `english_identity_resolver.rs:154-167`); `OlUnavailable`/`MalformedResponse` are provider outages/garbage. The resolver produces **no** "deterministic exhausted-search" reason today. `LowConfidence` is **not** a resolver verdict — it is a label the *legacy* flatten wrapper stamps on `NeedsConfirmation`/`Conflict` when collapsing them to `Pending` (`work_service.rs:934,959`); the engine maps the raw `Resolution` and never uses it. The genuine non-resolving **terminal** cases are `NeedsConfirmation` (ambiguous) and `Conflict` — distinct variants, never `Unresolved`. A `Resolution::Conflict` carries a `kind` (`identity.rs:394-402`), **but the kind alone does not tell you whether the conflict contradicts a *particular* work's established anchor.** `run_quorum` returns `Conflict{QuorumTie}` for an equal-size no-majority split **even among *anchored* clusters** (`english_identity_resolver.rs:353-371`) — so a `QuorumTie` is **not** necessarily anchorless provider noise; it can be a genuine disagreement between two work anchors. The resolver is stateless about a given work's stored identity, so the settled-work decision (REQ-003 From-`Provisional`/From-`Confirmed`) MUST be made at the engine layer by **comparing the fresh resolution's captured anchors against the work's established confirmed anchor** (the same-kind work-anchor contradiction check, `sqlite_work_identity.rs:244-283`) — **never** by the `Conflict` kind. The legacy helper `is_terminal_pending` terminalizes `NoCandidates`/`LowConfidence` (`async_resolver.rs:293-298`) — terminalizing **`NoCandidates`** (and collapsing verdicts into `PendingReason` buckets) is the bug being corrected: the engine MUST NOT terminalize `NoCandidates`, though it **does** still terminalize an ambiguous `NeedsConfirmation` in background mode (REQ-005). | Treating a transient `Unresolved` (incl. `NoCandidates`) as a dead-end (premature terminalization); treating the ambiguous `NeedsConfirmation` as transient (infinite loop); reviving the `PendingReason`-bucket or `LowConfidence`-as-verdict model. | High |
| ST-003 | `set_identity_status` writers (whole-repo enumeration this session): `ensure_identity_and_enrichment:2904`, `retry_all_incomplete:1489`, `converge_pending_due:1621/1635` (off-road), `finish_created_work:3000`; plus `run_unified_enrichment:3137-3357` and `converge_identity_pending` (`crates/livrarr-identity/src/async_resolver.rs:52-56`) | The resolve→badge step is **duplicated across three wrappers** (add `:2904`, manual retry `:1489`, convergence `:1621/1635`). A fourth writer, `finish_created_work:3000`, is **create-time derivation** — it persists the already-derived badge once at create, NOT a resolve-wrapper (separate concern, out of this feature's collapse). The shared enrichment road **never writes the badge** (zero `set_identity_status` in `:3137-3357`); the "reusable" background primitive **merges anchors but never flips the Confirmed/Provisional badge**. | Adding another copy of the resolve→badge wrapper logic; this feature collapses the **three wrappers** into one (it does not touch the create-time derivation writer). | High |
| ST-004 | `EnrichmentResult.identity_not_found` hard-set `false` at every construction site (`crates/livrarr-enrichment/src/lib.rs:1439,1515,1724`); its only reader is the consume-branch `work_service.rs:3080-3082`. The validator still **computes** `ValidationOutcome.all_success_rejected` (`crates/livrarr-enrichment/src/llm_validator.rs:72,280`) but a whole-repo search finds **no live consumer** of it (only its def/compute/return + one unit test) — the old wiring to `identity_not_found` (Step 8.5) was deleted (`design-metadata-refactor-road.md:26`). | **`NotFound` has no live producer.** `identity_not_found` is always `false`, so the only branch that writes `NotFound` (`work_service.rs:3082`) never fires. `NotFound` is a **legacy** stored value — present in old rows, never newly produced. | Designing the path to **produce** `NotFound`; assuming a NotFound write is needed. | High (~95%) |
| ST-005 | `crates/livrarr-db/src/sqlite_work_identity.rs:9-116` (`confirm_anchor`), `:203-238` (`merge_missing_anchors`); `sqlite_work.rs:565` (`set_identity_status`), `sqlite_work_identity.rs:406` (`set_needs_review`) | Merging an anchor writes the anchor ledger + the denormalized work columns but **never** the identity badge — anchor-write and badge-write are separate operations. Anchor merges are additive/monotonic (an established anchor is untouched). | Assuming a successful anchor merge flips the badge (it does not — that is the bug in ST-003). | High |

## 1. Problem Statement

There is no single, complete path that settles a book's identity. The logic is scattered across at least
three callers, each re-implementing "resolve, then stamp the badge" (§0b ST-003). The one shared background
primitive that was supposed to consolidate this has a latent defect — it merges anchors but never flips the
badge to `Confirmed`/`Provisional` — and the shared enrichment road does not settle identity at all.

Two consequences follow. **First, the same bug must be fixed N times**, and the copies drift: the background
copy never flips the badge for a resolvable work (ST-003/ST-005) yet terminalizes a *transient* `NoCandidates`
as a false dead-end (ST-002) — the silent-limbo *and* premature-terminal failures M9 forbids. **Second, there
is no clean foundation to wire new callers to** — every door re-grows its own identity logic.

This feature builds the **one complete identity path** — owning every end-state and taking a single
patience knob — as the foundation. It does **not** wire any caller to it (deferred) and does **not** decide
which works to converge in the background (deferred). It builds the engine, correctly and completely, so the
later wiring feature is pure plumbing.

## 2. Requirements

- **REQ-001** — Single identity authority. There MUST be exactly one path that, given a work and a caller
  mode, settles that work's identity to a final `IdentityStatus`. The path performs identity resolution and
  the badge write together, so a caller delegating to it needs no identity logic of its own. *(Building the
  path; moving callers onto it is out of scope — §4.)*

- **REQ-002** — Complete end-state coverage. For any input work, the path MUST leave the work in exactly one
  defined identity state it **produces**: `Confirmed`, `Provisional`, `Conflict`, `NeedsReview`, or `Pending`.
  `Pending` is produced for **either** a transient, retryable `Unresolved` verdict (no provider responded / a
  provider abstained) **or** an ambiguous `NeedsConfirmation` verdict in **interactive** mode (a person can
  pick a candidate later — REQ-005). (`NotFound` is a legacy state the path never produces — ST-004 — but
  respects if already present, REQ-006.) No input may be left in an undefined state, nor terminalized while its
  verdict is still transient.

- **REQ-003** — Verdict → badge mapping (monotonic). The mapping is **monotonic relative to the work's
  current badge** (REQ-004): the badge only ever rises (`Pending`→`Provisional`→`Confirmed`) or moves to a
  genuine `Conflict`; a weak verdict never lowers it.
  - **From `Pending`:** `Resolved` with a work anchor (OL/GR/HC) → `Confirmed`; `Resolved` with only an
    ISBN/ASIN bridge → `Provisional`; `Unresolved` (any reason — no provider responded / a provider abstained;
    `NoCandidates`/`OlUnavailable`/`MalformedResponse`) → stays `Pending`, merge any captured anchors, still
    eligible to retry (**mode-independent** — a transient outage is never terminalized, ST-002);
    `NeedsConfirmation` (candidates exist, no resolving hard id — ambiguous) → per caller mode (REQ-005);
    `Conflict` (quorum tie / conflicting same-kind anchor) → `Conflict`.
  - **From `Provisional`:** a **work anchor upgrades** it to `Confirmed`; a fresh verdict that does **not
    contradict the established anchor** (an ISBN/ASIN bridge, a transient `Unresolved`, an ambiguous
    `NeedsConfirmation`, or a `Conflict` whose captured anchors don't contradict the established anchor — e.g.
    an anchorless `QuorumTie`) **leaves it `Provisional`** — never back to `Pending`/`NeedsReview`; only a fresh
    resolution whose anchors **genuinely contradict the established anchor** (a different same-kind work anchor,
    decided by comparison against the established anchor — ST-002, **not** by the `Conflict` kind) → `Conflict`.
  - **From `Confirmed`:** never lowered; a new anchor merges (stays `Confirmed`); a fresh `Conflict` whose
    captured anchors do **not** contradict the established anchor (e.g. an anchorless `QuorumTie`) **leaves it
    `Confirmed`**; only a fresh resolution whose anchors **genuinely contradict the established anchor** (a
    different same-kind work anchor, by comparison against the established anchor — ST-002) → `Conflict`.

- **REQ-004** — Monotonic, no-clobber, one-way. The path MUST only add *missing* anchors and *raise* the
  badge. It MUST NOT overwrite an established anchor, MUST NOT downgrade a settled badge
  (`Confirmed`→`Provisional`/`Pending`), and MUST NOT write any enrichment or metadata payload (cover,
  description, tags, series strings, …). Identity flows to enrichment, never the reverse.

- **REQ-005** — Patience knob (the only caller-visible difference). The path MUST accept a caller mode
  (interactive vs background). The mode MUST NOT change the identity reached for a resolvable work, and MUST
  NOT change the handling of a **transient `Unresolved`** verdict — both modes keep such a work `Pending` and
  eligible to retry (a transient outage is never terminalized; *when* to stop retrying a perpetually-
  unresolved work is the deferred background-pacing concern, §4, not this engine). The mode governs **only**
  the treatment of an **ambiguous `NeedsConfirmation`** verdict (candidates exist, no resolving hard id) on a
  **currently-`Pending`** work: in **background** mode it transitions to terminal `NeedsReview` (no human is
  waiting — surfaced, not silent limbo, M9); in **interactive** mode it remains `Pending` (a person can pick a
  candidate). An already-settled (`Provisional`/`Confirmed`) work is **never downgraded** to
  `NeedsReview`/`Pending` by a non-resolving verdict (REQ-004). *(This is the knob that lets one path safely
  serve both an attended add and an unattended loop.)*

- **REQ-006** — Respect terminal states. The path MUST NOT re-litigate a work already in a terminal state:
  an open `Conflict`, a `NotFound`, and a `NeedsReview` are left untouched. A `NeedsReview` returns to play
  only via an explicit user/manual reset — never an automatic re-attempt (M9: terminal, never an indefinite
  retry loop).

- **REQ-007** — Monotonic, safe re-run. Running the path again on an already-settled work MUST never
  **downgrade** a badge, re-open a terminal, or duplicate anchors. It MAY **improve** — upgrade
  `Provisional`→`Confirmed`, or fill a previously-missing anchor — when a stronger verdict arrives. (On a
  `Confirmed` or terminal work with nothing to add, it is a no-op.)

- **REQ-008** — Single home for every badge transition (engine owns the write). The path itself **performs**
  every identity-badge + anchor write it produces (the live states in REQ-002) — this is REQ-001's "resolve
  and write together." A caller never re-derives or re-writes the badge; the path returns a **report** of what
  it did (final badge + what changed) for logging/audit only, **not** a value for a caller to persist. *(In
  this engine-only build the duplicate writers still exist; the cross-caller enforcement that no OTHER code
  writes the badge lands with the wiring feature. `NotFound` is excluded — no live producer, ST-004 / Q-001.)*

## 3. UI/Interface Design

**No UI.** This is a backend engine. Surfacing `Pending`/`NeedsReview`/`Conflict`/`NotFound` to the user is a
separate concern and out of scope (§4).

## 4. Non-Requirements

Explicit scope exclusions.

- **Wiring callers to the path.** Add, manual import, refresh, manual Retry, and background convergence are
  NOT moved onto the path in this feature. The path is built and tested standalone; the cutover is the next
  feature.
- **Background selection + pacing.** Deciding *which* works to converge and *when* (the due-incomplete query,
  the backoff clock, batch bounds, cadence) is the convergence feature, not this one.
- **Enrichment / materialize.** Filling in details, covers, tags is out — the path settles identity and stops.
- **Harvest-all-IDs / metadata pre-caching.** Considered this session and **explicitly cut** (PO, 2026-06-20):
  collecting every cross-provider ID up front + caching payloads is a pipeline restructure with a throttling
  risk, and is not needed for the convergence fix. A possible separate future unit — not this engine.
- **The resolver's matching logic.** The path *uses* the existing resolver (provider fan-out, quorum,
  deterministic-then-LLM matching) as-is; it does not change how the resolver decides a match.
- **Distinguishing a true exhausted-search from a transient no-response.** Today the resolver cannot tell "no
  provider responded (outage)" from "searched and the book is genuinely unknown" — both surface as
  `Unresolved{NoCandidates}` (ST-002). Per the Option-A decision (PO, 2026-06-20) the engine treats **all**
  `Unresolved` as transient/retryable. Adding a distinct "exhausted-search" resolver reason — so a
  genuinely-unknown work can terminalize instead of retrying forever — is a **resolver contract change,
  deferred** (a possible future unit). Safe failure mode meanwhile: a perpetually-no-response work stays a
  visible, eligible `Pending` (never a false terminal); bounding its retries is the deferred pacing layer.
- **Producing / writing `NotFound`.** It has no live producer (ST-004) — the path never creates it. Cleaning
  up the now-dead signal, the always-`false` field, and the dead write-branch is a separate hygiene task, not
  this feature.
- **UI surfacing** of any identity state (§3).
- **Thin / enrichment-status concerns.** `EnrichmentStatus` is the other track; the path touches only the
  identity track.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Does the path produce/write `NotFound`? | **RESOLVED — out of scope** (PO + grounded, 2026-06-20) | `NotFound` has **no live producer** (ST-004): the all-rejected LLM detector is gone, the signal is hard-wired `false`, the write branch is dead. The path produces **5** states and never creates `NotFound`; it respects any existing `NotFound` row as terminal (REQ-006). Deleting the dead signal/field/branch is separate hygiene (§4). |
| Q-002 | In **interactive** mode, does an ambiguous `NeedsConfirmation` stay `Pending` instead of surfacing `NeedsReview` while a person is waiting? | **RESOLVED — yes** (PO lock, 2026-06-20) | The only mode-dependent non-resolving verdict is `NeedsConfirmation`: interactive stays `Pending` for a later user pick; background → `NeedsReview`. No deterministic exhausted-search resolver verdict exists today; adding one is deferred (§4). |
| Q-003 | Are **two** caller modes enough (interactive, background), or is a third needed (a `Bulk` latency tier exists in the resolver)? | **RESOLVED — two modes** (PO lock, 2026-06-20) | Patience is binary (attended/unattended); the resolver's latency tiers (incl. `Bulk`) are orthogonal pacing that map onto the two modes. Confirm the exact tier→mode mapping when tracing the resolver at architecture. |
| Q-004 | Does an **interactive** `NeedsConfirmation` stay `Pending` awaiting a user pick, while **background** `NeedsConfirmation` → `NeedsReview`? | **RESOLVED — yes** (PO lock, 2026-06-20) | Matches the attended/unattended split (REQ-005). |
| Q-005 | Should the path harvest **all** IDs + pre-cache metadata? | **RESOLVED — no, cut** (PO, 2026-06-20) | Reversed: scope creep onto the foundation + a throttling risk, and not needed for the convergence fix. Deferred to a possible separate future unit (§4). |
| Q-006 | Does the path re-attempt a `Provisional` work on a later pass (to upgrade it to `Confirmed`)? | **RESOLVED — yes for `Provisional`, no for `NeedsReview`** (2026-06-20; r1 R-002) | Re-resolving a `Provisional` is monotonic and only improves (REQ-007). `NeedsReview` is terminal (REQ-006, M9) — auto-re-attempting it would be the indefinite retry loop M9 forbids; user reset only. No exhaustive ID top-up (harvest cut). |
| Q-007 | Is `NoCandidates` a deterministic dead-end (terminalize in background) or transient (retry)? | **RESOLVED — transient/retry** (PO Option A, 2026-06-20; r1 both families) | The resolver emits `NoCandidates` **only** when no provider responded (`english_identity_resolver.rs:154-167`), a transient case its own comment says "converges on a later pass." Terminalizing it would kill provider outages. The engine keeps all `Unresolved` transient; the genuine terminal cases are `NeedsConfirmation`/`Conflict`. A distinct exhausted-search reason is a deferred resolver change (§4). |
| Q-008 | Can the engine always tell, from a resolver `Conflict` payload, whether it contradicts a *particular* work's established anchor? (A `QuorumTie` among anchored clusters surfaces only the representative's side — `run_quorum:365-371`.) | **RESOLVED (the WHAT) — detection mechanism deferred to architecture** (r4 openai R-007) | The settled-work rule is **contradiction-based, not kind-based** (REQ-003, ST-002): the engine compares fresh captured anchors against the established confirmed anchor (`sqlite_work_identity.rs:244-283`). Architecture must confirm the `Conflict` payload surfaces enough to detect a contradiction (or have the resolver expose both tied anchors). The WHAT is fixed here; the detection mechanism is HOW. |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001/003): A `Pending` work whose resolver returns a work anchor (OL/GR/HC) ends
  `Confirmed`, with the new anchor persisted.
- [ ] **AC-002** (REQ-003): A `Pending` work that resolves to only an ISBN/ASIN bridge (no work anchor) ends
  `Provisional`.
- [ ] **AC-003** (REQ-003/005): In **background** mode, a `Pending` work whose resolver returns
  `Unresolved{NoCandidates}` (no provider responded) stays `Pending` and remains eligible to retry — it is
  **NOT** terminalized to `NeedsReview` (a transient outage must not become a premature dead-end, ST-002).
- [ ] **AC-004** (REQ-005): The treatment of `Unresolved{NoCandidates}` is **mode-independent** — in
  **interactive** mode the same work also stays `Pending` (the patience knob does not touch a transient verdict).
- [ ] **AC-005** (REQ-005): A resolvable work reaches the **same identity** (same anchors, same badge) under
  both interactive and background mode.
- [ ] **AC-006** (REQ-003): A **`Pending`** work with a transient `Unresolved` verdict
  (`NoCandidates`/`OlUnavailable`/`MalformedResponse`) stays `Pending`, merges any captured anchors, and is
  left eligible to retry.
- [ ] **AC-007** (REQ-003): A quorum tie / conflicting same-kind anchor ends `Conflict` (terminal).
- [ ] **AC-008** (REQ-004): The path never overwrites an established anchor and never downgrades `Confirmed`;
  a populated anchor and a `Confirmed` badge survive a re-run with a weaker verdict.
- [ ] **AC-009** (REQ-004): During an identity settle, no enrichment/metadata payload is written (no cover,
  description, tags, or series strings change).
- [ ] **AC-010** (REQ-006): A work in `Conflict`, `NotFound`, or `NeedsReview` is left untouched by the path.
- [ ] **AC-011** (REQ-007): Running the path twice on a `Confirmed` work is an identity no-op (same badge, no
  duplicate anchors).
- [ ] **AC-012** (REQ-002/006): The path never writes `NotFound` (no live producer, ST-004); an existing
  `NotFound` row is left untouched (per AC-010).
- [ ] **AC-013** (REQ-008): The path itself performs the badge + anchor writes for the work; its return value
  is a report (final badge + what changed) for audit only — a caller does not need to write the badge.
- [ ] **AC-014** (REQ-003/007): A `Provisional` work that re-resolves to a work anchor upgrades to
  `Confirmed`, with its existing bridge anchors preserved.
- [ ] **AC-015** (REQ-003/004): A `Provisional` work that re-resolves to a verdict that does **not** contradict
  its established anchor (ISBN/ASIN-only `Resolved`, a transient `Unresolved`, an ambiguous `NeedsConfirmation`,
  or a `Conflict` whose anchors don't contradict it — e.g. an anchorless `QuorumTie`) stays `Provisional` —
  never downgraded to `Pending`/`NeedsReview`; a fresh resolution that genuinely contradicts the established
  anchor still follows AC-007/REQ-003 → `Conflict`.
- [ ] **AC-016** (REQ-003/005): In **background** mode, a `Pending` work whose resolver returns
  `NeedsConfirmation` (ambiguous, no resolving hard id) ends terminal `NeedsReview`.
- [ ] **AC-017** (REQ-005): In **interactive** mode, the same `NeedsConfirmation` work stays `Pending`
  (awaiting a user pick), not terminalized.
- [ ] **AC-018** (REQ-003/004): Preservation is keyed on *non-contradiction of the established anchor*, never
  on the `Conflict` *kind*: a `Confirmed`/`Provisional` work that re-resolves to a `Conflict` whose captured
  anchors do **not** contradict its established anchor (e.g. an **anchorless** `QuorumTie`) is **not**
  downgraded (badge + anchor survive); **but** a re-resolution whose anchors **do** contradict the established
  anchor (incl. a `QuorumTie` arising **between anchored clusters** that carries a different same-kind work
  anchor) **does** raise `Conflict` (per AC-007).
