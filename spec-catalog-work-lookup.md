---
feature: catalog-work-lookup
stage: spec
status: approved-scope
version: 2
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004]
---

# Bug Spec: reliable catalog Work lookup

## Executive summary

An unusual selected edition must not prevent Livrarr from finding its Work in other
catalogs. Repair the existing lookup sequence and harmless title normalization;
preserve the chosen ISBN and use established catalog IDs directly. Also enlarge
Work search-result covers by 30%. See [requirements](#2-requirements) and
[acceptance checks](#6-acceptance-criteria). The user approved this scope on
19 September 2026. The broader Add-save change remains separate.

## 0a. Design Principles

A Work is independent of edition. ISBN agreement may corroborate identity; ISBN
absence or disagreement cannot veto a Work match. User selection remains authoritative.
Use existing matching, route settlement, retry accounting and review authorities.
Do not create a second matcher or bypass ambiguity/author/volume safeguards.
Worktree: `/mnt/opt/livrarr-worktrees/add-metadata-preservation`; canonical documents:
`/mnt/opt/livrarr`. Never build or restart the parked root rewrite.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | live-add-trace-20260919T000839Z/CATALOG-LOOKUP.md and deployed 9f366b77 | Jekyll has ISBN 9789176393871; initial title search rejected &/and, later HC/OL ISBN calls missed. | Treating a missing edition as a missing Work. | high |
| ST-002 | provider_queue.rs derive_route_anchor_query/dispatch_enrichment | Existing provider-local title-search fallback uses active routes and recorded terminal misses; Hardcover currently prefers ISBN over its Work ID. | A duplicate fallback engine or route writes outside settlement. | high |
| ST-003 | handlers work::add; metadata WorkService::refresh; SQLite reset_for_manual_refresh | Add's delayed Refresh clears recorded misses and repeats ISBN lookup. | Depending on a future refresh to repair a miss in the present attempt. | high |
| ST-004 | identity_matching.rs canonical_phrase; identity-layer normalized storage | Matcher and legacy identity_key drop &, retain and. The v2 stored group key instead lowercases display main and strips a leading article, preserving punctuation and &. | Changing comparison without considering stored lookup keys/collisions, or merging existing Works automatically. | high |
| ST-005 | identity-layer rewrite REQ-027 and wiki insights 95–99 | One clear Same-title/Agree-author candidate may settle without edition corroboration; ambiguity uses existing review. GR Book/Work namespaces differ. | Treating Goodreads Work IDs as Book-page IDs, or adding Google Books as Work identity. | high |
| ST-006 | SearchPage.tsx LibraryResult/OlResult | Actual cover, placeholder and library-result cover are 88 × 128 CSS pixels. | Resizing only loaded covers, or changing result count. | high |

## 0c. Prior Art

Searched current docs/, wiki/ and root identity specifications for ISBN, fallback,
normalization and REQ-027; inspected the deployed source and retained runtime trace.

| ID | Artifact | Bearing on this change |
|----|----------|------------------------|
| PA-001 | ARCHITECTURE.md Part 1; PRINCIPLES.md | Work/edition distinction, one authority and user intent. |
| PA-002 | spec-identity-layer-rewrite.md REQ-027; wiki/insights/identity.md 95–99 | Existing bounded provider-local search, settlement, review and namespace rules. |
| PA-003 | build/reviews/provider-priority-cache/live-add-trace-20260919T000839Z/CATALOG-LOOKUP.md | Logs and compiled matcher reproduction establish the observed defect. |
| PA-004 | tests/behavioral/test_ilr_contracts.rs; tests/behavioral/test_responsiveness_add.rs | Existing real SQLite/router/provider seams; author tests against actual source before choosing exact harness. |
| PA-005 | wiki/domain/search-result-metadata.md | Broader Add metadata preservation remains pending and is outside this fix. |

## 1. Problem Statement

A user selects Jekyll from search with an uncommon edition ISBN. The Work is saved
but gains no catalog Work IDs despite available candidates. The built matcher
rejects the equivalent &/and title, and later enrichment repeatedly asks catalogs
for the same missing edition. Work search-result images are also smaller than requested.

## 2. Requirements

- **REQ-001**: Prefer an existing usable catalog record ID over edition ISBN for that
  catalog. Preserve typed namespaces: direct lookup only where that ID is a supported
  fetch key; a Goodreads Work ID must never address a Goodreads Book endpoint.
- **REQ-002**: Retain OpenLibrary, Hardcover and Goodreads Work-search eligibility;
  Google Books remains edition-only. For eligible catalogs without a known Work ID, a healthy ISBN
  NotFound must trigger the existing title/primary-author Work search during the same
  logical attempt. A clear match enters the normal identity road and becomes durable
  and visible through the real detail response. Preserve the selected ISBN. Do not
  reinterpret provider errors/timeouts as NotFound; preserve retry and search budgets,
  provider applicability, generation checks, ambiguity review and deduplication.
- **REQ-003**: Equivalent ordinary English &/and title spellings must match with the
  same author. Keep meaningful subtitle/collection/adaptation/volume and author
  distinctions. Preserve consistent matching/storage behavior and existing data:
  use bounded read-time compatibility for both existing and fresh rows. Keep stored
  key recipes, display titles and identity generations unchanged during reads. Share
  the conjunction fold, use exact-key fast paths and same-user/primary-author candidate
  scope, and revalidate actual titles so a lossy old key cannot erase a missing
  conjunction. Cover group readers, exact-tuple guards and legacy batch Add dedup.
  No startup rewrite, automatic merge, dropped rows or re-enabled disabled heal.
- **REQ-004**: Increase search-result cover width and height by exactly 30%:
  88 × 128 becomes 114.4 × 166.4 CSS pixels. Apply consistently to loaded discovery
  covers, placeholders and local-library matches on Work search; keep aspect ratio,
  mobile layout, readability and the current maximum of twelve provider results.

## 3. Interface Design

No new setting or API field. Existing review handles ambiguous matches. Book
Information continues to show established catalog identifiers. Google Books remains
an edition metadata source, deliberately excluded from Work identity.

## 4. Non-Requirements

No larger result list, more-results link, broad Add-save/date/overwrite repair,
provider-order change, identity redesign, new dependencies, main merge or release.
No repopulating frozen Work identifier columns: catalog fetch IDs come from active
routes, and the selected ISBN remains edition-owned. No mass refresh or automatic
repair of the live library. Existing Jekyll can be
checked using the normal explicit Refresh after deployment if authorized.

## 5. Open Questions

No product decision remains for the approved scope. Use bounded compatibility reads
for old stored titles while preserving merge containment. If this cannot be done
correctly within that boundary, report the exact blocking invariant before expanding
architecture. No storage-key migration or startup heal is approved. Grok sign-in and
both reviewer model probes passed; actual supported maximum effort settings are
recorded in build/reviews/catalog-work-lookup/preflight/VERIFIED-MODELS.json.

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): An established Hardcover/OpenLibrary catalog ID is used
  directly even when the selected edition has an unrecognized ISBN; no ISBN lookup
  replaces that supported catalog fetch. Goodreads namespace protections remain green.
- [ ] **AC-002** (REQ-002): Through the real Add/handler, real SQLite and actual provider
  adapters with controlled HTTP, an obscure-ISBN Work acquires a clearly matching
  catalog Work ID after ISBN NotFound in the same logical attempt; chosen ISBN survives,
  and the real detail response exposes the catalog ID. No extra refresh is required.
- [ ] **AC-003** (REQ-003): The actual matcher accepts the Jekyll &/and pair with the same
  author, rejects a different author/collection/volume, and persists/finds Work identity
  consistently without merging or losing pre-existing records. Both pre-existing
  &/and rows remain discoverable with their distinct routes/Edition owners intact;
  reads preserve stored keys, display and generation. A conflicting catalog ID uses
  existing review; old batch-dedup keys still find the correct Work and reject an
  omitted conjunction. Startup heals remain disabled.
- [ ] **AC-004** (REQ-002): Errors do not become misses, ambiguous distinct Work IDs
  use the existing review path, and an unsuccessful fallback terminates without a loop.
- [ ] **AC-005** (REQ-004): Browser/render verification shows all three Work-search
  cover cases at 114.4 × 166.4 CSS pixels with a usable narrow-screen layout. Existing
  frontend checks pass. No new test is required merely to mirror CSS constants.
