# Orphaned behavioral-test triage — 2026-07-11

**Subject:** the 30 files in `tests/behavioral/` that are NOT `[[test]]` targets in
`crates/livrarr-behavioral/Cargo.toml` (never compiled by any gate; discovered in the
work-service-split review, scope verified by manifest scan — see wiki insight 65).
**Method:** two-tier agent triage — 3 parallel Haiku readers (content classification with a
verified dead-API list), then 1 Sonnet adjudicator (symbol-level existence checks via Serena on
every keep/uncertain verdict + spot-audits of dead/redundant; 11 Haiku verdicts overturned).
Orchestrator spot-verified the load-bearing claims noted below. Full agent evidence carries
`path:line` citations (task transcripts, session 2026-07-11).

## Verdict key

- **REVIVE-CHEAP** — content aligns with today's code; blockers are mechanical (the `librarr_`
  crate-name typo, a missing manifest entry). Cost S/XS each.
- **REVIVE-REWORK** — subject is valuable and uncovered, but the driven API drifted; needs a
  port, not a typo fix.
- **SPEC-DOC** — self-referential harness: defines and tests its own in-file mocks/traits,
  never touches production code. Historical spec value only.
- **DEAD** — built on deleted APIs or a deliberately-removed product direction.
- **REDUNDANT** — registered suite already covers the subject.

## Tally: REVIVE-CHEAP 10 · REVIVE-REWORK 9 · SPEC-DOC 5 · DEAD 5 · REDUNDANT 1

| File | Verdict | One-line reason | Cost |
|---|---|---|---|
| test_auth_crypto_v21 | REVIVE-CHEAP | AuthCryptoService real, 5/5 method match; `librarr_` typo only | S |
| test_sqlite_auth | REVIVE-CHEAP | real pool/migrations/UserDb, no mocks; typo only | S |
| test_config_v21 | REVIVE-CHEAP | AppConfig/validate_config real, fields match; import path fix | S |
| test_domain | REVIVE-CHEAP | zero typos, 4/4 symbols exact; pure manifest omission | XS |
| test_db_download | REVIVE-CHEAP | DownloadClientDb real, methods match; typo only | S |
| test_download | REVIVE-CHEAP | download-client mock pattern stable; typo only | S |
| test_verify_a1 | REVIVE-CHEAP | WorkCandidate 12/12 field match; documents current accepted behavior | S |
| test_verify_d1 | REVIVE-CHEAP | conflict-anchor trio exact-signature-verified; current behavior | S |
| test_verify_d2 | REVIVE-CHEAP | 1 green regression + 1 correctly-#[ignore]'d red gate (ISBN/subtitle dedup) | S |
| test_verify_g2 | REVIVE-CHEAP | red gate matching today's code exactly — see "Thin retry" note below | XS |
| test_db_auth | REVIVE-REWORK | UserDb/SessionDb real; calls test_helpers constructors that no longer exist | S |
| test_db_peripheral | REVIVE-REWORK | Config/History/NotificationDb real; same test-helper gap | S |
| test_http_v21 | REVIVE-REWORK | 9/10 tests target real client-kind contracts; 1 rides deleted rate_limit | S |
| test_api_work | REVIVE-REWORK | WorkApi trait real + uncovered; imported from wrong (typo'd) crate | M |
| test_api_secondary | REVIVE-REWORK | 9 Api traits real + uncovered; same | M |
| test_metadata_redesign_phase3a | REVIVE-REWORK | overturned from DEAD: AddWorkRequest 17/17 exact, drives live add() | S |
| test_verify_e2 | REVIVE-REWORK | its proof mechanism (inline ol_key gate) is gone; port to settle_identity shape | M |
| test_cup_convergence | REVIVE-REWORK | 8 good convergence scenarios; drives `converge_pending_due` which never shipped (real API: per-work `converge_work` + job) | M |
| test_http_auth | REVIVE-REWORK | ServerAuthService/build_router real, but hand-builds a 7-field AppState vs today's ~60-field struct; needs a shared test builder | L |
| test_startup_v21 | SPEC-DOC | self-invented harness; InMemoryDb gone; uses 3 deliberately-deleted request fields | — |
| test_health_v21 | SPEC-DOC | zero production imports; tests its own mock | — |
| test_job_runner_v21 | SPEC-DOC | zero production imports; "RealJobRunner" is in-file | — |
| test_jobs | SPEC-DOC | zero production imports; JobService mirror exact but siblings drifted | — |
| test_tagwrite | SPEC-DOC | TagMetadata 12/12 match but no TagWriter trait exists (free fns); error enum drifted | — |
| test_server_auth | DEAD | AuthService trait + test factory confirmed gone (arch moved off trait DI) | — |
| test_db_core | DEAD | InMemoryDb confirmed gone; `librarr_db` typo | — |
| test_metadata | DEAD | HardcoverMatcher deleted (Phase-5 removed LLM disambiguation) | — |
| test_metadata_redesign_phase2 | DEAD | 12/12 #[ignore] skeleton for an unshipped spec | — |
| test_metadata_redesign_phase4_5 | DEAD | drives deleted llm_scraper configs; removed product direction | — |
| test_metadata_redesign_phase3b_6_7 | REDUNDANT | shipped subjects covered by registered mc_add_doors / unified_identity_path / s6_retry | — |

## Orchestrator verification notes

- **"Thin retry" (test_verify_g2) — product question, not a plain bug.** Verified in my own read
  of `convergence_service.rs`: both `retry_all_incomplete` (filter = Failed|Unenriched or
  Pending) and `converge_work` (enrichment_incomplete = Unenriched|Failed) exclude
  `EnrichmentStatus::Thin` — a thin-but-enriched work is never auto-retried. BUT the shipped
  convergence design (insight 57, `converge_outcome`, exhaustively unit-tested) deliberately
  treats Thin as *settled*. So the G2 file red-gates a behavior the current design codifies as
  intended. Registering it means first deciding: should Thin works be background-retried?
  (PO call; if "no", G2's assertions are wrong-by-design and it drops to DEAD.)
- The auth story after adjudication: **4 of 5 auth files are revivable** (crypto + sqlite cheap;
  db-layer + http-layer rework), but the largest (test_server_auth, 778 lines) is dead
  architecture. Auth coverage is recoverable, mostly cheaply — it is currently zero.
- Structural blocker found in passing: `AppState` (~60 fields) has no shared test builder —
  blocks test_http_auth and any future handler-integration test. One helper unblocks the class.
- Provenance mystery solved along the way: 12+ files carry a `librarr_`/`livrarr_` crate-name
  typo — they were bulk-generated with a misspelled name and could never have compiled, which is
  how an entire generation landed on disk unwired without anyone noticing.

## Decision needed (PO)

1. Register the **10 REVIVE-CHEAP** files as one small unit (fix typos, add manifest entries,
   red/green-triage on first run)? Note the Thin-retry call gates only test_verify_g2.
2. The **9 REVIVE-REWORK** + the AppState-builder infrastructure: fold into the standing
   "commit the behavioral suite / CI" decision (recommended), or schedule as its own unit?
3. The **11 SPEC-DOC/DEAD/REDUNDANT**: delete, or move to an archive dir so the manifest gap
   stops lying about coverage?
