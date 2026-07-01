# Handoff: Metadata Remediation — Phase 3 WIRING (fresh session)

Written 2026-07-01 to hand off from a long, degraded session. Packet 1 (the queue
ENGINE) is committed and green; the remaining work is WIRING the engine in. The
prior session made a real mapping error (below) and burned time misusing the
reviewer CLIs — both are diagnosed here so you don't repeat them.

Working dir `/mnt/opt/livrarr`, branch `metadata-remediation`. IGNORE
`/mnt/opt/scryer/livrarr` — stale duplicate, never read/edit it.

## Session-start (do these first)
1. Read `wiki/insights.md`. Use Serena MCP for code nav.
2. Read the two locked artifacts: `docs/metadata-remediation-phase3-queue-design.md`
   (LOCKED v4 design) and `docs/handoff-metadata-remediation-phase3.md` (the build
   handoff / checklist). These are authoritative; do NOT reopen the design.
3. **Before dispatching ANY reviewer, read the reviewer-dispatch notes** (this is what
   broke last session): memory `feedback_gemini_cli_auth`, `reference_gemini_dispatch_mcp_hang`,
   `reference_dispatch_review`, `feedback_dispatch_budget`.

## Where we are
- **Packet 1 — the queue ENGINE — is DONE and COMMITTED: `c1f0aab`.**
  `crates/livrarr-http/src/outbound_queue.rs`. Process-global per-`RateBucket`
  wait-then-send queue: on-demand dispatcher, RAII permit, (priority desc,
  enqueue_sequence asc) ordering, per-bucket in-flight cap (2), `RateBucket::None`
  bypasses pacing+cap. Green: `cargo test --workspace` = 1030 passed / 0 failed;
  fmt/clippy clean. Reviewed by all 3 families; 2 concurrency bugs caught+fixed
  (dispatcher-exit lost-wakeup race; cancellation pacing TOCTOU).
- **The engine is INERT** — nothing calls it yet. `do_fetch` still uses the old
  per-instance `RateLimiterMap`.

## ⚠ CRITICAL correction to the wiring map (confirmed against source)
The prior session mapped "which providers are on the fetcher" by enumerating
`FetchRequest` constructions and concluded GR/HC/OL/GB are on the fetcher. That is
only HALF true. There are TWO outbound paths per provider:
- **Lookup/search** (search a provider by title/ISBN): builds `FetchRequest` in
  `work_service.rs` / `series_query_service.rs` / `author_service.rs` etc. → goes
  through `HttpFetcher::do_fetch`. ✓ on the fetcher.
