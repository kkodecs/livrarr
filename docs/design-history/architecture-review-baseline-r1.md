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
