# Metadata Remediation Plan — 2026-06-29

Remediation of the metadata subsystem per the cross-family-verified audit
(`docs/metadata-audit-2026-06-28.md`, findings M-001..M-021). `PRINCIPLES.md`
and `ARCHITECTURE.md` describe the target state; the findings are violations of
them. This plan groups the work by the rule each finding breaks, sequences it,
and flags the decisions.

**Branch:** `metadata-remediation` (off `main`). Green baseline frozen at
`da2a839` — the in-flight WIP that triggered the audit; the audit was run after
it, so its line citations match that commit. fmt/clippy/test all green (994 pass).

**Process:** lean. One implementer (Sonnet) per unit → I (Opus) review the diff →
cross-family reviews the code → commit. Tests authored by a different family than
the code. Heavy spec/IR only where blast radius demands it (Phase 5). Scale rigor
to risk.

---

## The six problems (grouped by the rule they break)

**A. Stuck states & ignored user actions** — *breaks: User Intent Is Final;
Uncertainty Is Visible Not Silent; Operations Must Be Recoverable.*
Resolving an identity conflict does nothing — it neither clears the badge nor
applies the choice; affirm is a no-op; Readarr import data is dropped; raising a
conflict is non-transactional. Findings: M-019, M-020, M-010, M-016.
**See Phase 0 below — the design evolved past the audit's framing.**

**B. One front door for calling book websites (transport + rate limiting)** —
*breaks: The Canonical Transport Is the Only Transport; One Authority Per Concern;
Wrong Patterns Must Not Compile.* Rate limiting is scattered across two live
limiters, one unthrottled path (identity), two dead limiters, and multiple
uncoordinated instances; Audnexus/Audible bypass the transport entirely.
Findings: M-001, M-009, M-006 (rate parts), M-018.

**C. One "is this the same book?" brain (matching)** — *breaks: Identity Has One
Confidence Hierarchy; One Authority Per Concern.* The identity engine has a
private title-matcher that diverges from the canonical one (colon bug, accents,
stop-words); 6+ normalizers exist. Findings: M-002, M-008. Highest blast radius.

**D. Stop duplicating provider code; shrink the god file** — *breaks: One
Authority; Providers Are Interchangeable; Least Necessary Code.* Hardcover query
pasted ×4; discovery re-parses providers a second time inside a 3,684-line
service. Findings: M-003, M-004, M-005.

**E. Data completeness & safe writes** — *breaks: One Authority (merge);
Operations Must Be Recoverable.* Empty genre list blocks other providers; cover
gate skipped on cache path; app-level CAS with no DB guard; convergence reports
done but re-selects. Findings: M-013, M-012, M-014, M-017.

**F. Dead/misleading code & never-written fields** — *breaks: Least Necessary
Code; legibility.* Dead rate-limiter module, dead 24h cache, dead supersede sync,
audiobook cover sizes computed then dropped. Findings: M-006 (dead), M-011
(DELETE), M-015, M-007.

---

## Phase 0 — refined (beyond the audit)

The audit framed the conflict bug as "the badge never clears." Grounding the
**creation** side revealed the deeper truth and a better design (PO-directed,
2026-06-29):

**What a conflict actually is, and what's already verified.** Three different
situations are lumped as "conflict":
1. **Different ID, same source** (`detect_conflicting_anchors`): the existing
   book has a *confirmed* anchor; a later pass found a different value. **The
   detector ignores WHO set the existing anchor** — so it will raise a conflict
   even against a user's own pick. There is a `setter` stamp (`AnchorSetter::User`
   vs Auto*) that records user picks; the detector throws it away.
2. **No anchor, LLM doubts the guess** (`llm_identity_verify`): fires only when
   nothing is confirmed — both sides are guesses.
3. **Quorum tie at add-time** (`quorum_tie_conflict`, `existing_work_id = 0`):
   no work exists yet — both guesses.

**Confidence hierarchy applied:** a user's UI pick is verified (top, never
override). A system-set anchor is trusted but not user-verified. No anchor = guess.

**The now-build (backend only, in progress):**
1. Never raise a conflict against a `User`-set anchor (respect the pick).
2. Make resolve/dismiss take effect: apply the action, recompute + write the
   badge in one transaction, and stamp the resolved anchor `User` (the user just
   verified it — which also stops the same conflict re-raising). Stop re-asking.
3. Make `raise_identity_conflict` transactional (M-016).
4. Fix the `list_anchors` setter read-default landmine (defaults to `User` on
   parse failure — would wrongly protect a machine anchor); fix `supersede_anchor`
   to sync all 5 columns (M-015, now needed by ReplaceAnchor).

**Deferred to the look-up rework (Phases 2–3):** the app *automatically*
verifying IDs against the source (redirect? still valid?) to auto-settle
system-vs-system cases without asking the user, and reshaping the resolution UI
to "pick the right book."

**Still in Phase 0, not yet started:** M-010 (Readarr import data dropped before
merge — `enrichment/lib.rs` Step 8 re-reads from DB and discards the in-memory
Readarr payload). M-020 (affirm) is largely subsumed by the now-build's
badge-recompute, but verify the last-chaseable-anchor case.

---

## Sequence & dependencies

| Phase | What | Findings | Risk |
|---|---|---|---|
| **0** | Honor user intent; kill stuck conflict states | M-019, M-020, M-010, M-016 (+M-015) | Low, high value |
| **1** | Cleanup & cheap wins (parallel to 0, but shares files — run after) | M-006, M-011 (delete), M-007 | Low |
| **2** | Gateways per provider + shrink the god file | M-003, M-004, M-005 | Low–Med |
| **3** | One process-global rate limiter on every path; Audnexus/Audible onto transport | M-001, M-009, M-018 | Med (adds latency) |
| **4** | Data completeness + convergence | M-013, M-012, M-014, M-017 | Med |
| **5** | One matching authority (LAST, gated by tests + DB diff) | M-002, M-008 | High |

**The trap:** Phase 1 must NOT delete `RateBucket::Audnexus/Audible` — Phase 3
revives them (they look dead only because those clients bypass the fetcher).

**Phase 0 and 1 share files** (`sqlite_work_identity.rs`, `work_service.rs`,
`enrichment/lib.rs`) — run them sequentially, not as parallel coding agents.

---

## Pending product decisions

- **M-008** — cover match threshold (0.6) looser than identity (0.75). Intentional?
  What threshold when unified? Feeds Phase 5.
- **M-001 latency** — throttling identity slows add/refresh on purpose (don't get
  banned > be fast). Accepted by principle; noted as user-visible.
- **M-011 cache** — DECIDED: delete the dead 24h cache in Phase 1.

## Gaps the audit didn't cover (verify before claiming conformance)

- **Tag-writing = user-initiated only** (new hard invariant). The tag-write path
  was never audited. Verify no automatic/silent writes exist; if they do, that's
  a new High finding.
- **Privacy = only public info leaves** (new hard invariant). Outbound call sites
  not specifically traced. Confirm no file paths/checksums/history/prefs are sent.
- **Compile wall partially built.** `livrarr-handlers/Cargo.toml:7-9` shows it
  depends on `http` + `matching` and NOT `jobs` (target = domain + jobs only), and
  pulls raw `reqwest`. Tightening it is an added cleanup item the audit (metadata-
  scoped) didn't cover.
