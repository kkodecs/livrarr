# Key Architectural Decisions

## Executive summary

Standing decisions that shape Livrarr: copy on import, TOML-only configuration, direct indexer
support, and the security rules. The one most often second-guessed is the
[two web clients](#ssrf-trusted-infrastructure-pattern): addresses the admin configured, such as
download clients and indexers, use the trusted client, because they normally live on a private
network; addresses that come from outside data use the safe client.

> **Source scope:** Root PRINCIPLES.md and ARCHITECTURE.md take precedence. These decisions are preserved as practical context; exact source and current acceptance come from project state. In particular, the old Readarr hardlink path is not a new exception to the root import policy.

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

> **September 18 factual clarification:** The hashes below apply to Livrarr login passwords, session tokens and Livrarr-issued API keys. Reusable provider, LLM, indexer/Prowlarr, download-client and SMTP credentials are stored as plaintext in the current database. The earlier summary below did not enumerate all such categories. See [current root wording](../../ARCHITECTURE.md#secure-by-default) and [verified source evidence](../../build/reviews/wiki-follow-ups-2026-09-18/post-deploy/credentials/REPORT.md).

- Passwords: argon2id
- Session tokens / API keys: stored as SHA-256 hashes, plaintext shown once
- Download client passwords: stored plaintext (Servarr convention), redacted in API responses
- No anonymous access to any endpoint except login, setup, and health

## SSRF: Trusted Infrastructure Pattern

Two HTTP clients live on `AppState`:

- **`http_client`** — unrestricted; no SSRF resolver. Used for admin-configured infrastructure that is *expected* to live on private networks: download clients (qBittorrent, SABnzbd, Transmission), indexers (Prowlarr, NZBHydra2, Jackett, direct Torznab), admin-approved Readarr origins, LLM endpoints, etc. Readarr's trusted client never follows redirects, so its API key cannot be forwarded to a redirected origin.
- **`http_client_safe`** — wraps `SsrfSafeResolver`; rejects any private/loopback/link-local/reserved IP at DNS resolution time. Used for **runtime-derived** URLs whose value comes from outside admin configuration: cover proxy fetching metadata-provider image URLs, anything pulled from a scraper response, etc.

Readarr has a two-stage trust boundary: an unapproved public origin may connect through the SSRF-safe no-redirect client; an origin on the admin-managed `readarr_origins` allowlist connects through the trusted no-redirect client and may intentionally resolve to private infrastructure. Approval therefore accepts DNS rebinding/private-answer risk for that exact normalized origin; removal restores the public-only rule.

Manual import Retry follows the same rule (as of 2026-10-05, security-before-release): when a
grab has no saved content path, its lookup at the download client uses the trusted client, as the
first import attempt and the background poller do
(`crates/livrarr-server/src/import_service.rs:224`, `:232`; `crates/livrarr-server/src/main.rs:219`).

Plus `TrustedOrigins` (built from configured indexers + download clients at startup, rebuilt on config change) lets the grab flow allow private-IP download URLs that match a configured origin even when the called client is normally SSRF-safe.

**Why this split exists.** Alpha3 used `http_client_safe` for download-client and indexer test endpoints. Every user running qBittorrent / SAB / Prowlarr on a private IP — which is the typical Docker / NAS / LAN deployment — was broken. Alpha4 (commit `79cb402`) fixed this by switching all admin-configured infrastructure to the unrestricted client and introducing the `TrustedOrigins` allowlist for the grab flow.

**Reviewer trap (do not repeat).** Audit reviewers (and CC) that see "user-provided URL flows into HTTP request" will instinctively suggest `http_client_safe`. That instinct is **wrong** when the URL is admin-configured infrastructure. Admin = trusted by definition. The threat model is not "admin attacks the server" — admin already controls the server. The threat model is *external* input (untrusted runtime data) being used to pivot inside the network. Hence the runtime-derived bucket gets SSRF protection; the admin-configured bucket does not.

**Practical rule.** Before flagging a `http_client` call as a SSRF gap: where does the URL come from? If it's from `settings`, `download_clients`, `indexers`, `metadata_config`, or any admin-configured field → that's intentional. If it's from a network response body, a scraped page, or any per-request external input → that's the case `http_client_safe` exists for.

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/decisions/key-decisions.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.
