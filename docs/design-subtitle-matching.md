# Design — identity/cover bug fix (r3)

Author: CC. Date: 2026-07-25. Stage: design, re-scoped after the identity-sidebar session.
r1 → `docs/design-history/design-subtitle-matching-r1.md`.
r2 → `docs/design-history/design-subtitle-matching-r2.md`.

Origin: PO manual testing of the merged identity-edit feature, 2026-07-25.

## What r3 is, and what it is not

**r3 is the bug fix only: five contained changes.** r2 had grown into a partial
redesign of the matching rules and the cover pipeline. The identity-sidebar session
(`build/findings-identity-sidebar-2026-07-25.md`) split the work on one test —
*does it stand alone without a new identity model, and would a rewrite have to undo
it?* Five changes pass. Everything else is deferred to an identity-layer rewrite and
is **out of scope here**, tracked at `docs/brief-identity-layer-rewrite.md`.

Each of the five is either a deletion or a provider-layer honesty fix. **None of them
builds machinery the rewrite would have to tear out.**

## Why the design changed shape between r2 and r3

r2 proposed *tuning* two title comparisons. The sidebar established that the premise
under both was wrong:

- **The subtitle is edition-level, not work-level.** OpenLibrary's work record for
  Einstein (`/works/OL4288870W.json`) has `title: "Einstein"` and no subtitle field at
  all; Hardcover's canonical handle for the work is `einstein` with `subtitle` as a
  separate column. The library's stored title was never the wrong one — the rules have
  been rejecting the work-level truth as a mismatch.
- **The Goodreads number used as a work anchor is an EDITION id.** Book ids 10884,
  2059858 and 6602781 all resolve to Goodreads work 985244 (fetched and verified).
  A GR book id therefore *cannot* answer "same work?" — two copies of one book carry
  different numbers by design.

So the fix is not a better threshold. It is deleting two comparisons that were asking
an unanswerable question, and leaving the honest plumbing behind them.

---

## PO decisions on record (2026-07-25)

| Decision | Ruling |
|---|---|
| Relax the subtitle rule | GO. Author agreement alone is corroboration. |
| Publication-year guard | REJECTED — "edition shaped and most users do not care." |
| Cover gate | **DELETE** (sidebar session). r2's "compare main titles" is superseded. |
| Deleting the gate leaves a wrong GR id uncaught | **Accepted on the record.** |
| The 17 dead-ended works | Clear all 17, let them re-chase. |
| Covers on already-enriched works | Deferred with item 8 — see Out of scope. |
| Modal wording | Deferred — PO parked it for his own copy call. |
| A user-confirmed id must outrank heuristics | Deferred; becomes structural in the rewrite. |

---

## C1 — Delete Rule A

`crates/livrarr-domain/src/identity_matching.rs:510-522`. Today a one-sided-subtitle
grey is trusted only when a hard identifier independently agrees:

```rust
TitleVerdict::Grey { cause: GreyCause::OneSidedSubtitle, .. } => matches!(
    id_verdict(a, b),
    IdVerdict::WorkKeyEqual | IdVerdict::EditionBridge
),
```

Becomes `=> true`. The `work_key_contradiction` veto at `:507` stays first, unchanged.

**Why the corroboration can never arrive.** `EditionBridge` requires ISBN or ASIN
*equality*, and Goodreads lists a different printing's ISBN than the one in the user's
file. Confirmed by outcome: **50 `OneSidedSubtitle` declines in the entire log, zero
rescues.** The rule is structurally incapable of helping the case it exists for.

**Why no author parameter is added.** All three production callers already compute
`author_verdict` and require `AuthorVerdict::Agree` on the grey arm — verified
individually:

| Caller | Author bar | Line |
|---|---|---|
| `flm_match` (settle road) | requires `Agree` | `crates/livrarr-identity/src/async_resolver.rs:346-352` |
| `verify_gr_payload` | `Same` → `Agree`\|`Abstain`; grey → `Agree` | `crates/livrarr-identity/src/english_identity_resolver.rs:385-394` |
| `proven_agreement` (preview siblings) | requires `Agree` | `crates/livrarr-metadata/src/work_service.rs:2526-2533` |

"Equal main titles **and** agreeing author" is therefore the emergent rule at all three
seats without threading a new argument through the authority. An authorless payload
yields `Abstain` and is still declined on the grey arm.

