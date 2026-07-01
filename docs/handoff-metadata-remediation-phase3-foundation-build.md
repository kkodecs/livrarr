# Handoff: Metadata Remediation — Phase 3 Foundation (BUILD)

Generated 2026-07-01. Fresh CC session to START THE BUILD of the outbound-queue
consolidation + circuit-breaker migration. Working dir `/mnt/opt/livrarr`, branch
`metadata-remediation`. IGNORE `/mnt/opt/scryer/livrarr` (stale duplicate — never
read/edit it).

## Session-start — do these FIRST (mandatory, before any work)
1. **Read `build/foundation/principles.md`** — the 15 principles, HIGHEST authority
   (override spec/IR/tests on conflict). P3 (respectful pacing), P6/P15 (interactive
   never blocks / RPi4-fast), P10 (failure isolation), P14 (simplicity) are load-bearing.
2. **Read the architecture** — `wiki/architecture/overview.md` (crate dependency graph).
   Key: all arrows point toward `livrarr-domain`; `livrarr-http → domain` only, so the
   breaker types land in `livrarr-http`, NOT domain (http can't depend up into
   enrichment).
3. **Read the wiki** — `wiki/insights.md` first, then `wiki/domain/metadata-principles.md`
   (M1-M10; M2 same-treatment, M3 covers-matter, M9 converge-never-dead-end) and the
   relevant `wiki/integrations/*.md` (GR/OL/HC/Audnexus/GB rate facts). Check
   `wiki/index.md` before re-deriving any subsystem.
4. Use Serena / LSP / code-index for code nav — NOT raw `grep`/`find` (denied in this
   sandbox).

## THE plan (authoritative — do NOT re-open the design)
`docs/metadata-remediation-phase3-foundation-plan.md` (**rev 5, FINAL**). Plan review is
CLOSED after **5 cross-family rounds** (Gemini + Codex); all 14 findings folded. Read it
in full — it IS the build spec. Build it; don't re-litigate the approach.

## Where we are
- **Step A DONE + committed (`af50cd5`):** the fetcher's search/lookup path now routes
  through the process-global `outbound_queue`; the old per-instance `RateLimiterMap` is
  deleted; the `priority` seam is added. Green: `cargo build/fmt/clippy` clean, **1030
  tests pass**.
- **The queue ENGINE is committed (`c1f0aab`):**
  `crates/livrarr-http/src/outbound_queue.rs` — pacing + in-flight cap + priority. It has
  NO circuit breaker and NO failure-feedback yet (those are B2).
- **Two live limiters remain:** the outbound queue (search path) and the enrichment queue
  (`livrarr-enrichment/src/provider_queue.rs`, which houses the circuit breaker
  `BreakerState` + `CircuitState`/`CircuitBreakerConfig`). Consolidating them is this
  build.

## The build (from the plan — sequenced; each stage its own reviewed unit)
- **B0 — Goodreads anti-ban (tiny, ships first):** GR interval 1s→1500ms; remove the GR
  ISBN `/search` tier (robots.txt violation). Plan §B0.
- **B1 — Provider transport conversion, HARDCOVER FIRST as the template:** convert each
  provider `*Client` from raw `HttpClient` to `HttpFetcher`; review Hardcover, then
  replicate GR/OL/GB/Audnexus/Audible. Plan §B1.
- **B2 — Circuit breaker at the queue:** relocate `CircuitState`/`CircuitBreakerConfig` to
  `livrarr-http`; move `BreakerState` (REUSE, don't reinvent); structured
  `report_outcome(bucket, Outcome)` fed by the PROVIDER CLIENTS; `FetchError::CircuitOpen`;
  per-bucket config (GR ~1h cool-off + immediate-trip; GB ~Pacific-midnight on quota);
  book-provider buckets only. Plan §B2.
- **B3 — Covers:** OL-cover-specific ~3s pacing (other cover hosts stay fast); cover-proxy
  through the fetcher at Interactive priority; cover ISBN helpers through the fetcher.
  Plan §B3.
- **B4 — Priorities:** interactive Add / cover-proxy → Interactive/High; background/bulk →
  Low.
- **C — Retire the enrichment queue's transport duties** (pacing + breaker; KEEP
  applicability/suppression/retry). Trace `WillRetry{RateLimit}` consumers first.

