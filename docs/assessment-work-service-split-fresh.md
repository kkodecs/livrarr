# Independent assessment — the work_service god box (M-005)

**Written 2026-07-10, per the PO's anchoring-control assignment: this was produced BEFORE reading
`design-work-service-split.md` or `reviews-work-service-split.md`.** Sources read: the audit
(status reconciliation + M-005 + roadmap), `work_service.rs` in full (3 chunked reads),
`convergence_service.rs` in full, the `WorkService` trait, the behavioral-test inventory, git
history of the file, and one compile probe. Anchoring disclosure: the handoff file itself leaked
four coupling-map facts and the reviewers' one-line verdicts; I re-verified every leaked fact in
source myself (all four held) rather than assuming them.

---

## 1. What is actually wrong (evidence, not vibes)

Three candidate problems, in order of how real the evidence says they are:

### 1a. Regrowth-by-default — REAL, and the core problem

The file grew 3,684 → 3,742 lines **during a remediation cycle that explicitly tried to shrink
it** ("Phase 2 … shrink the god file", commit `8a4bed1f`) and despite a −346-line extraction
(`convergence_service.rs`). Net: ~+400 lines of new material landed here in ~3 weeks (git log:
covers consolidation `e6a1f2f7`, merge-two-works `94a692bf`, Phase-5 identity-key work
`d692ad05`, `488b262c`).

Why new code lands here — two structural attractors:

- **The trait attractor.** `WorkService` is a 20-method kitchen-sink contract
  (`crates/livrarr-domain/src/services/work.rs:350`). Any new work-level capability becomes a
  trait method (merge-two-works did exactly this), and the impl must land in the file that
  implements the trait — or be hand-delegated out, which is friction someone must choose to pay.
- **The capability attractor.** `WorkServiceImpl` is the only place holding db + enrichment +
  http + llm + resolver together (`work_service.rs:32-65`). Any feature needing two of those is
  cheapest to write as one more method here.

A split that does not weaken at least one attractor will regrow. That is the design bar.

### 1b. Comprehension cost — real, secondary

146.9K / 3,742 lines: 5× the crate's next-largest module (`series_query_service.rs`, 67.7K), too
big to load whole into an AI working context (3 chunked reads), scroll-hostile for a human.
Mitigated day-to-day by Serena symbol nav and the file's good section banners. A tax, not an
outage.

### 1c. Field-level coupling — mostly NOT the problem (verified)

I built the field→concern matrix from the full read. It is cleaner than "god object" suggests:

| Field (`work_service.rs:32-65`) | Used by |
|---|---|
| `db` | everything (it's the DB; expected) |
| `enrichment` | refresh reset (:1403), run_unified (:3073, :3083) — enrich-road only |
| `http` | discovery lookups, phase-1 cover (:2890), cover gate/materialize (:3133-3219) |
| `llm` | `llm_filter_search` ONLY (:2269) — **discovery-only** |
| `data_dir` | covers/detail/delete paths |
| `refresh_locks` | `refresh` ONLY (:1397) |
| `bulk_refresh_users` | `try_start_bulk_refresh` ONLY (:2030) |
| `lookup_cache` | `lookup_filtered` ONLY (:1748, :1804) — **discovery-only** |
| `resolver` | discovery fast-path (:1720), refresh re-chase (:1425), add-time settle (:2748), scatter pre-completion (:3051) |
| `http_client`, `merge_engine`, `tag_service` | **DEAD** — all three carry `#[allow(dead_code)]` (:46, :53, :55) |

No method mutates another concern's state behind its back; every shared mutable is a
single-concern sync primitive. The one legitimate cross-concern coordinator is
`run_unified_enrichment` (:3028) with **exactly 4 callers** (Serena-verified): `refresh` :1480,
`try_dedup_by_normalized` :2654, `ensure_identity_and_enrichment` :2801,
`convergence_service.rs:137`. Plus `settle_identity` (already an external fn) called from 3
sites here.

**Consequence:** this is a god *box* (everything in one room), not a god *object* (everything
touching everything). The fix is rooms with meaningful walls — not a re-architecture of object
state.

### Deflation note — the original urgency is consumed

M-005's stated payoff was "the lever that makes M-002/M-003/M-004 fixable without fear"
(audit :327). Those three are **fixed, without the split** (reconciliation table). What's left is
maintainability: regrowth control + comprehension, plus B2's remainder (`lookup_*` methods still
on the service). No correctness deliverable rides on this. That calibrates ambition: **zero
behavior change is justified by this feature.**

