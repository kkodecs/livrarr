# Livrarr Quality-Remediation Plan — 2026-07-12

This document is the execution plan for fixing the findings of the 2026-07-12 code-quality probe (`code-quality-probe-2026-07-12.md`, repo root — cross-family reviewed, zero findings refuted): three fix waves ordered by risk, with the process level, parallelization layout, and open decisions for each. Item numbers (#N) refer to the probe document.

**Status: PLAN — owned by the CC orchestrator (PO handed ownership 2026-07-12); PO go still required per wave.**

---

## Journey state (LIVING — edit at the end of every session, read at the start of the next)

This section is the multi-session state carrier for the quality journey. Rules: the closing
session updates Done/Next/Blockers here (with dates); the opening session reads THIS section
plus the newest `~/Projects/kk-build/build/state/handoff-*.md` before touching anything.
kk-build state files remain authoritative for gates; this section is the narrative index.

- **As of 2026-07-13 ~01:00 UTC (session: pipeline-hygiene item-2 verification):**
  - Quality waves: NOT started, but the sequencing precondition is now MET — the
    pipeline-hygiene unit is COMMITTED AND PUSHED as `0eac1e39` (PO signed off 2026-07-13;
    31 files, +2456/−894). Wave 1 is unblocked, needs only PO go. Before dispatching:
    the "#39" numbering reconciliation pass + `/kk-reindex` (snapshot indexes predate
    the suppression deletion and the new suite). Note: this plan references
    `code-quality-probe-2026-07-12.md` (repo root), which remains LOCAL-only/untracked —
    fine for same-machine waves; ship it separately (own PII pass) if it should travel.
  - Item 1 (suppression deletion): DONE + reviewed (unchanged from 2026-07-12; see that
    session's log entries).
  - Item 2 (door-gate suite): VERIFIED 2026-07-13. Per-row conformance read vs
    `design-door-gate.md` done — 19/22 rows conformant as authored; 3 weakened-expectation
    gaps found and FIXED (Layer B now asserts seam work_ids on every row, incl. the
    load-bearing B8 dedup-adoption pin; C1 asserts `source_provider_data: None`; roads.md
    R2 got its convention line; B14 aligned to the packet's bridge-only seed). Gates:
    fmt clean · clippy 0 · door-gate 22/22 · workspace 1522/0/299 (148 suites).
    `git add -f tests/behavioral/test_door_gate.rs` done (staged). Gemini review: both
    schema rounds failed (r1 bare PASS, r2 non-schema → INCOMPLETE); free-form fallback r3
    was the real review — per-row table 20/22 CONFORMS, VERDICT FAIL on 3 findings, all
    dispositioned on evidence (P0 + P1 refuted mechanically: bounded 4750ms advance < 5s;
    start_paused ⇒ current-thread determinism; P2 declined out-of-packet-scope). Closed per
    item-1 calibration; PO adjudicates at sign-off. verify.py tests + review both
    unstampable by design → 2 override_log entries in pipeline-hygiene.yaml mirror item 1.
  - kk-build friction FILED (TASKS.md 2026-07-13): dispatch-authoring 600s budget not
    enforced (new task); gemini schema-mode datapoint appended to the reliability item
    (the model-drift/fabrication item was already closed 2026-07-12 — config re-pins 3.5-flash).
  - PO directives folded 2026-07-12/13 (see Process calls): explicit cross-family review
    map · Serena-first with the worktree exception + reindex cadence · docs-sync subagent
    at every wave close.
  - CC adjustments folded into the waves below (marked `[CC 2026-07-12]`): #23 moved to
    Wave 2 · roads-map dead-code candidates added to Wave 1 · #38 pinned to zero-upgrades ·
    #15 also drops the `content_type.parse().unwrap()` · #37 recommended PARK.
  - Known editorial debt RESOLVED 2026-07-13 (pre-Wave-1 reconciliation): probe #39's three
    bundled sub-items are now #39a (per-book regex — Wave 1 agent 1b), #39b (anchor-redirect
    TODO — out of scope), #39c (ignored dedup bug — out of scope); probe doc annotated to
    match. All other plan #N references verified one-to-one against the probe. Note: #8
    legitimately appears twice (probe #8 bundles the call_sink allows [solo pass] and the
    EnrichmentWorkflowImpl db field [agent 1e]).
  - Item 3 (N4 identity-edit validation): RUN 2026-07-13 with the PO — split verdict.
    N4 picker fix PROVEN live (GR leg now returns the correct subtitled record for
    work 71 World War Z; pre-N4 it returned nothing). But the quorum adopted neither
    offered key (gr AND asin dropped; mechanism unverified — needs a run_quorum trace)
    so the work still lacks gr_key. Both designed resume doors are dead: the "try again"
    refresh caller of clear_anchor_dead_ends was never wired (its trait doc mandates it),
    and the UI identity-edit promised by the Unverified badge tip does not exist.
    PO-decided 2026-07-13: fix = its own small unit (wire try-again + quorum adoption
    trace/fix + tooltip). Evidence: session log 01:30 entry; snapshot
    livrarr.db.pre-n4-validation-20260713; log lines livrarr.log.2026-07-13 @01:21:13.
  - **Wave 1 GO given (PO, 2026-07-13 ~01:45 UTC, overnight autonomous mandate):** run
    Wave 1 to completion, continue into Wave 2's D1/D2-independent groups (2b/2c/2e/2f/2g/2h),
    PREPARE D1/D2 for morning ratification (do not ratify), no Wave 3. Stop only for the
    handoff's Stop Conditions. Full operating instructions:
    `~/Projects/kk-build/build/state/handoff-quality-waves.md` — the next session starts there.
  - **Wave 1 EXECUTED + COMMITTED (2026-07-13 ~08:20 UTC, overnight run):** 7 agents (6 worktree
    + frontend in main tree) + 3 solo passes; gates fmt-clean · clippy-0 · tests 1513/0/299
    (Δ from 1522 fully reconciled: −2 rate_limiter tests, −7 llm_validator in-file tests);
    cross-family review r1 gemini PASS / codex FAIL×2 both dispositioned on evidence, no code
    change (qBit form-body %20 refuted against qBittorrent's own requestparser.cpp; lock-wording
    corrected — 1 dependency-edge line, zero version changes); docs-sync applied (12 wiki edits,
    claim-listed + spot-verified). Item outcomes vs plan: DONE #4(re-scoped: + metadata shim
    re-export), #5, #6(QBitTorrent KEPT — live via QueueResponse; QueueItemResponse deleted),
    #8(6 allows not 7 — enumeration authoritative; + db-field drop), #10(FOUR copies not 3),
    #11, #12(+10 extra byte-identical sites routed), #13, #14, #15, #16, #17, #18, #38(26 deps
    hoisted, 109 decls; data-encoding SKIPPED — real version split "2"/"2.10.0"), #39a.
    DROPPED-REFUTED: #7 (PriorityModel.cover is the LIVE REQ-006 picker input — probe annotated).
    Also: untracked compile-load-bearing fixture force-added (tests/behavioral/fixtures/
    test_cover_100x150.png — third instance of the register-without-add class, now incl. fixtures).
    New dead-code candidates queued in roads.md: download-crate QueueItem/QueueResponse/QBitTorrent.
    FLAGGED DEBT (needs own pass, predates wave): wiki/architecture/metadata-pathway.md still
    describes the pre-Phase-5 LLM-validator flow across multiple sections.
  - Awaiting PO (morning): D1 (qBit truth table) + D2 (swallowed-writes policy) ratification ·
    Wave 3 go/park calls · the identity-fix unit (try-again wiring + quorum adoption + tooltip)
    scheduling.
  - **D1 READY FOR RATIFICATION (2026-07-13):** docs/d1-qbit-state-truth-table-2026-07-13.md —
    sourced from qBit 5.0 API docs + both live classifiers, cross-family verified with 3 folds
    applied (forcedMetaDL row; checkingUP→Queued; moving→Downloading) and dispositions on
    record. One decision: ratify as folded → 2a implements the single shared classifier.
  - **Wave 2 EXECUTED + COMMITTED (2026-07-13 ~11:20 UTC, overnight run) — the six D1/D2-independent
    groups (2b, 2c, 2e, 2f, 2g, 2h), red-test-first throughout:** 10 red pins authored (Codex;
    two rounds — round 1's OL pins were rejected for testing a local harness instead of the real
    path; fixed by making the OL client generic over its fetcher, then re-authored in-crate
    against the REAL fetch), gemini test review PASS via free-form fallback (schema mode failed
    twice — known gotcha), every pin verified red by the orchestrator BEFORE its fix, every pin
    green after. Fixes: Readarr-path canonical row mapper (stale copy deleted) · OL strong-signal
    tiers return circuit/retry-later on transient errors, fall through ONLY on genuine no-match
    (post-review fold: HTTP 404 = new ProviderFetchError::NotFound, dead ol_keys keep their fuzzy
    recovery — codex r2 P1, confirmed at source, fixed + regression-pinned) · CancellationToken
    through the series worker into BOTH GR pagers with select!-ed sleeps + tick consults
    (poller/rss/retention; the two single-atomic-call cleanup ticks deliberately unchanged) ·
    poison-tolerant locks across outbound_queue · combining-mark stripping via
    unicode_normalization (scores change for some non-Latin scripts — pinned, Thai divergence
    empirically confirmed) · cover endpoints on the ApiError envelope (ErrorBody deleted).
    Gates: fmt clean · clippy 0 · workspace 1528/0/299 effective (150 suites; sole failure was
    the documented goodreads-tracer breaker flake, green on isolated re-run). Reviews: gemini
    PASS ×2 (tests r3 free-form, code r2), codex FAIL→folded (r2 P1 above); dispositions in
    build/reviews/quality-waves/. Smoke: DB snapshotted (livrarr.db.pre-wave2-smoke-20260713),
    dev-restart green, post-restart log zero errors with rss/poller ticks completing end-to-end;
    SAB 403 warn verified pre-existing (1419/day since at least 07-11). Docs-sync: wiki verified
    CLEAN for all 8 behavior changes (audit entry in wiki/log.md); two pre-existing drifts
    reported (enrichment-pipeline.md OL cover_url claim contradicts the corrected
    metadata-pathway.md; handlers.md lacks a cover.rs section).
    FOLLOW-ON DEBT (recorded, not done): tokens still don't reach INSIDE rss_sync_run's per-user
    loop or give handler-spawned workers shutdown-linked lifetimes (fresh tokens are
    uncancellable) — same class as the pre-existing handler-spawn gap; candidate future unit.
    **#36 INCOMPLETE (caught at the PO's plan audit, 2026-07-13 morning):** Wave 2 shipped only
    its green structural pin (work+anchor both present after create) — the actual one-transaction
    change (route creation through the confirm_anchor_in_tx path, sqlite_work_identity.rs:20) was
    NOT implemented; the pin passes without it absent a crash. Goes to the next session alongside
    2a/2d (small; the tx helper already exists).
  - **D2 PROPOSED (one line, 2026-07-13):** a failed best-effort DB write is warn!-logged with
    entity context and never counted as success (no `.ok()`-swallow, no `let _ =`, no
    success-log-before-result); where the caller can act on the failure (retryable step,
    user-facing operation), the error propagates instead. Ratify → 2d sweeps #19-#22 under it.

