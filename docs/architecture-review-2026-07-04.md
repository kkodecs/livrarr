# Whole-App Architecture Review — 2026-07-04

**Tip reviewed:** code = `main @ 00daf3a` (the remediation-epoch merge). Two commits since
(`648dfec`, `c9b3ccb`) verified docs-only (`git log --stat 00daf3a..HEAD`), so all code claims
hold at HEAD. Entry point: `docs/architecture-review-briefing.md`.

**Method & trust model.** Briefing-driven. Inputs: (1) the three governing artifacts read in
full this session (root `PRINCIPLES.md`, `build/foundation/principles.md`,
`docs/canonical-model.yaml`, plus `wiki/architecture/roads.md` and `wiki/insights.md`);
(2) the machine drift-audit (`audit_canonical.py`, exit 0, run this session); (3) three
read-only subagent surveys (branch archaeology; ARCHITECTURE-doc pair; livrarr-server
composition root), every load-bearing claim carrying doc-line + code-line citations;
(4) orchestrator spot re-verification at source for the claims marked ✔ below (Cargo
manifests, caller searches, tracker state, package.json). One subagent claim was **refuted**
during synthesis and excluded (a "dead `MetadataProvider` trait in domain" — it is the live
provider-ID enum, `crates/livrarr-enrichment/src/lib.rs:91`,
`crates/livrarr-external-data/src/provider_client.rs:23`; wiki insight 53 stands).
Claims marked (agent) rest on a single subagent's cited verification; the cross-family
refutation pass (below) is their independent check. Per the roads.md lesson, reviewers were
instructed to refute, not confirm.

**Not in scope** (stated so absence ≠ clearance): frontend architecture (briefing excludes
it; touched only for E2E wiring), runtime/domain-logic correctness (the 2026-06-28 metadata
audit owns that; not re-audited), DB schema content, performance (speed-baseline docs stand),
security posture (deferred-items list stands; SSRF trusted-infra pattern not re-audited).

---

## Verdict

**The architecture itself is sound; the governance layer around it is what drifted.**
The structure contract holds under machine audit (0 seam violations, 0 stale entities,
0 ghost modules). The flow contract (14 roads) was adversarially verified 2026-07-04 and
committed. The compile wall is real at the manifest level
(`crates/livrarr-handlers/Cargo.toml:7-9` — domain+http+matching only ✔). Zero `OnceLock`s
in AppState (agent, full-repo search). The canonical model's amendment log is honest — and in
one case *behind* reality: a conformance debt (S1) was paid and nobody recorded the win.

The live problems concentrate in four places: **(1)** the authority *documents* disagree
with code and each other in specific, verified spots — including the highest-authority
principles doc not being in version control at all; **(2)** bookkeeping lag (model
amendments, tracker issues, wiki pages trailing shipped code); **(3)** the known
consolidation debt (three parallel import implementations, the user-cover fork, an orphan
crate); **(4)** one broken pipeline stage (E2E points at a script that exists only on an
unmerged branch). Nothing found indicts the runtime design. No new correctness-critical
code drift surfaced within this review's scope.

---

## Findings

Severity: HIGH = governs correctness of future work · MED = real drift/debt, bounded ·
LOW = hygiene · INFO = metric/policy input. ✔ = re-verified at source by the orchestrator.

