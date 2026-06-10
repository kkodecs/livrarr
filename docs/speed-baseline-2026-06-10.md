# Speed Baseline — 2026-06-10

Captured on main @ `d1b8768`, dev server, 144-work library (131 en / 6 fr / 4 pl / 3 es).
Harness: `scripts/speed-baseline.py` (local tooling, untracked). Raw data:
`build/reports/speed-baseline-2026-06-10-*.json` (local, untracked).

## Numbers

| Operation | Wall time | Notes |
|---|---|---|
| Search (lookup) | 1.4–3.1 s | 10–22 candidates; all providers queried in parallel, 8 s/leg timeout |
| Add a work (anchored) | 1.8–3.6 s POST | Enrichment runs synchronously inside the POST; lands enriched+confirmed |
| Add, user-perceived | ~4–7 s | lookup + add |
| Refresh one work | 2.0–2.4 s | Synchronous |
| Bulk refresh (144 works) | ~8 min | ~3.3 s/work, perfectly linear; no degradation; #135 flag did not stick |
| Manual-import scan | ~1.3 s/row | 87 rows/109 s and 90 rows/112 s; warm cache changed nothing |
| Provider RTT (direct) | GB 340 ms · HC 310 ms · Audnexus 30 ms | medians of 3; OL/GR not probed raw (app-only policy) |

## Structural findings

1. **Lookup is parallel; enrichment is not.** `work_service.rs` joins GB/OL/HC/GR
   concurrently (WCC chunk A), but `livrarr-enrichment` contains zero concurrency
   primitives — the scatter fetches providers sequentially. Per-work wall ≈ sum of
   provider RTTs + pacing + LLM validation (~2.2 s) instead of the max leg (~1 s).
2. **Bulk = serial × serial.** One work at a time × one provider at a time — hence the
   perfect linearity. At alpha-7 scale (1,000 works) this is ~55 minutes per bulk refresh.
3. **The scan path appears not to consult the 24 h metadata cache** (warm ≈ cold across
   two runs). Verify before any cache tuning; pacing dominates regardless.
4. **Per-provider instrumentation is absent.** #131 (provider health writes nothing) and
   the file log has been dead since April while the status page still claims it. Per-leg
   timers partially exist for GB/OL in lookup but report into the void.

## Incident (recorded for Sprint B)

The bulk-refresh capture ran the 13 foreign works through the unguarded refresh merge
(audit F1/#133) and wrote wrong-book / wrong-language values onto ~8 of them (e.g.
`series_name: "Bridgertons"` on Pan Tadeusz; GB+GR both contributed, field-level
provenance confirms). Fully reverted from `livrarr.db.pre-migrate-20260610-134634`
(verified byte-identical); damaged state preserved at
`testdata/livrarr.db.f1-damaged-20260610-183104` as forensic evidence. Process rule
adopted: snapshot before any write-heavy run; check the open critical-bug list against
the code path first.

## Decisions (PO, 2026-06-10)

- **`v0.1.0-alpha6` is gated on Sprint B (correctness core) + the enrichment-scatter
  parallelization.** The cut does not ride Sprint A.
- Parallelization is the **principled** cut: identity phase resolves all anchors
  (incl. ASIN — pairs with #144) first; then one join across providers. Pragmatic
  chained-pair variant rejected.
- Immediate next: instrumentation (#131 + logging) as Sprint B's opener, so E's
  before/after is measured against this baseline.

## Projected effect of the gating work

| | Today | Parallel scatter | + pipelined bulk (post-a6) |
|---|---|---|---|
| Refresh one work | ~2.2 s | ~1 s | — |
| Add (perceived) | 4–7 s | ~3–4 s | — |
| Bulk, 144 works | ~8 min | ~2.5 min | < 1 min, pacing-bound |
