# Handoff: Metadata Remediation — Phase 3 (the outbound rate-limit queue)

Generated 2026-06-30 for a fresh Claude Code session to IMPLEMENT Phase 3. This
effort runs **lean** (not formal kk-build): Sonnet 5 implements, Opus reviews,
cross-family (Gemini 3.5-flash + Codex) reviews the code. The artifacts below are
the source of truth.

## Where we are
- Branch `metadata-remediation` (off main). Working dir `/mnt/opt/livrarr`.
- **Phase 0, 1, 2 are DONE and committed.** HEAD ≈ `c18752c` (Phase 2). Green:
  `cargo fmt/clippy/test` clean, **1020 tests pass**.
- **Phase 3 design is LOCKED (v4)** after THREE cross-family design-review rounds
  (v1/v2/v3 all FAIL → converged; every finding folded). PO approved "one confirm
  round then build"; design loop is closed. **Do NOT re-open the design** — build it.

## Read first (in order)
1. **`docs/metadata-remediation-phase3-queue-design.md`** — THE design (v4). Authoritative.
2. `docs/metadata-remediation-plan-2026-06-29.md` — the 6-phase plan (Phase 3 row).
3. `docs/metadata-audit-2026-06-28.md` — findings M-001, M-009, M-018.
4. Memory `project_phase3_rate_limit_queue.md` — the locked PO direction.
5. Review history (the case table of folded findings): `build/reviews/metadata-remediation-phase3/review-design-*.json` (r1=Gemini-3.1-pro, r2=Gemini-flash, r3/r4=Gemini-flash+Codex).

## What to build (the design in one breath)
Replace the scattered/partial rate limiting with **ONE process-global wait-then-send
queue** that every outbound provider call passes through, paced per provider, with
a bounded in-flight count. Nothing dropped. It fixes: M-009 (6 uncoordinated
limiter copies → one), M-001 (identity + 3 cover paths unthrottled), M-018
(convergence sweep amplifies it), and puts Audnexus/Audible onto the transport.

## Implementation checklist (every item is a folded review finding — honor each)
- [ ] **Real per-bucket priority queue**, NOT a reuse of `RateLimiterMap` (replace it). `RateLimiterMap`'s timestamp-reservation is not reorderable and leaks slots on cancel.
- [ ] **On-demand dispatcher task** per bucket: a `dispatcher_running` bool in the per-bucket `Arc<Mutex<state>>`; enqueue spawns the dispatcher on the CURRENT runtime if not running; dispatcher exits (clears the flag) when it drains empty. **Not a static OnceLock task** (that hangs under `cargo test`'s multi-runtime model). Queue STATE is process-global/shared; the TASK is on-demand.
- [ ] **Dispatcher loop order:** drain (exit when empty, no coalescing) → sleep for pacing (interval since last ACTUAL dispatch) → `semaphore.acquire_owned()` → pop top by `(RequestPriority desc, enqueue_sequence asc)` → hand caller a turn-signal **carrying the `OwnedSemaphorePermit`** (RAII: released on completion OR caller-cancel). Pop at dispatch moment, not before.
- [ ] **Keep the per-provider `Semaphore`** (in-flight cap) — separate control from pacing. Transfer as `OwnedSemaphorePermit`.
- [ ] **Reuse domain `RequestPriority`** (`livrarr-domain/src/lib.rs:1262`, `Low<Normal<High<Interactive`, derives Ord). Add a monotonic `enqueue_sequence`. Default priority `Normal` at every call site now ⇒ pure FIFO. Do NOT invent a `QueueClass`.
- [ ] **Route EVERY provider path through the shared `HttpFetcher`:** GR/HC/OL/GB/Audnexus/Audible clients, the identity fan-out (`english_identity_resolver`), the 3 cover paths (`cover_alternatives`, `preadd_cover_service`, `cover_service`), AND the admin connection-test endpoints `test_hardcover`/`test_audnexus` (`crates/livrarr-handlers/src/config.rs:266-317`, currently raw `http_client()`).
- [ ] **Collapse the 6 `HttpFetcherImpl::new()` sites → one shared instance** (`main.rs:172,553,608,623,640`, `cover_backfill.rs:8`). The two provider-client construction blocks (`main.rs` ~251 enrichment-queue, ~444 identity/cover) both rebuild to share it.
- [ ] **Retire the enrichment `TokenBucket` reject path.** FIRST trace consumers of `WillRetry{RateLimit}` (`provider_queue.rs:549-555`) — confirm none depend on a reject vs a slow success — THEN remove. Pacing now comes from the fetcher queue.
- [ ] **Audnexus 304 cache** stays client-side, ABOVE the queued send (only the GET enqueues). Verify `FetchResponse` exposes status + headers for the 304/Last-Modified round-trip.

## DO NOT
- Don't reopen the design or re-litigate the approach (3 rounds done).
- Don't route the **LLM caller** through the queue — OUT of scope (PO; different provider/quota).
- Don't merge `http_client` vs `http_client_safe` (SSRF split, wiki insight 37) — the queue wraps both.
- Don't unify the per-provider intervals or change behavior beyond the routing.

## Process (lean)
- Implementer: **Agent tool, `model: "sonnet"` (= Sonnet 5)**, dense packet.
- Then Opus (you) reviews the diff; verify behavior preservation + the dispatcher protocol; run `cargo fmt/clippy/test --no-fail-fast` yourself (don't trust the agent's "clean" — a stale rust-analyzer snapshot lied once).
- Code review: `cd ~/Projects/kk-build && python3 hooks/dispatch-review.py metadata-remediation-phase3 code /mnt/opt/livrarr --prompt-file <p> --reviewers gemini,codex` — NO `--model` (config pins Gemini 3.5-flash + gpt-5.5). Gemini's MCP-hang is fixed (`--allowed-mcp-server-names none` in agents.py).
- **Commit hygiene:** new files (e.g. a new queue module) are NOT caught by `git add -u` — use `git add` on the new paths explicitly and check `git diff --cached --name-status` before committing (this bit twice). End commit messages with the Co-Authored-By trailer.
- **Sequencing:** the M-009 one-shared-fetcher collapse can land as a trivial first commit; the queue + rewire is the main reviewed unit. Snapshot the DB before any write-heavy live test.

## Open product/verify items
- Trace the `WillRetry` consumers before retiring the reject (above) — if any real consumer depends on it, surface to PO.
- Verify `FetchResponse` header surface suffices for Audnexus 304 before that rewire.
