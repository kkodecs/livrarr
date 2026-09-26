# Database ownership

`livrarr-db` owns SQL, migrations, persistence implementations and atomic database
transitions. SqliteDb implements repository contracts; tests use real SQLite.
[Migration pattern](../patterns/migration-pattern.md) ·
[test database limitations](../patterns/test-doubles.md).

## Start here

| Concern | Navigation |
|---|---|
| Pool, writer admission and startup backfills | `pool.rs` |
| Concrete database handle | `sqlite.rs` |
| Persistence APIs and requests | `api/` and corresponding domain contracts |
| Work, Author, file and import persistence | owning `sqlite_*` modules |
| Captured identity settlement and review | `identity_layer.rs`; later unfinished authority work is separate |
| Schema evolution | `migrations/`, embedded startup migration owner |
| Isolation evidence | `cross_user_isolation_tests.rs` and real-route tests |

Resolve actual symbols and callers with Serena. Module and method counts change;
the old handwritten API inventory is preserved as history rather than maintained
as a second contract.

## Transaction and ownership rules

Production is WAL with four connections and one SQLite writer. All write-bearing
transactions enter through the shared BEGIN IMMEDIATE authority, with foreign keys
and busy timeout configured per connection. Network and file operations stay outside
transactions. Ordinary operations are short; startup cutover/backfills may need an
explicitly atomic larger transaction before serving.

User-scoped mutations and reads prove ownership. Global infrastructure and a
scheduler’s cross-user enumeration are distinct contracts, not a blanket bypass.
Import ID, numeric Work ID, a shared cache, or a displayed queue row alone cannot
prove the caller owns a mutation.

Applied migrations are immutable. Use explicit ON CONFLICT updates rather than
INSERT OR REPLACE. Enums/discriminators need one explicit codec; JSON-quoting a TEXT
code is a different value. Do not reconstruct schema facts from only the first and
last migration in a range.

Identity generation claims, audits and Work birth events belong at their actual
transactional moments. Descriptive merge must not write identity on the side.
Retained merge history has independent dependency and mutation guards, including
empty inventories; that design’s acceptance is tracked in project state.

## Fixtures

The standard in-memory database is single-connection and cannot exercise WAL or
multi-connection contention. A read after opening its only writer can deadlock.
Tests for activation must distinguish migrated, identity-ready and installed
repository states; convenient preinstalled helpers can conceal the boundary.
[Fixture lessons](../insights/tests-and-fixtures.md#lesson-102).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/crates/db.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="livrarr-db"></a>
<a id="db-traits"></a>
<a id="userdb"></a>
<a id="sessiondb"></a>
<a id="workdb"></a>
<a id="workdbcreate"></a>
<a id="authordb"></a>
<a id="libraryitemdb"></a>
<a id="rootfolderdb"></a>
<a id="grabdb"></a>
<a id="downloadclientdb"></a>
<a id="remotepathmappingdb"></a>
<a id="historydb"></a>
<a id="notificationdb"></a>
<a id="configdb"></a>
<a id="enrichmentretrydb"></a>
<a id="indexerdb"></a>
<a id="authorbibliographydb"></a>
<a id="seriesdb"></a>
<a id="seriescachedb"></a>
<a id="importdb"></a>
<a id="playbackprogressdb"></a>
<a id="listimportdb"></a>
<a id="provenancedb"></a>
<a id="providerretrystatedb"></a>
<a id="externaliddb"></a>
<a id="db-requestresponse-structs"></a>
<a id="usersession"></a>
<a id="work"></a>
<a id="author"></a>
<a id="libraryitem"></a>
<a id="grab"></a>
<a id="download-client"></a>
<a id="historynotification"></a>
<a id="config"></a>
<a id="indexer"></a>
<a id="bibliographyseries-cache"></a>
<a id="series"></a>
<a id="import"></a>
<a id="list-import"></a>
<a id="external-ids"></a>
<a id="provenance"></a>
<a id="provider-retry-state"></a>
<a id="enrichment-merge"></a>
<a id="test-helpers"></a>
