# Design — subtitle matching + identity-edit honesty (r1)

Author: CC. Date: 2026-07-25. Stage: design (pre-review).
Origin: PO manual testing of the merged identity-edit feature, 2026-07-25. Four
observations, all reproduced at source against the live `./testdata` database and
`/tmp/livrarr.log`.

Governing artifacts this design touches:
- `docs/metadata-remediation-phase5-matching-spec.md` — the locked matching rules
  (`title_id_trust` / AC-004 is defined there). **This design changes one arm of
  that rule and therefore does not ship without cross-family review.**
- `ARCHITECTURE.md` Part 1 — "accuracy over resolution" for covers.

## PO decisions on record (2026-07-25)

| Decision | Ruling |
|---|---|
| Relax the subtitle rule | GO. Author agreement alone is sufficient corroboration. |
| Publication-year guard | REJECTED — "edition shaped and most users do not care." |
| A user-confirmed id must outrank the heuristics | GO in principle, **sequenced after S1** (see Deferred). |
| The 17 dead-ended works | Clear all 17, let them re-chase. |
| Covers on already-enriched works | One-time sweep, don't wait for natural refresh. |
| Modal wording (`unproven`) | In scope for this unit. |

---

## The finding

One rule accounts for **every** Goodreads-key rejection in the PO's entire log
history: `GreyCause::OneSidedSubtitle`. 50 declines, 50 of that cause, zero of
any other.

```
2026-07-14T23:55:25 identity fan-out responder provider="goodreads"
    title="Einstein: His Life and Universe" gr="10884.Einstein" isbn="9780743264730"
2026-07-14T23:55:25 gr payload trust declined on grey title cause=OneSidedSubtitle
2026-07-14T23:55:25 identity quorum resolved ol="OL4288870W" gr="" hc="33148"
```

The work's title is `Einstein`; Goodreads' is `Einstein: His Life and Universe`.
Identical main titles, identical author (`Walter Isaacson`), and the correct
Goodreads key was handed over and thrown away. Three attempts, then
`work_anchor_dead_ends` at `attempt_count = 3` and never retried again.

Observed blast radius in the PO's library:
- **17 works** carry a `gr_work` dead end (`attempt_count = 3` on all of them).
- **21 works** have no `gr_key`.
- **44 works** are English, have an `ol_key` and a `gr_key`, and are showing a
  non-Goodreads cover — i.e. eligible to gain a Goodreads cover once the gate
  stops stripping.

### Why the existing escape hatch never fires

`title_id_trust` already permits a one-sided-subtitle grey — but only when a hard
identifier independently agrees (`crates/livrarr-domain/src/identity_matching.rs:514-520`):

```rust
TitleVerdict::Grey { cause: GreyCause::OneSidedSubtitle, .. } => matches!(
    id_verdict(a, b),
    IdVerdict::WorkKeyEqual | IdVerdict::EditionBridge
),
```

For Einstein the seed ISBN is `9780743264747` and Goodreads reports
`9780743264730` — a different printing of the same book. `EditionBridge`
requires ISBN or ASIN *equality*, so it does not fire. Goodreads systematically
lists a different edition's ISBN than the one in the user's file, which makes the
hatch dead in practice for exactly the cases it was written for.

### Second, independent instance of the same mistake

`apply_gr_cover_gate` (`crates/livrarr-enrichment/src/merge_engine.rs:291-333`)
re-derives the same judgement with a *different* mechanism: raw token Jaccard
over the **full** title, threshold 0.6
(`crates/livrarr-enrichment/src/cover_gate.rs:3`,`:43-45`).

```
12:50:33  cover gate: stripping GR cover_url work_id=136
          other=Skip { jaccard: 0.25, via: DeterministicSkipNoLlm }
```

That line was emitted **seconds after the PO manually certified `gr_key = 10884`
on work 136**. The gate does not know, and cannot ask, that a human already
settled the question. It also strips `payload.gr_key` alongside the cover
(`merge_engine.rs:326-327`).

Goodreads is **first** in the English ebook cover order
(`crates/livrarr-enrichment/src/cover_rank.rs:44-53`), and a `Validated` cover may
be replaced by another `Validated` cover
(`crates/livrarr-domain/src/enrichment_types.rs:58`). So this gate is the only thing
standing between these works and a Goodreads cover.

---

## S1 — one rule: equal main titles + agreeing author = same book