---

- **PO RATIFICATIONS (2026-07-13 morning, in-session):** D1 RATIFIED as folded (the truth
    table in docs/d1-qbit-state-truth-table-2026-07-13.md is now the binding contract — group
    2a implements the single shared classifier next session). D2 RATIFIED as proposed (the
    one-line policy above binds — group 2d sweeps #19-#22 under it, red-test-first). Wave 3:
    items 1-5 (the cheap pure-move splits: #28, #24, #25, #27, #29) are GO, one at a time on
    a quiet tree per the plan; #37 (ID newtypes) PARKED; items 6-10 remain approve-piecemeal.
  - Post-wave wiki fix-up (same morning): enrichment-pipeline.md purged of the deleted LLM
    Validator + false OL cover_url claim + stale GR-LLM claim; handlers.md gained the missing
    cover.rs section. The BIG metadata-pathway.md pre-Phase-5 rewrite remains open (own pass).
  - **Group 2a EXECUTED + COMMITTED (2026-07-13 ~18:00 UTC, fresh session):** ONE shared
    qBittorrent classifier per the ratified D1 table — `classify_qbit_state` in
    livrarr-download (`QbitStateClassification { ui_status, import_safe }`, both projections
    from one table row); consumers: the poller import gate (`.import_safe` — checkingResumeData
    no longer triggers import, the live bug) and `queue_service::fetch_qbit_progress`
    (`.ui_status.as_str()` into `download_status` — canonical vocabulary replaces the raw
    passthrough; SAB arm untouched, no ratified SAB table). `map_qbit_state` +
    `is_completed_state` DELETED. Red-first held: codex authored the 26-row × 2-projection
    pins (test_qw2_class_a_pins.rs `qw2a_*`), my red run pre-fix, all green after; gemini
    tests PASS. Gates: fmt clean · clippy 0 · workspace 1530/0/299, 150 suites (Δ+2 = the
    pins, no flake this run). Smoke: snapshot livrarr.db.pre-2a-smoke-20260713 → dev-restart
    green → poller tick end-to-end, zero errors. Code review: gemini PASS + codex PASS
    (gemini's one listed item = stale echo of the resolved Wave-2 OL finding, dispositioned
    noise — build/reviews/quality-waves/review-code-qw2a-disposition.md). TRACE CORRECTIONS:
    `map_qbit_state` had ZERO production callers — the queue UI received the raw qBit state
    string (frontend never read it), so the UI projection got its FIRST live consumer here;
    D1 doc annotated (header → RATIFIED+IMPLEMENTED + correction block). NEW DISCOVERY
    (follow-on queued): `tests/implementation/` (6 files) is entirely unregistered and has
    never compiled — insight 65 amended, `create_test_notification`'s Wave-1 KEEP was based
    on a phantom caller (roads.md row re-opened), triage pass mirroring
    docs/orphan-test-triage-2026-07-11.md is a new follow-on for PO prioritization.
    #36 placement decided: rides 2d — red pin identified during trace (today an
    invalid-anchor error in `create_work_with_anchor` leaves the work row committed;
    after the one-transaction fix it rolls back — black-box pinnable).
  - **Group 2d + #36 EXECUTED + COMMITTED (2026-07-13 ~19:50 UTC, same session):** the D2
    swallowed-writes sweep — 20 sites total, not the probe's 8: the four cited functions
    (#19 import grab-status ×2 + the FINAL status write + the history event; #20
    resolve_ol_key success-log-under-result; #21 series worker link/count + the
    silent-resolution roster/count pair + the series-books heal count, with `linked += 1`
    now counting only Ok links; #22 CWA/email block: media-mgmt config, root-folder list,
    work read, spawn_blocking JoinError, email config) PLUS codex's review catch (five
    `let _ = update_chapter_scan_result` in try_extract_chapters — folded). ALL warn-arm
    with entity context; zero propagation candidates (each grounded in the disposition).
    **#36 CLOSED** (the 8885464e "incomplete" record is superseded): `create_work_with_anchor`
    = ONE transaction (shared `insert_work_row` helper — one authority for the works
    INSERT — then `confirm_anchor_in_tx`, commit; conflict arm delegates to `create_work`,
    no anchor write, semantics preserved). Red-first: codex-authored rollback pin
    (empty-anchor error must leave NO work row) verified RED at my run, GREEN after;
    orchestrator fold from gemini's test review adds the conflict-path green guard.
    Reviews: gemini tests PASS (free-form r4 — schema failed again), code gemini PASS /
    codex FAIL→resolved (R-5 folded as above; R-4 "propagate resolve_ol_key persist
    failure" REFUTED at the single caller fetch_bibliography_entries:531-533, which
    consumes the returned key immediately — full dispositions in
    build/reviews/quality-waves/review-code-qw2d-disposition.md). Gates: fmt clean ·
    clippy 0 · workspace 1532/0/299 expected (1531 pre-fold snapshot + conflict guard;
    final number in the session log). Smoke: snapshot livrarr.db.pre-2d-smoke-20260713,
    dev-restart green, poller + RSS ticks end-to-end, zero errors. Wiki: no content-page
    falsifications (API-listing lines only — audit entry in wiki/log.md); no new insight
    (error-handling below wiki granularity).
  - **Wave 3 items 1-5 EXECUTED + COMMITTED (2026-07-13 ~20:30 UTC, same session):** PO
    live-word amendment: run in PARALLEL, not one-at-a-time ("push forward on the rest -
    use subagents - in parallel if possible", 2026-07-13) — five sonnet worktree agents,
    disjoint crates, Wave-1 protocol (no Serena editing, plain edits + git grep,
    crate-scoped gates + downstream compile proofs per agent). Outcomes: #28 merge engine
    → livrarr-enrichment/src/merge_engine.rs (24 symbols; 7 re-exported, 17 verified
    zero-caller stay private) · #24 db lib.rs → 31 per-entity src/api/*.rs modules +
    barrel (byte-identical trait/struct name-set: 33 traits/41 structs) · #25 domain
    lib.rs → entities/enrichment_types/infra_config/util modules (105 items accounted;
    TEMP(pk-tdd) banner deleted) · #27 goodreads.rs → goodreads/{mod,client,parsers,
    llm_repair}.rs (157 test attrs before==after; two include_str! fixture paths
    mechanically corrected) · #29 main() 871→497 lines via 12 named private init fns
    (AppState wiring block deliberately left inline — the licensed too-tangled case;
    two clone→to_path_buf conversions forced by borrowed params). Merge: five branch
    diffs applied to one tree (disjoint, zero conflicts) + ONE review fold. Gates:
    fmt clean · clippy 0 · workspace 1532/0/299 — IDENTICAL to pre-wave baseline (the
    pure-move signature). Smoke: snapshot livrarr.db.pre-wave3-smoke-20260713 →
    dev-restart green, full startup sequence + all ticks, zero errors (#29's
    startup-order claim verified live). Reviews: gemini PASS (stale-echo noise) / codex
    PASS with one CONFIRMED P2 fold — the domain/enrichment splits' `pub mod` minted
    unconsumed public paths (AR-13 class); narrowed to private `mod`, root re-exports
    only (disposition: review-code-wave3-disposition.md). Docs-sync subagent swept
    wiki/docs citations (claim list spot-verified; historical audit docs left as
    knowingly-stale per policy).
  - **Quality lane state after Wave 3: items 1-5 DONE. Remaining:** Wave 3 items 6-10
    stay approve-piecemeal (not started); #37 PARKED. Follow-ons awaiting PO
    prioritization: tests/implementation/ orphan triage (2a entry) · metadata-pathway.md
    pre-Phase-5 rewrite (own pass) · identity-fix unit (PO schedules).
  - **INDEXER-CITIZENSHIP UNIT EXECUTED + COMMITTED (2026-07-14 ~00:45 UTC):** design-first
    (design-indexer-rate-limits.md, 4 review rounds: r2 3 folds incl. the raw-reqwest
    canonical-transport violation surfaced + origin-vs-name keying + cold-cache
    empty-success; r3 2 folds incl. MY misclassification of qBit auth/add as indexer
    fetches; r4 PO-ordered gut-check PASS — codex-only per PO gemini waiver). 8 red-first
    pins (7 citizenship + 1 breaker; codex-authored; orchestrator-verified red→green).
    Implementation: sonnet agent + a fold round (codex r6 R-7 configured-origin grab
    keying FOLDED with its own red pin; gemini r6 R-8 normalized_origin hoisted to
    livrarr-http — one authority, zero new dep edges; R-10 doc line). r7 confirmation:
    BOTH families PASS, zero findings. Gates: fmt clean · clippy 0 · workspace
    1541/0/299 (152 suites, +9 pins fully reconciled) · smoke green (snapshot
    livrarr.db.pre-indexer-smoke-20260714; RSS tick on origin-keyed lanes, zero errors,
    no MaM rate-limit warning that tick). Insight 71; dispositions in
    build/reviews/quality-waves/review-code-indexer-disposition.md. Residuals recorded:
    unused `url` dep in livrarr-download (roads-class), duplicate-display-name first-match,
    zero-indexers AllIndexersFailed under cache_only (pre-existing).
  - **NEXT UNITS ORDERED (PO, 2026-07-13 evening):** (1) indexer rate-limiting fixes
    (unthrottled grab-file fetch, dead search cache, no 429 backoff — per the
    code-verified audit) FIRST, then (2) the identity-fix unit (wire the "try again"
    refresh caller of clear_anchor_dead_ends + trace-then-fix quorum key-adoption
    [World War Z / work 71] + honest Unverified-badge tooltip). Alpha6 scope is
    DISCUSSED AFTER both land — not yet set (the 2026-06-10 "Sprints A–F" gate remains
    the last ratified definition until that discussion). PARKED meanwhile:
    tests/implementation/ orphan triage, Wave 3 items 6-10, secret-hygiene drafts,
    metadata-pathway.md rewrite, #37. The PO's SABnzbd 403 install issue is slated for
    the alpha6 test window (test enablement, not release scope).

