---
feature: fresh-author-index
stage: spec
status: approved-scope
version: 2
req_ids: [REQ-001, REQ-002, REQ-003]
---

# Bug Spec: complete author database setup

## Executive summary

A fresh installation of the current development build cannot add its first Work because author creation requires an index that startup omitted. Restore the required setup and prove the first Add works through the real server. Existing author records must remain unchanged. See [requirements](#2-requirements) and [acceptance checks](#6-acceptance-criteria). The user approved this repair before returning to metadata preservation.

## 0a. Design Principles

Fix schema setup through its existing authority. Preserve records, user ownership and merge containment. Never restore automatic merging merely to create an index. A conflicting old database must fail clearly without discarding or modifying its authors. Keep existing migrations immutable and preserve restart/upgrade compatibility.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|---|---|---|---|---|
| ST-001 | fresh-author-confirmation-20260919/RESULT.json | Deployed 98d35a37 and prior 9f366b77 return HTTP 500 through real Add on a fresh normal database; adding only the missing index to scratch makes the same Add return 200. | Treating this as a provider outage or a test-only missing fixture. | high |
| ST-002 | sqlite_author_link.rs create_or_adopt_author_tx (live Add); sqlite_author.rs create_author (legacy writer); migration077 | Both writers target the partial unique index on user_id and non-null normalized_name. Migration077 adds the column, not the index. | Dropping duplicate protection or silently changing matching semantics to avoid the error. | high |
| ST-003 | db lib.rs create_test_db_with_legacy_work_index | The standard helper creates the index itself. | Using that helper as proof fresh installation is repaired. | high |
| ST-004 | VERSION-HISTORY.json; canonical wiki/domain/author.md | Startup author-merging/backfill call was removed in August; existing Oasis has the index. Latest local alpha6 tag predates this dependency. | Re-enabling historical merges or claiming a published release was reproduced. | high |
| ST-005 | pool.rs MAX_SCHEMA_VERSION and existing migration history | Shared compatibility version is 83, separate from the highest SQL migration number (currently88). | Automatically equating migration number with compatibility version or rewriting applied migrations. | high |
| ST-006 | author canonicalization and partial index | Null normalized keys are intentionally outside the unique index; identical keys may exist for separate users. | Backfilling or changing keys/names as an incidental setup fix. | high |

## 0c. Prior Art

Searched canonical wiki/domain/author.md, wiki/patterns/migration-pattern.md, author/identity/data-state/history-review insights and the preserved live reproduction before writing this spec.

| ID | Artifact | Bearing |
|---|---|---|
| PA-001 | build/reviews/catalog-work-lookup/fresh-author-confirmation-20260919/REVIEW.md | Actual executable and HTTP reproduction, one-index control, exact version limits. |
| PA-002 | wiki/patterns/migration-pattern.md | Immutable migration, backups, startup order and compatibility guard. |
| PA-003 | wiki/domain/author.md and wiki/insights/identity.md | User scoping, contributor/identity protections and no incidental automatic merge. |
| PA-004 | spec-bugfix-175-duplicate-authors.md in source worktree, if present; pool.rs backfill_author_identity | Historical combined repair/merge/index design is evidence, not permission to restore merging. This repair explicitly supersedes its prohibition on a bare unique-index migration: fail-closed index creation is allowed; duplicate merging/key rewriting remains forbidden. |
| PA-005 | PRINCIPLES.md and ARCHITECTURE.md | Existing authority, preservation of user data and real-entry tests. |

## 1. Problem Statement

Starting the current executable with an empty data directory and completing normal account setup succeeds, but Add Dune / Frank Herbert returns HTTP500 and saves zero authors and zero Works. The error is an invalid ON CONFLICT target because idx_authors_identity does not exist. Test helpers create it and therefore conceal the installation defect.

## 2. Requirements

- **REQ-001**: Normal fresh startup establishes the author uniqueness structure before serving Add requests. The authenticated real Add path can create an author and Work from title/author alone; it must not require provider success, a test-only schema patch or a second manual step. Restart is repeatable.
- **REQ-002**: Existing correctly indexed databases and unindexed databases whose non-null normalized author keys are unique can adopt the repair without changing existing author records, keys, identities or relationships. Preserve user-scoped uniqueness and nullable-name exemptions. If existing duplicate non-null keys prevent the required uniqueness, fail setup explicitly with its cause and preserve the original records and prior migration state; do not merge, delete, rename, silently ignore the failure or serve a falsely repaired database.
- **REQ-003**: Keep shipped migration checksums and existing schema/data compatibility semantics intact. Apply the repair through existing normal initialization, covering repeat startup and indexed-database upgrade. Tests must use genuine normal schema/startup for the claimed fresh-install behavior. No unrelated author matching, providers, frontend, metadata preservation or release-version changes.

## 3. Interface Design

No new UI or API fields. A new user can add a Work successfully after normal setup. Invalid older databases receive a clear setup failure instead of silent data repair. Existing API duplicate/identity behavior remains unchanged.

## 4. Non-Requirements

No author merge/backfill, normalized-key rewrite, duplicate cleanup, changed title matching, provider traffic, Add-time metadata preservation, cover changes, release publication or main merge. Root source is a parked rewrite and must not be built, edited or deployed. Source worktree is /mnt/opt/livrarr-worktrees/add-metadata-preservation; canonical documents and evidence are under /mnt/opt/livrarr.

## 5. Open Questions

No product decision needed for the confirmed repair. PM may choose the smallest existing-authority implementation after bounded source investigation. Preserve conflicting older data by explicit setup refusal; implementing duplicate remediation is outside scope. Local deployment follows passing required reviews, full Rust checks, fresh-start verification and a private live-database-copy upgrade check.

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Start the real server on an empty data directory, create the account through its actual route, then authenticate and Add a Work by a new author. Assert successful response and durable author/Work with providers unavailable or unused; no enrichment completion or cover is required. Before Add, assert normal startup established the required partial unique index (unique, columns user_id/normalized_name, predicate normalized_name IS NOT NULL). No helper-created index or direct author seed may make this pass.
- [ ] **AC-002** (REQ-001, REQ-003): Restart the normally initialized database and demonstrate author creation remains available; repeats do not damage existing records or break compatibility checks. Through the real author creation/adoption writer, a second same-user canonical key adopts the existing author (created=false, same id, one row); do not prove this solely through an earlier exact-name service lookup that bypasses the index-dependent writer.
- [ ] **AC-003** (REQ-002): Upgrade both already-indexed and unindexed clean existing databases through the production migration/setup authority. Existing author IDs/keys and representative relationships remain unchanged; the real writer accepts a new author. Include null normalized keys and separate-user same-key cases without inventing new canonicalization policy.
- [ ] **AC-004** (REQ-002): On an old unindexed database with duplicate non-null author keys for one user, the real server exits without serving requests, with a useful uniqueness/index error. Those authors/links and previously recorded successful migrations remain unchanged, and the new repair is not recorded as successful. No automatic merge/backfill runs.
- [ ] **AC-005** (REQ-003): Applied old migrations stay byte-identical. Existing schema/data compatibility guards still accept supported state and reject newer incompatible state. Required format, full-workspace clippy and cargo tests pass; the new regression is registered and committed.
