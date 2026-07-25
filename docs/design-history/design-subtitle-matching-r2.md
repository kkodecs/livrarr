# Design — subtitle matching + identity-edit honesty (r2)

Author: CC. Date: 2026-07-25. Stage: design (round 1 reviewed, revised).
r1 snapshotted at `docs/design-history/design-subtitle-matching-r1.md`.
Origin: PO manual testing of the merged identity-edit feature, 2026-07-25. Four
observations, all reproduced at source against the live `./testdata` database and
`/tmp/livrarr.log`.

**r2 changes:** S4b is rebuilt — r1's mechanism was a no-op (both reviewers, and
confirmed at source). S1b's "no cover lost" guarantee was false and is withdrawn.
S4a resets the convergence clock. See §Review round 1.

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
behavior.

`CoverGateOutcome` keeps its `jaccard` field and the 0.6 threshold, so the shape
the existing behavioral tests assert on is preserved.

**r1 claimed "no cover that survives the gate today may be lost." That claim was
false and is withdrawn.** Gemini R-1 constructed the counter-shape and it holds
arithmetically:

- `TITLE_STOPWORDS` is `["a","an","the","of","and","in","on","for","to"]`
  (`crates/livrarr-domain/src/text_norm.rs:7`) — "novel" is not in it.
- Work `Dune Messiah: A Novel` → full tokens `{dune, messiah, novel}`.
  Goodreads `Dune: A Novel` → `{dune, novel}`. Full-title Jaccard = 2/3 = 0.67,
  **above** the 0.6 bar: passes today.
- Main titles only: `{dune, messiah}` vs `{dune}` = 0.5: rejected after S1b.

So covers *are* lost. Every one of them is a **wrong** cover — shared subtitle
boilerplate inflating the score while the actual titles differ. That is the bug
S1b exists to remove, not collateral damage.

**Gemini's recommendation — "allow a cover to pass if either the full raw title or
the parsed main title clears the threshold" — is rejected.** It is a strict
superset of today's accept set, so it preserves the Dune Messiah defect by
construction and adds the Einstein fix on top. The corrected claim is narrower and
true: *the only covers S1b removes are ones whose main titles disagree, which no
correct gate should have accepted.*

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

**S2a. A refusal is misfiled as "this book does not exist" — and three consumers
inherit the lie.** (r2: PO enlarged this item. r1 scoped it to the modal message
alone, which would have left two of the three consumers broken and been declared
done.)

Any non-429 4xx folds into `ProviderOutcome::NotFound`
(`crates/livrarr-external-data/src/provider_client.rs:2366-2368`) and is recorded as
`outcome = "not_found"`. That single misclassification propagates:

1. **The modal blames the user.** `reason: "not_found"`
   (`crates/livrarr-metadata/src/work_service.rs:2114-2131`) renders as *"No book was
   found for that identifier — double-check the value."*
   (`frontend/src/pages/work-detail/components/IdentityEditModal.tsx:206`). The value
   was correct.
2. **The sidebar health indicator stays green.** It counts providers whose
   `status == "error"` (`frontend/src/components/Sidebar/Sidebar.tsx:373-431`), fed by
   `current_error_of` over the 24h call stats
   (`crates/livrarr-handlers/src/system.rs:130-136`, `:178-191`). But
   `fn is_error` is `rate_limited | timeout | error`
   (`crates/livrarr-db/src/sqlite_provider_calls.rs:42-44`) — **`not_found` is not an
   error.** The PO's four refusals left the dot green while Goodreads was refusing
   every request.
3. **The circuit breaker never learns.** `fetch_goodreads_html_via` reports
   `BreakerSignal::Failure` only for 5xx and `TripImmediately` only for an anti-bot
   body (`crates/livrarr-external-data/src/goodreads/client.rs:187-201`). **A 4xx
   reports nothing at all**, so the central mechanism keeps dispatching into a
   provider that is turning us away.

**PO directive (2026-07-25):** the call must ride the centralized mechanism — it
already does, via `RateBucket::Goodreads` through the process-global outbound queue
— and a failure must surface on the provider status line in the left nav.