- **Enrichment/detail** (fetch a book's details by known ID/anchor): goes through
  **`ProviderClient`** (`crates/livrarr-external-data/src/provider_client.rs`), whose
  per-provider `*Client` structs use a **raw `livrarr_http::HttpClient`, NOT the
  fetcher** — CONFIRMED at `provider_client.rs:23` (`use livrarr_http::HttpClient;`).
  Paced today by the enrichment `TokenBucket` in
  `crates/livrarr-enrichment/src/provider_queue.rs`, not by the fetcher.

**Implication:** flipping `do_fetch` onto the queue paces LOOKUPS only. The
high-volume **enrichment scatter bypasses the fetcher** and is a separate, larger
routing job. And retiring the enrichment `TokenBucket` BEFORE the enrichment path is
on the queue would leave enrichment UNPACED — a sequencing trap.

**Do NOT trust the prior door-map's completeness.** Re-run a CLEAN cross-family
trace-check (correct reviewer flags — see below) to produce a trustworthy map before
committing to the wiring sequence.

## Reshaped wiring plan (re-validate with a clean trace-check first)
- **Step A — flip `do_fetch` onto the queue.** Replace `self.rate_limiters.acquire(
  &req.rate_bucket)` in `crates/livrarr-http/src/fetcher.rs` (~line 165, the single
  chokepoint both `fetch`/`fetch_ssrf_safe` funnel through) with
  `outbound_queue::shared().acquire(req.rate_bucket, req.priority)`. Add a
  `priority: RequestPriority` field to `FetchRequest`
  (`crates/livrarr-domain/src/services/http.rs:31`; `RequestPriority` is at
  `crates/livrarr-domain/src/lib.rs:1261`), default `Normal` at ~30 construction
  sites (the seam). Delete `RateLimiterMap` + fetcher's duplicate `interval_for`.
  VERIFY nothing reads `RateLimiterMap` except `do_fetch` before deleting (Codex's
  flag). Paces LOOKUPS + downloads(None-bypass) unchanged.
- **Step B (the big one) — route the bypassing paths onto the fetcher/queue:** the
  **enrichment `ProviderClient` per-provider clients** (GR/HC/OL/GB/Audnexus/Audible),
  Audnexus/Audible raw reqwest, the identity fan-out (`english_identity_resolver`,
  livrarr-identity), and the cover paths incl. `crates/livrarr-handlers/src/coverproxy.rs`.
- **Step C — retire the enrichment `TokenBucket`** (`provider_queue.rs`) — ONLY after
  B. Trace `WillRetry{RateLimit}` consumers first (design's open item).
- **Cover pacing decision (PO call):** covers use `RateBucket::None` = unpaced, but
  OL cover API is rate-limited (100/IP/5min); both reviewers say give covers a real
  bucket. Decide before/with Step B.
- **Optional cleanup:** collapse ~12 `HttpFetcherImpl::new()` instances → 1. Now
  cosmetic (global queue coordinates pacing), NOT a fix.

**Out of scope (do not route through the queue):** the LLM caller
(`llm_caller_service.rs`, `title_cleanup.rs`) — different provider/quota, design
excludes it; Readarr client + download-client pollers (qBit/SAB/Transmission) —
user's own infra, no ban risk (trusted-infra pattern, insight 37).

## ⚠ REVIEWER DISPATCH — use the correct flags (this broke last session)
Prefer `~/Projects/kk-build/hooks/dispatch-review.py` (bakes in pinned models +
mcp-off + sandbox). If invoking CLIs manually, the flags are MANDATORY:
- **gemini:** `--allowed-mcp-server-names none --model gemini-3.5-flash` (+ `-y
  --output-format json`). Without mcp-off it hangs/spawns on MCP-init.
- **codex:** `codex exec - --json -m gpt-5.5 --sandbox danger-full-access` with the
  prompt on a real file fd (`< file`). **Without `danger-full-access`, codex's
  bubblewrap sandbox fails (`bwrap: loopback`) and it CANNOT read any files** — it
  reviews blind. Last session ran codex blind all session because of this.
- Codex can't read files unless danger-full-access works → either use it OR inline
  all code/diff in the prompt.
- Gemini can hit **Vertex 429 (RESOURCE_EXHAUSTED)** after many calls this account —
  it retries; space calls out. Don't launch both CLIs concurrently right on the heels
  of a heavy `cargo` run (a possible OOM spike killed both once, unconfirmed).

## Process for wiring (lighter than the queue's test-first-RED)
Routing, not a novel algorithm → the up-front cross-family TRACE-CHECK does the heavy
lifting (catch a bypassing door on paper), not a full red design round. Then: build
(Sonnet 5 implementer, `model: "sonnet"`) → Opus review + full gate
(`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace --no-fail-fast` — keep 1030 green as regression guard, add
door-routing tests) → cross-family CODE review (correct flags!) → PO commit. Commit
only when the PO says; new files need explicit `git add`.

## Key anchors (verified this session)
- Queue (committed): `crates/livrarr-http/src/outbound_queue.rs` — `shared()`,
  `OutboundQueue::acquire(bucket, priority)`, `OUTBOUND_IN_FLIGHT_CAP`.
- Chokepoint: `crates/livrarr-http/src/fetcher.rs` `do_fetch` (`rate_limiters.acquire`).
- `FetchRequest`: `crates/livrarr-domain/src/services/http.rs:31` (9 fields, no priority yet).
- `RequestPriority`: `crates/livrarr-domain/src/lib.rs:1261` (Low<Normal<High<Interactive, derives Ord).
- Enrichment bypass: `crates/livrarr-external-data/src/provider_client.rs` (`HttpClient`).
- Enrichment TokenBucket: `crates/livrarr-enrichment/src/provider_queue.rs`.