---

## Sequencing precondition [CC 2026-07-12]

The pipeline-hygiene unit (suppression deletion — done, reviewed; door-gate suite — in
flight) is UNCOMMITTED on this tree and its Wave-adjacent crates overlap agents 1b/1e.
Commit pipeline-hygiene FIRST; every wave starts from a clean base.

---

## Process calls

- **No item runs the full kk-build pipeline.** Nothing here adds functionality, entities, or flows — there is no spec/IR content to write. Kept from the pipeline: a state file for tracking, red-test-first on behavior-affecting fixes, and the cross-family review gate per wave (review → fix → re-review; both families must return real verdicts).
- **Cross-family review map [PO directive 2026-07-12] — the explicit, complete list:**
  1. Per-wave merged-diff review (both families, unprimed prompt: no embedded conclusions,
     explicit license to reject; inline whole units for gemini, file-reading pass for codex).
  2. Wave 2 D1: the qBittorrent state truth TABLE is cross-family verified against qBit
     docs/poller history BEFORE agent 2a implements — the table is the artifact, not the diff.
  3. Wave 3: items #26, #30, #32, #9-part-2, #37 get their short design note cross-family
     reviewed BEFORE code (#9p2 touches the one-matching-authority; #37 keeps its dedicated
     round). Pure moves (#28, #24, #25, #27, #29, #31) ride the wave diff review only.
  4. Any red pinning test authored for Wave 2 is Codex-authored (test_write family policy)
     and Gemini-reviewed, as usual.
- **Serena-first [PO directive 2026-07-12]:** all MAIN-SESSION code navigation and editing
  goes through Serena (symbol lookup, references, symbolic edits) — no raw grep/file-spelunking.
  Parallel worktree agents are the ONE exception and must NOT edit through Serena
  (cross-contamination gotcha: Serena writes to the activated project, not the worktree) —
  they use plain file edits + code-index. Run `/kk-reindex` before each wave and after each
  wave's merge (code-index/Zoekt are snapshot indexes; a wave's deletions/renames stale them).
