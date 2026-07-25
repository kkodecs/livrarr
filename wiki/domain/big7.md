# BIG7 Entities

The seven core media entities in Livrarr. These are the only media entities that matter for core workflows.

| Entity | What it represents | Scope |
|--------|-------------------|-------|
| **Author** | A person who writes books | User-scoped |
| **Series** | An ordered collection of works | User-scoped |
| **Work** | A title — the primary entity (Principle 1) | User-scoped |
| **Release** | A specific edition/format available for download | Not persisted |
| **Grab** | A download action for a release | User-scoped |
| **LibraryItem** | A file on disk in the organized library | User-scoped |
| **List** | A user-curated collection of works | User-scoped |

## Key Relationships

```
Author ──< Work ──< Release
  │          │         │
  │          │         └──> Grab (user-scoped)
  │          │                │
  │          │                └──> LibraryItem (user-scoped, file on disk)
  │          │
  │          └──< Series membership (many-to-many)
  │
  └──< Author monitoring (background)

List ──< Work (many-to-many)
```

## Scoping Rules

- **User-scoped — six of the seven.** Author, Series, Work, Grab, LibraryItem and List each
  carry a `user_id` on the row: `crates/livrarr-domain/src/entities.rs:436` (Author), `:456`
  (Series), `:378` (Work), `:577` (Grab), `:475` (LibraryItem), `:643` (Import — the list
  session). Reads and writes are fenced by it, proven per entity in
  `crates/livrarr-db/src/cross_user_isolation_tests.rs`: works `:388`, authors `:488`, series
  `:607`, grabs `:513`, library items `:735`, imports `:675`. One user cannot fetch another's
  work at all (`:376`).
- **Not persisted:** Release — a transient indexer search result, never written to the
  database (`crates/livrarr-domain/src/services/release.rs:67-78`), so nothing scopes it.
- **Infrastructure (admin-only):** Root folders, download clients, indexers, remote path mappings

No unscoped queries on user-scoped tables. Ever. (Principle 4)
