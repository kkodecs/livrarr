# BLOCKED — unpassable assertion in `collision_preview_unions_ledger_and_columns_without_cross_tenant_leakage`

## The contradiction

`tests/behavioral/test_identity_edit.rs:637` asserts the OK preview response for
**user B** (same value `"12345"`, different tenant) contains the owning work's id
nowhere in its serialized JSON:

```rust
assert!(!allowed.to_string().contains(&owner.to_string()));   // :637
```

- `owner` is the first work created in a fresh `create_test_db()` → its id is
  **`1`** (no migration inserts `works` rows; SQLite AUTOINCREMENT starts at 1 —
  verified against `crates/livrarr-db/migrations/*.sql`).
- The same test requires the response to be **certifiable** for user B:
  `assert!(!preview_id(&allowed).is_empty())` (`test_identity_edit.rs:635`).
- A certifiable preview response is contract-bound to carry the resolved record
  including the canonical value (`docs/design-identity-edit.md` §Preview 5:
  "Response: resolved record (title, author, year, language, cover_url,
  canonical value, slot)"; merge notes: "Preview response shape:
  `resolved.{title,author,slot,canonicalValue}`"), and the sibling test
  `gr_url_preview_returns_certified_record_and_opaque_token`
  (`test_identity_edit.rs:581`) pins `resolved.canonicalValue == "12345"` for the
  identical value.
- `"canonicalValue":"12345"` **contains the substring `"1"`** — the owner's id
  string. Line 637 therefore fails against ANY design-conformant response, and
  no implementation choice can satisfy 635 + the pinned response shape + 637
  simultaneously. (Independently, a hex/uuid `previewId` contains the digit `1`
  ~86% of the time, so the assert would be flaky even without `canonicalValue`.)

This is a latent defect in the merged suite: the gated file was verified only
compile-red (`docs/identity-edit-merge-notes.md` §Verification state), so this
green-time contradiction was never executed before now.

## Proposed reading

The assertion's intent (per the test doc-comment and design §Preview 4:
"Another user's id/title can never be returned") is that the **collision
payload** — the only place an owner id/title is ever emitted — is absent for a
cross-tenant holder. That is already covered by `test_identity_edit.rs:634`
(`collision` absent/null) and `:636` (owner title absent). The mechanically
correct form of line 637 is one of:

```rust
assert!(!allowed.to_string().contains("owningWorkId"));                     // (a) field-level
assert!(!allowed.to_string().contains(&format!("\"owningWorkId\":{owner}"))); // (b) value-level
```

Either preserves the leak check without matching the single character `1`
inside unrelated legal payload (`canonicalValue`, `previewId`, years).

## State

Everything else is green: with this one test excluded the gated suite is
**32/33 passed, 1 failed (this assertion)**; the durable suite is 10/10. The
implementation enforces the intended property — the collision object (the only
carrier of `owningWorkId`/`owningWorkTitle`) is emitted solely for same-user
owners (`crates/livrarr-metadata/src/work_service.rs::preview_identity_edit`,
user-filtered `find_anchor_owner`). Awaiting a ruling on the one-token
assertion fix; not applied unilaterally per the race packet's escalation rule.
