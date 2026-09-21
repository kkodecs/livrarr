---
feature: add-metadata-preservation
stage: spec
status: scope_reset_by_user
version: 5
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008]
---

# Preserve metadata when Add creates a Work

## Executive summary

The current job is to save the useful information already available when you select a search result and click Add, then apply the agreed rules for later enrichment. Save the cover address even if downloading its image fails. Keep original publication year separate from an edition date. See [included work](#included-work), [verification](#verification), and [deferred work](#deferred-work).

The user restored this scope on 20 September 2026. The expanded cover-recovery and permanent original-result history work is removed from this feature and recorded in the [deferred-work record](build/reviews/add-metadata-preservation/scope-reset-20260920/RECEIPT.md#changes). The application fix remains unimplemented; the user has resumed development within this scope. Prior design approvals do not authorize the expanded work or approve a replacement design.

## Authority and starting point

This scope follows the user's request to save information supplied by each metadata provider at Add time, the [provider mapping](wiki/domain/search-result-metadata.md#proposed-field-mapping), the agreed original-year/edition-date distinction, and the agreed provider overwrite rules. The user explicitly directed: do not add to scope without approval; restore the original plan; put extras on the to-do list.

The [previous specification and design](build/reviews/add-metadata-preservation/scope-reset-20260920/predecessors/project/spec-add-metadata-preservation.md) are preserved as history. The old 135-check implementation plan is withdrawn. Existing tests and partial scaffolding are preserved for selective reuse, not authority to restore removed requirements.

## Design Principles

Keep the existing creation authority, provider priority cache and image writer. Save correct values and provenance together; missing input stays missing. Preserve existing user ownership and identity rules. This feature adds no retry system or immutable source history.

## System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|---|---|---|---|---|
| ST-001 | [Creation investigation](build/reviews/add-metadata-preservation/INVESTIGATION.md#q1-the-real-path-and-where-a-snapshot-can-ride) | The actual identity settlement creates the Work; ordinary Add fields currently stop before its transaction. | Post-success recovery as a substitute for initial persistence. | high; source-confirmed baseline |
| ST-002 | [Provider mapping](wiki/domain/search-result-metadata.md#provider-comparison) and [captured responses](build/reviews/add-metadata-preservation/provider-captures/MANIFEST.json) | Provider fields are optional; Google volume dates describe editions, and search language is not a book fact. | Inventing absent values or original publication years. | high for recorded captures |
| ST-003 | [Cover rules](wiki/insights/covers.md) | A saved URL is distinct from saved image bytes; current manual-choice and provider-containment behavior remains. | False dimensions/manual-image claims or new retry workflows. | high |
| ST-004 | [Priority policy](wiki/domain/enrichment-priorities.md) | Provider ordering is already stored in the database and cached at startup. | New priority hardcodes or settings work. | high |
| ST-005 | [Identifier mapping](wiki/domain/search-result-metadata.md#proposed-field-mapping) and [captured responses](build/reviews/add-metadata-preservation/provider-captures/MANIFEST.json) | Google volume ids, ISBN-10/13, OpenLibrary Work/author keys, Hardcover Work ids and Goodreads Book/Work ids are separate namespaces with existing consumers. | Conflating Book, Work, volume or author identifiers, or making extra source references new identity authority. | high |

## Prior Art

The existing metadata mapping, direct-Add investigation and enrichment policy were read before this narrowed design; historical proposals do not override the user's scope reset.

| ID | Artifact | Bearing on this fix |
|---|---|---|
| PA-001 | [Field mapping](wiki/domain/search-result-metadata.md#proposed-field-mapping) | Complete supported inventory; no new provider research needed. |
| PA-002 | [Direct Add review](wiki/architecture/direct-add-seed-review.md) | Use the actual creator and save required fields within its transaction. |
| PA-003 | [Enrichment priorities](wiki/domain/enrichment-priorities.md) | Reuse existing language/provider ordering and personal ownership guards. |
| PA-004 | [Scope reset](build/reviews/add-metadata-preservation/scope-reset-20260920/RECEIPT.md) | Removes expanded cover recovery, full original-result history and broader cleanup; retained tests need scope filtering. |

## Included work

- **REQ-001** — Provider mapping. Carry useful already-fetched fields from Google Books, OpenLibrary, Hardcover and Goodreads through the search result, browser Add request and server. Retain the agreed inventory: title/trustworthy subtitle, supplied author names/references, actual language, publication information and publisher, description/completeness, series/position, categories, pages, rating/count/provider, correctly typed identifiers, and cover URL/source where available. Unknown remains unknown; keep Hardcover's subtitle exclusion. Preserve necessary context for fields that do not honestly fit an ordinary Work field. Do not invent facts or make a second provider request to recover information already supplied.
- **REQ-002** — Initial save. When Add creates a new Work, save all supplied mapped values and their provider provenance in the same database transaction as the new Work row and its single birth event, before reporting success. The initial response and subsequent reads must reflect the saved data even if enrichment fails. A failure to save any of those required values or their provenance rolls back the Work insertion and birth event and must not report successful partial creation. A post-settlement write is not sufficient. Image downloading remains outside this transaction. Use existing creation and storage paths, extending them only as needed for this inventory. Copied provider facts are not personal edits.
- **REQ-003** — Dates. Work year means original publication year. Preserve an edition date separately and keep supplied precision without inventing month/day. Google Books volume dates do not become original Work years. Hardcover book release dates and OpenLibrary first-publication years are supported original-publication facts; Hardcover edition release dates and Google volume dates describe editions. Goodreads search supplies no date. Only supported original-publication facts may supply Work year during Add or later enrichment. Unclassified legacy input and ambiguous cached dates must not fill original year even when it is blank, or overwrite a known date; preserve supplied date precision and unclassified context without inventing meaning. No backfill or cleanup of existing library records.
- **REQ-004** — Cover address. Save the initial external cover URL and source independently of image retrieval. Preserve any existing explicit-choice semantics. A failed download must not erase the address or claim that bytes, dimensions or a manual image were saved. Keep existing download, image-writing, containment, quality and manual-choice behavior. No new pending-download queue, retry state machine or startup/refresh/convergence recovery pass.
- **REQ-005** — Agreed overwrite rules. Use the existing database-backed, startup-cached priority policy. Lower-priority providers cannot replace a populated higher-priority provider value; higher-priority providers may replace ordinary provider data; the same provider may refresh it. Empty or unusable offers retain both saved value and provenance. Fill eligible blank fields. Keep existing title/author/language protections, personal edits and intentional clears. Unknown, missing or excluded incumbent sources receive no ranked precedence: after the existing personal, title, author and language guards, an eligible listed offer may replace an ordinary value from such a source. A genuinely recorded provider or Readarr source that is in the applicable field/language list uses its configured rank. Do not invent a synthetic rank. Apply the rule to existing live and cached enrichment paths without redesigning their scheduling or cache admission.
- **REQ-006** — Correct initial pairs. Save selected series/name-position and rating/count/provider together as supplied, including honest missing partner values. Keep the source and shortened-description distinction. This does not authorize a broader repair of all existing enrichment grouping behavior or immutable history of every original selected field.
- **REQ-007** — Existing behavior. Re-adding/adopting an existing Work must not use the selection to overwrite its saved metadata or personal edits. Retain current duplicate responses, legacy Add compatibility, identity matching, author adoption and candidate-cache behavior. Additional references are source facts, not new identity authority.
- **REQ-008** — Proportionate verification. Test the changed production paths, including provider failure after Add, persistence, date meaning, overwrite protection and duplicate preservation. Reuse relevant tests already written. Keep the deployed 12-result limit, enlarged search covers and current settings surface. No unrelated refactors, new UI or live-library test writes.

## Verification

- **AC-001:** Representative captured responses from each of the four providers retain the complete REQ-001 inventory and honest absences through actual discovery and browser Add. This includes parser additions for Google rating/count and Goodreads description/pages/count, the already-parsed Goodreads Work id, trustworthy bare title, description completeness, series/position, rating/count/provider, date meaning/precision/source, typed identifiers and cover URL/source. Missing book language stays unknown; search/default language does not become a book fact.
- **AC-002:** A real search-page Add sends the complete mapped inventory, including description/completeness, series/position, rating/count/provider, publication meaning/precision, typed identifiers and cover URL/source. The creation response and database reopen retain all supplied values and provider provenance with providers unavailable. Work insertion, its single birth event and all mapped-value/provenance writes commit in one transaction. Failure of the value/provenance save rolls back that Work insertion and birth event and reports no successful creation; retry may create once. No second provider lookup is needed to recover those fields.
- **AC-003:** Edition-only dates remain separate from original Work year through Add and later enrichment: the captured Google date 2005-08-02 stays an edition date with its precision/source, with original year blank. A legacy year without known meaning cannot fill original year. Supported Hardcover book and OpenLibrary first-publication facts may fill it subject to ownership and priority rules. Goodreads search remains honestly undated; ambiguous cached dates cannot become original-year offers. Preserve existing known original dates and personal ownership.
- **AC-004:** Failed image retrieval leaves the supplied cover URL/source saved without false saved-image state. Existing successful download and manual-choice behavior remains intact; automatic retry/restart recovery is not a criterion.
- **AC-005:** Lower-ranked offers, empty offers and personal edits/clears are protected; higher/same-provider updates and eligible blank fills follow the agreed rules in current live/cached paths. An eligible listed offer may replace an ordinary unranked incumbent after existing guards; known provider/Readarr provenance uses its applicable rank. Empty/unusable offers retain both value and provenance.
- **AC-006:** Initial related fields stay coherent, re-Add does not overwrite existing metadata, and legacy Add still works.

## Deferred work

The [deferred-work record](build/reviews/add-metadata-preservation/scope-reset-20260920/RECEIPT.md#changes) holds proposals for durable cover recovery, broader cover-writer refactoring, permanent original-result history, and broader enrichment pair repairs. These require separate explicit approval before work starts. No settings editor, identity redesign, catalog-lookup change, performance project, release or main merge/push is added here.

## Development status

Source work stays in the isolated `fix/add-metadata-preservation` checkout. The live application and library are unchanged. The [current status](build/state/STATUS-add-metadata-preservation.md) and [continuation](build/state/continuation-add-metadata-preservation.md) identify the resumed work. A replacement design and focused test selection must match this scope before implementation resumes; historical passing reviews apply only to their historical artifacts.
