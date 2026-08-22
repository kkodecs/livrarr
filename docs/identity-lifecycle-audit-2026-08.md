# Identity Lifecycle Audit — August 2026

Read-only audit of the F2 identity layer (rebuilt by identity-layer-rewrite,
merged `ba99290e`, closed 2026-08-20). Conducted 2026-08-20/21. No code was
changed; every finding is backlog. Spec: `spec-identity-lifecycle-audit.md`.
Working artifacts: `build/audit-identity-lifecycle/` (coverage ledger, lane
documents, per-lane findings, verification reports, saved snapshot queries).
Master findings ledger: `build/audit-identity-lifecycle/FINDINGS.md`.

## Plain-English summary

We read every function in the identity system — 1,418 of them, tracked on a
checklist so nothing was sampled or skipped — and checked the code against the
spec, the architecture rules, and a safe copy of the real database. Three model
families did the reading and then cross-checked each other's serious findings.
**Every serious finding survived adversarial verification: 40 were checked, none
was refuted.**

The verdict on the rebuilt identity engine itself: **the core road is sound** —
the matching rules, the Goodreads id separation, the generation/race protocol on
the main road, and the provider transport discipline all held up under a hostile
read. The damage is concentrated in three rings around that core:

1. **The review surface (user-facing) is the weakest area.** Seven kinds of
   review card accept the user's decision, do nothing, and record it as done.
   The two group-resolution choices apply the wrong action. A dismissed card can
   come back. The conflict page can list conflicts nobody can act on. This is
   the "user intent is final" principle failing at the exact surface built to
   honor it.
2. **The old identity columns are frozen but still consulted.** Dozens of code
   paths still read the pre-rewrite columns, which are now permanently empty —
   so import duplicate-detection runs half-blind, ISBN cover lookups are dead,
   files are tagged without their ISBN, and the zero-network re-add optimization
   never fires. The data all exists in the new route tables; these readers were
   never rewired. One door also still *writes* the old columns — putting a
   Goodreads work-id into a column that means book-id, the exact mixup class
   that cost four fix rounds in the rewrite.
3. **The not-yet-used cutover path would lose data.** The ceremony path a real
   pre-F2 install would take on upgrade discards legacy identity evidence and
   user confirmations. Nothing is damaged today (this install activated through
   the clean-empty path) — but it must be fixed before any external install
   upgrades.

Also settled: work-21's dirty title is root-caused (a title-splitting defect
plus a heal that correctly skips user-provenance titles); the manual-import
confirmation gap is confirmed at its exact source; the two "missing" epubs are
present on local disk (needs one check on oasis before closing); and one
long-standing residual (the insight-80 outside-protocol writers) was proven
unreachable in production and reclassified as dead code to remove.

## Numbers

| | |
|---|---|
| Functions enumerated / dispositioned | 1,418 / 1,418 (ledger v3; exclusions listed with reasons) |
| Findings after merge (r2) | 6 P0 (+1 P0-or-P1 PO call) · 23 P1 · 27 P2 · 15 P3 |
| P0/P1 cross-family verified | 53 items, 4 verification passes, **0 refuted**, 21 adjusted (scope/severity) |
| Snapshot | `snapshot-livrarr-20260821.db` (98 works; live DB untouched) |
| Independent duplicate discovery | 5 finding clusters found by 2–3 readers blind |

## Reader scorecard (PO-requested comparison)

Three readers; codex and grok worked the SAME three lanes (persistence,
convergence, review) blind to each other; the anthropic forks worked the other
three lanes.

| Reader | Ground | Output | Precision | Depth |
|---|---|---|---|---|
| codex (gpt-5.6-sol max) | C/D/F, 615 fns, ~84 min | 44 findings: 7 P0, 23 P1 | 0 refuted; 9 adjusted (3 demotions, 1 strengthened, condition notes) | Found every P0 class: review no-ops, cutover data loss, absorption archive gap, dismissal durability, plus the presentation-layer bundle. Also correctly REFUTED a pre-loaded residual. |
| grok (4.6 xhigh) | C/D/F blind, same ground, ~23 min | 14 findings: 0 P0, 3 P1 | 0 refuted; 1 over-claim corrected (double-bump radius) | High precision, honest per-row coverage — but found roughly a third of codex's haul on identical ground and missed the entire P0 class. Its headline finds were also found by others. |
| anthropic forks (fable) | A/B/E, 803 fns | 48 findings: 8 P1, 24 P2, 16 P3 | 8/8 P1s confirmed by codex (4 with scope corrections, 0 refuted) | Different ground — not directly comparable; caught the audit's own tooling bug (ledger truncation) and two scope gaps. |