### AR-01 · HIGH · The product-principles doc is untracked, and "principles" is three documents
`build/foundation/principles.md` (15 product principles; the doc CLAUDE.md names **highest
authority**) sits in gitignored `build/` (`.gitignore:2`) — not in version control, invisible
to any clone ✔. Root `PRINCIPLES.md` is a *different, complementary* document (7 engineering
principles + red flags; it delegates product principles to `ARCHITECTURE.md` Part 1 at
`PRINCIPLES.md:9`) ✔. `ARCHITECTURE.md` Part 1 restates product principles in a third form,
and diverges (see AR-02's tag-write item). The briefing called this "two copies to
reconcile"; it is actually **three documents needing one authority chain**.
**Fix:** move the 15-principle doc into the tracked tree (recommend `docs/principles.md`),
update the CLAUDE.md pointer, grep kk-build for the `build/foundation/principles.md` path
(the doc header says it is injected into every reviewer prompt) and repoint; `ARCHITECTURE.md`
Part 1 defers to it rather than restating.

### AR-02 · HIGH · Root `ARCHITECTURE.md` was authored from intended design, not code — 5 verified errors
(agent; doc-line + manifest cites) 3 of 15 dependency-table rows are wrong:
`:148` identity "domain, http, db, external-data" — actual has **no db**
(`crates/livrarr-identity/Cargo.toml:7-16`); `:150` materialize "domain, http, db" — actual
is domain, http, **tagwrite** (`crates/livrarr-materialize/Cargo.toml:13-15`); `:153` library
"domain, db, matching" — actual is domain, db only ✔ (`crates/livrarr-library/Cargo.toml:13-14`).
`:156` handlers "domain + jobs only — COMPILE WALL" — actual composition is
domain+http+matching, no jobs ✔ (the wall *invariant* holds; the stated composition is
false). `:269-270` the provider-addition checklist is stale in two steps ✔: "implement the
`ProviderClient` trait (defined in livrarr-domain)" — it is a `pub enum` in
livrarr-external-data (`provider_client.rs:35`, re-exported `livrarr-external-data/src/lib.rs:29-31`)
— and "add the provider to the enrichment dispatch table in `livrarr-enrichment`" — the
registration point is that enum, not an enrichment-crate table (Codex R-3; replace the whole
checklist, not one sentence). Plus one product-behavior
divergence: `:74`/`:237` "tag writing... never done automatically" contradicts both
principles P5 ("tag writing happens at import time",
`build/foundation/principles.md:26`) and the code (import writes tags when metadata is
present, `import_service.rs:94-108` (agent)) — **the code conforms to P5; the
ARCHITECTURE.md wording is the drift.**
**Root cause and the structural fix:** crate-dependency facts are restated in four places
(canonical-model.yaml `seams` [the authority], ARCHITECTURE.md table, docs/ARCHITECTURE.md
table, wiki overview graph) — a one-authority violation in the doc layer. Fix pass on
ARCHITECTURE.md + replace its table with a pointer to `docs/canonical-model.yaml`.

### AR-03 · MED · `docs/ARCHITECTURE.md` is a stale duplicate — salvage 3 lines, delete
(agent) Frozen ~2026-05-10, pre-modularization: "13-crate workspace" (actual 17,
`Cargo.toml:4-20`), missing external-data/identity/enrichment/materialize, missing Google
Books + Audible from the provider roster. Salvage before deletion: the frontend row (root
doc never mentions the frontend at all), the migration **pre-backup** detail (`VACUUM INTO`
before migrations, `crates/livrarr-db/src/pool.rs:200-201` (agent)), the "multi-user from
day one" framing.

### AR-04 · MED · Conformance win S1/#143 is paid in code but unrecorded everywhere else
`livrarr-library` no longer depends on tagwrite (the S1 off-model edge) — deps are
domain+db only ✔. The materialize route #143 prescribes was never taken (delegate-injection
solved it instead; insight 51 L7). But: GitHub **#143 is still OPEN** ✔, the canonical
model's `amendments[]` has no closing row (last row 2026-06-11), the `library→materialize`
seam permission is now vestigial intent, and wiki insight 48 still claims "reds ≤2,
currently 1: Release" (Release conformed 2026-06-11 per amendment row 4; #141 CLOSED ✔).
**Fix:** amendment row recording S1 closed-as-built; close #143 with that note; trim or
annotate the unused seam permission; correct insight 48.

### AR-05 · MED · `livrarr-jobs` is an orphan crate — the designed seam was never wired
Zero dependent crates workspace-wide: no `livrarr-jobs` in any member manifest, zero
`livrarr_jobs::` references outside the crate ✔ (index search) — independently corroborated
by the drift-audit (both its permitted inbound edges listed unused) and by the agent's
manifest sweep. Its documented purpose ("compile-wall-safe job triggering from handlers",
insight 1, `ARCHITECTURE.md:156` which wrongly says handlers depend on it) never happened;
the live pattern is handler-level spawning (insight 9g). **Decision (PO):** wire it as
designed, or delete it. Deletion is NOT free (Codex R-1): the canonical model encodes the
crate and both its inbound permissions (`docs/canonical-model.yaml:75,77,79`) and the crate
defines the `JobService` trigger surface (`crates/livrarr-jobs/src/lib.rs:3-17`) — so delete
= model amendment + crate removal, wire = make the documented seam real. Recommend
**amend-and-delete** — a ghost seam that authority docs describe as wired invites designs
against a false model (this review caught ARCHITECTURE.md doing exactly that), and the
crate is trivially re-addable when a real handler-trigger need arrives.

### AR-06 · MED · The pipeline's E2E stage cannot run against main
kk-build `config.yaml` declares `e2e.command: pnpm -C frontend e2e`; main's
`frontend/package.json:6-15` has **no `e2e` script** ✔. The Playwright runner + smoke test
exist only on unmerged `feat/playwright-e2e` (1 commit, 2026-06-07; verified absent from
main: no playwright.config.ts, no e2e/ dir, no @playwright/test dep (agent ✔ both sides)).
**Fix:** land that branch (small, self-contained) or change the config; recommend land.

