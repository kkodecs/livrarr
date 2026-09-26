---
feature: readarr-independent-copies
stage: spec
status: approved-scope
version: 2
req_ids: [REQ-001, REQ-002, REQ-003]
---

# Bug Spec: Readarr independent copies

## 0a. Design Principles

Preserve the source and existing library contents. Every successful Copy import leaves independent library storage. Keep the shared importer as the single owner of materialization and collision checks. The user confirmed this policy and accepted fixing new Readarr imports and retries on September 18, 2026. Previously imported library-wide inventory/repair and deployment are separate work.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | build/reviews/wiki-follow-ups-2026-09-18/readarr-copy/REVIEW.md; 736 source hashes | Reviewed baseline is af865bdb plus containment patch, isolated from parked merge work. | Building the parked checkout or dropping containment. | high |
| ST-002 | Serena ImportRunner::process_files and ImportWorkflowImpl::import_file_locked, September 18 | Readarr requests HardlinkFirst; existing-target skip/adoption runs before materialization. | Treating a mode-only change as sufficient retry coverage. | high |
| ST-003 | Shared importer and standard filesystem hardlinks; reproduction will use temporary files on one filesystem | Multiple hardlink names may share mutable bytes. A copy must protect the original even when hardlinks are available. | Passing only a cross-device test where hardlink creation already fails. | high |
| ST-004 | Shared importer existing-row/adoption branches | A matching row skips; an unrecorded size-matching target is adopted; collisions must retain existing contents and ownership. | Replacing an existing target with source bytes on retry or detaching a target owned by another Work. | high |

| ST-005 | Existing Readarr adapter and September 18 call trace | Identifier namespaces/fetchability are unchanged; no new external identifier is produced or interpreted by this correction. | Inferring new provider semantics from the local fixture. | high |
| ST-006 | Existing RdBook/RdBookFile parsers, unchanged by this fix | Payload shapes remain those accepted by the existing adapter; fixture only supplies enough existing fields to drive materialization. No new real-provider capture was made. | Claiming new external compatibility from this filesystem regression. | high |
| ST-007 | connect_readarr_verified / ReadarrClient::get | Existing origin admission and no-redirect transport remain on the tested production path; tests approve their isolated local HTTP fixture. | Treating a mocked import outcome as a filesystem test. | high |
| ST-008 | ReadarrClient::get / verify_protocol | Provider status/protocol rejection remains unchanged. Live anti-bot response shapes were not sampled for this local fix. | Silently redefining provider block handling. | high |

No external provider parsing, identifier namespace, transport or endpoint changes. Readarr HTTP fixtures exercise existing adapters only; this fix changes local file materialization.

## 0c. Prior Art

Searched current docs/ and wiki/ for Readarr, hardlink, materialization and adoption; inspected the prior source finding and current import guide. Existing session evidence establishes the policy; no new provider or retrieval design.

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | ARCHITECTURE.md and CLAUDE.md hardlink policy | Library imports copy; downstream CWA hardlink-first is distinct. |
| PA-002 | wiki/architecture/import-pipeline.md | Records confirmed Readarr mismatch, shared core ownership, collision and recovery obligations. |
| PA-003 | build/reviews/wiki-follow-ups-2026-09-18/readarr-copy/REVIEW.md | Verified call chain and existing-target retry gap; no real library inventory. |
| PA-004 | tests/behavioral/test_consolidation_import_workflow.rs; tests/behavioral/test_irf_u2_import_defer.rs | Real SQLite/file core coverage and reusable production-router/authentication harness. |

## 1. Problem Statement

A Readarr import on a filesystem that supports hardlinks requests shared storage. Editing one name can change the other. Changing only its materialization mode leaves retry paths able to retain previously linked targets. Reproduce through the real authenticated Readarr start route, a local Readarr HTTP fixture, real SQLite and temporary source/library roots on the same filesystem.

## 2. Requirements

- **REQ-001**: A new successful Readarr file import creates independent storage with the source contents unchanged.
- **REQ-002**: A Copy retry that accepts an existing target, whether already recorded or orphaned, leaves independent storage and preserves the existing target's contents. Both AlreadyImported and orphan-adoption paths are in scope. A failure must be explicit and must not destroy either file; an interrupted retry must leave complete old or complete new library contents, never a partially rewritten target. Existing independent files remain usable; duplicate retries do not create duplicate rows.
- **REQ-003**: Preserve collision ownership, missing-source/error reporting, unrelated materialization modes and crash recovery. Validate against the isolated containment baseline; make no real-library repair or deployment.

## 3. Interface Design

No UI or external request-shape changes. Existing Readarr source-validation failures remain failures even on retry; a missing source does not prove the target has no other hardlinks.

## 4. Non-Requirements

No library-wide repair, new setting, migration, metadata/identity change, import undo change, CWA change, dependency or UI redesign. No guarantee about files never revisited by an import retry.

## 5. Open Questions

None for the agreed correction. Report baseline test failures separately from regressions.

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Real authenticated Readarr import completes and records its LibraryItem; source/destination are distinct files on a hardlink-capable filesystem, have equal initial contents, and an in-place destination edit leaves source bytes unchanged.
- [ ] **AC-002** (REQ-002): Retry with an orphan hardlinked target adopts independent storage; target contents and source contents are preserved, and later target edits cannot change the source.
- [ ] **AC-003** (REQ-002): Retry with a recorded hardlinked target preserves its row and current library contents while separating storage; an independent edited target is not overwritten from the source.
- [ ] **AC-004** (REQ-003): A target owned by another Work and a size-mismatched orphan remain collisions with unchanged bytes/ownership; existing recovery and other-mode tests retain their behavior.