**Fix, at the classification, not at each consumer.** Introduce a refusal class
distinct from a genuine miss at the point the status is read, and let all three
consumers read it:

- The preview seam reports the refusal, and the modal shows the honest copy. The
  `provider_unavailable` branch and its wording already exist
  (`IdentityEditModal.tsx:207`) — only the classification feeding it is wrong.
- The call record carries an error-class outcome so `is_error` sees it and the
  sidebar dot turns. This requires no UI work: `Sidebar.tsx` and
  `current_error_of` already do the right thing with a correctly-classified record.
- The 4xx branch reports `BreakerSignal::Failure` to the Goodreads bucket.

**Robust to the unresolved question.** It is not established whether the PO's
12:27 failures were a 4xx or an empty 200 body — the path is silent (see S2b), and
both branches terminate at `NotFound`. The empty-body branch additionally reports a
**false `Success`** to the breaker (`client.rs:203`). S2b's logging resolves which,
and the fix must handle both: an empty/unparseable body is also not "this book does
not exist."

**Scope line retained:** a genuine 404/410 must still not burn retries forever.
What changes is that a *refusal* stops being indistinguishable from a *miss*.

### Cross-provider survey (r2, PO question: "all providers, not just GR?")

Surveyed all six fetch providers at source. The answer splits.

**The misclassification is Goodreads-only — it is the sole deviant from the house
standard.** `ProviderFetchError` already states the rule: `NotFound` means
"genuinely absent upstream (HTTP 404/410) — a no-match, never a transient failure"
(`crates/livrarr-external-data/src/types.rs:114-143`).

| Provider | Refusal (e.g. 403) maps to | Record class | Sidebar |
|---|---|---|---|
| **Goodreads** | `ProviderOutcome::NotFound` (`provider_client.rs:2366-2368`) | `not_found` | **stays green** |
| OpenLibrary | `ProviderFetchError::Other` (`openlibrary.rs:20-25`) → `PermanentFailure` (`provider_client.rs:495`) | `error` | red |
| Audnexus / Audible | `ProviderFetchError::Other` → `PermanentFailure` (`provider_client.rs:850`) | `error` | red |
| Hardcover | `HardcoverError::Http` (`hardcover.rs:95-100`) | `error` | red |
| Google Books | 403 → `NotConfigured` (quota, deliberate); other → `WillRetry{ServerError}` (`google_books.rs:643-666`) | `skipped_policy` / `error` | red |

So S2a's classification fix is **Goodreads conforming to the existing standard**,
not a new policy. Scope stays on Goodreads; no other provider client changes.

**One defect IS system-wide: the breaker never learns about a 4xx.** Every client
carries the identical shape — report `BreakerSignal::Failure` on 5xx, report
nothing on 4xx:

- `goodreads/client.rs:187-192` · `openlibrary.rs:82-90` · `hardcover.rs:95-100` ·
  `google_books.rs:197-216`

A provider that starts refusing every request with 403 therefore never trips its
own breaker, and the outbound queue keeps dispatching into it. **S2c (new):** a
non-2xx that is not a genuine 404/410 reports `BreakerSignal::Failure` to that
provider's bucket, in every client.

**Explicitly NOT changed: `is_error` keeps excluding `not_found`**
(`crates/livrarr-db/src/sqlite_provider_calls.rs:41-43`). A book genuinely absent
from a provider is not a provider-health event; counting it would pin every
sidebar dot red permanently. The defect was Goodreads mislabelling which bucket it
was in, never the bucket definition.

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

Delete `gr_work` dead-end rows only, **and set `next_convergence_at = NULL` on the
same works in the same transaction.** The selector ANDs
`(next_convergence_at IS NULL OR next_convergence_at <= now)` across the whole
predicate (`crates/livrarr-db/src/sqlite_work.rs:1433`), so a future clock defers
the repair even with the dead end gone (Gemini R-3).

