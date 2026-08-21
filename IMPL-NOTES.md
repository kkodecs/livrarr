# IA-E05 implementation notes

## Red

Command:

```
cargo test -p livrarr-external-data --lib hardcover_queue_full
```

Failing tests (compile on frozen base, fail by assertion):

- `hardcover_queue_full_matches_across_anchor_and_seeded`
- `hardcover_rate_limited_matches_across_anchor_and_seeded`
- `hc_post_maps_fetcher_queue_full_off_the_http_catchall`
- `hc_post_maps_fetcher_rate_limited_off_the_http_catchall`
- `hc_post_maps_wrapped_http_429_off_the_http_catchall`
- `search_audible_maps_fetcher_queue_full_to_queue_full`
- `search_audible_maps_fetcher_rate_limited_to_rate_limited`
- `search_audible_maps_wrapped_http_429_to_rate_limited`
- `lookup_audible_by_asin_maps_fetcher_queue_full_to_queue_full`
- `lookup_audible_by_asin_maps_fetcher_rate_limited_to_rate_limited`
- `lookup_audible_by_asin_maps_wrapped_http_429_to_rate_limited`
- `audible_client_maps_queue_full_to_budget_exempt_will_retry`
- `audible_client_maps_rate_limited_to_will_retry_rate_limit`

Excerpt:

```
HC anchor/hckey: expected WillRetry(QueueFull), got WillRetry { reason: ServerError, ... }
queue full must not collapse into Http (ServerError budget burn), got Http("queue full, retry after 1s")
```

## Quality gate

- `cargo fmt --all -- --check`: 0 diffs
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings
- `cargo test --no-fail-fast`: 2282 passed / 0 failed / 297 ignored (176 suites)

## Changed files

- `crates/livrarr-external-data/src/hardcover.rs` — `QueueFull`/`RateLimited` on `HardcoverError`; `hc_post` maps `FetchError::QueueFull`, `RateLimited`, and wrapped HTTP 429
- `crates/livrarr-external-data/src/audible.rs` — same transport mapping on search and ASIN lookup; client maps those to `WillRetry{QueueFull}` / `WillRetry{RateLimit}`
- `crates/livrarr-external-data/src/provider_client.rs` — Hardcover client uses one pause/retry mapper so QueueFull does not fall through as a miss or `ServerError`
- `crates/livrarr-external-data/src/author_link.rs` — exhaustive `HardcoverError` match (type-forced)

## Type-forced extras

New `HardcoverError` variants forced Display, `map_hardcover_error`, and the Hardcover client matches. Audible client `Err(_)` catch-alls were not type-forced but were required: transport `QueueFull` still became `WillRetry{ServerError}` there.

## Limitations

A completed HTTP 429 *response* (Ok, status 429) is unchanged: Hardcover stays `Http("HTTP 429")` → `ServerError`; Audible stays `Other("Audible returned 429")` → `ServerError`. The packet scoped the three transport catch-all `Err` arms. Other providers and breaker signals are untouched.