- **Docs-sync subagent at every wave close [PO directive 2026-07-12]:** after a wave merges
  and gates pass, dispatch a dedicated Sonnet docs subagent to sweep `wiki/` + `docs/` + this
  plan for statements the wave falsified (deleted symbols, moved files, renamed fns, DONE-able
  queue rows) and apply the mechanical fixes + a `wiki/log.md` entry. The subagent returns a
  claim list; the orchestrator spot-verifies load-bearing claims before commit — judgment
  edits to `wiki/insights.md` stay with the orchestrator. Doc updates are part of the wave's
  definition of done, not an afterthought.
- **One exception in kind:** #37 (ID newtypes) gets a dedicated design-review round before any code — it rewrites every persistence/service interface signature.
- Verification cadence per wave: `cargo fmt --check` / `cargo clippy --workspace --all-targets` / full test suite after merge; Wave 2 additionally gets `scripts/dev-restart.sh` + live smoke of the touched flows.

---

## Wave 1 — mechanical, zero intended behavior change (1 session)

Deletions and dedups where the compiler and existing tests are the safety net. **Parallel: 7 agents, disjoint crates, no file overlap.** Merge in any order; each agent runs crate-scoped checks before handing back.

| Agent | Crate scope | Probe items |
|---|---|---|
| 1a | livrarr-enrichment | #4 delete llm_validator.rs (+ mod decl); #7 remove unreachable cover-priority field — _outcome 2026-07-13: agent's source check refuted both as-specified; #7 DROPPED (`PriorityModel.cover` is the live REQ-006 picker input, `lib.rs:984` + `:734`), #4 re-scoped to the solo passes (needs the metadata shim re-export edit, `livrarr-metadata/src/lib.rs:47`)_ |
| 1b | livrarr-server | #5 delete infra/rate_limiter.rs + its tests in state.rs + fix stale `wiki/crates/server.md:40-41`; #39a hoist per-book regex (`readarr_import_workflow.rs:419`); [CC 2026-07-12] close the roads.md dead-code queue: `create_test_library_item` + sibling helpers (api_secondary_impl.rs) and `build_tag_metadata`/`read_cover_bytes` (infra/import_pipeline.rs) — both queued as NEW candidates in wiki/architecture/roads.md since 2026-07-04; update the roads table rows to DONE in the same change |
| 1c | livrarr-download | #6 delete dead traits/structs (ProwlarrClient, QBitClient, QueueItem*); #16 replace hand-rolled `urlencoded()` with `urlencoding::encode` |
| 1d | livrarr-db | #10 one `parse_media_type` in sqlite_common (pick ONE error variant); #11 one `to_str`/`from_str` in sqlite_common |
| 1e | livrarr-metadata | #13 fence-strip helper ×3→1; #12 route cover-path formatting through cover_write_gate builders; #14 dimension-backfill twin blocks → one helper; #8 drop reserved `db` field/param on EnrichmentWorkflowImpl |
| 1f | livrarr-handlers | #15 dedupe download/stream serve block — the shared block also loses `content_type.parse().unwrap()` (panic-on-bad-data) [CC 2026-07-12] |
| 1g | frontend | #18 listImportPreview onto the shared client (also fixes the 401 auth-store desync) |

