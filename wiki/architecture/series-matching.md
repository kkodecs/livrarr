# Series Matching

Series are metadata groupings that connect related works. A Series is one of the BIG7 entities.

## How Series Work

- Series are discovered during enrichment (Hardcover, OpenLibrary)
- A Work can belong to multiple Series (e.g., main series + omnibus)
- Series have ordering (position within series)
- Author monitoring can auto-add new works in monitored series

## Author Monitoring

Background job (24h interval) polls providers for new works by monitored authors:
- 1-second delay between provider requests (rate limit courtesy)
- 429 responses trigger notification (NarrationType::RateLimitHit)
- New works create notifications; optionally auto-added based on user policy

## Matching Strategy

Fully deterministic (Phase 5, 2026-07-03): provider IDs first (ISBN, ASIN, GR/OL/HC keys), then the one matching authority's verdicts (`livrarr-domain/src/identity_matching.rs`, insight #59). No LLM chooses a match anywhere; ambiguity parks for user review (grey never absorbs).