## 2. Line budget by concern (from the full read, ±)

| Concern | ~Lines | Share | Self-contained? |
|---|---|---|---|
| Discovery/search (lookup, lookup_filtered, eager_match, llm_filter, 4 provider lookups, pick/cover-rank helpers, CachedLookup, in-file tests :3502-3742) | ~1,250 | 33% | **Yes** — nothing in the lifecycle calls into it; fields http/llm/lookup_cache/config-read/resolver-fast-path |
| Add/create core (add :466-926, resolve_identity, preflight, try_dedup, find_or_create_author, ensure_identity_and_enrichment, race-loser, finish_created_work) | ~980 | 26% | No — this IS the door→road core |
| Enrichment orchestration (run_unified_enrichment :3028-3281) | ~290 | 8% | The shared road; 4 callers |
| CRUD/queries (get/detail/list/paginated/update/delete/search_works) | ~340 | 9% | Yes — db-only |
| User merge (preview/merge_works + conflicts fn) | ~165 | 4% | Yes — db-only |
| Refresh/converge/bulk shims (refresh :1388-1506, delegates, chaseable) | ~200 | 5% | refresh shares the road |
| Covers/files (upload/download_cover, delete/is_supported/unproxy) | ~130 | 4% | Yes |
| Constructors/stubs/impl-headers | ~280 | 7% | — |

## 3. Facts that gate the shape choice (all verified this session)

1. **The visibility dispute is settled: Gemini is wrong.** Compile probe (scratchpad
   `visibility_probe.rs`, rustc, ran, printed 14): a child module CAN read its parent module's
   private struct fields, two levels deep. Pattern A (module directory, submodules over the same
   struct) has **zero visibility cost**. Conversely the *sibling* pattern needs `pub(crate)`
   fields — which is exactly why `db` and `resolver` are already `pub(crate)` (:40, :64) for
   `convergence_service.rs`.
2. **The sibling-extraction precedent carries a where-clause tax.** Each free function in
   `convergence_service.rs` repeats the full 6-generic, ~12-bound clause (:31-49, :241-259) —
   ~40 of its 422 lines are bounds. Any extraction that keeps functions generic over the whole
   `WorkServiceImpl<D,E,H,L,M,T>` pays this per function.
3. **Killing the dead fields shrinks the generics.** `merge_engine: M` and `tag_service: T` dead
   → the type can become `WorkServiceImpl<D,E,H,L>`; move discovery (sole `llm` user) out and it
   is `WorkServiceImpl<D,E,H>`. Every impl-block header, the convergence where-clauses, and all
   5 constructors get simpler. This is the cheapest real win in the file.
4. **Behavioral coverage of the orchestration paths EXISTS but is unverified-green and not in
   CI.** `tests/behavioral/` carries direct suites: `test_mc_add_doors.rs`,
   `test_mc_refresh_orchestration.rs`, `test_consolidation_work_service.rs` (44.9K),
   `test_wcc_add.rs`, `test_id_completeness.rs` (49.4K), `test_unified_identity_path.rs`,
   `test_s6_retry_all_incomplete.rs`, `test_se_refresh_gate.rs`, plus eager-match/discovery
   suites. In-file tests cover only discovery helpers. **First concrete work item under any
   shape: run this suite and pin a green baseline.**
5. **The house pattern for god-service surgery is trait-split + single impl** (insight 39:
   SettingsService → 7 narrow traits, ONE `LiveSettingsService` struct; handlers bind `Has*`).
   And the crate already demonstrates B2's target for one provider: `lookup_openlibrary` is a
   1-line delegate (:2399) into `livrarr-external-data`.

## 4. Options

- **O0 — do nothing.** Rejected: the regrowth evidence is this cycle's own; every audit re-pays
  the read tax. But O0 correctly prices the stakes: maintenance, not fire.