**Free rider — the modal stops threatening the user's data.** `proven_agreement` is the
identity-edit modal's sibling keep/drop verdict and routes through the same function.
The PO saw `Open Library: unproven` — a *drop* chip meaning "confirm and I will clear
your Open Library id." With C1 that sibling proves agreement and reads `keep`. No code;
a test must pin it.

### Accepted risk

A named omnibus volume: library `The Lord of the Rings` vs provider
`The Lord of the Rings: The Fellowship of the Ring`. Equal mains, agreeing author,
subtitle is prose rather than a number — `OneSidedSubtitle`, so C1 accepts it. Today it
is rejected. **This is a genuine behavior regression for that shape.**

Bounded by construction: `GreyCause` precedence is `VolumeAsymmetry` >
`SubtitleDisagreement` > `OneSidedSubtitle`
(`crates/livrarr-domain/src/identity_matching.rs:68-83`), so `OneSidedSubtitle` provably
means equal mains, no volume evidence on either side, and not two disagreeing subtitles.
A numbered marker (`Book One`, `#2`, `Volume II`) parses into `series_markers` and lands
in `VolumeAsymmetry` or `VetoVolume` instead. `work_key_contradiction` still vetoes
first.

**Correction to the r2 risk framing (verified 2026-07-25).** The sidebar reported that
this false positive *already exists today* via V1 of the 2026-07-14 conformance audit —
`flm_title`, colon-truncation plus word-set containment, runtime-proven to return `true`
for "Dune"/"Dune Messiah". **That is stale: `flm_title` no longer exists in the tree.**
The only survivor is `flm_match` (`async_resolver.rs:318`), which routes through
`title_verdict` + `title_id_trust` + `author_verdict` — the shared authority — and is
still reached twice from `settle_identity` (`:174`, `:198`), so the audit's placement was
right while its mechanism is gone. V1 is fixed; the omnibus risk above is genuinely
introduced by C1 and is not pre-existing on that path. **V2–V8 were not re-verified and
remain 11 days stale** — they do not block C1–C5 but they size the rewrite.

Note also that r2's "Dune Messiah: A Novel" example belonged to the **cover gate**, not
to Rule A, and is moot under C2.

---

## C2 — Delete the Goodreads cover gate

`apply_gr_cover_gate` (`crates/livrarr-enrichment/src/merge_engine.rs:291-333`) and the
`cover_gate` module (`crates/livrarr-enrichment/src/cover_gate.rs`) are removed. The call
site in `MergeEngine::merge` goes with them.

**What it was.** For an English work with an OL key, a Goodreads payload's `cover_url`
survived only if raw token Jaccard over the **full** title cleared 0.6
(`cover_gate.rs:3`,`:43-45`). Einstein scored 0.25 and lost its cover **seconds after the
PO manually certified `gr_key = 10884`** (log, 12:50:33). The gate also stripped
`payload.gr_key` (`merge_engine.rs:327`) — declaring the key untrustworthy while leaving
the work's stored key in place, which is incoherent on its own terms.

**Why deletion rather than tuning.** It is an identity mechanism in cover clothing,
built to serve the since-abandoned goal of showing a cover before identity settles. The
question it asks — "is this payload really this book?" — is already answered upstream by
whoever set `gr_key`, and it is asked with the noisiest available signal.

**Why nothing replaces it.** Goodreads is already first in the English ebook cover order
(`crates/livrarr-enrichment/src/cover_rank.rs:44-53`), and `resolve_cover` already
protects a user's choice (`cover_resolution.rs:77-79`). Deleting the gate yields the
right covers with zero new machinery.

**Direction of change — strictly additive.** With the gate gone nothing strips a
Goodreads cover, so covers can only move *toward* Goodreads, never away. The 44 works
that qualify (English, `ol_key` + `gr_key` present, cover not already Goodreads,
`cover_trust != user`) can gain one; the 59 already showing a Goodreads cover keep it.
**This supersedes r2's warning that 59 covers were at risk — that risk belonged to r2's
"compare main titles" proposal, not to deletion.**

### Accepted risk (PO, on the record)

With the gate gone and the routes model not yet built, a wrong Goodreads id yields a
wrong cover with nothing to catch it. Mitigated by C1 shipping alongside (ids more likely
correct) and by the user's override always winning. The PO accepted this explicitly.

---

## C3 — A provider refusal is not "this book does not exist"