### S1a. `title_id_trust` — drop the id-corroboration requirement for `OneSidedSubtitle`

`crates/livrarr-domain/src/identity_matching.rs:510-522`.

```rust
match title {
    TitleVerdict::Same => true,
    // The subtitle is decoration, not identity: equal mains with a true
    // subtitle on one side is the same work. The caller's author bar is
    // the corroboration (every caller requires Agree on this arm).
    TitleVerdict::Grey { cause: GreyCause::OneSidedSubtitle, .. } => true,
    _ => false,
}
```

The `work_key_contradiction` veto at `:507` stays first and unchanged.

**Why no author parameter is added here.** All three production callers already
compute `author_verdict` themselves and require `AuthorVerdict::Agree` on the
grey arm. Verified, each one:

| Caller | Author bar | Line |
|---|---|---|
| `flm_match` (async resolver) | requires `Agree`, unconditionally | `crates/livrarr-identity/src/async_resolver.rs:346-352` |
| `verify_gr_payload` (English resolver) | `Same` → `Agree`\|`Abstain`; grey → `Agree` | `crates/livrarr-identity/src/english_identity_resolver.rs:385-394` |
| `proven_agreement` (identity-edit preview) | requires `Agree` | `crates/livrarr-metadata/src/work_service.rs:2526-2533` |

So "main title equal **and** author agrees" is the emergent rule at all three
seats without threading a new argument through the authority. An authorless
Goodreads payload yields `Abstain` and is still declined on the grey arm — the
anti-bot/empty-payload protection is unaffected.

### S1b. Cover gate — compare main titles, and stop ignoring the author

`crates/livrarr-enrichment/src/cover_gate.rs:39-58`.

Two changes:
1. Compute the Jaccard over `parse_title(...).main` on both sides instead of the
   raw title. `Einstein` vs `Einstein: His Life and Universe` becomes 1.0.
2. Consult the author, which the gate is already handed on both sides
   (`cover_gate.rs:21`,`:29`) and currently never reads. A proven
   `AuthorVerdict::Disagree` is a Skip regardless of title score.

`Abstain` (one side has no author) falls through to the title decision — today's
behavior. **This is deliberate: no cover that survives the gate today may be lost
to this change.** The only new rejection is a title that clears the bar while the
authors provably disagree, which is a tightening the current gate lacks.

`CoverGateOutcome` keeps its `jaccard` field and the 0.6 threshold, so the shape
the existing behavioral tests assert on is preserved.

### S1c. Free consequence — the modal stops threatening the user's data

`proven_agreement` (`work_service.rs:2525`) is the modal's sibling keep/drop
verdict and routes through the same `title_id_trust`. The PO saw
`Open Library: unproven` on the Benjamin Franklin edit — a *drop* chip, meaning
"confirm and I will clear your Open Library id and re-match it." With S1a that
sibling proves agreement and reads `keep`. No separate fix required; a test must
pin it.

---

## S2 — stop blaming the user for a provider refusal

The PO's four Einstein attempts at 12:27 failed; the identical input succeeded at
12:50. Confirmed transient Goodreads refusal, not bad input.

Two defects made that undiagnosable:

**S2a. A refusal is reported as "not found."** Any non-429 4xx folds into
`ProviderOutcome::NotFound` (`crates/livrarr-external-data/src/provider_client.rs:2366-2368`),
which the preview surfaces as `reason: "not_found"`
(`crates/livrarr-metadata/src/work_service.rs:2114-2131`), which the modal renders as:

> "No book was found for that identifier — double-check the value."
> (`frontend/src/pages/work-detail/components/IdentityEditModal.tsx:206`)

The value was correct. Fix: carry the refusal apart from a genuine miss so the
preview can distinguish them, and say so — *"Goodreads didn't answer. Try again in
a moment."* The `provider_unavailable` branch and its copy already exist
(`IdentityEditModal.tsx:207`); the classification is what is wrong, not the UI's
vocabulary.

Note the deliberate scope line: 4xx→`NotFound` is correct for the *enrichment
retry budget* (a permanent miss must not burn retries). This design does not
change that mapping for enrichment; it changes what the **preview** seam reports
to a human who is standing there watching.

