# SQLite Migration Pattern

## Rules

1. **Never edit applied migrations.** sqlx checksum validation fails if you modify already-applied migration files. Always create new migrations.
2. **Migrations run at startup.** Embedded in the binary via sqlx. Fatal on failure.
3. **Pre-migration backup:** `VACUUM INTO 'livrarr.db.pre-migrate-YYYYMMDD-HHMMSS'` — timestamp only, no version component. Fatal if the backup fails. Skipped when the DB file does not yet exist (fresh install), **not** when there is nothing to migrate.
4. **Backup retention:** keep the 3 most recent. Selection is by filename prefix (`livrarr.db.pre-migrate-`) sorted lexicographically, which is chronological because the names are timestamps.
5. **Each migration in a transaction** where possible. Exceptions must document recovery procedure.

## Naming

Files in `crates/livrarr-db/migrations/`. Format: `NNN_description.sql` (e.g., `021_add_library_item_imported_at.sql`).

## Constraints

- `INSERT OR REPLACE` is banned (it's DELETE + INSERT in SQLite — changes rowid, cascades FK deletes). Use `INSERT ... ON CONFLICT (...) DO UPDATE SET ...`.
- No `CHECK` constraints for enum columns (altering CHECK requires full table rebuild — impractical on Pi).
- `NOT NULL`, `UNIQUE`, and FK constraints are encouraged.

## Enum Serialization in DB

- **Single-word variants:** `lowercase` (e.g., `enriched`, `failed`)
- **Multi-word variants:** `snake_case` (e.g., `permanent_failure`, `will_retry`)
- Rationale: lowercase collapses word boundaries in multi-word values, hurting readability

## Connection Pragmas (every connection)

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
PRAGMA foreign_keys = ON;
PRAGMA journal_size_limit = 67108864;
PRAGMA wal_autocheckpoint = 1000;
```

`foreign_keys` and `busy_timeout` are per-connection — must be in pool builder config.
