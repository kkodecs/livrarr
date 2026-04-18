# BIG7 Entities

The seven core media entities in Livrarr. These are the only media entities that matter for core workflows.

| Entity | What it represents | Scope |
|--------|-------------------|-------|
| **Author** | A person who writes books | Global (shared) |
| **Series** | An ordered collection of works | Global (shared) |
| **Work** | A title — the primary entity (Principle 1) | Global (shared) |
| **Release** | A specific edition/format available for download | Global |
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

- **Global shared:** Author, Series, Work, Release — all users see the same data
- **User-scoped:** Grab, LibraryItem, List — filtered by user_id, always
- **Infrastructure (admin-only):** Root folders, download clients, indexers, remote path mappings

No unscoped queries on user-scoped tables. Ever. (Principle 4)