- **O1 — pure module-directory move (Pattern A).** Zero visibility cost (probe). Fixes 1b only;
  both attractors survive intact — same trait, same struct, same capabilities in reach. History
  says it regrows (+400 lines through a shrink cycle). Cheap, low risk, low yield.
- **O2 — per-concern context structs** (bundle fields into `DiscoveryCtx`, `EnrichCtx`, …).
  Compiler-enforced *field* boundaries. But §1c shows cross-concern field reach is not the
  disease — and the boundary that matters (who may invoke the canonical pipeline) is a
  *method*-reach question context structs don't govern. Cost is real in THIS codebase: `db: D`
  is an owned generic shared by every concern (so sub-structs force `Arc`-ing or borrow
  gymnastics), 5 constructors + server wiring churn, `self.x` → `self.ctx.x` across ~3,700
  lines, and the generics migrate into every sub-struct. Highest-churn option aimed at the
  weakest-evidenced problem.
- **O3 — trait split, impl stays whole** (house pattern). Kills the trait attractor; handlers
  bind narrower. But by itself the file stays 3,742 lines.
- **O4 — extract Discovery as its own service** (own struct + own narrow trait), continuing
  convergence-style but with a *struct*, not free fns over the god struct. Discovery is 33% of
  the file, verified self-contained, read-only (no ordering invariants with the lifecycle), and
  takes `llm` + `lookup_cache` with it. `WorkServiceImpl` drops to `<D,E,H>`.

## 5. My recommendation (slices, each stands alone)

**Slice 0 — pin the baseline.** Run the behavioral suite; record green (or triage). No refactor
lands without it. (Follow-the-test-process rule.)

**Slice 1 — dead-weight removal.** Delete `http_client`/`merge_engine`/`tag_service` fields, the
`M`/`T` generic params, the commented-out `refresh_all` block (:1515-1536), and the now-noise
constructor params (server + test call sites are compiler-guided). ~100 lines gone; every
where-clause in the crate's orbit shrinks. Zero behavior. *Flag: the field comments say
"reserved for future slices (S8+)" — deleting is the M-006/A1 precedent (dead code that lies),
and re-adding later is one constructor param; PO sanity-check wanted.*

**Slice 2 — Discovery moves out (O4 + the O3 slice it needs).** New `DiscoveryService` trait
(lookup, lookup_filtered, eager_match_by_author — `search_works` stays: it queries the user's
library, not the world) + `DiscoveryServiceImpl` struct owning http/llm/lookup_cache/resolver +
config-read; the 4 provider lookups, pick/cover-rank helpers, `CachedLookup`, and the in-file
discovery tests move with it. Handlers rebind (`HasDiscoveryService`), AppState +1 Arc. The god
file drops ~1,250 lines to ~2,500 and `WorkServiceImpl` becomes `<D,E,H>`. No ordering-invariant
risk: discovery is read-only search. (B2's "provider code into external-data gateways" can
follow later, provider-by-provider, as separate small diffs — `lookup_openlibrary` shows the
target form.)

**Slice 3 (optional, after re-assessing) — module-directory the remaining lifecycle file**
(Pattern A: `work_service/{mod,add,refresh,crud,covers,merge}.rs`), pure moves over the same
struct, zero visibility cost per the probe. Comprehension-only. **The add/enrich/refresh core's
internals are NOT restructured** — no context structs, no signature changes on the road — until
the suite from Slice 0 is green in CI or at least routinely runnable.

**What I would not do now:** context structs (O2) anywhere near the core; any splitting of
`run_unified_enrichment`'s 4-caller road; bundling B2 gateway work into the same diffs.

**Regrowth control that survives the refactor:** the narrow traits are the wall for the trait
attractor; for the capability attractor, the honest device is smallness + the existing door→road
design-gate discipline (insight 46), not a new abstraction. If harder enforcement is wanted
later, that is the compile-wall/jobs-crate hammer — a separate, deliberate decision.

## 6. Risks & what would change my mind

- Behavioral suite may not be green today (local-only, point-in-time) — Slice 0 exists for this;
  a red baseline pauses everything else.
