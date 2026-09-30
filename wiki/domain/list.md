# List import

Lists are import sessions, not persistent reading collections. Supported source
families include Goodreads CSV, OpenLibrary and Hardcover.

Preview parses source rows and proposes matches without adding Works. Confirmation
processes selected rows through the shared creation/identity path. Each row may
succeed or fail independently; report partial results honestly. Track the precise
Work and import association, not whichever Work was most recently inserted.

Undo is scoped to the authenticated user's import and the documented deletion
authority. Import ID alone is not proof of user ownership. Do not infer undo or
merge availability from old service inventories; consult current feature state.

Undo also cleans up authors (since 2026-09-30, `c48f9b02`). Before the works are
deleted it collects their authors (both author columns and contributors); after
they are gone it deletes those that are empty and untouched, owned by the user
and added at or after the import's `started_at` (`julianday` comparison). List
imports write no author mark, so the rule works for imports made before the fix.
Kept: authors that existed before the import, that still have a work, or that the
user monitors or edited. The keep-checks are one shared definition with the Readarr
undo (`crates/livrarr-db/src/sqlite_import.rs`). A failed author lookup stops undo
before anything is removed. A failed delete after the works are gone is logged and
undo completes; leftover empty authors are deleted by hand (accepted limit, PO
2026-09-29, `build/reviews/small-bugs-pass/packet-2-code/DECISION.md`).
Source metadata seeds the Work and must preserve explicit choices; it does not
replace Livrarr's own enrichment policy.

[Creation](../architecture/work-creation-pipeline.md) ·
[identity/test lessons](../insights/tests-and-fixtures.md).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/domain/list.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="list"></a>
<a id="what-lists-are"></a>
<a id="lifecycle"></a>
<a id="key-properties"></a>
