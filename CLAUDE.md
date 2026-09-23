# Livrarr — Project Rules

## Executive summary

[Scope additions require explicit user approval](#process-discipline-from-v21-retro).

Use the [permanent model assignments](#permanent-model-assignments) for production
code and tests: Opus 5.5 writes production code (Fable 5.1 until 2026-09-23), and each
artifact has two assigned reviewers from other model families. [The PM never edits an artifact](#the-pm-never-edits-an-artifact-po-direction-2026-09-22);
every fix or fold goes to a seat. The [principles](#principles) remain the
highest project authority; follow [document stewardship](#document-stewardship)
for durable project records.

PM startup/resume/handoff: first read the short
PM context entry at `~/Projects/kk-build/templates/claude-md/pm-context.md`.
Use selected current state and evidence for the next decision. Mandatory rules below
remain in force; this pointer grants no new staffing or approval authority.

## Session Start (mandatory — do these before any work)

1. **Read applicable `wiki/insights.md` learnings before work.** For PM startup, resume or process work, read the short PM context entry first, scan insight headings and open the entries relevant to the next dispatch or acceptance decision. Follow their detailed sources when needed. Implementation seats still read the full insight index before coding. All insights remain authoritative; scoped reading does not waive an applicable rule.
2. **Use Serena MCP for ALL code navigation** — symbol lookup, dependency walking, reference tracking. Do NOT grep source files or read entire files when Serena can answer the question. Always try Serena first.
3. **Check `wiki/` before re-deriving domain knowledge** — read `wiki/index.md` to find relevant pages before reasoning from scratch about how a subsystem works.
4. **Update `wiki/` when you learn something new** — at block boundaries or when you discover domain knowledge a future session would need, update or create wiki pages per `/pk-wiki`. If you don't write it, the next session re-derives it.

## Document stewardship

**Project documents live with Livrarr (PO direction, 2026-09-07).** Store
handoffs, session logs and feature state in `/mnt/opt/livrarr/build/state/`;
review notes and evidence in this project's `build/reviews/`; reports in
`build/reports/`; and product documentation in its existing `docs/` and `wiki/`.
Use the recorded project document root when reviewing a separate source worktree.
`~/Projects/kk-build` holds shared tooling and instructions. Existing central paths
needed by older hooks may remain as compatibility symlinks to the project files;
new documents and current links use the project paths. Preserve historical evidence
and resolve existing files before replacing a legacy path.

For build startup, accepted milestones and handoff, follow the
[steward role](build/ops/document-steward/ROLE.md), which links the reusable workflow.
Read [the continuation](build/ops/document-steward/CONTINUE.md) and only relevant
[map entries](build/ops/document-steward/DOCUMENTS.yaml); resolve the existing seat
from live PM state and Herdr. Routine upkeep is packet-driven within a free write window.

### Executive summaries (PO direction, 2026-09-18)

Start every new or substantively revised prose document with an **Executive
summary** immediately below its title. This applies to reports, proposals, specs,
designs, wiki pages and handoffs. Write it in plain English: lead with what the
reader needs to know, the practical consequence, and any recommendation or decision
needed. State material uncertainty or unfinished work clearly. Include descriptive
links to the relevant sections farther down in the same document so the reader can
choose how much detail to read. Keep the summary proportional to the document;
technical detail belongs in the linked sections. Preserve accepted historical
versions when revising an existing document.

Name the exact action, object and starting condition before describing a problem.
For example, distinguish creating a new Work, importing an ebook or audiobook
file, and adding an existing Work again; do not call all three "adding a book."
Use a concrete expected-versus-observed example, and distinguish what was tested
from what is inferred about a user interface or other path.

## Design Documents — Supersede, Never Overwrite (PO-approved 2026-07-24)

A design document under `docs/` is revised in place ONLY after the previous revision is
snapshotted to `docs/design-history/<name>-r<N>.md`. Never rewrite a design round over
its predecessor: the identity-edit RCA (2026-07-24) found r2's reasoning permanently
unrecoverable because the file had been overwritten each round, so the basis of a
false ground-truth claim could not be reconstructed. Standard practice elsewhere is the
same — accepted decisions are superseded and linked, never edited away. The snapshot is
part of the same change as the revision, not a later cleanup.

## Principles

See `ARCHITECTURE.md` Part 1 (Product Principles) and `PRINCIPLES.md` (universal engineering principles). Highest authority in the project. Override everything below when conflicts arise. (Formerly split across `docs/principles.md` and gitignored `build/foundation/principles.md`; consolidated into the tracked root docs 2026-07-05 — see `docs/architecture-review-2026-07-04.md` AR-01.)

## Build Cycle (pk-auto-build)

- **Phase order is non-negotiable:** Spec → IR → Behavioral Tests → Code → Implementation Tests → Deploy → Retro
- **Audit before commit, never after.** No phase-gate commit without a passing audit.
- **Cross-family separation:** Follow the [permanent model assignments](#permanent-model-assignments) below. No model reviews its own family's output.

### Permanent model assignments

PO direction, 2026-09-18; coding seat changed to Opus 5.5 by PO direction 2026-09-23
(outright swap, no contest; basis in
`build/reports/opus-5-5-coding-seat-assessment-2026-09-23.md`). These are the permanent
Livrarr assignments and override older generic kk-build seating defaults for the roles
listed here.

| Responsibility | Model | Reasoning effort |
|---|---|---|
| Write production code | Anthropic Claude Opus 5.5 (`claude-opus-5-5`) | xhigh |
| Review production code | OpenAI GPT-6 Astra **and** Google Gemini 3.8 Flash | Astra: xhigh; Flash: `thinkingLevel: HIGH` (its ceiling) |
| Write tests | OpenAI GPT-6 Astra | xhigh |
| Review tests | Google Gemini 3.8 Flash **and** xAI Grok 4.6 | Flash: HIGH (ceiling); Grok: xhigh (`max` is invalid and silently ignored) |

Effort record corrected 2026-09-23 (PO direction): the earlier "max" entries for Gemini and
Grok named levels those runners do not accept. Gemini 3.x exposes `thinkingLevel` LOW/HIGH
(newer builds add MINIMAL/MEDIUM); the seat's `.gemini/settings.json` pins HIGH. Grok's real
enum tops out at xhigh. Other defaults from the same direction: PM (Fable 5.1) runs at
`high`; project subagents run Opus 5.5 at `xhigh` for investigations and `high` for small
bounded fixes (definitions under `.claude/agents/`); grounding briefs run Sonnet 5 at `high`.
Astra at `max` is unverified; probe once before relying on it.

- Both assigned reviewers independently review the same artifact revision. Both
  passing reviews are required for acceptance; one review does not replace the pair.
- Use the specified model versions and effort levels explicitly. Record the actual
  runner model identifier/version and effective effort with each dispatch/review.
  Verify the runner accepts them; report an unavailable combination rather than
  silently substituting a model, using a floating default or lowering effort.
- These assignments apply to future dispatches, including remaining review and
  correction work on an active feature. Preserve the actual provenance of completed
  work; an earlier Opus implementation must not be relabeled as Fable-authored.
- PM coordination, test execution, and roles not listed here retain their existing
  assignments. This change does not alter permissions or other quality gates.

### The PM never edits an artifact (PO direction, 2026-09-22)

The PM session coordinates and verifies; it does not produce the work. The PM must not
edit a spec, IR, design, contract, test or production code artifact, even for a one-line
fix, fold or draft. Every such change goes to a seat under the assignments above. The
PM's own writes are limited to state, session logs, handoffs, packets, reviews of
evidence, plans and the wiki. The PM still re-runs a seat's claimed commands, reads the
diff, commits, deploys and records results; those are verification, not authoring.
Reason: once the coordinator edits, it stops verifying, and family separation breaks
because the same model then sits on both sides of the gate. Reviewers caught the PM's
own folds on two consecutive rounds of the identity-conflict-authority spec (2026-08-27
to 28); that is the failure this rule prevents.

## Rust Quality Gate

- `cargo fmt --all -- --check` must show zero diffs
- `cargo clippy --workspace --all-targets` must show zero warnings
- `cargo test` must show zero failures
- Run all three before claiming any phase complete

## Ecosystem Crates — Never Hand-Roll

| Capability | Use | Never |
|-----------|-----|-------|
| Hashing | `argon2`, `sha1`, `sha2`, `blake3` | Custom hash implementations |
| Encoding | `data-encoding` (hex/base32/base64) | Custom encode/decode |
| Bencode | `bendy` | Hand-rolled bencode parsers |
| Serialization | `serde` + format crates | Custom format parsers |
| HTTP | `reqwest` + `rustls` | Raw TCP or system OpenSSL |
| Random | `getrandom`, `rand` | Custom RNG |
| Constant-time | `subtle` | Manual timing-safe comparison |
| XML | `quick-xml` | Hand-rolled XML parsers |

## Audit Rules (from v2.1 retro)

- Auditor does NOT validate commit hash match in deploy log (it's a human reference)
- Auditor ignores `build/plans/build-state-*.json`, `build/reviews/audits/`, `build/LATEST_AUDIT.md` in git state checks — these are process bookkeeping, not source code
- Audit hook timeout: 600s (10 minutes)

## Test Generation Rules (from v2.1 retro)

- Skip implementation test generation for modules that only exist as inline test harness code (no importable library types)
- `gen_test.py` writes output to `build/generated/`, not `/tmp/` — Gemini reviewer is workspace-sandboxed
- Test what needs testing. Don't generate tests for test infrastructure.

## Code Stage Gate (from playback-enhancements process exception, 2026-05-24)

- **CC must not advance past Step 4a (behavioral tests) in Code stage without tests compiling and failing (red).** Implementation without red tests is a stage violation. The test suite must be in place and verified red before any Step 4b implementation work begins. Writing implementation first and tests second inverts TDD and defeats the purpose of the test-driven gate.
- **Tests drive the real door (from alpha-hardening retro, 2026-07-23).** Every test — red-first or added later — exercises the production entry path for the behavior it claims to cover: the real route + middleware via the real router, the real `SqliteDb` writer, the real adapter/client seam, the real component. Never an injected outcome, a toy router, or hand-built state the production path could not produce. A green suite over injected results proves nothing: 9 alpha-hardening units shipped green that way and the audit returned 21 findings, most of that class. Any test whose entry point is constructed state needs an explicit justification in its packet.
- **Pin the named observable, not the whole state — and when a fix "needs" an unrelated subsystem, suspect the test first (from identity-review-fixes retro, 2026-08-26).** A test that asserts a whole state snapshot silently makes every neighbouring behaviour part of the contract. Two U5 cases did this, and satisfying them bent machinery the fix had no business touching (an import collision heuristic, an attach shortcut); re-pinning to the acceptance criterion's exact observables made those edits unnecessary. The same shape appeared independently on the other side of that cycle's model contest — three production surfaces added to keep one snapshot test green — so it is a property of over-pinning, not of one builder. Assert the observables the spec names; leave the rest unpinned.

## Process Discipline (from v2.1 retro)

- **Do not add to scope without explicit user approval (permanent PO direction, 2026-09-20).** Implement only the agreed outcomes. New features, adjacent fixes, broader refactors, recovery systems and extra deliverables require explicit approval before adding them to the active plan or dispatching design, tests or code for them. Record suggestions in the project to-do list as unapproved; a reviewer recommendation, PM decision, passing review, or instruction to "make decisions as needed" is not approval to expand scope. Routine implementation choices within the agreed scope remain autonomous. If an unexpected dependency would materially expand the work, explain it and obtain approval for that addition while continuing independent in-scope work. No response means no approval.
- `build/foundation/cycle-retrospective.md` is created at Phase 0 start
- CC must write to it immediately when process friction occurs (audit FAIL, hook timeout, workaround needed) — not at retro time
- Retrospective (Phase 6) updates this CLAUDE.md with critical learnings from the cycle
- **CC does not skip, defer, or downgrade approved plan items without explicit user approval.** If CC believes an approved item should be skipped or deprioritized, it must flag this to the user with reasoning and wait for a decision. Silently substituting a lesser fix (e.g., LazyLock on regexes instead of the approved quick-xml rewrite) is a process violation. This applies equally to implementation sequence items, review findings, and any other approved work. (from pre-release review, 2026-04-07)

## Architecture

- 17-crate Rust workspace. All dependency arrows point toward `livrarr-domain`.
- `livrarr-server` is the composition root — depends on everything, nothing depends on it.
- Trait-based boundaries between all modules. `SqliteDb` for production, `create_test_db()` (SQLite `:memory:`) for tests.
- `trait-variant::make(Send)` for async traits, not `async-trait` (except where `async-trait` already used in v2).
- `chrono` for datetime, not `time`. Project-wide.

## Key Decisions

- **Hardlink policy:** Copy for import (tag writing breaks links), hardlink-first for CWA
- **No env var config overrides.** TOML only. Servarr convention.
- **DEFERRED-001:** ~~Prowlarr-only → direct Torznab indexer support.~~ Resolved — indexer system accepts any Torznab/Newznab URL directly (url + api_path + api_key). Prowlarr is optional.

## Dev Workflow

- **Dev restart:** `scripts/dev-restart.sh` — kills server, builds backend+frontend, deploys UI, starts server, health checks.
- **Frontend deploy cleans old bundles** — the script removes `testdata/ui/assets/` before copying to prevent stale cache hits.

## Lessons (from usenet retro, 2026-04-05)

- **Prototype each API parameter independently.** The SABnzbd `search=<nzo_id>` assumption was wrong — it searches by name, not ID. End-to-end prototype success can mask parameter-level bugs.
- **GPT-5.4 is more thorough on invariant analysis than Gemini.** Plan for 4-5 rounds from Stream B on features touching state management.

## Lessons (from metadata audit, 2026-04-15)

- **Add a duplication check step after implementation, before audit.** AI writes code function-by-function without holding the whole system in view — it will reimplement logic that already exists elsewhere (e.g., `refresh_all` reimplementing the same pipeline as `refresh`). After implementation, have a cross-family reviewer specifically look for structurally duplicated logic. One prompt, cheap, high signal.
- **Systematic domain audits catch what tests and review miss.** A top-down read of every function in a domain (not just changed files) surfaces dead fields, unreachable branches, and missing write paths that no other process finds.

## Lessons (from foreign language retro, 2026-04-10)

- **Prototype external endpoints before writing parsers.** `curl` the URL, inspect the response, verify data is present. "200 OK" doesn't mean the response is parseable or SSR. 4 of 8 SRU/scrape providers had format issues discoverable with a 5-second test.
- **Check OpenLibrary language filter before building custom providers.** OL's `search.json?language={code}` may cover a language well enough to skip building a custom provider. Check result count and whether titles are in the native language.
- **`trait_variant::make(Send)` produces non-dyn-compatible traits.** Use enum dispatch for heterogeneous provider collections. Test dyn-compatibility with `cargo test`, not `cargo check` (cfg(test) items aren't checked by `cargo check`).

## Lessons (from metadata-refactor retro, 2026-06-09)

- **Door→road wiring is untested — trace it at design.** Behavioral tests cover the *pipeline* (`run_unified`/materialize), not the *door→pipeline wiring*. A handler can compile and pass the entire suite while routing **off** the one road — the add-from-search door did exactly that (set `skip_sync_enrichment` + spawned its own enrich/cover route), caught only by an explicit deep audit, forcing the Option-A cutover. At design, trace every entry door/handler into the canonical pipeline and confirm no skip-gate or ad-hoc spawn bypasses it. Threading a value into a struct (`candidate_id` into `WorkCandidate`) is **not** the same as the door being wired. This is the 2nd consecutive feature where a signal reached one path but not all of them (cf. metadata-modularization's cross-layer threading) — promote it from self-audit to a design-gate check.
- **Explicit System Truths pay off — keep the section content-rich.** The spec enumerating ST-001…008 (provider audio capabilities, GB quota, anti-bot facts) correlated with **0 missing-system-truth findings** this cycle — the class that recurred in prior features (raw-SQL writers, SABnzbd param, OL language filter). `verify.py` already requires the section; this cycle shows the *content* is what kills the recurring class. Enumerate real environment facts, never a pro-forma placeholder.

## Lessons (from responsiveness retro, 2026-07-12)

- **Performance work: measure before designing.** A perf REQ enters design only with a measurement/probe artifact attached. This cycle's natural experiment: the two levers measured first (keepalive tuning, bulk-refresh concurrency) settled on one cheap capture each; the one designed first (Hardcover batching) consumed a probe + design draft + a full dual-family review round before the same measurement logic cancelled it.
- **Registering a behavioral test and force-adding its file are ONE change.** `tests/` is gitignored for new files; a `[[test]]` entry in `livrarr-behavioral/Cargo.toml` whose file was never `git add -f`'d is green locally and uncompilable on every fresh clone. The manifest guard does NOT catch this direction (two files sat unshipped for a full feature cycle). Register + `git add -f` together, always.

## Communication with the user — speak in simple English (PERMANENT)
Speak to the user in simple, plain English. Confusing them is counterproductive and a
failure, no matter how correct the content. Do NOT make the user parse implementation
jargon — REQ-numbers, AC-IDs, function names, config keys, type names. Translate
every question or status into the plain-English DECISION or fact they actually need,
in their terms. Lead with the decision/answer, then a short plain why. After you
generate any response to the user, REWRITE it once more for clarity and accessibility
before sending — strip jargon, shorten, make the call obvious. A correct answer
the user cannot easily understand is a failed answer.