**S2b. The preview path logs nothing at all.** Four fetches over 42 seconds
produced 4 rows in `provider_call_records` and **zero** lines in
`/tmp/livrarr.log`. Verified: the log jumps 12:25:07 → 12:32:15 with nothing
between. `fetch_goodreads_html_via` returns `HttpStatus(code)` silently on
non-2xx (`crates/livrarr-external-data/src/goodreads/client.rs:187-191`), and the
parse-failure warning at
`crates/livrarr-external-data/src/goodreads/parsers.rs:358-365` never fired — which
is how we know it was transport, not drift.

Fix: one `info!` at preview entry (work, slot, canonical value) and one at exit
(outcome class), plus a `warn!` on the silent non-2xx branch carrying the status.
A user-facing interactive path must not be invisible.

---

## S3 — modal wording

`SiblingChip` renders the raw backend cause verbatim
(`IdentityEditModal.tsx:296-312`): `Open Library · unproven`.

The three causes (`work_service.rs:2695`,`:2697`,`:2706`) become plain English,
and the chip states the *consequence*, not the verdict:

| Cause | Today | Proposed |
|---|---|---|
| `disagrees` | `Open Library · disagrees` | `Open Library — points at a different book, will be cleared` |
| `unproven` | `Open Library · unproven` | `Open Library — couldn't confirm it's the same book, will be cleared` |
| `unverifiable` | `Open Library · unverifiable` | `Open Library — couldn't reach Open Library, will be cleared` |

Keep chips as chips; the long form goes in the existing `title` tooltip if the
text overflows. Also retitle the section from `Other identifiers` to
`Your other identifiers` — the PO's question was literally "are these different
from the ids already on the work?", so the panel is failing to say whose they are.

---

## S4 — one-time data repair

Neither S1 nor S2 changes existing rows. Two one-shot startup passes, each
marker-guarded in `_livrarr_meta` following the established pattern
(`normalized_identity_backfill_complete`, `history_backfill_generation`,
`identity_key_generation`).

**S4a. Clear the `gr_work` dead ends.** `attempt_count >= 3` excludes a work from
the chaseable-anchor query (`crates/livrarr-db/src/sqlite_work.rs:1419-1421`,
threshold `DEAD_END_THRESHOLD = 3` at
`crates/livrarr-metadata/src/work_service.rs:195`). Without this the rule change is
invisible to all 17.

Delete `gr_work` dead-end rows only. Marker: `subtitle_rule_deadend_clear_v1`.

Honesty about which 17: 13 are English works whose Goodreads title carries a
subtitle — `The Innovators`, `The Code Breaker`, `World War Z`,
`Courage Is Calling`, `Wisdom Takes Work`, `The Man from the Future` and others
named in the decline log. **4 are foreign-language** — `Master Thaddeus` and
`Pan Tadeusz` (pl), `La Casa De Los Espíritus` (es), `Die Krone Der Sterne` (de)
— and they appear **nowhere** in the 50 declines. Their dead ends have a
different, un-diagnosed cause. Clearing them re-tests them; this design does not
claim it fixes them.

**S4b. Cover re-check sweep.** 44 works qualify (English, `ol_key` and `gr_key`
present, cover not already from Goodreads, `cover_trust != user`). The merge is
only re-run by a refresh, so the sweep marks these works due for convergence
rather than reimplementing the cover decision. `cover_trust = user` is excluded
and `resolve_cover` refuses to touch it anyway
(`crates/livrarr-enrichment/src/cover_resolution.rs:77-79`) — a manual cover
override stays untouched, twice over.

Marker: `subtitle_rule_cover_sweep_v1`. Ordered after S4a so a work needing both
gets one pass, not two.

---

## Risk

**The real one: a named volume of an omnibus.** Library title
`The Lord of the Rings`, Goodreads title
`The Lord of the Rings: The Fellowship of the Ring`. Equal mains, agreeing
author, subtitle is prose rather than a number — so `GreyCause` is
`OneSidedSubtitle` and S1a now accepts it. Today it is rejected. This is a true
behavior regression for that shape and cannot be argued away.

Mitigations, in order of strength:
1. The higher-risk shapes still block, by construction. `GreyCause` precedence is
   `VolumeAsymmetry` > `SubtitleDisagreement` > `OneSidedSubtitle`
   (`identity_matching.rs:68-83`), so `OneSidedSubtitle` provably means: equal
   mains, no volume evidence on either side, not two disagreeing subtitles. A
   numbered volume marker (`Book One`, `#2`, `Volume II`) parses into
   `series_markers` and lands in `VolumeAsymmetry` or `VetoVolume` instead.