Measured impact on the PO's data: 15 of the 17 already have
`next_convergence_at = NULL` and become eligible immediately; works 8
(`Master Thaddeus`) and 111 (`Solaris`) carry a clock ~50 minutes out and are
`enrichment_status = failed`, so they would have been picked up by branch (2) at
that time anyway. The reset costs one column in an `UPDATE` we are already issuing
and makes the repair deterministic instead of merely likely — accepted on those
grounds, not because it unblocks anything.

Marker: `subtitle_rule_deadend_clear_v1`.

Honesty about which 17: 13 are English works whose Goodreads title carries a
subtitle — `The Innovators`, `The Code Breaker`, `World War Z`,
`Courage Is Calling`, `Wisdom Takes Work`, `The Man from the Future` and others
named in the decline log. **4 are foreign-language** — `Master Thaddeus` and
`Pan Tadeusz` (pl), `La Casa De Los Espíritus` (es), `Die Krone Der Sterne` (de)
— and they appear **nowhere** in the 50 declines. Their dead ends have a
different, un-diagnosed cause. Clearing them re-tests them; this design does not
claim it fixes them.

**S4b. Cover re-check sweep — rebuilt in r2.**

### Why r1's mechanism was a no-op

r1 said the sweep "marks these works due for convergence." Verified at source:
that does nothing. Two independent reasons, either one fatal:

1. **The selector never picks them.** `list_convergence_due`
   (`crates/livrarr-db/src/sqlite_work.rs:1408-1445`) has exactly three branches:
   identity `pending`; enrichment not in (`enriched`,`thin`); or a missing
   chaseable anchor. The 44 target works are `confirmed` + `enriched` with their
   work anchors present — no branch matches. `next_convergence_at` is ANDed onto
   that predicate, so making a work "due" cannot make an unselected work selected.
2. **Even if selected, `converge_work` would skip the merge.** `converge_outcome`
   (`crates/livrarr-metadata/src/convergence_service.rs:221-232`) returns
   `Completed` for a confirmed+enriched work with no chaseable anchor, and the job
   then clears its clock (`crates/livrarr-server/src/jobs/convergence.rs:104-107`).
   No re-enrichment, no merge, no cover decision.

r1 would therefore have written `subtitle_rule_cover_sweep_v1` over 44 untouched
works and reported success. Credit: Codex R-1 named this precisely.

**One premise correction.** Codex asserted "the project's documented recurring
background retry job is removed, and the replacement is user-triggered,
single-pass, no recurring loop … reachable only from a POST route." Half right.
`retry_all_incomplete` is indeed that replacement
(`convergence_service.rs:266-269`), but a recurring `convergence` job **does**
exist and runs on an interval (`crates/livrarr-server/src/jobs/mod.rs:129-135`,
`crates/livrarr-server/src/jobs/convergence.rs:27`) — the PO's own shutdown log
shows `job 'convergence' cancelled during interval sleep`. The wrong premise does
not rescue r1: the recurring job that exists uses the selector in (1) above, which
still excludes all 44. Conclusion upheld on corrected grounds.

### The r2 mechanism

A marker-guarded startup pass that calls the canonical refresh path directly, in
`livrarr-metadata` alongthe existing one-shot passes
(`crates/livrarr-metadata/src/cover_startup.rs:21-25` is the precedent: ordered,
sequential, one caller).

**Selector** — explicit, evaluated once, ids captured up front:

```sql
SELECT id FROM works
 WHERE user_id = ?
   AND language = 'en'
   AND ol_key IS NOT NULL AND ol_key <> ''
   AND gr_key IS NOT NULL AND gr_key <> ''
   AND COALESCE(cover_source,'') <> 'goodreads'
   AND cover_trust <> 'user'
```

44 rows on the PO's data. `cover_trust = 'user'` is excluded here and
`resolve_cover` refuses to touch it anyway
(`crates/livrarr-enrichment/src/cover_resolution.rs:77-79`) — a manual override is
protected twice.

