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
