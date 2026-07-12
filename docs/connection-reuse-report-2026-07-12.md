# Connection-reuse measurement (REQ-011 / U-B3 gate artifact)

**Date:** 2026-07-12 · **Instance:** dev :8789, U-B1 cache build + RUST_LOG-honoring filter · **DB snapshot first:** `testdata/livrarr.db.pre-ub3-measure-20260712`.

## Method

`init_tracing` now honors `RUST_LOG` when set (it replaces the config-derived filter wholesale; unset = exactly the old behavior — `crates/livrarr-server/src/main.rs`). Server restarted with `RUST_LOG=livrarr=debug,reqwest::connect=debug,hyper_util::client::legacy::connect=debug,hyper::client::connect=debug`, then the §3 refresh harness ran **twice back-to-back** (works 7/1/2 — Dune, Summer Knight, Jade City; refresh = `Freshness::Bypass`, so every pass makes real provider fetches by design). Windows: pass 1 16:24:21–26, pass 2 16:24:26–30 UTC. New-connection events parsed from the reqwest connect target in `/tmp/livrarr.log` (byte offset 20645437 onward); request counts from `provider_call_records` (success + not_found = real HTTP; skip records excluded). Raw JSONs: `speed-baseline-2026-07-12-refresh-pass1.json` / `-refresh.json`.

## Numbers

| Provider host | Real requests (2 passes) | New connections | Reused |
|---|---|---|---|
| www.goodreads.com | 6 | 1 | 5/6 |
| audible (api.audible.com) | 4 | 1 | 3/4 |
| api.audnex.us | 4 | 1 | 3/4 |
| openlibrary.org | 4 | 1 | 3/4 |
| api.hardcover.app | 4 | 1 | 3/4 |
| www.googleapis.com | 2 | 1 | 1/2 |
| **Total** | **24** | **6** | **18/24 (75%)** |

(The only other connect events in the window were two unrelated background pollers — seedbox SFTP host and Prowlarr — one connection each.)

## Findings

1. **Connections survive the pacing gaps.** Per host, exactly ONE handshake for the whole window — reuse held within a pass (1–3.5s inter-request pacing per bucket) and across the ~4s between passes. reqwest's default pool (90s idle timeout, ST-009) already does the job on refresh-shaped traffic.
2. **The theoretical cold-connection tax is one handshake per host per idle-gap >90s** — i.e., per burst, not per request. Immaterial next to per-request provider latency (hundreds of ms) and pacing floors.
3. Incidental confirmation of REQ-009 semantics from a second angle: pass-2 request counts equal pass-1's — user-triggered refresh (`Bypass`) really does re-fetch rather than ride the new provider cache.

## Verdict (the REQ-011 gate decision)

**SKIP keepalive/pool tuning.** Measured coldness is one handshake per host per burst — there is nothing material to recover. No tuning unit will be designed or implemented; ST-009's open question ("do real provider connections go cold?") is answered: not on representative refresh traffic.