**Head-to-head verdict (codex vs grok, identical ground): codex by a wide
margin on recall — all seven P0s are codex finds; grok's precision was equally
good but its coverage was a shallow, correct subset.** Grok's distinct
contribution was mechanical detail (the 1+N generation-advance arithmetic) that
sharpened a shared finding.

**Round 2 (PO-ordered): codex re-ran the anthropic lanes blind** (fresh context;
existing findings/report files forbidden; note it had body-verified 8 of the 48
anthropic findings earlier, so 2 of its 13 results are marked tainted for
scoring). Outcome, verified: **5 genuinely NEW findings + 3 real extensions on
ground two anthropic readers had walked** — including the strongest unique find
of the audit (Goodreads Book ids treated as work keys inside the quorum/verdict
layer, the live mechanism family of the historical Sapiens dead-end) and the
Google Books quota-failure-reads-as-no-results defect. Calibration cuts both
ways: codex's two P0 filings both demoted to P1 at verification (it
over-severitizes at the door layer), and one defect rated P3 by the anthropic
reader and P0 by codex settled at P1 — under-call and over-call on the same
item. **Overall reader ranking on this audit: codex first on recall by a
distance, on both halves of the ground; anthropic forks second (better
severity calibration than volume); grok third (precise but shallow).** Feeds
the model-assessment ledger.

## P0 details

See `build/audit-identity-lifecycle/FINDINGS.md` for the merged table with full
evidence pointers. One-line versions:

1. **AUD-P0-1** — Seven review-action kinds no-op and then write a "resolved"
   audit + generation bump: fabricated settlement records (worse than filed;
   found at verification).
2. **AUD-P0-2** — GroupIdentity resolution: DifferentFromAll edits the wrong
   work; AttachOrMerge can discard the user's proposal.
3. **AUD-P0-3** — Work absorption deletes loser routes/state with no archive and
   claims only the winner's generation (concurrent loser edit erasable).
4. **AUD-P0-4** — Non-empty cutover discards legacy evidence + user
   confirmations and skips required preparation. *Never yet run anywhere; must
   be fixed before any pre-F2 install upgrades.*
5. **AUD-P0-5** — Cutover can activate with a skipped, operator-invisible route
   collision. *Same condition.*
6. **AUD-P0-6** — Route ownership has no unique index; one route can go active
   on two works through ordinary settlement.
7. **AUD-P0-7?** — Dismissed review cards can be re-minted (bounded by the
   attempt ledger). **PO call: P0 or P1.**

## Backlog handoff (proposed grouping — PO decides)

- **Fix-wave 1 — review surface** (AUD-P0-1/2/3/7?, AUD-P1-7, AUD-P1-15,
  IA-F05/F07/F08/F09): make every accepted review action real, park machine
  conflicts, wire the conflict page end-to-end, durable dismissals.
- **Fix-wave 2 — frozen-scalar rewiring** (AUD-P1-1/5/8/11/12/13/17, IA-C18):
  one route-projection authority feeding dedup, covers, retag, monitors, reuse;
  stop the confirm-door scalar writes; repair the 5 bad ISBN-10 routes + the
  works-22/24 rows; purge the poisoned GR cache rows.
- **Fix-wave 3 — cutover hardening** (AUD-P0-4/5/6, IA-C07/C09/C17): before any
  external pre-F2 install upgrades; includes the unique ownership index and the
  version-guard bump (83→85).
- **Fix-wave 4 — handoffs + ledgers + matching layer** (AUD-P1-4/9/10/14/18/20/22/23,
  IA-B04, IA-E05): consume every captured handoff (incl. Refresh-All),
  decision-time generations, failure-never-burns, the GR Book-id-as-work-key
  verdict fix, no more implicit different-from-all on rename, honest GB quota
  errors, a registered identity-correction path.
- **Readarr items** (AUD-P1-19/21) hand directly to readarr-import-removal.
- **Dead-code removal** (IA-B08/B09 cluster, IA-C15/C19, insight-80 writers,
  IA-E06 todo!() arms) + presentation bundle (AUD-P1-16) + P2/P3 hygiene.
- **Docs**: spec-fold the fileless door-matrix precedent (IA-005); oasis check
  for the two epubs (IA-001).

