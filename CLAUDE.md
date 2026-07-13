# Livrarr — Project Rules

## Session Start (mandatory — do these before any work)

1. **Read `wiki/insights.md`** — active learnings that prevent avoidable mistakes. Do this first.
2. **Use Serena MCP for ALL code navigation** — symbol lookup, dependency walking, reference tracking. Do NOT grep source files or read entire files when Serena can answer the question. Always try Serena first.
3. **Check `wiki/` before re-deriving domain knowledge** — read `wiki/index.md` to find relevant pages before reasoning from scratch about how a subsystem works.
4. **Update `wiki/` when you learn something new** — at block boundaries or when you discover domain knowledge a future session would need, update or create wiki pages per `/pk-wiki`. If you don't write it, the next session re-derives it.

## Principles

See `ARCHITECTURE.md` Part 1 (Product Principles) and `PRINCIPLES.md` (universal engineering principles). Highest authority in the project. Override everything below when conflicts arise. (Formerly split across `docs/principles.md` and gitignored `build/foundation/principles.md`; consolidated into the tracked root docs 2026-07-05 — see `docs/architecture-review-2026-07-04.md` AR-01.)

## Build Cycle (pk-auto-build)

- **Phase order is non-negotiable:** Spec → IR → Behavioral Tests → Code → Implementation Tests → Deploy → Retro
- **Audit before commit, never after.** No phase-gate commit without a passing audit.
- **Cross-family separation:** Anthropic writes code, OpenAI writes tests, Google reviews and audits. No model reviews its own family's output.

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

## Process Discipline (from v2.1 retro)

- `build/foundation/cycle-retrospective.md` is created at Phase 0 start
- CC must write to it immediately when process friction occurs (audit FAIL, hook timeout, workaround needed) — not at retro time
- Retrospective (Phase 6) updates this CLAUDE.md with critical learnings from the cycle
- **CC does not skip, defer, or downgrade approved plan items without explicit user approval.** If CC believes an approved item should be skipped or deprioritized, it must flag this to the user with reasoning and wait for a decision. Silently substituting a lesser fix (e.g., LazyLock on regexes instead of the approved quick-xml rewrite) is a process violation. This applies equally to implementation sequence items, review findings, and any other approved work. (from pre-release review, 2026-04-07)

## Architecture

- 10-crate Rust workspace. All dependency arrows point toward `livrarr-domain`.
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