**Solo passes after merge** (touch many crates; do not parallelize): #8 remove the 7 stale `#[allow(dead_code)]` on call_sink fields; #38 centralize repeated external deps into `[workspace.dependencies]` — [CC 2026-07-12] STRICTLY unify-in-place, ZERO version upgrades (upgrades are their own reviewed change, never a rider); #17 rename `normalize_isbn` → `strip_isbn_punctuation` (cross-crate rename).

Close the wave: full gate + cross-family review of the whole diff → fix → re-review.

## Wave 2 — behavior fixes; red pinning test first on each (1-2 sessions)

Two up-front decisions, then parallel by crate:

- **Decision D1 (proposed in-wave, PO ratifies):** the single qBittorrent state truth table — which states mean "completed, safe to trigger import." Source it from qBit docs + the poller's history, not assumption.
- **Decision D2 (policy, one line, PO ratifies):** swallowed DB writes become warn-and-continue on best-effort paths, propagate where the caller can act on the failure.

| Group | Crates | Probe items |
|---|---|---|
| 2a | download + server | #1 one shared qBit classifier per D1; poller consumes it |
| 2b | db | #2 Readarr import path onto the canonical row mapper (kill the hard-coded tag fields); #36 work+anchor creation in one transaction (confirm_anchor_in_tx path — verify it exists at execution) |
| 2c | external-data | #3 OL ISBN/key tiers mirror HC's per-error-variant handling (CircuitOpen → circuit outcome, transport → retry-later; no silent downgrade to fuzzy fallback) |
| 2d | library + metadata + server | #19-#22 swallowed-writes sweep per D2 (incl. the mis-counted `linked += 1`, the unconditional success log in resolve_ol_key, CWA fire-and-forget logging, and the canonical mapper's silent try_get defaults) |
| 2e | metadata + server jobs | #33 CancellationToken through the GR pagination loops; #34 ticks that ignore `_cancel` (download_poller, rss_sync, maintenance) actually consult it |
| 2f | http | #35 poison-tolerant locks throughout outbound_queue (match the guard's own discipline) |
| 2g | matching | #9 (part 1) replace the identity-function `unicode_general_category` + partial table with `unicode_normalization::char::is_combining_mark` — match scores can change for non-Latin scripts; pin with tests |
| 2h | handlers + frontend | #23 route cover.rs's three ad-hoc error schemes through ApiError — moved from Wave 1 [CC 2026-07-12]: error RESPONSE BODIES change shape, so it belongs with the live-smoke wave; verify frontend tolerance in the smoke |

