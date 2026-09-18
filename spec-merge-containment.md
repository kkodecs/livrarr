---
feature: merge-containment
stage: spec
status: approved
version: 1
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004]
---

# Spec: merge containment

## 0a. Design Principles

Preserve books and files; refuse unavailable operations explicitly. Keep normal creation, unambiguous attachment, import and consumption. This is temporary containment authorized by the PO on 2026-09-07, not completion of the stopped identity-conflict-authority feature. Preserve its source and evidence separately.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | pre-containment-source/RECEIPT.json in the original checkout's build/reviews/merge-containment; isolated git HEAD af865bdb | 1082 files from the dirty checkout are snapshotted and verified; the containment checkout starts before the uncommitted merge framework and migrations. | Editing or discarding the original work to make containment compile. | high |
| ST-002 | Serena reads of identity_road::settle, require_continuation, WorkIdentityRepository::commit_review_continuation and absorb_work_into at af865bdb | Manual merges, automatic settlement and GroupIdentity review can combine or rewrite existing books; one-member GroupIdentity actions can rewrite the existing book too. | UI-only restrictions or allowing DifferentFromAll merely because only one member is named. | high |
| ST-003 | WorkDetailPage.tsx and ReviewPage.tsx at af865bdb | The page offers Merge Duplicate and GroupIdentity actions; PendingRoute also has a working independent Link action. | Removing all review functionality or disabling unambiguous import attachments. | high |
| ST-004 | Original testdata database read-only census, 2026-09-07 | The inspected development DB has zero legacy merge archives and retains pending GroupIdentity and PendingRoute cards; no new merge-schema tables were observed. | Treating that observation as proof about other installations or running migrations on the live DB. | high |

No external provider protocol changes are required.

## 0c. Prior Art

| ID | Artifact | Bearing on this feature |
|----|----------|-------------------------|
| PA-001 | /mnt/opt/livrarr/spec-identity-conflict-authority.md, problem statement | Records destructive GroupIdentity decisions and unsafe merging; full remediation is stopped. |
| PA-002 | /mnt/opt/livrarr/ir-v1-identity-conflict-authority.yaml, build_units | The partial new foundation is not releasable alone; containment uses an isolated earlier baseline. |
| PA-003 | tests/behavioral/test_irf_u1_refusal.rs | Existing real-router/authentication/SQLite harness and typed refusals; old assertions deliberately preserve merge behavior and must change where this PO decision supersedes them. |
| PA-004 | tests/behavioral/test_irf_u2_import_defer.rs | Real import tests establish unambiguous attachment and safe deferral with original files retained. |

## 1. Problem Statement

The PO stopped the merge project and approved temporarily disabling combining existing books and its unfinished dependent actions. Current UI and backend paths still permit those operations. Reproduction: two stored books can enter the merge preview/execute routes; GroupIdentity review offers and executes merge or re-identification actions.

## 2. Requirements

- **REQ-001**: Explicitly refuse work merge previews/execution, GroupIdentity resolution, and automatic absorption without changing stored books, files or existing review/history records. Both authenticated HTTP and direct repository entry points are covered; authentication and user ownership remain enforced. PendingRoute resolution and ordinary dismissal remain available.
- **REQ-002**: Keep automatic ordinary creation and unambiguous attachment; a cohort requiring absorption must remain reviewable or safely deferred with source files preserved. Identity-changing edits requiring GroupIdentity are refused before creating a card; other work edits remain available.
- **REQ-003**: Remove active work-merge controls and GroupIdentity confirmation controls; explain temporary unavailability while preserving visible pending cases, history and independent safe actions.
- **REQ-004**: Deliver in an isolated checkout, preserve all stopped work, and validate normal search/add/download/import/read/listen paths using existing meaningful tests and an isolated smoke environment. No live library mutation, replacement or release is part of the implementation step.

## 3. UI/Interface Design

Replace the merge affordance with unobtrusive unavailable text. GroupIdentity cards remain visible with Dismiss and an explanation; PendingRoute retains Link it. No new workflow, screen or setting.

## 4. Non-Requirements

No merge execution, undo implementation, new migration, new metadata matching design, repair of unrelated backlog, deletion of pending records, or deployment over the existing instance. Author deduplication is outside the stopped work-merge scope.

## 5. Open Questions

None for containment implementation; baseline validation may expose unrelated blockers, which must be reported rather than absorbed into a rewrite.

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Real authenticated router calls to merge preview/execute and both review aliases explicitly refuse with unchanged books, routes, review cards, history and linked files; unauthorized calls remain unauthorized.
- [ ] **AC-002** (REQ-001): Direct real SQLite settlement/continuation calls cannot absorb or use GroupIdentity to rewrite a book; no partial writes commit.
- [ ] **AC-003** (REQ-002): Ordinary new-book creation and one-book attachment succeed; multi-book absorption candidates stay unresolved without losing source files or changing existing identities.
- [ ] **AC-004** (REQ-003): Rendered work/review pages offer no active work merge or GroupIdentity confirmation; pending cases and safe PendingRoute/Dismiss actions remain visible.
- [ ] **AC-005** (REQ-004): Preservation hashes match and all changed-scope checks pass; broad validation failures are recorded with baseline attribution rather than waived or hidden.