### AR-07 · MED · Branch archaeology: 13 confirmed; 11 safe to archive; 1 superseded prototype
(agent; per-branch evidence with hashes in its report, appendix pointer below) The briefing's
"13 legacy branches" count is exact; the presumption "all shipped" is **true for 11** — every
ahead-commit patch-equivalent in main or verified shipped via main's tree + CHANGELOG (incl.
the 166-commit `feature/transmission`, all 8 residual commits individually checked).
Not leftovers: `feat/playwright-e2e` (real absent content → AR-06) and
`prototype/gb-first-search` (4 commits: Audible-client half **shipped** — identical
`AudibleCatalogClient` in external-data; the pre-add cover-picker flow half was **rejected**
— alpha6 shipped the deliberate opposite, "no pre-add picker step"). Disposition: archive or
delete the 11 + the prototype; nothing to salvage (the surviving idea, cover override-lock,
already shipped). Separate PO policy call: main's branch protections were admin-bypassed by
the epoch push — decide whether the rules or the workflow changes.

### AR-08 · MED · Dead code: 9 verified items ready for one deletion batch
The roads.md table's 7 remaining items stand; its one single-family caveat is now closed:
`ReadarrImportService::update_work_enrichment` has zero callers ✔ (full-repo symbol search —
only the db-trait method, tests, and its own impl at `readarr_import_service.rs:207-215`).
Two new finds, both double-verified (agent + index ✔): `services/release_service.rs`
(`ReleaseService` — only its own definition; live = `livrarr_download::…::ReleaseServiceImpl`,
`main.rs:653`) and `services/manual_import_service.rs` (shadowed by the live top-level
`manual_import_service.rs`, `main.rs:809`; its doc-comment even references an
already-deleted field). Execute as one reviewed batch; snapshot-before-delete discipline.

### AR-09 · MED · Three startup passes bypass the job runner's safety rails
(agent) `chapter_backfill`, `cover_startup`, `series_backfill` run as bare `tokio::spawn` in
`main.rs:946-961` — no panic isolation, no `job_statuses()` visibility, no
`CancellationToken`, unlike all 8 JobRunner-managed jobs (`jobs/mod.rs:86-141`). cover_startup
does disk migration + crash recovery; a panic there is a silent partial startup. **Fix:** a
one-shot mode on JobRunner (or a startup-pass wrapper) giving the same isolation/visibility.

### AR-10 · LOW · main.rs hygiene
(agent) 64% of main.rs (~750/1167 lines) is the single AppState construction block; the
LLM-caller + merge-engine build sequence is copy-pasted 4× (`main.rs:359-373, 547-563,
710-725, 766-781`) — extract a helper; "Step 7/8/9" comment numbering is reused at
`main.rs:105/119/127` and `917/930/933/965` — renumber during the same touch. Also
composition-root comment drift (Codex R-2, verified ✔): `state.rs:144-153` still labels
`provider_queue` / `enrichment_service` "Phase 1.5 plumbing… not yet on the live enrichment
path" — those fields ARE the live path (`main.rs:542-565` wires them into the
work-service/unified-enrichment construction; insight 51 L8) — fix the comments in the same
touch.

### AR-11 · LOW · Cover-image logic is scattered at the edges
`mediacover.rs:105-111` does inline `image::load_from_memory` + JPEG re-encode in the
handlers crate ✔; `cover_service.rs:296-328` (agent) does its own magic-byte sniffing,
hardcoded 8000×8000 cap, and re-encode beside `livrarr_metadata::cover_resolution`'s
existing dimension logic. Both are R3-adjacent; fold into the Phase-2 cover-gate
unification rather than fixing piecemeal.

### AR-12 · MED · Wiki reference layer needs one refresh unit
`wiki/crates/server.md`: 4 phantom AppState fields (provider_health, goodreads/ol rate
limiters, refresh_in_progress — all deleted from code), ~15 missing current fields, Jobs
section omits 2 live registered jobs (convergence, tag_convergence), and describes the two
AR-08 dead services as live (agent; doc:code cites in its report). Plus the 4-page
correction queue at the bottom of roads.md, plus the insight-48 fix (AR-04). One deliberate
wiki-refresh unit, not drive-by edits.

### AR-13 · INFO → rule agreed 2026-07-04 · Public surface: the metric needed a rule, now it has one
The drift-audit's "713 pub types outside the spine" (livrarr-domain 272, handlers 154,
server 66) merges two different things — legitimate cross-boundary plumbing and types that
never needed to be public. The rule that separates them (PO-agreed 2026-07-04; the durable
one-liner is a new Red Flag in `PRINCIPLES.md`):