Close the wave: full gate + dev-restart + live smoke (add a book, poll a download, refresh) + cross-family review → fix → re-review.

## Wave 3 — structural moves, one at a time, interleaved with normal work

Each is a focused session with a short design note; land only on a quiet tree. Ranked lowest by both reviewers — approve piecemeal; nothing else depends on these.

Order (cheapest/safest first):
1. #28 merge engine → its own module in livrarr-enrichment (pure move)
2. #24 db/lib.rs → per-entity trait/DTO modules (pure move)
3. #25 domain/lib.rs → entities/infra/util modules + delete the stale `TEMP(pk-tdd)` scaffolding banner
4. #27 goodreads.rs → client/parsers/llm_repair modules (pure move)
5. #29 main() → named init functions
6. #26 series_query_service split (reuse the work-service-split playbook)
7. #30 manual-import business logic (audio grouping, work resolution) behind a service trait
8. #32 7-arg DB methods → request structs (removes most `too_many_arguments` allows)
9. #9 (part 2) consolidate m4_scoring's fuzzy engine onto domain text_norm primitives — one matching authority
10. #31 WorkDetailPage.tsx → per-component files (frontend; can run parallel to any Rust item)
11. #37 **ID newtypes** — design-review round first (pattern, serde/sqlx impls, staging — possibly one ID per commit). Highest type-safety payoff, widest churn. **Explicitly parkable — PO call. [CC 2026-07-12] recommendation: PARK — widest churn in the list, competes with alpha feature momentum; revisit after user feedback settles.**

## Out of scope (tracked, not fixed here)

- #39b anchor-redirect TODO (`sqlite_work_identity.rs:330-340`) — needs redirect-detection machinery that doesn't exist; future feature.
- #39c ignored dedup bug (`test_verify_d2.rs:187`) — functional bug with its own pending fix constraint, not a quality item; keep on the bug backlog.
- Everything in the probe's "deliberately not listed" section (decided/tracked elsewhere).

## Open PO decisions

1. Go/no-go per wave (Wave 1 needs no other decisions).
2. Wave 2: ratify D1 (qBit truth table, proposed in-wave) and D2 (error policy).
3. Wave 3: which items to run, and whether #37 (ID newtypes) runs or parks.
