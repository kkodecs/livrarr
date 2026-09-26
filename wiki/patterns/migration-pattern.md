# SQLite migrations

Applied migrations are immutable. Add a new numbered SQL file under
`crates/livrarr-db/migrations/`; never edit a shipped file to repair history.
Migrations are embedded and run before serving; failure stops startup.

The documented backup protocol uses VACUUM INTO before migrations for an existing
database and keeps three timestamp-named backups. A fresh database has no prior
file to back up. Verify backup, migrations, version checks and identity readiness
in their real boot order; a partial migration test is not a startup test.

Use transactions where possible. Exceptions must explain recovery. All ordinary
write-bearing transactions use the shared BEGIN IMMEDIATE authority, remain short
and contain database work only. Production connection policy includes WAL,
synchronous=NORMAL, busy_timeout=5000, foreign_keys=ON, a 64MiB journal size limit
and wal_autocheckpoint=1000. Foreign keys and busy timeout are per connection.

Avoid INSERT OR REPLACE; it is DELETE plus INSERT and can cascade data loss. Use
explicit ON CONFLICT updates. NOT NULL, UNIQUE and foreign keys express invariants;
the existing convention avoids enum CHECK constraints that require table rebuilds
to evolve.

Persist enum/discriminator codes with an explicit shared codec: lowercase for simple
names and snake_case for multiword names unless an existing compatibility contract
says otherwise. serde JSON string output includes quotes and is not plain SQL TEXT.

Schema writer, migration report and binary version guard must agree. Prove current
schema facts by the complete migration sequence, not selected endpoints. Nonempty
clean databases must pass readiness; collisions need actual resolvable artifacts.
[Cutover lessons](../insights/history-and-review.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/patterns/migration-pattern.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="sqlite-migration-pattern"></a>
<a id="rules"></a>
<a id="naming"></a>
<a id="constraints"></a>
<a id="enum-serialization-in-db"></a>
<a id="connection-pragmas-every-connection"></a>