- Handler/AppState churn in Slice 2 is wide but shallow (compiler-guided; ~mechanical).
- If someone shows me a real bug whose mechanism was cross-concern *field* reach inside this
  struct, O2's wall earns its cost and I'd reconsider.
- If the PO's appetite is one slice only: Slice 1 + Slice 2 is the 80% — Slice 3 is polish.

Confidence: probe result 100% (ran it). Field matrix / caller counts: high (read + Serena).
"Discovery-first extraction is the right first structural move": ~80%. "Context structs are
wrong here": ~70% — the strongest counterargument is regrowth discipline, which I'm answering
with traits + smallness instead of field walls.

---

## 7. Reconciliation — added AFTER reading the design + reviews (same day)

**The probe settles the visibility dispute against Gemini.** Its review's verdict ("Reject
Pattern A") rests first on the claim that submodules cannot read parent-private fields
(reviews §1) — **false**, verified by compile+run. What survives of Gemini's review: the
generic-boilerplate tax of repeating `<D,E,H,L,M,T>` bounds across files (true — my §3.2/3.3
address it by *shrinking the generics*), and the regrowth-inside-submodules worry (true — shared
with Codex and me).

**Scorecard vs my independent take.**
- *The design* (pure Pattern-A move): every grounded fact in it independently checks out
  (line budget, field matrix, 4 callers, coherence constraint, dead trio, test caveat — its
  §11.3 is my Slice 0). Its §8 `unproxy_cover_url` lead verified and sharpened: the metadata
  copy (`work_service.rs:3459`) is a behavior-identical duplicate of
  `livrarr_domain::unproxy_cover_url` (`livrarr-domain/src/lib.rs:1041-1048`) but is NOT
  caller-less — `cover.rs:239` calls it; fix = repoint one caller, delete copy (Slice 1). Where
  we part: the design's anti-regrowth lever is a CLAUDE.md convention (§9); I judge the two
  attractors (§1a) stronger than a convention — this cycle's +400 lines happened *under* an
  active shrink effort.
- *Codex*: the strongest review, and closest to mine — leaves-first (its finding 6 = my Slice 2),
  core untouched until ordering-invariant tests exist (= my Slice 3 gate), "file size isn't the
  problem" (= my god-box-not-god-object §1c), the `retry_all_incomplete → svc.refresh` cycle
  flagged (real, `convergence_service.rs:312`). Difference: Codex reaches for small context
  structs on leaves "only where they reduce bounds"; my Slice 2 makes the biggest leaf a real
  service instead, which reduces bounds *more* (drops `L`, `llm`, `lookup_cache` from the god
  struct) and also kills the trait attractor for search.
- *Gemini*: context structs everywhere, including a `CreationContext` for the core. Honest
  update in its favor: per-call borrow-contexts (its sketch) avoid the owned-generic-`D`
  restructuring I'd feared for stored sub-structs — cheaper than my §4-O2 assumed. Still wrong
  for the core: creation's transitive needs (via `ensure_identity_and_enrichment` →
  `run_unified_enrichment`) span ~6 of the 9 live fields, so its "narrow" context is nearly the
  whole struct; its own sketch keeps the full `<D,E,H,L,M,T>` generics; and context-threading
  through the add cascade's call chain is ceremony with no wall where the wall would matter.
- *The reviews doc's convergence summary overstates Gemini*: "leaves first, core last" is
  Codex-only — Gemini never sequences, and its lead example targets the core.

**Net fork for the PO** (all three views + mine reconciled):
1. **Tidy** — the design as written (Pattern A pure move). Mechanically sound (probe), zero
   decoupling; all three post-design views judge it insufficient against regrowth on its own.
2. **Carve out the leaf, shrink the box** — my §5 slices (baseline → dead weight + generic
   shrink → Discovery out as service+trait; core untouched; Pattern-A polish optional later).
   Codex-compatible; adopts the design's mechanics for whatever remains.
3. **Context-struct decoupling** — Gemini-style compile walls. Strongest future-proofing on
   paper; highest churn; its walls are widest exactly where thinnest is possible, and the
   present-day field-reach problem it solves does not exist (§1c).

Recommendation unchanged: **option 2.**