**Consumer** — `WorkService::refresh(user_id, work_id, RefreshSurface::…)`
(`crates/livrarr-domain/src/services/work.rs:398-403`), the same road the UI's
refresh button and `retry_all_incomplete` use. The sweep re-runs the real merge; it
does not reimplement the cover decision, and it does not mutate
`enrichment_status` to trick a selector into eligibility (considered and rejected:
faking `Failed` on 44 healthy works is user-visible state corruption to work around
our own query).

**Pacing** — Gemini R-2 is right that a burst is dangerous, and more so under r2
since the sweep now genuinely dispatches provider traffic. Goodreads refused the PO
four times at 12:27 today under lighter load than this. The pass processes works
**sequentially, one refresh at a time, with a fixed inter-work delay**, and honors
the cancellation token so shutdown is not blocked. It reuses the outbound queue's
existing Goodreads bucket and breaker rather than adding a second pacing notion; if
the breaker trips mid-sweep, remaining works fail their refresh and are left for the
next boot — which requires:

**Marker discipline** — `subtitle_rule_cover_sweep_v1` is written **only after every
selected id has been attempted**, and per-work failures are counted and logged. A
partial sweep leaves the marker absent so the next boot resumes; because the
selector excludes works whose cover is already `goodreads`, a resumed sweep
naturally skips the ones that already succeeded. Idempotence comes from the
selector, not from a checkpoint table.

Ordered after S4a so a work needing both gets one pass, not two.

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

**Second risk: S4b cost.** 44 works × 5 providers. Goodreads already refused the PO
four times today under lighter load. r2 paces the sweep sequentially with a fixed
delay and defers the marker until every id is attempted (§S4b), so a tripped
breaker degrades into "resume next boot" rather than a silent partial repair
stamped as complete.

**Third risk: S1b removes some covers.** Named honestly in §S1b — the
`Dune Messiah: A Novel` / `Dune: A Novel` shape passes today at 0.67 and will be
rejected at 0.5. Those covers are wrong covers, but a user who liked the picture
will see it change. `cover_trust = user` overrides are untouched.

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
5. **S2a** — all three consumers, not just the message. Through the real preview
   handler with a provider returning 403: (a) the response carries the unavailable
   reason, not `not_found`; (b) the written `provider_call_records` row is an
   outcome `is_error` accepts, and `health_summary` reports Goodreads
   `status: "error"` — the assertion that would have caught the green-dot defect;
   (c) the Goodreads bucket received a `BreakerSignal::Failure`. Repeat (a)-(c)
   for a 200 with an empty body, which today reports a false `Success` to the
   breaker.
6. **S4a** — real `SqliteDb`: seed a `gr_work` dead end at 3 **and a future
   `next_convergence_at`**, run the pass, assert the row is gone, the clock is
   NULL, and `list_convergence_due` now returns the work. Assert no-op on second
   boot (marker present).
7. **S4b** — real `SqliteDb`, and this is the test r1 would have passed while
   doing nothing: seed a work that is `identity_status = confirmed`,
   `enrichment_status = enriched`, `language = 'en'`, with `ol_key` + `gr_key`
   present and `cover_source = 'hardcover'`. Assert the sweep **selects and
   refreshes it** on first boot. Assert a `cover_trust = 'user'` work is not
   selected. Assert that a sweep whose refreshes fail leaves the marker **absent**
   and is retried on the next boot. Assert no-op on second boot after success.
   A test that only checks "marker written" is not acceptable coverage here — the
   marker was exactly what r1 got right while getting the repair wrong.
8. **Frontend** — vitest on `SiblingChip` for all three causes.

Registering any new behavioral test file and `git add -f`-ing it are ONE change
(`CLAUDE.md`, responsiveness retro).

## Review round 1

Dispatched 2026-07-25 via `hooks/dispatch-review.py subtitle-matching design`.
Reviewers: gemini-3.5-flash (google), gpt-5.5 (openai). **Both returned FAIL.**
Artifacts: `build/reviews/subtitle-matching/review-design-{google,openai}-r1.json`.

Every finding was re-verified at source before disposition — no reviewer citation
was accepted unopened.

