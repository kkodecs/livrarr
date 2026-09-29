# Upgrading from alpha6

## Executive summary

Starting a newer build on an existing alpha6 library upgrades it automatically. There are no
commands to run and nothing is asked. Before anything changes, startup saves one copy of the
database beside it in `/config`; that copy is the only way back to alpha6, because alpha6 cannot
open an upgraded database. Keep it until you are sure the upgrade is good.

- [What the upgrade does](#what-the-upgrade-does): converts the library in place and keeps the
  user's confirmations, status and audiobook cover choices.
- [Backup and rollback](#backup-and-rollback): one protected copy per upgrade, not one per start,
  and the four rollback steps.
- [What the log lines mean](#what-the-log-lines-mean): the backup, summary and refusal messages.
- [Named limits](#named-limits): cases the upgrade does not cover.
- [Tested and not yet tested](#tested-and-not-yet-tested): the release-image acceptance, including
  an actual rollback to alpha6, has not run yet.

Sources: [plan r2](../../build/reports/identity-cutover-inplace-plan-2026-09-27.md) (sections 5
and 8), [bug spec](../../spec-identity-upgrade-inplace.md), and the as-built reports for
[migration 091](../../build/reviews/identity-upgrade-inplace/packet-2-migration-091/REPORT.md)
and [startup backup](../../build/reviews/identity-upgrade-inplace/packet-3-startup/REPORT.md).

## What the upgrade does

The starting condition is an alpha6 library (database migration 073) with books in it. The user
replaces the alpha6 image with a newer one and starts it. No book is added or imported.

1. **Backup.** Startup copies the database before any migration or repair runs
   ([backup](#backup-and-rollback)).
2. **Migrations.** The pending migrations run. The last one, 091, converts the library to the new
   identity layer in place.
3. **Switch-over.** The existing startup check activates the new identity layer and logs one
   summary line.

What carries over: titles, authors, series, identifiers, monitoring, covers and edits. A match the
user picked in alpha6 counts as confirmed, and those books show the green "Confirmed" badge. An
alpha6 audiobook cover choice, which two earlier migrations would otherwise drop, is saved before
them and put back by 091.

Odd cases never stop startup and are never asked about. Each one is written to the audit log and
counted in the summary line:

- **Look-alike books** stay separate; nothing is merged.
- **A shared identifier** stays with the first book; the others do not get it.
- **Old unanswered match questions** are closed, keeping the current match.

One visible difference: a book alpha6 flagged "needs review" now shows "Confirmed" if it has an
identifier. The badge has only Pending and Confirmed.

A fresh install and an already-upgraded library skip the conversion and take no copy.

## Backup and rollback

**One copy per upgrade.** The copy is named `livrarr.db.pre-migrate-v<NNN>-<UTC>`, where `<NNN>`
is the migration the database was at and `<UTC>` is `YYYYMMDD-HHMMSS`. From alpha6 it looks like
`livrarr.db.pre-migrate-v073-20260928-191808`. It is written first as a `.partial` file and then
renamed, so a half-written copy is never mistaken for a finished one.

A restart during an unfinished upgrade, including a Docker restart loop after a failure, reuses
the same copy and adds none. The copy is protected from clean-up until the next upgrade finishes.
Other `pre-migrate` copies are trimmed to the newest three.

**When an upgrade counts as finished.** Only after every startup repair has completed, including
the Goodreads cover repair that runs in the background after the server starts. In practice that
is one start after the background pass ends. Until then the upgrade stays "in progress" and its
copy stays protected.

**Rolling back to alpha6.**

1. Stop the container.
2. In the `/config` folder, move `livrarr.db` aside, then copy the `livrarr.db.pre-migrate-v073-…`
   file to `livrarr.db`.
3. Delete `livrarr.db-wal` and `livrarr.db-shm` if they exist. Left in place, they belong to the
   upgraded database, not the copy.
4. Set the image back to alpha6 and start it.

Changes made after the upgrade are not in the copy. If you restore, edit, and upgrade again, that
upgrade takes a new copy containing the edit.

## What the log lines mean

| Log line | Meaning |
|---|---|
| `pre-upgrade backup: livrarr.db.pre-migrate-v073-…` | A new upgrade started and its copy was saved. |
| `pre-upgrade copy already saved: <file>` | A restart during an unfinished upgrade; the existing copy is reused. |
| `pre-upgrade copy retaken: <file>` | A restart before the upgrade had changed anything; the copy was rewritten in place. |
| `reclaimed unrecorded pre-upgrade copy <file>` | An earlier start was interrupted after writing a copy but before recording it; that copy was removed. |
| `removed stale partial copy <file>` | A half-written copy from an interrupted start was removed. |
| `Database migrations complete` | All migrations, including the conversion, are applied. |
| `Identity upgrade complete: 139 books, 604 identifiers (299 you had confirmed); 0 look-alike books kept separate; 0 shared identifiers; 0 old questions closed` | The conversion was activated. Logged once, on the start that activates it. The numbers are this library's. |
| `upgrade finished; rollback copy: <file>` | Every startup repair has finished; this copy is now the rollback copy. Appears on a later start, not the first. |
| `Pre-upgrade copy failed: the original rollback copy <file> is unavailable: …` | See [the missing-copy limit](#named-limits). Startup stops. |
| `Migration failed: …` | A migration failed. Earlier migrations stay applied; the failed one is undone. Restarts reuse the same copy. |

## Named limits

- **Missing copy after progress.** If the upgrade has already changed the database and its copy is
  gone, every start is refused with the "original rollback copy … is unavailable" message. The only
  way forward is to put that file back in the data folder. This is deliberate: the database is never
  saved again under the original's name once it has changed.
- **Development databases.** A database that passed migration 084 before this change cannot get
  its alpha6 audiobook cover choice back.
- **The command-line path takes no copy.** `livrarr … identity-cutover` runs the same migrations,
  and so the same conversion, without a backup.
- **Offline cover repair.** If the Goodreads cover repair never finishes (for example, the server
  stays offline), the upgrade stays in progress. A later release then reuses this copy instead of
  taking a new one, so a rollback returns to the state before this upgrade.
- **Out of scope.** A bad legacy author key still refuses startup ("author-link cutover
  incomplete"). That problem is separate from this upgrade.
- **For developers.** A new startup repair gated on a marker must be added to the repair list in
  `crates/livrarr-db/src/upgrade_backup.rs`, or its release takes no automatic copy.

## Tested and not yet tested

Tested: the conversion on a copy of a real 139-book alpha6 library, run with the real binary. It
matched the manual conversion row for row, plus the confirmations. The backup behaviour on that
copy: one file with an unchanged checksum across restarts and refused starts, and one summary
line.

Not yet tested: the release image in Docker, a 10-minute restart loop, and an actual rollback to
alpha6 from the copy. That alpha6 refuses an upgraded database is read from the source, not run.
These checks are the planned acceptance step.
