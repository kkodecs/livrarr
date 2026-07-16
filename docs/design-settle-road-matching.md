# Design: settle-road title trust — one authority, cause-aware grey

**Unit:** identity-fix (quality-waves), fix 3 — expanded per PO 2026-07-14 to cover both
settle-road text gates. Supersedes the withdrawn `design-gr-key-adoption.md` draft.
**Status:** IMPLEMENTED 2026-07-14 ([REV] markers were binding; design gate closed
after r5/r6/r7 — r6 gemini PASS, r7 gemini PASS + codex R-5 prose fix folded). Code
review r8: BOTH families PASS, zero findings. Delivered exactly as specified, plus
two implementation-time records: (a) the DRY hoist — `id_verdict`'s nested
present/eq/differs helpers extracted to module level and shared with
`work_key_contradiction` (one definition); (b) test_unified_identity_path.rs: 7
fixtures relied on the old containment gate's prefix tolerance ("ACxxx Dune" vs
"Dune") — titles corrected to match; ac_008's contradiction-merge expectation flipped
per this design's ledger (contradicting identities merge nothing). The optional B5
discovery-door pin was not authored (authoring run stopped) — deferred on record.
**Governing rules:** `docs/metadata-remediation-phase5-matching-spec.md` (PO-locked
2026-07-02): D2 (grey never acts on text alone; interactive → candidates, background →
park), D3 (parse don't truncate; one-sided true subtitle demotes to grey; colon-truncation
removed everywhere), D4 (auto bar = exact cleaned-main equality; IDs outrank text),
AC-004 (a grey pair is "never silent auto-merge on text alone; an agreeing ID
(ISBN/anchor) may still confirm it"), REQ-001 (one identity authority).

## Problems (both live, both verified)

**P1 — the WWZ drop.** A work titled "World War Z" can never adopt its correct Goodreads
key: GR's canonical record carries the true subtitle, `title_verdict` demotes the pair to
`Grey` (one-sided-subtitle branch, `identity_matching.rs:230-238`), and the REQ-024 trust
gate `verify_gr_payload` (`english_identity_resolver.rs:341-364`, applied `:158-169`)
requires exactly `Same`. The correct key is stripped pre-quorum on every refresh;
dead-ends accrue. Reproduced on work 71 (2026-07-13 01:21/04:13,
`testdata/logs/livrarr.log.2026-07-13`).

**P2 — the flm gate violates the locked rules.** `flm_title`
(`crates/livrarr-identity/src/async_resolver.rs:275-308`) colon-truncates
(`s.split(':').next()`), spaced-dash-truncates, then applies word-SET containment. Proven
at runtime (temp in-file test, 2026-07-14): `flm_title("Dune", "Dune Messiah") == true`
and `flm_title("Dune", "Dune: House Atreides") == true`. It gates `merge_missing_anchors`
+ Confirmed/Provisional on the `Resolved` arm and anchor absorption on the `Unresolved`
arm (`settle_identity`, `async_resolver.rs:141-208`) — the last check before identity
locks. Violates D3 ("colon-truncation removed everywhere"), REQ-001 (a second normalizer
deciding sameness), and `ARCHITECTURE.md:200/:214`. ST-03's wrong-book class (same-author
sibling volumes) passes it.

## Doors [REV r5 — added per codex R-2 / gemini F1]

Every entry surface that reaches the two changed gates, LSP-enumerated (Serena
references, unbounded), 2026-07-14:

**`verify_gr_payload` runs inside `LiveEnglishIdentityResolver::resolve`
(`english_identity_resolver.rs:158-169`). `resolve` production callers:**

| Door | Wiring | Reaches the strip? |
|---|---|---|
| `settle_identity` (`async_resolver.rs:135`) | see its own six doors below | yes (fan-out path) |
| `WorkService::resolve_identity` (`work_service.rs:411`) ← manual-import `find_or_create_work` (`manual_import.rs:1130`) and list import `resolve_candidate_from_row` (`list_service.rs:76`) | user/list-picked candidate → RawHarvest seed | only when the seed lacks a work anchor or isn't user-confirmed — see bypass note |
| Discovery `lookup_filtered` (`discovery_service.rs:345`) | search resolver fast-path | yes (fan-out path) |