| Finding | Disposition | Basis |
|---|---|---|
| **Codex R-1** (P1) — S4b has no working mechanism; can silently no-op while writing its marker | **ACCEPTED**, S4b rebuilt | Confirmed twice over: the selector excludes confirmed+enriched works (`sqlite_work.rs:1408-1445`), and `converge_outcome` returns `Completed` without merging (`convergence_service.rs:221-232`). The most valuable finding of the round. |
| **Codex R-1 premise** — "the recurring background retry job is removed" | **REJECTED as stated** | A recurring `convergence` job exists (`jobs/mod.rs:129-135`, `jobs/convergence.rs:27`); the PO's shutdown log shows it. The conclusion survives on corrected grounds because that job uses the same excluding selector. |
| **Gemini R-1** (P1) — S1b's "no cover lost" guarantee is false | **ACCEPTED as a finding** | Counter-shape verified arithmetically against `text_norm.rs:7`. The overclaim is withdrawn in §S1b. |
| **Gemini R-1 recommendation** — pass if *either* full or main title clears the bar | **REJECTED** | A strict superset of today's accept set; preserves the `Dune Messiah` defect by construction. Reasoning recorded in §S1b. |
| **Gemini R-2** (P1) — 44-work burst will trip the Goodreads breaker | **ACCEPTED** | More acute under r2 than r1, since r2 actually dispatches traffic. Sequential pacing + deferred marker in §S4b. |
| **Gemini R-3** (P1 → **P3**) — clearing dead ends without resetting the convergence clock | **ACCEPTED, severity reduced** | The clock is ANDed onto the predicate (`sqlite_work.rs:1433`), so the mechanism is real. But 15 of 17 works are already NULL and the other 2 are `failed` and would be selected ~50 min later regardless. Taken for determinism, not because it unblocks the repair. |

Neither reviewer challenged S1a — the load-bearing change, and the one the prompt
asked hardest about. Neither found an additional `OneSidedSubtitle` shape beyond the
named-omnibus case already in §Risk, and neither contested the three-caller author-bar
claim. That is **weak** evidence in its favor, not strong: absence of a challenge is
not verification, and round 2 should be asked to attack S1a specifically rather than
re-review S4b.

## Deferred

**S5 — a user-confirmed identifier outranks every heuristic.** PO ruling: GO in
principle, sequenced after S1. `MergeInput` carries `current_work: Work` and no
anchor provenance (`crates/livrarr-enrichment/src/merge_engine.rs:102-108`), so the
cover gate cannot currently ask "did a human set this?" — the answer lives in
`work_identity_anchors.setter` in another crate. S1 fixes every case observed in
testing; S5 is the safety net for cases not yet observed. Re-open once step 5
reports.

## Out of scope

- **The 4 foreign-language dead ends' actual cause — deferred to the next
  foreign-language work block (PO, 2026-07-25).** Works 8 `Master Thaddeus` (pl),
  9 `Pan Tadeusz` (pl), 33 `La Casa De Los Espíritus` (es), 71
  `Die Krone Der Sterne` (de) each carry a `gr_work` dead end at
  `attempt_count = 3` and **appear nowhere in the 50 `OneSidedSubtitle` declines**
  — so the subtitle rule is provably NOT their cause and S1 will not fix them.
  S4a clears their marks and re-tests them; if they dead-end again the mechanism
  is still unknown.

  Starting hypotheses for that investigation, none tested: Goodreads search
  returning nothing for a non-English title; the title on file being the
  translated/original form while GR indexes the other; or the foreign provider
  routing (`ProviderPriority::Foreign` drops OL/HC — `merge_engine.rs:270-282`)
  interacting with GR differently than the English path. Note works 8 and 9 are
  the *same book* under its English and Polish titles, which makes them a useful
  paired probe.
- The 4xx→`NotFound` mapping on the enrichment retry path (S2a changes the
  preview seam only).
- `title_verdict` / `parse_title` themselves. Their classification is correct and
  is what makes S1a safe; only the *consequence* of `OneSidedSubtitle` changes.
