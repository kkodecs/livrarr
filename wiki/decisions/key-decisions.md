# Key Architectural Decisions

## Hardlink Policy
- **Import:** Copy (tag writing breaks hardlinks)
- **CWA downstream:** Hardlink-first, copy fallback (CWA copy is never modified)

## Config: TOML Only
No environment variable config overrides. TOML only. Servarr convention.

## Indexer System
Direct Torznab/Newznab URL support (url + api_path + api_key). Prowlarr is optional, not required.

## AppState: Concrete Types via Type Aliases
Not `Arc<dyn Trait>` (trait_variant non-dyn-compatible), not generics (12+ type params too viral). pk-confer unanimous.

## Enrichment: 3 Modes, Not 5
Background, Manual, HardRefresh. Matches existing EnrichmentMode enum. Simpler.

## Import Lock: (user_id, work_id)
Not per-grab. Prevents filesystem races when multiple grabs complete for the same work.

## Refresh: Wait Semantics
Async Mutex, no RefreshInProgress error. Second caller waits for the first to finish.

## Orphan File Adoption on Retry
If target file exists but no DB record, adopt the file. Handles crash recovery.

## LLM Context: Typed LlmValue
Not serde_json::Value. Prevents accidental secret leakage across the LLM boundary.

## Dedicated Error Enums for RSS/Monitor
Typed errors even for background-only jobs. pk-confer unanimous.

## Handler Isolation (Phase 5, Approved)
Split handlers into `livrarr-handlers` crate depending on `livrarr-domain` but NOT `livrarr-db`. Compile-time enforcement that handlers can't bypass the service layer.

## SQLite: chrono, Not time
Project-wide datetime handling uses `chrono`. No mixing.

## Security
- Passwords: argon2id
- Session tokens / API keys: stored as SHA-256 hashes, plaintext shown once
- Download client passwords: stored plaintext (Servarr convention), redacted in API responses
- No anonymous access to any endpoint except login, setup, and health
