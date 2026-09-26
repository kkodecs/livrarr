# Test doubles and real entry paths

Persistence tests use real SQLite with migrations, not an in-memory imitation of
the database. Fakes previously diverged on joins, foreign keys, nulls, transactions,
ordering and collation. Stub external dependencies independently.

The standard `create_test_db()` fixture is an in-memory, single-connection database.
It cannot reproduce production WAL or multi-connection writer contention. Tests
requiring those semantics need a suitably configured real database fixture. A read
after taking the only connection’s writer can deadlock; fetch required facts before
opening that transaction.

Activate the actual boundary under test. A preinstalled identity repository hides
installation failures, and a service call does not cover a different handler path.
Drive the real router/middleware, repository writer, adapter or component with
controlled external seams. Assert the acceptance criterion’s observables rather
than unrelated whole-state snapshots. Explicitly named no-write table sets are a
different, justified contract.

Register and track test files and compile-time fixtures together. Verify feature
requirements and ignored tests: file presence, compilation and execution are three
different facts. Process-global queues/hooks need one shared test guard; local
fixtures can reuse numeric IDs and still interfere through global state.

Bound waits, clean up blocking workers and join them before assertions. Build
asynchronous SQLite fixtures before pausing Tokio’s clock. These details and the
recent identity fixture constraints live in [test lessons](../insights/tests-and-fixtures.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/patterns/test-doubles.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="test-doubles-pattern"></a>
<a id="no-in-memory-db"></a>
<a id="test-db-helper"></a>
<a id="what-gets-stubbed"></a>
<a id="test-db-principle"></a>
