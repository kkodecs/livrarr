# Hardcover batch-retrieval probe (U-B2 entry gate, REQ-010)

> **STATUS: REQ-010 CANCELLED by PO 2026-07-12** — this probe is a historical record, not an open gate. See spec-responsiveness.md REQ-010 for the decision and reason.

Recorded: 2026-07-11T23:20:55.142744+00:00  ·  endpoint: https://api.hardcover.app/v1/graphql  ·  token: admin-configured (redacted)
Probe ISBNs (from the live library): 9780330518918, 9782266111201, 9780446502306, 9780446608978, 9781022709751

| Probe | Mechanism | HTTP | Latency | Req bytes | Resp bytes | Verdict |
|---|---|---|---|---|---|---|
| P1_single_eq | one _eq lookup (baseline the app already does per-work) | 200 | 302ms | 190 | 136 | OK |
| P2_aliasing | 5 aliased _eq lookups in ONE request (ST-010: documented GraphQL feature) | 200 | 265ms | 667 | 162 | OK |
| P3_in_filter | one editions(_in: [5 isbns]) query (ST-010: Hasura-convention, previously UNCONFIRMED) | 200 | 249ms | 269 | 136 | OK |

## P1_single_eq

- Mechanism: one _eq lookup (baseline the app already does per-work)
- Summary: `{"verdict": "OK", "row_counts": {"editions": 1}}`
- Response `data` sample (truncated): `{"editions": [{"isbn_13": "9780330518918", "book_id": 381445, "language": {"language": "English"}, "book": {"title": "Pandora's Star"}}]}`

## P2_aliasing

- Mechanism: 5 aliased _eq lookups in ONE request (ST-010: documented GraphQL feature)
- Summary: `{"verdict": "OK", "row_counts": {"e0": 1, "e1": 0, "e2": 0, "e3": 0, "e4": 0}}`
- Response `data` sample (truncated): `{"e0": [{"isbn_13": "9780330518918", "book_id": 381445, "language": {"language": "English"}, "book": {"title": "Pandora's Star"}}], "e1": [], "e2": [], "e3": [], "e4": []}`

## P3_in_filter

- Mechanism: one editions(_in: [5 isbns]) query (ST-010: Hasura-convention, previously UNCONFIRMED)
- Summary: `{"verdict": "OK", "row_counts": {"editions": 1}}`
- Response `data` sample (truncated): `{"editions": [{"isbn_13": "9780330518918", "book_id": 381445, "language": {"language": "English"}, "book": {"title": "Pandora's Star"}}]}`

## Gate verdict (REQ-010 entry gate)

- **Both batch mechanisms are CONFIRMED live.** Aliasing (documented) and the
  Hasura `_in` list-filter (previously unconfirmed, ST-010 low-confidence)
  both returned valid data with zero GraphQL errors. The two agree exactly:
  the one ISBN with Hardcover coverage came back through both; the other four
  returned empty (coverage miss, not filter failure — P2's per-alias empties
  and P3's union are consistent).
- **`_in` is the recommended mechanism** for the follow-up design: smaller
  request body (269 vs 667 bytes at N=5, grows per-item slower), and response
  rows carry `isbn_13` so de-multiplexing to works is direct. Aliasing remains
  a proven fallback.
- **Coverage caveat for the batching design:** only 1/5 library ISBNs hit
  Hardcover's editions table on this sample. ISBN-anchored batching has weak
  yield on real libraries; the design should weigh batching on the hc_key /
  book_id axis (where HC coverage is definitionally 100%) and treat isbn
  batches as best-effort. A batch MISS must not be cached or recorded as a
  provider not-found for the work (it is an anchor-coverage miss).
- Batch-size limits were NOT probed beyond N=5; the follow-up batching design
  must pick a conservative batch max and keep the per-item fallback (REQ-010).
- Quota cost: 1 request per batch either way (Hardcover limit: 60 req/min).
