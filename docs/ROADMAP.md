# Roadmap

## Current Status: Alpha 3 (in development)

### Active Work

**Service Layer Consolidation** (pk-auto-build cycle)
- Spec, contracts, elaborate, TDD phases: COMPLETE
- Implementation (Phase 4 — handler migration): NEXT
- 13 service traits defined, 15 service impls across 5 crates
- 500 tests passing, 34 ignored (need handler migration)
- Phase 5 approved: split handlers into `livrarr-handlers` crate for compile-time service layer enforcement

### Completed Features

- Core library management (import, organize, tag writing)
- Multi-provider metadata enrichment (Hardcover, OpenLibrary, Audnexus, GoodReads)
- LLM-based metadata validation
- qBittorrent + SABnzbd download client integration
- Direct Torznab/Newznab indexer support (Prowlarr optional)
- Author monitoring with auto-add
- RSS sync with auto-grab
- Multi-user support with admin/user roles
- Calibre-Web Automated integration
- React frontend (full SPA)
- Docker deployment

### Must Complete Before Alpha 3

| Item | Status |
|------|--------|
| Service layer consolidation (handler migration) | In progress |
| Handler compile-time isolation (Phase 5) | Approved, pending |
| Login/setup endpoint rate limiting | Not started |
| `_livrarr_meta` version gate table | Not started |

## Deferred to Beta

| Item | Rationale |
|------|-----------|
| `async-trait` -> native async fn migration | Mechanical but high-touch, all crates affected |
| `livrarr doctor` CLI command | Read-only integrity scanner |
| Cursor-based pagination | Replaces offset-based |
| HttpOnly cookie sessions | Security hardening |
| Testkit crates | Test infrastructure cleanup |
| SSRF validation + resolver pinning | Security hardening (7 items) |

## Deferred Indefinitely

| Item | Notes |
|------|-------|
| Pagination response wrappers | Existing behavior preserved |
| Cover caching semantics | Existing behavior preserved |
| Architectural tests for "exactly once" | Post-consolidation CI gate |

## Future Ideas (build/plans/)

- Playback integration
- Request system (user requests for works)
- Shared storage across users
- Alpha distribution
- Send to email (Kindle via SMTP)
