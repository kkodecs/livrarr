# Architecture review source baseline — 2026-09-07

**Prepared explanation:** a conceptual map for the guided review, not yet discussed or accepted by the user.

The reviewed source is `/mnt/opt/livrarr-worktrees/merge-containment` at `af865bdb76d3cd1e42302f9791830479a26220f5`, including its reviewed uncommitted containment patch. [PM evidence](../../build/reviews/architecture-review-2026-09-07/system-map-006/EVIDENCE.json) records 22 matching patch-manifest files and the crate manifests. These facts do not establish equivalence with the parked `/mnt/opt/livrarr` source.

Crate names below omit `livrarr-`:

| Responsibility | Main pieces |
| --- | --- |
| Book records and knowledge | metadata, identity, matching, enrichment, external-data |
| Finding and obtaining copies | download: releases and grabs |
| Organized files, covers and embedded metadata | library, materialize, tagwrite |
| Reading and listening | frontend, handlers, library |
| Web/server coordination | server connects implementations and background work |
| Shared foundations | domain, db, http |

Jobs, cli and behavioral are additional workspace members. There are **17 members**; server declares **13 production workspace dependencies**, including identity and jobs.

Two documentation discrepancies remain: [CLAUDE.md](../../CLAUDE.md) says 10 crates; the [older source overview](/mnt/opt/livrarr-worktrees/merge-containment/wiki/architecture/overview.md) says 11 server dependencies and excludes identity/jobs. Dependency declarations do not prove jobs runtime wiring. These discrepancies do not establish excess complexity.

No use-case trace, runtime check, completed review section or implementation authorization is implied.

## Boundary update — 2026-09-07

**Status correction:** the map and book-record explanation have been delivered. The earlier preparation label describes their previous status; “ok continue” permitted discussion, not architecture approval.

[Targeted containment evidence](../../build/reviews/architecture-review-2026-09-07/boundary-008/EVIDENCE.json) shows matching and identity share one domain comparator and repeat title/author/no-conflicting-key acceptance. Identity also handles conflicts and interaction; they are not separate comparison engines.

Enrichment merging retains populated title, author and language. Its database writer accepts carried title/author, so protection is upstream. Generation-checked provider-reference handoffs settle against the existing book; the candidate retains its identity core. Settlement includes group reconciliation; containment sends multi-work automatic merges to review.

This was not an all-writer/runtime audit; dangerous live reidentification is not established. Open: who owns match acceptance, and what identity effects may enrichment have?

**Proposal only:** enrich descriptions and propose same-book links; changing the selected book requires explicit user action. Consider a narrower interface. No implementation approval, completed section or review time is recorded.
