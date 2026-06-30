# Phase 3 Design — One Outbound Queue (the rate-limit floor)

Status: DESIGN LOCKED v4 (2026-06-30). Branch `metadata-remediation`. Findings
M-001, M-009, M-018. PO LOCKED: queue first (the floor — anti-ban, respectful to
providers); wait-then-send, nothing dropped; LIFO/FIFO priority is a LATER phase
but the SEAM must exist now. Three design-review rounds (v1/v2/v3 FAIL →
converged); all findings folded; PO chose "one confirm round then build" — the
confirm round (v3) surfaced 2 small impl-detail P1s, folded into v4; design loop
stops here, build proceeds.

## Goal
Every outbound call to a metadata/book provider passes through ONE process-global
wait-then-send queue, paced per provider, with a bounded number of in-flight
calls. No outbound provider path bypasses it.

## Review history
- **v1 (reuse `RateLimiterMap`) — FAILED** (2× Gemini + Codex): not a reorderable
  queue, cancellation leaks a slot, pacing≠concurrency (don't drop the Semaphore),
  LLM caller omitted.
- **v2 (real queue) — FAILED** (Gemini-flash + Codex): dispatcher mechanics
  underspecified. Seven findings, all valid, folded.
- **v3 (precise protocol) — FAILED** (Gemini-flash + Codex), 1 finding each, both
  P1 impl-detail (multi-runtime dispatcher lifecycle; FIFO tie-breaker). Folded
  into v4. Convergence floor reached; design loop closed per PO.

## The design (v3)

### A real per-bucket queue + one dispatcher task per bucket
Replace `RateLimiterMap` with, per `RateBucket`, a priority queue serviced by a
dispatcher task.

**Process-global STATE, on-demand TASK (R-007 + R-009 reconciled):** the queue
STATE (per-bucket `Arc<Mutex<{ heap, dispatcher_running: bool }>>`) is
process-global — shared by every `HttpFetcherImpl` (OnceLock/shared Arc), so the
M-009 collapse holds and a second fetcher reaches the SAME queues. But the
dispatcher TASK is spawned **on demand**, NOT as a static long-lived task:
- On enqueue: lock state; if `dispatcher_running` is false, set it true and
  `tokio::spawn` the dispatcher **on the current runtime**.
- Dispatcher loop: when it drains the queue empty, set `dispatcher_running = false`
  and EXIT.
This fixes the multi-runtime test hang (R-009): a static task spawned on test 1's
runtime would die with that runtime and later tests would hang on an initialized-
but-taskless OnceLock. On-demand spawn always runs on the live runtime and
self-cleans when idle — no leaked tasks, no `CancellationToken` plumbing needed.

**Dispatcher loop protocol (precise — folds Gemini R-005/R-006 + Codex R-005):**
1. If the queue is empty → set `dispatcher_running = false` and EXIT (the next
   enqueue respawns it). Drain fully before exiting — never leave items waiting
   (R-005: no coalescing).
2. Sleep until the bucket's next allowed send time (interval since the last
   ACTUAL dispatch).
3. Acquire an **owned** in-flight permit: `semaphore.acquire_owned().await`.
4. **Only now** pop the top item, ordered by `(RequestPriority desc,
   enqueue_sequence asc)` — highest priority, FIFO within a priority via a
   monotonic enqueue sequence (Codex R-010; without the sequence a same-priority
   heap reorders and the "wait in line" floor is violated). Pop at the dispatch
   moment so a just-arrived higher-priority request can win (R-006); don't hold a
   popped item across the wait.
5. Advance the last-dispatch clock; hand the caller a "your turn" signal **that
   carries the `OwnedSemaphorePermit`** (Codex R-005). The caller holds the permit
   across the HTTP send + body read; RAII drop releases it on completion OR on
   caller cancellation (no leaked permit, no leaked slot).

**Cancellation-safe by construction:** the clock only advances on an actual
dispatch, and the permit is an RAII guard — a caller dropped while queued is
skipped; a caller dropped after dispatch releases its permit on drop.

**Shutdown (R-009):** handled by the on-demand model above — the task self-exits
when the queue drains, so nothing leaks across test runtimes or at shutdown.

Per-bucket intervals carry over (OL/GR/HC/GB 1s, Audnexus 2s, Audible 150ms,
Indexer 500ms; fetcher.rs:70-81). Wait is unbounded, nothing dropped (PO floor).

### The priority seam — REUSE `RequestPriority` (R-008)
The domain already has `RequestPriority` (`livrarr-domain/src/lib.rs:1262`,
derives `Ord`, variants `Low < Normal < High < Interactive`, doc-commented "used
for queue ordering"). Thread it + a monotonic `enqueue_sequence` on the outbound
request, priority defaulted to `Normal` at every call site THIS phase (⇒ pure FIFO
by sequence, behavior unchanged). The dispatcher orders by `(priority,
enqueue_sequence)`. The later tuning phase only changes the per-caller default
(user-facing → `Interactive`/`High`, background/convergence → `Low`) + the pop
comparator — NO call-site migration, because both fields already thread through.
Do NOT invent a new `QueueClass`.

### Keep the in-flight concurrency cap
Retain the per-provider `Semaphore` as a SEPARATE control from pacing (pacing =
start spacing; semaphore = in-flight cap). Transferred to the caller as an
`OwnedSemaphorePermit` per the loop above.

### Move 2 — route EVERY provider path through the queue
Each provider client (GR/HC/OL/GB/Audnexus/Audible) issues HTTP via the shared
`HttpFetcher`. Brings the unthrottled identity fan-out, 3 cover paths, and
Audnexus/Audible raw clients onto the queue (M-001/M-018); collapses the 6
`HttpFetcherImpl::new()` sites to one shared instance (M-009). **Plus the
admin connection-test routes** `test_hardcover` / `test_audnexus`
(`config.rs:266-317`, currently raw `http_client()`) — route them through the
fetcher too (Codex R-006: operator-triggered bypass otherwise).

### Explicitly OUT of scope (PO 2026-06-30)
- **LLM caller** (`llm_caller_service.rs`) — user's AI provider, own quota, not a
  book-site ban risk. Stays on its own client.
- LIFO/FIFO ordering POLICY (the field is wired; the policy is the next phase).
- Per-user fairness, adaptive 429 backoff, queue metrics.

## Open items / risks (resolve at impl)
1. **Reject→wait contract change.** Retiring the enrichment `TokenBucket` removes
   its `WillRetry{RateLimit}` reject (provider_queue.rs:549-555). Trace consumers;
   confirm none depend on a reject vs a slow success before removal.
2. **Audnexus 304 cache** stays client-side, ABOVE the queued send (only the GET
   enqueues); `FetchResponse` exposes headers, so 304/Last-Modified is plumbable.
3. **Build cost is real** — new queue infra + per-bucket dispatcher tasks + permit
   handoff + rewiring every provider client. Largest, highest-risk unit of the
   remediation.
4. **SSRF split preserved** (insight 37): the queue wraps both `http_client` and
   `http_client_safe`; does not merge them.
5. **Two provider-client construction sets** (main.rs ~251, ~444) both rebuild to
   share the queue.

## Process
v3 → (PO decides: one more design-review to confirm the protocol, OR proceed to
implement with this as the spec). On build: implement (Sonnet 5) → Opus review →
code review (Gemini 3.5-flash + Codex). M-009 one-shared-fetcher collapse may land
as a trivial first commit; the queue + rewire is the main reviewed unit.