The PO's four Einstein attempts at 12:27 failed; the identical input succeeded at 12:50.
A transient Goodreads refusal, misfiled at the moment it happened. **One bad
classification, three broken consumers:**

1. **The modal blames the user.** `ProviderOutcome::NotFound`
   (`crates/livrarr-external-data/src/provider_client.rs:2366-2368`) → `reason: "not_found"`
   (`crates/livrarr-metadata/src/work_service.rs:2114-2131`) → *"No book was found for
   that identifier — double-check the value."*
   (`frontend/src/pages/work-detail/components/IdentityEditModal.tsx:206`)
2. **The sidebar health dot stays green.** It counts providers with `status == "error"`
   (`frontend/src/components/Sidebar/Sidebar.tsx:373-431`) via `current_error_of`
   (`crates/livrarr-handlers/src/system.rs:130-136`,`:178-191`), but `is_error` is
   `rate_limited | timeout | error`
   (`crates/livrarr-db/src/sqlite_provider_calls.rs:41-43`) — **`not_found` is not an
   error.** The dot stayed green while Goodreads refused every request.
3. **The breaker never learns.** `fetch_goodreads_html_via` reports
   `BreakerSignal::Failure` only for 5xx and `TripImmediately` only for an anti-bot body
   (`crates/livrarr-external-data/src/goodreads/client.rs:187-201`). A 4xx reports
   nothing, so the outbound queue keeps dispatching into a provider that is refusing us.

**PO directive:** the call must ride the centralized mechanism — it already does, via
`RateBucket::Goodreads` through the process-global outbound queue — and a failure must
surface on the provider status line in the left nav.

**Fix at the classification, not at each consumer.** All three consumers already behave
correctly given a correctly-classified record; **no UI work is required.**

### Cross-provider survey — the misfiling is Goodreads-only

`ProviderFetchError` already states the house rule: `NotFound` means "genuinely absent
upstream (HTTP 404/410) — a no-match, never a transient failure"
(`crates/livrarr-external-data/src/types.rs:114-143`). Every provider honors it except
Goodreads.

| Provider | Refusal (e.g. 403) maps to | Record class | Sidebar |
|---|---|---|---|
| **Goodreads** | `ProviderOutcome::NotFound` (`provider_client.rs:2366-2368`) | `not_found` | **green** |
| OpenLibrary | `ProviderFetchError::Other` (`openlibrary.rs:20-25`) → `PermanentFailure` (`provider_client.rs:495`) | `error` | red |
| Audnexus / Audible | `Other` → `PermanentFailure` (`provider_client.rs:850`) | `error` | red |
| Hardcover | `HardcoverError::Http` (`hardcover.rs:95-100`) | `error` | red |
| Google Books | 403 → `NotConfigured` (quota, deliberate); other → `WillRetry{ServerError}` (`google_books.rs:643-666`) | `skipped_policy` / `error` | red |

So C3's classification half is **Goodreads conforming to the existing standard**, not new
policy. No other provider client changes for this half.

**Explicitly NOT changed: `is_error` keeps excluding `not_found`.** A book genuinely
absent from a provider is not a provider-health event; counting it would pin every
sidebar dot red permanently. The defect was Goodreads mislabelling which bucket it was
in, never the bucket definition.

**Scope line retained:** a genuine 404/410 must still not burn retries forever. What
changes is that a *refusal* stops being indistinguishable from a *miss*.

**Robust to the unresolved question.** Whether the 12:27 failures were a 4xx or an empty
200 body is not established — the path is silent (C4), and both terminate at `NotFound`.
The empty-body branch additionally reports a **false `Success`** to the breaker
(`goodreads/client.rs:203`). The fix must handle both: an unparseable body is also not
"this book does not exist."

---

## C4 — The breaker learns from a 4xx, in every client

Unlike C3, this one **is** system-wide. Every client carries the identical shape —
report `Failure` on 5xx, report nothing on 4xx:

`goodreads/client.rs:187-192` · `openlibrary.rs:82-90` · `hardcover.rs:95-100` ·
`google_books.rs:197-216`

A provider that starts refusing every request with 403 therefore never trips its own
breaker. A non-2xx that is not a genuine 404/410 reports `BreakerSignal::Failure` to that
provider's bucket, in every client.

**Bundled here: log the preview path.** Four fetches over 42 seconds produced four rows
in `provider_call_records` and **zero** log lines — the log jumps 12:25:07 → 12:32:15
with nothing between, which is why the PO had to reproduce the failure by hand. One
`info!` at preview entry (work, slot, canonical value), one at exit (outcome class), and
a `warn!` on the silent non-2xx branch carrying the status. This is also what resolves
the 4xx-vs-empty-body question above.

