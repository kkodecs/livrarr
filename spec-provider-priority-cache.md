---
feature: provider-priority-cache
stage: spec
status: approved-scope
version: 1
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004]
---

# Bug Spec: load enrichment priorities from the database

## Executive summary

Enrichment ignores the provider priorities stored in the database. It should load
them once at startup and use that same in-memory copy for fresh and cached
metadata. English metadata should prefer Hardcover, then Google Books, then
Goodreads. See [the requested behavior](#2-requirements).

This change adds no settings screen. It preserves foreign-language exclusions,
personal-edit protection and cover behavior. The Add metadata preservation fix
and protection against lower-priority overwrites are separate pending work.
See [scope limits](#4-non-requirements) and [verification](#6-acceptance-criteria).

## 0a. Design Principles

The database is the authority for ordinary-metadata and audio-detail priorities.
One startup snapshot governs both live and cached enrichment; no per-field or
per-Work database lookup is needed. A database configuration error must be visible,
not silently replaced by a hardcoded runtime order. Provider eligibility, identity
validation, personal ownership and cover-quality rules remain separate rules.

The user explicitly authorized this correction and requested no editable settings
screen. The implementation belongs on the isolated metadata-preservation branch.
Root main contains a parked rewrite and must not be built or restarted.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | Existing migration 057, domain ProviderPolicySnapshot and SQLite loader; source inspected September 18 | Existing table stores independent per-language ebook/audiobook ordered lists; no live consumer loads them. | Assuming those three seeded generic rows already reproduce current enrichment behavior. | high |
| ST-002 | merge_engine.rs PriorityModel, new and merge_from_cached; enrichment lib.rs enrich_work | Ordinary content and description use the same order; live and cached paths independently recreate hardcoded orders, and the merge constructor currently ignores its model. | Fixing only the startup constructor without checking actual consumers. | high |
| ST-003 | main.rs build_enrichment_pipeline; merge_engine.rs drop_language_incompatible_providers; language.rs provider_priority | English/unknown currently excludes Google Books at dispatch; foreign excludes HC/OL, also at merge. English aliases route as English. | A ranking change that cannot affect dispatch, or weakening foreign-language safeguards. | high |
| ST-004 | cover_rank.rs and merge_engine.rs merge_impl | Audio-detail order is currently also used for audiobook-cover selection; covers have their own quality and containment rules. | Letting a metadata database reorder inadvertently change cover behavior. | high |
| ST-005 | Existing GoogleBooksClient, provider queue and September 18 provider samples | Google Books keeps existing ISBN-anchored fetches, key requirement, payload normalization, shared transport and missing/failed response handling. No new endpoint, ID namespace or response parser is introduced. | Sending Goodreads Work IDs as Book IDs, bypassing identity admission, or claiming new provider-protocol guarantees from stubbed responses. | high |
| ST-006 | Existing client and transport adapters, unchanged by this correction | Transport redirect/admission and known provider failure classification remain unchanged; no new anti-bot capture was made. | Claiming this correction fixes provider outages or proves live anti-bot behavior. | high |

## 0c. Prior Art

Searched project docs/ and wiki/ for provider policy and provider priority; read
the current metadata-provider guide, metadata principles and recorded source
investigation. The existing schema and loader are reusable integration points.

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | wiki/domain/metadata-sources.md | Discovery, enrichment and covers have distinct rules; this change concerns enrichment metadata ordering. |
| PA-002 | wiki/domain/metadata-principles.md; wiki/insights/metadata.md | User values remain protected; fresh/cached contributions share one merge boundary and language guard. |
| PA-003 | build/reviews/add-search-research-2026-09-18/overwrite-rules/REPORT.md | Establishes effective current lists, ignored database settings, cover coupling and current overwrite limitations. |
| PA-004 | migration 057 and tests/behavioral/test_metadata_refactor_policy.rs | Prior database-persistence design exists; preserve old migration checksums and existing explicit customized priorities. |
| PA-005 | tests/behavioral/test_metadata_refactor_pipeline.rs; test_mc_anchor_grounding.rs | Real SQLite, production queue/merge and external-client stub seams are available for regression tests. |

## 1. Problem Statement

Changing a provider priority in the database has no effect on enrichment. Both
fresh provider results and cached results use lists embedded in code instead.
The startup merge constructor even ignores its supplied priority model.

Reproduce with a temporary SQLite database: store a nondefault order, load it,
pass it to the real merge/service setup, offer conflicting provider descriptions,
then inspect the saved description and source. The old hardcoded order wins.

## 2. Requirements

- **REQ-001**: After migrations, startup loads and validates provider priorities
  from SQLite into shared immutable process memory before requests/background
  enrichment begin. Fresh and cached enrichment use this snapshot. Restart reloads
  changed database values; changing rows during a process lifetime does not change
  that process's active order. Startup must report a load/validation failure.
- **REQ-002**: Seed ordinary English/unknown-language metadata as Hardcover →
  Google Books → Goodreads → Readarr → OpenLibrary → Audible. Other languages use
  Google Books → Goodreads → Readarr → Audible unless an explicit language policy
  exists. Normalize English aliases as today. Keep foreign HC/OL exclusions and
  existing payload-language guards. Enable Google Books for English enrichment
  through the existing client, subject to its normal configuration and identity
  requirements. Content and descriptions use the same ordinary-metadata list.
- **REQ-003**: Store audio-detail ordering as Audible → Audnexus → Hardcover →
  Goodreads → OpenLibrary → Google Books. Reuse the existing table's ebook list
  for ordinary metadata and audiobook list for audio details, documenting those
  meanings. Preserve ebook/audiobook cover orders, quality checks and Goodreads
  cover containment independently of metadata-priority edits.
- **REQ-004**: Use a new forward migration, leaving migration 057 unchanged.
  Initialize missing English/default lists and upgrade the exact old seeded
  generic defaults. Preserve explicitly customized lists. Reject malformed stored
  provider/kind/rank values instead of silently clamping ranks or using hardcoded
  fallback orders. No UI, live reload, new network endpoint or dependency.

## 3. Interface Design

No browser or HTTP request changes. Database priority edits take effect on the
next server restart. The process holds one consistent snapshot, rather than a
user/browser session cache. Existing metadata configuration remains responsible
for provider credentials and availability.

## 4. Non-Requirements

No changes to cover priorities, discovery order/count, provider parsers, identity
matching, existing-Work Add, Add-time metadata persistence, date semantics or the
current rules comparing replacement values with stored provenance. No library
sweep or automatic refresh caused by the migration. No main merge or release.

## 5. Open Questions

None for this bounded correction. Foreign Hardcover/OpenLibrary eligibility stays
as previously documented; Google Books is already first there. Production rollout
will use reviewed source and normal validation, not the parked root checkout.

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): The actual startup construction loads stored policy;
  nondefault database ordering controls persisted description/source through
  real SQLite, production enrichment/merge and external-provider fixture seams.
- [ ] **AC-002** (REQ-001): Cached reuse honors the same stored order without
  provider dispatch. Editing rows leaves the existing process unchanged and a
  newly constructed startup instance sees the change.
- [ ] **AC-003** (REQ-002): With HC absent and GB/GR both offering descriptions,
  default English enrichment chooses GB; HC wins when it offers a usable value.
  Foreign guards and English/unknown aliases retain their intended behavior.
- [ ] **AC-004** (REQ-003): A database audio-detail reorder changes the selected
  audio detail without changing audiobook-cover selection or cover-upgrade rules.
- [ ] **AC-005** (REQ-004): Fresh and upgraded databases have the requested defaults;
  custom lists survive migration. Bad policy values cause a clear startup/load
  error. Existing cover and metadata-protection regressions remain green.