**Bypass (by design, verified at source):** a `user_confirmed` seed already carrying a
work anchor (ol/gr/hc) short-circuits at `resolve` `:73-84` — no fan-out, no strip. So a
user's explicit pick that carries a GR key keeps it, unconditionally. User-confirmed
bridge-only seeds (ISBN/ASIN only) DO fan out — and their bridge ID is exactly the
corroboration evidence the new grey arm consumes.

**`flm_match` is called only from `settle_identity` (`async_resolver.rs:145` Resolved
arm, `:186` Unresolved arm). `settle_identity` production callers (six):**

| Door | Site |
|---|---|
| every add door (all six funnel here) | `ensure_identity_and_enrichment` (`work_service.rs:2161`) |
| async add completion | `complete_add` (`work_service.rs:1101`) |
| single + bulk refresh | `refresh` (`work_service.rs:1590`, chaseable-gated block) |
| pre-scatter settle | `run_unified_enrichment` (`work_service.rs:2441`) |
| background convergence | `converge_work` (`convergence_service.rs:105`) |
| retry-all-incomplete | `retry_all_incomplete` (`convergence_service.rs:284`) |

Author-monitor is the documented deliberate exception (asserts a hard key, never
resolves — `settle_identity` doc, `async_resolver.rs:101-104`). No other caller exists.

## Change 1 — the authority exposes the grey CAUSE (one computation)

`livrarr-domain/src/identity_matching.rs`:

```rust
pub enum GreyCause {
    /// Equal mains; the ONLY demotion trigger was a true subtitle on exactly
    /// one side. No volume asymmetry, no subtitle disagreement.
    OneSidedSubtitle,
    /// Equal mains; both sides carry true subtitles and they differ.
    SubtitleDisagreement,
    /// Equal mains; volume evidence on exactly one side.
    VolumeAsymmetry,
    /// Mains not equal; similarity >= 0.75 (TITLE_GREY_FLOOR).
    NearMain,
}

pub enum TitleVerdict {
    Same,
    Grey { score: f64, cause: GreyCause },   // cause is NEW
    Different,
    VetoVolume,
}
```

`title_verdict_with_positions` computes the cause inside its existing equal-main branch
(`:230-238`) — the same conditions it already evaluates, now named once. Priority when
triggers co-occur: `VolumeAsymmetry` > `SubtitleDisagreement` > `OneSidedSubtitle`
(most-dangerous wins, so `OneSidedSubtitle` means *solely* that). No sibling predicate,
no re-derivation anywhere.

Compiler flushes every `Grey { score }` pattern in the workspace (non-exhaustive struct
pattern) — each consumer site is consciously touched; sites that don't care write
`Grey { score, .. }` and keep exact current behavior. Consumer enumeration at
implementation time is Serena-unbounded, not search-capped.

## Change 2 — one shared title+ID trust predicate; author bars stay per-seat

[REV r5 — restructured per codex R-1 (contradiction veto) and codex R-3 (author-bar
ambiguity): the shared predicate covers title+ID only; each seat states its author bar
explicitly per arm.]

New in `identity_matching` (single-sourced policy, consumed by both seats):

```rust
/// AC-004/D4/REQ-006 trust shape for a text-corroborated identity.
/// NOT a full acceptance decision: callers apply their seat's author bar.
/// Takes RAW evidence, not the collapsed IdVerdict: id_verdict short-circuits
/// to WorkKeyEqual before checking sibling providers (identity_matching.rs:
/// 346-351), which would mask a mixed agree+contradict payload. [REV r6, codex
/// R-4] The contradiction test runs FIRST, against every work-key pair.
pub fn title_id_trust(title: &TitleVerdict, a: &IdEvidence, b: &IdEvidence) -> bool {
    // REQ-006: "contradiction (same provider, different keys) vetoes... the
    // collision shape → Conflict, never auto-same." Checked per-provider over
    // raw evidence so one agreeing key cannot mask a contradicting sibling.
    if work_key_contradiction(a, b) {
        return false;
    }
    match title {
        TitleVerdict::Same => true,
        TitleVerdict::Grey { cause: GreyCause::OneSidedSubtitle, .. } => {
            matches!(
                id_verdict(a, b),
                IdVerdict::WorkKeyEqual | IdVerdict::EditionBridge
            )
        }
        _ => false,
    }
}

// module-private: any of ol/gr/hc present on BOTH sides and different.
fn work_key_contradiction(a: &IdEvidence, b: &IdEvidence) -> bool { /* ... */ }
```