---

## C5 — Unstick the 17 dead-ended works

`attempt_count >= 3` excludes a work from the chaseable-anchor query
(`crates/livrarr-db/src/sqlite_work.rs:1419-1421`, threshold `DEAD_END_THRESHOLD = 3` at
`crates/livrarr-metadata/src/work_service.rs:195`). Without this repair C1 is invisible to
all 17 — nothing will ever look at them again to notice the rule changed.

Delete `gr_work` dead-end rows only, **and set `next_convergence_at = NULL` on the same
works in the same transaction** — the clock is ANDed onto the whole predicate
(`sqlite_work.rs:1433`), so a future clock defers the repair even with the dead end gone.
Measured: 15 of 17 are already NULL; works 8 and 111 carry a clock ~50 minutes out and are
`enrichment_status = failed`, so they would be picked up by branch (2) anyway. Taken for
determinism, not because it unblocks anything.

Marker: `subtitle_rule_deadend_clear_v1`, following the existing `_livrarr_meta` pattern
(`normalized_identity_backfill_complete`, `history_backfill_generation`).

**Honest limit — 4 of the 17 are not this bug.** Works 8 `Master Thaddeus` (pl),
9 `Pan Tadeusz` (pl), 33 `La Casa De Los Espíritus` (es), 71 `Die Krone Der Sterne` (de)
appear in **none** of the 50 `OneSidedSubtitle` declines, so C1 provably will not fix
them. C5 re-tests them; if they dead-end again the cause is still unknown. Deferred to the
next foreign-language block (memory `project_foreign_gr_deadends`). Works 8 and 9 are the
same book under its English and Polish titles — a natural paired probe.

**Framing note from the sidebar:** under the rewrite's model these 17 are
*under-connected*, not broken. Still worth doing; the justification changes, not the
action.

---

## Sequencing

| Step | Content | Gate |
|---|---|---|
| 0 | This design (r3) | **Round 2 review SKIPPED — PO ruling, 2026-07-25.** See below |
| 1 | C1 + C2 (two deletions) | Red-first tests, then delete |
| 2 | C3 + C4 | Independent of step 1; same unit |
| 3 | C5 | Requires C1 live, else a no-op sweep |
| 4 | Verify on the PO's data | Count how many of the 17 re-chase; confirm covers move to Goodreads |

## Test plan

Per `CLAUDE.md` "tests drive the real door" — every test exercises the production entry
path, no injected outcomes.

1. **`title_id_trust` unit** — `Grey{OneSidedSubtitle}` with `IdVerdict::NoEvidence` now
   trusts; a work-key contradiction still does not. The 8 existing tests at
   `identity_matching.rs:1892-2005` are re-read individually, not bulk-updated:
   `..._with_edition_bridge` (`:1908`) and `..._with_work_key_equality` (`:1926`) still
   pass but stop being load-bearing, so a new test must assert the no-evidence case they
   were standing in for.
2. **`verify_gr_payload`** — the Einstein case verbatim from the log: seed `Einstein` /
   `Walter Isaacson` / isbn `9780743264747`; payload `Einstein: His Life and Universe` /
   `Walter Isaacson` / isbn `9780743264730`. Red before, green after. Plus an authorless
   payload still declining.
3. **C1 free rider** — through the real preview seam: a work with an `ol_key`, certify a
   Goodreads key whose title carries a subtitle, assert the Open Library sibling returns
   `keep`, not `drop`.
4. **C2** — through the real merge: an English work with `ol_key` + `gr_key` and a
   Goodreads payload whose title carries a subtitle keeps its `cover_url` and its
   `gr_key`. Assert a `cover_trust = user` work is still untouched. Delete
   `tests/behavioral/test_ewl_cover_gate.rs` with the module it tests — **its six tests
   are not migrated**; they assert the behavior being removed.
5. **C3** — all three consumers, not just the message. Real preview handler, provider
   returning 403: (a) the response carries the unavailable reason, not `not_found`;
   (b) the written `provider_call_records` row is an outcome `is_error` accepts and
   `health_summary` reports Goodreads `status: "error"` — the assertion that would have
   caught the green-dot defect; (c) the Goodreads bucket received a
   `BreakerSignal::Failure`. Repeat for a 200 with an empty body, which today reports a
   false `Success`.