2. `work_key_contradiction` still vetoes first.
3. Recovery is user-driven and cheap: the id is clearable per slot, and the
   deferred S5 makes the user's correction stick permanently.

Residual accepted: an omnibus whose volume is named, not numbered, can mis-link.
The PO has weighed this against 17 stuck works and 44 wrong covers.

**Second risk: S4b cost.** 44 works × 5 providers in a burst. Goodreads already
refused the PO once today under lighter load. The sweep must ride the existing
convergence pacing rather than dispatching 44 refreshes at once, or it will trip
the Goodreads breaker and the sweep will report failures that look like this
design not working.

---

## Sequencing

| Step | Content | Gate |
|---|---|---|
| 0 | This design | Cross-family review (Gemini + Codex), both verdicts required |
| 1 | S1a + S1b + S1c pin | Red-first tests, then implement |
| 2 | S2a + S2b | Independent of step 1; can land in the same unit |
| 3 | S3 | Frontend; vitest + typecheck |
| 4 | S4a then S4b | Requires step 1 live, else it is a no-op sweep |
| 5 | Verify on the PO's data | The 17 re-chase; the 44 re-cover; count the outcome |

## Test plan

Per `CLAUDE.md` "tests drive the real door" — every test exercises the production
entry path, no injected outcomes.

1. **`title_id_trust` unit** — `Grey{OneSidedSubtitle}` with `IdVerdict::NoEvidence`
   now trusts; with a work-key contradiction still does not. The 8 existing tests
   at `identity_matching.rs:1892-2005` must be re-read individually, not bulk-
   updated: `title_id_trust_allows_one_sided_subtitle_grey_with_edition_bridge`
   (`:1908`) and `..._with_work_key_equality` (`:1926`) still pass but stop being
   load-bearing, and a new test must assert the no-evidence case they were
   standing in for.
2. **`verify_gr_payload`** — the Einstein case, verbatim from the log: seed
   `Einstein` / `Walter Isaacson` / isbn `9780743264747`, payload
   `Einstein: His Life and Universe` / `Walter Isaacson` / isbn `9780743264730`.
   Red before, green after. Plus the authorless payload still declining.
3. **Cover gate** — the 6 existing behavioral tests in
   `tests/behavioral/test_ewl_cover_gate.rs` stay green unchanged (verified by
   reading them: identical → 1.0, paren-strip → Apply, genuinely-different →
   Skip, empty candidate → Skip at 0.0). New: the Einstein main-title case
   applies; a matching title with a disagreeing author skips.
4. **Preview siblings (S1c)** — through the real preview seam: work with an
   `ol_key`, certify a Goodreads key whose title carries a subtitle, assert the
   Open Library sibling comes back `keep`, not `drop`.
5. **S2a** — a provider 403 through the real preview handler surfaces the
   unavailable reason, not `not_found`.
6. **S4a/S4b** — real `SqliteDb`: seed a `gr_work` dead end at 3, run the pass,
   assert the row is gone and the work is selected by the chaseable-anchor query;
   assert a `cover_trust = user` work is untouched by the sweep; assert both
   passes are no-ops on second boot (marker present).
7. **Frontend** — vitest on `SiblingChip` for all three causes.

Registering any new behavioral test file and `git add -f`-ing it are ONE change
(`CLAUDE.md`, responsiveness retro).

## Deferred

**S5 — a user-confirmed identifier outranks every heuristic.** PO ruling: GO in
principle, sequenced after S1. `MergeInput` carries `current_work: Work` and no
anchor provenance (`crates/livrarr-enrichment/src/merge_engine.rs:102-108`), so the
cover gate cannot currently ask "did a human set this?" — the answer lives in
`work_identity_anchors.setter` in another crate. S1 fixes every case observed in
testing; S5 is the safety net for cases not yet observed. Re-open once step 5
reports.

## Out of scope

- The 4 foreign-language dead ends' actual cause (S4a re-tests them; diagnosis is
  separate work).
- The 4xx→`NotFound` mapping on the enrichment retry path (S2a changes the
  preview seam only).
- `title_verdict` / `parse_title` themselves. Their classification is correct and
  is what makes S1a safe; only the *consequence* of `OneSidedSubtitle` changes.