[REV r6] `id_verdict`'s own equality-first collapse is deliberately left untouched —
its "work-key equality wins" ordering is live ratified behavior for the quorum and
dedup consumers (wiki insight 59). Whether the mixed agree+contradict case should
also veto at THOSE seats is flagged as a follow-up question for the conformance
cleanup unit, not silently changed here.

Seat application:

- **`verify_gr_payload` (P1):** `title_id_trust(&title, &payload_evidence,
  &seed_evidence)` — raw `IdEvidence` on both sides, never a precomputed collapsed
  `IdVerdict` [REV r7, codex R-5: prose aligned to the signature]; author bar per arm:
  `Same` arm keeps today's `Agree | Abstain`; the grey arm requires `Agree` strictly
  (weaker title evidence ⇒ authorless payloads are not enough). No circularity: this
  gate only runs when `seed.gr_key` is None (`english_identity_resolver.rs:161`), so
  gr_key equality can never self-confirm inside the predicate (`eq(None, x)` is false,
  `identity_matching.rs:334-356`).
  [REV r5, codex R-1] The contradiction veto is new on the `Same` arm here too: a GR
  payload whose OL/HC key contradicts the seed's established key no longer earns trust
  from title equality alone.
- **`flm_match` (P2):** `flm_title` and `canon_author` are DELETED. Gate becomes
  `title_id_trust(&title_verdict(seed, identity), &identity_evidence, &seed_evidence)`
  — raw `IdEvidence` both sides here too [REV r7, codex R-5] — AND
  `author_verdict == Agree` strictly, on both the Resolved and Unresolved arms (today
  flm requires author equality; an authorless identity already fails its empty-check —
  behavior preserved via the authority). An anchor-less seed has no IDs → `NoEvidence`
  → the grey arm never fires → merge on `Same` only; the existing
  `record_pending_anchor` else-arm (`async_resolver.rs:168-180/:194-206`) IS the D2 park
  — anchors are held pending, not lost. Strict tightening for the F1-risk population.

What stays forbidden at both seats, unchanged: different mains (incl. `NearMain` grey),
`SubtitleDisagreement`, `VolumeAsymmetry`, `VetoVolume`, title-less/anti-bot payloads,
and now any same-provider work-key contradiction.

## Why this does not reopen REQ-024's hallucinated-key hole

- A hallucinated key's different-book payload fails equal-mains ("Dune" vs "Dune Messiah"
  = `Different`, Jaccard 0.5). AC-021's pin cases (`test_wcc_resolver.rs:429-450`) pass
  unchanged.
- The residual (different book, byte-identical main, subtitle-only difference) now
  additionally requires an *independently agreeing hard ID* — which a hallucinated key's
  payload cannot supply against the seed's established anchors. This is AC-004's ratified
  confirmation shape, and the same evidence class the quorum's edition bridge already
  trusts (`agree`, `english_identity_resolver.rs:528-619`).
- [REV r5, codex R-1] A payload agreeing on title but contradicting on a work key is now
  rejected outright — "IDs outrank text" applies in the negative direction as well.

## Behavior changes to expect (honest ledger)

- flm STOPS matching: containment pairs ("Dune" → "Dune Messiah", "Foo" → "Study Guide
  for Foo"), colon-cut siblings, true-subtitle pairs without ID corroboration on
  anchor-less seeds, and [REV r5] work-key-contradicting identities even on exact title
  match. Any existing test pinning those is pinning the violation — each such flip is
  enumerated and dispositioned at implementation review, never silently rewritten.
- flm KEEPS matching: junk tails ("Dune: A Novel" — junk vocabulary → `Same`),
  punctuation-variant authors (via `author_verdict`), bare-vs-subtitled pairs whose IDs
  corroborate.
- WWZ outcome: adopts its GR key iff GR's payload carries a corroborating ID (checkable
  live — the responder line now logs real values, this unit). Work 71 itself carries
  ol/isbn/asin anchors, so seed-side evidence exists; the open question is GR's payload
  side.