6. **C4** — one non-2xx per client reports `Failure` to its own bucket.
7. **C5** — real `SqliteDb`: seed a `gr_work` dead end at 3 **and** a future
   `next_convergence_at`; run the pass; assert the row is gone, the clock is NULL, and
   `list_convergence_due` now returns the work. Assert no-op on second boot.

8. **The omnibus risk, explicitly.** Round 2 review was skipped, so the accepted risk
   must be recorded in the suite rather than left implicit. Assert that
   `The Lord of the Rings` vs `The Lord of the Rings: The Fellowship of the Ring`
   (equal mains, agreeing author, prose subtitle) **is now accepted**, with a comment
   naming it as the PO-accepted regression and pointing at §C1 Accepted risk. Assert
   alongside it that the numbered shape (`... Book One`, `#2`) is still rejected via
   `VolumeAsymmetry`/`VetoVolume` — that pair is what makes the acceptance bounded
   rather than open-ended. If the second assertion ever fails, the bound is gone and
   C1 must be revisited.

Registering a new behavioral test file and `git add -f`-ing it are ONE change
(`CLAUDE.md`, responsiveness retro).

---

## Out of scope — deferred to the identity-layer rewrite

Tracked at `docs/brief-identity-layer-rewrite.md`; findings at
`build/findings-identity-sidebar-2026-07-25.md`.

- **The routes model** — our own work id as the anchor, provider ids as plural optional
  routes, editions first-class, the borrowed Hardcover/OpenLibrary/Goodreads shape.
- **The cover rank and placeholders** — including extracting the cover embedded in the
  user's own file, which does not exist today and is a new capability, not a re-ranking.
- **The sibling panel going informational**, and the modal wording the PO parked.
- **The cover re-check sweep** (r2's S4b, rebuilt once and still unshipped). Deferred
  with the rest of item 8; the restart-vs-self-retry question goes with it. Note that C2
  makes the sweep purely a matter of *when* the PO sees the change, not *whether*.
- **"Your confirmation wins" as structure.** The fact is already stored and honored
  elsewhere (anchor `setter = user`); C2 removes the one consumer that ignored it.
- **The eight-site matching conformance list (V2–V8)**, unverified since 2026-07-14.
- **The badge's meaning** once a work with zero routes becomes valid — it starts
  describing connectedness rather than identity, and the "no metadata until identity
  settles" gate becomes wrong.
- **Readarr imports**, which hold an ISBN, an ASIN and the files themselves and resolve
  no identity at all.

## Review history

**Round 1 (r1, both FAIL)** — gemini-3.5-flash, gpt-5.5. Artifacts at
`build/reviews/subtitle-matching/review-design-{google,openai}-r1.json`. Codex R-1
established that r1's cover sweep was a no-op that would still have written its
completion marker; that item is now deferred entirely. Gemini R-1 falsified r1's "no
cover can be lost" guarantee, which is moot under C2's deletion. Gemini R-3 (reset the
convergence clock) survives as part of C5. Full disposition table in
`docs/design-history/design-subtitle-matching-r2.md`.

**Neither reviewer challenged the rule change itself** — the load-bearing item, and the
one the prompt asked hardest about.

**Round 2: SKIPPED. PO ruling, 2026-07-25 — "very small probability = acceptable risk."**

The reasoning, recorded so it is auditable rather than assumed:

- Both r1 findings were defects in **machinery that r1 added** (a no-op sweep; an
  invented comparison with a false guarantee). r3 adds no machinery — C1 and C2 are
  ~14 lines of deletion, C3 is a relabel, C4 is one line per client, C5 is a `DELETE`.
- Most of r3 is verifiable without a reviewer: C1's blast radius was checked at all three
  callers with lines quoted; C2's deletion completeness is answered by the compiler;
  C3/C4/C5 are answered by their tests.
- The one question a reviewer was uniquely suited to — *is there a title pair that lands
  in `OneSidedSubtitle` while being a genuinely different work, beyond the named-omnibus
  shape?* — is generative, and the PO judged the residual probability acceptable.
- The feedback loop is short and reversible: local, unpushed, five independently
  revertable changes, verified against the PO's own library within minutes.

**The obligation this transfers to the build:** the omnibus shape must be an *explicit
test* (test-plan item 8), not a silent consequence. A skipped review does not license an
unrecorded risk.