**2×2 — visibility = (is it a spine concept?) × (does it cross a crate boundary?):**
spine+crosses = public entities · non-spine+crosses = legitimate plumbing · non-spine+stays-in-
crate = **private target** · spine+stays = suspicious (a "core" thing nobody else uses isn't core).

**Should be private:** used only in its own crate · internal "how" helpers/scratch · builders &
half-built scaffolding · the raw shape of one external system (convert at the boundary) ·
storage/row shapes outside livrarr-db · internal errors/states translated before they leave ·
concrete `Live*Impl` guts behind a trait · test-only types. **Harder-private:** anything holding
a plaintext secret (narrow even across a boundary — Secure-by-Default + credential isolation).
**Do NOT cut (legit public):** `*DbRequest` (crosses service→db; the real rule is "never in a
handler-facing trait") · request/response + converted provider/identity payloads (they exist to
cross the seams; not-spine ≠ private).

**Mechanical test:** for a `pub` type, is it referenced by any crate other than its home? None →
delete (dead) · home-crate-only → `pub(crate)` · another crate → leave public. **Scope caveat
(load-bearing):** valid for livrarr-domain and other non-web crates; NOT for handler response
DTOs, whose boundary is the HTTP wire, not another crate — judge those by hand. Name-scanning is
a conservative floor for used/unused, but the dead-vs-privatize sub-split has transitive-
reachability false positives (proven: `DownloadProgressStatus` is a field of `DownloadProgress`
→ `QueueItem`, `grab.rs:15,36`) — treat the sub-buckets as a review worklist, not verdicts.

**First application — livrarr-domain (2026-07-04):** 346 pub types; **327 (95%) used by ≥1 other
crate** — the core vocabulary is NOT a junk drawer; the scary 713 is dominated by legitimate
plumbing, as the 2×2 predicts. 19 stragglers: 5 test-only (integration tests force `pub`),
6 privatize candidates (`ParsedTitle`, `SeriesMarker`, `TorznabItem`, `ResolvedTarget`,
`ProviderPolicyError`, `ProviderPolicySource` — pending a return-type check), ~8 delete-
candidates that fold into the AR-08 dead-code sweep (per-item verify + snapshot). Remaining work
rides the bucket-2 cleanup: apply the edits, extend the scan to the other non-web crates
(livrarr-external-data is the one likely to surface real raw-shape leaks, unlike handlers), and
wire the mechanical test into `audit_canonical.py` so it self-enforces ("make wrong patterns
fail, don't document them").

### AR-14 · INFO · Seam permissions wider than use — mostly fine, one indirection to watch
17 allowed-but-unused edges (audit). Most are harness/CLI breadth. Notable:
`server→enrichment/identity` are unused because construction rides livrarr-metadata's
re-export shim (`livrarr-metadata/src/lib.rs:40-41` — self-documented as scheduled for
deletion, AC-021 (agent)); when that shim dies, server's direct edges become real — already
legal in the model. `library→materialize` → AR-04.

---

## Day-one items from the briefing — resolution status

1. **Duplicate authority docs** → AR-01/-02/-03 (this review's main result).
2. **roads.md provenance** → resolved before this review started: verification record
   committed beside the map (`c9b3ccb`); its corrections/deletions queue → AR-08/AR-12.
3. **Branch archaeology** → AR-07 (full evidence table in the agent report; protections
   policy = PO decision).

## Standing decisions this review re-surfaces (unchanged, PO's court)

- **Background convergence ships disabled** (roads R2 DECISION): on default installs,
  batch-created works sit identity-pending unless a user manually retries — in tension with
  principle P6's "never silent limbo" for exactly the paths users don't watch. Engine built,
  flag off. Enable / off-but-surfaced / stay dark. (N4 live-validation also waits on this.)
- **Import consolidation (Phase 2)** — R7/R8/R9's three parallel file→LibraryItem
  implementations remain the largest live violation of Principle 1 / the one-road rule.
  This review found no reason to widen its scope and one more reason to do it (AR-11 folds
  in). **The stated blocker — "wait for the architecture review" — is now cleared.**
- **Suppression machinery idle** (`ProviderOutcome::Suppressed`, zero producers) — keep or
  remove, unchanged from the Phase-3 flag.

---

*Cross-family refutation: COMPLETE — Codex PASS (blind), Gemini PASS (retry round after one
empty-payload flake; saw Codex's findings). Zero refutations of substance; every sampled
single-source claim confirmed with independent citations; three reviewer refinements folded
(AR-02 provider checklist, AR-05 amend-and-delete, AR-10 state.rs comment drift). Full
record: `docs/architecture-review-2026-07-04-verification.md`.*