## Process (lean — NOT full kk-build)
- Implement with a **Sonnet 5** agent (`model: "sonnet"`, dense packet). Then **Opus
  reviews the diff + runs the full gate YOURSELF** — `cargo build`, `cargo fmt --all --
  check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace
  --no-fail-fast` (keep 1030 green + add door-routing tests, insight 46). **Do NOT trust
  the agent's "clean" claim — verify** (an agent reported clean while the workspace didn't
  compile this cycle).
- Cross-family CODE review each stage: `cd ~/Projects/kk-build && python3
  hooks/dispatch-review.py metadata-remediation-phase3-foundation code /mnt/opt/livrarr
  --prompt-file <p> --reviewers gemini,codex` (NO `--model`; models pinned in config.yaml).
- **GEMINI REVIEW GOTCHA (verified this session):** gemini times out at the 600s wall on
  file-heavy prompts — it's the file-READ loop, not the old MCP hang (that fix is already
  applied + working). **INLINE the diff + the governing docs into the review prompt and
  tell gemini NOT to open files** → single pass, fast. See memory
  `reference_gemini_dispatch_mcp_hang` and kk-build `wiki/tooling/cli-agent-dispatch.md`.
  Codex reads files fine — pair inlined-gemini with file-reading-codex.
- **Commit only when the PO says.** New files (a new module) need explicit `git add`;
  check `git diff --cached --name-status` before committing. End commit messages with the
  Co-Authored-By trailer.

## Key open items / watch-outs (from the review rounds)
- **GR 1.5s is a PO ACCEPTED-RISK** — still ~3-5× over GR's polite floor, backstopped by
  the 1h breaker + immediate-trip-on-anti-bot. Do NOT "fix" it to 5-7s without PO say-so.
- **R-11 (important):** a `CircuitOpen`/suppressed outcome must NOT consume a background
  task's enrichment retry budget — it's a PAUSED state resumed when the breaker closes;
  else convergence tasks dead-end terminally during an outage (M9 violation). Get this
  right in B2/C.
- **R-8/R-12:** the breaker's failure signal comes from the PROVIDER CLIENT (which parses
  the response), not `do_fetch` — a 200-OK DataDome challenge / GraphQL-error body must
  count as a failure. `report_outcome` is structured (Success / Failure / TripImmediately
  + optional custom cool-off).
- **B0 `/search` removal:** confirm GR autocomplete can't serve the ISBN case, or just
  drop the tier (HC/OL/GB already resolve ISBN).
- Two provider-client construction sets in `main.rs` (~251 enrichment queue, ~444
  identity/cover) both rebuild to take the fetcher.

## Deferred (NOT this feature)
Per-provider interval tuning (except GR 1.5s); GB daily-quota caching; OL CoverID caching
(the cover fast-follow); adaptive Retry-After; GR's exact 1h value; LIFO/FIFO policy; the
LLM caller; Readarr / download pollers / indexer search.

## Reference artifacts (read as needed — do not re-derive)
- Plan (build spec): `docs/metadata-remediation-phase3-foundation-plan.md` (rev 5).
- Review history (14 findings, both families):
  `build/reviews/metadata-remediation-phase3-foundation/review-design-{google,openai}-r*.json`.
- Locked queue-engine design: `docs/metadata-remediation-phase3-queue-design.md` (v4).
- Prior wiring handoff (superseded on scope by the plan):
  `docs/handoff-metadata-remediation-phase3-wiring.md`.

## DO NOT
- Re-open or re-litigate the design (5 review rounds done; it's closed).
- Skip the session-start reads (principles / architecture / wiki) — they carry the
  load-bearing constraints this build must honor.
- Put the breaker types in `livrarr-domain` (they go in `livrarr-http` — C-R11).
- Trust an implementer agent's "gate clean" without running the gate yourself.

## Next move
Start **B0** (Plan §B0): in `crates/livrarr-http/src/outbound_queue.rs::interval_for`,
change the `Goodreads` arm from `Duration::from_secs(1)` to `Duration::from_millis(1500)`;
and remove the GR ISBN `/search` tier in `resolve_detail_url`
(`provider_client.rs:1236`) so the GR path uses only gr_key-direct + title/author
autocomplete. Run the full gate (must stay 1030-green), then proceed to B1 (Hardcover
first). The design is FINAL — build it.