- [REV r5 — gemini F2 sharpened, its recommendation declined] A **bare** seed (no IDs at
  all) with a subtitled GR record stays grey-parked on every automated path, including
  Interactive-tier refresh. This is deliberate: an Interactive *refresh* is still a
  silent adoption with no user click, and locked D2 forbids acting on grey on text alone
  — gemini's suggestion to let interactive paths retain the key would relitigate that
  locked rule, so it is declined with this citation. The user paths that DO adopt the
  key: (a) pick the book in search/re-add — a user-confirmed pick carrying the key
  bypasses the strip entirely (`resolve` `:73-84`); (b) any later-acquired ID
  (ISBN/ASIN/OL/HC) unlocks the grey arm on the next pass. Plumbing stripped-key grey
  candidates onto the review surface (D2's "show candidates" for this shape) stays an
  explicitly deferred follow-up, recorded here.

## Tests

- Unit (identity_matching): cause taxonomy — one-sided subtitle / disagreement / volume
  asymmetry / near-main; [REV r5, gemini F4] explicit co-occurrence cases proving
  multi-trigger greys resolve to the higher-severity cause and never read as
  `OneSidedSubtitle`; junk tail still `Same`; `title_id_trust` truth table incl.
  contradiction vetoing BOTH arms [REV r5, codex R-1] and the MIXED-evidence cases
  [REV r6, codex R-4]: OL-equal + HC-different (and provider permutations) → never
  trusted; AC-021 shape (ISBN equal + same-provider work keys different) → never
  trusted — exercised at both SEATS (behavioral, through verify_gr_payload and the
  settle gate), so an implementation that computed trust from a collapsed `IdVerdict`
  before the veto would fail them [REV r7, codex R-5].
- Behavioral (red first, codex-authored): P1 — WWZ-shaped fixture (seed with ISBN,
  subtitled GR payload + agreeing ISBN) adopts gr_key; disagreeing-subtitle, no-ID, and
  contradicting-work-key variants stay stripped; Abstain-author cases on BOTH arms of
  BOTH seats [REV r5, codex R-3]. P2 — settle with resolved sibling identity ("Dune
  Messiah" for seed "Dune") does NOT merge anchors (today it would); subtitled identity +
  agreeing ID merges; anchor-less seed + subtitled identity parks pending; work-key
  contradiction with matching title does not merge [REV r5].
- Per-door coverage [REV r5, codex R-2]: the six settle doors ride the existing
  behavioral suites (add/refresh/convergence); the two direct-resolve doors get one pin
  each on the strip behavior (manual-import bridge-only seed; discovery lookup path).
- Demotion observability [REV r6, gemini P3]: when either seat declines a grey pair,
  the decline log line carries the computed `GreyCause` (and the strip site's existing
  debug line gains it), so "why did this park?" is answerable from logs alone.
- AC-021 + `dead_ended_completion_suppression_survives_plain_refresh` +
  `not_found_interactive_refresh_clears_dead_ends_and_rechases` stay green.
- Live: work-71 refresh (snapshot first per ops rule).

## Out of scope (this unit)

- V2/V3 provider fuzzy pickers (`audible.rs score_provider_candidates` shared by
  Audible/OL/HC; `google_books.rs score_candidates`) and strays V4-V8 — follow-up unit,
  own design note. [REV r5 — gemini F3 asked to fold `work_dedup.rs` colon-truncation
  into scope: REFUTED at current source — `normalize_title_for_match` is
  `parse_title(title).main` today (`work_dedup.rs:228-230`), the ST-01 colon-cutter is
  already gone; gemini cited the spec's *historical* pre-fix inventory. Nothing to fix
  there.]
- Review-surface plumbing for uncorroborated grey GR keys (deferred pending the live
  WWZ evidence).