## PO rulings (2026-08-22) — amendment to the frozen r2

The PO read the report and ruled "go all" on its open items. The r2 body
above is unchanged (snapshot: `docs/design-history/identity-lifecycle-audit-2026-08-r2.md`).

1. **AUD-P0-7 → P1.** A dismissed review card can come back, but the attempt
   ledger bounds how often, no data is damaged, and the user can dismiss
   again. It stays in fix-wave 1 (durable dismissals), so the ruling changes
   the ledger line, not the work.
2. **Fix-wave grouping blessed as proposed** (§ Backlog handoff), with one
   re-ordering: AUD-P1-18 leaves wave 4 early — the reviewed contest fix
   merges on its own review round (item 4).
3. **IA-001 closed with a root cause — the files were never missing.**
   "oasis" is this host, the only livrarr deployment (the `:8787`/`:8788`
   containers are Readarr/Bookshelf, not livrarr), so there was no second
   deployment to check. Both epubs are on disk at their recorded paths. The
   "missing file" signal is the enrichment materialize step's tag write:
   `crates/livrarr-metadata/src/work_service.rs:3782-3785` (and
   `crates/livrarr-metadata/src/cover_startup.rs:272-275`) hand
   `MaterializeRequest.file_paths` the library-RELATIVE `library_items.path`
   (`1/Jim Butcher/White Night.epub`) with no root-folder join;
   `crates/livrarr-materialize/src/lib.rs:343` passes them to
   `write_tags_batch`, and `crates/livrarr-tagwrite/src/lib.rs:144-149`
   checks `Path::new(file_path).exists()` against the server's working
   directory → `FileNotFound` → `crates/livrarr-materialize/src/lib.rs:345`
   propagates it with `?` (the comment at `:334-336` promises best-effort;
   the code is not) → `run_unified_enrichment: materialize failed`. The OTHER
   tag-write road, `crates/livrarr-server/src/tag_service.rs:41`
   (`{root}/{item.path}`), composes the absolute path and is the one that
   actually writes the files: White Night's on-disk mtime (2026-08-20
   13:22:51Z) sits inside the same refresh whose materialize step reported
   not-found at 13:23:05Z, and 20+ works show the identical import → not-found
   pair in the 2026-08-17..19 logs. Two roads for one concern (PRINCIPLES §1);
   the materialize road is structurally dead for every work that has files.
   **New backlog row AUD-P1-24 (P1, L1/L5): the materialize tag-write road
   passes library-relative paths — unify onto the one path-composing road.**
   Fix-wave 2. This also closes the "missing-files investigation" carried
   since identity-layer-rewrite.
4. **Merges (PO word 2026-08-22):** `exp/e05-queuefull` → `684adbba`
   (IA-E05); `exp/p110-ledger-burn` → `986f7498` (AUD-P1-10); `exp/ae-p109`
   → `269a8269` (AUD-P1-9; one comment-only conflict, PM-resolved; the two
   branches also collided semantically in `test_ilr_contracts.rs` — a
   harness parameter added by one and a call site added by the other — fixed
   by `fb301367`, the "small rebase" the handoff predicted). Contest
   entry A (AUD-P1-18): its normal codex review round
   (`build/contest-p118/REVIEW-A-r1.md`) returned FAIL — 8 P1 + 1 P2: the
   namespace split reached the new matching/capture seam but not the generic
   add path, the ListImport door, enrichment capture, the candidate-reuse
   gate, the conflict payloads, or the public projection (which it regresses
   by serializing a Work id into `gr_key`). NOT merged; branch kept as the
   wave-4 basis pending the PO's call (park / fix-round / override). Full gate on
   the merged tree at `fb301367` (2026-08-22T23:13Z): fmt 0 diffs, clippy 0
   warnings, `cargo test --no-fail-fast` 2,289 passed / 0 failed / 297
   ignored across 177 suites.
5. Spec + frozen report committed (`dd966b55`); `gr.jpg` left untouched (PO
   scratch, untracked).

## Method notes (for the next audit)

The machine-built coverage ledger caught its own generator truncating files at
`#[cfg(test)]` blocks (63+56 rows initially missing) — regenerate-and-diff is
part of the method now. Two scope gaps were caught by readers, not by the PM
triage (series-monitor door file; livrarr-matching crate) — the boundary rule
worked, the file map was the weak point. Blind duplicate discovery across
readers proved to be the strongest evidence class in the audit: five clusters
were found independently 2–3 times, and all five were verified intact.
