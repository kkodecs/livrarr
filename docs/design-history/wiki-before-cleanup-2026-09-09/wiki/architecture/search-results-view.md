# Search-results rendering

Start at [OlResult](/mnt/opt/livrarr-worktrees/merge-containment/frontend/src/pages/search/SearchPage.tsx:382)
in `frontend/src/pages/search/SearchPage.tsx`. It receives one `WorkSearchResult`
and an `onAdd` callback. It renders a cover (or `?` placeholder), title, conditional
series and provider labels, author, conditional rating, and an Add button.

The page requests results through `lookupWorks(query, selectedLang, showRaw)` at
line 95 and reads the response's `results` at line 101. The [list mapping](/mnt/opt/livrarr-worktrees/merge-containment/frontend/src/pages/search/SearchPage.tsx:288)
creates an `OlResult` for each `filteredOlResults` entry. Its supplied callback
calls `addWorkWithCover(work, work.coverUrl, false)`; the row's button invokes
`onAdd` at line 423. The row handles presentation and delegates the Add action.

In the review browser, use Ctrl+P to open the file and Ctrl+G 382 for the row,
or Ctrl+G 288 for its caller.

Verified 2026-09-08 against the reviewed containment worktree; [source evidence](../../build/reviews/architecture-review-2026-09-07/handoff-016/SEARCH-DISPLAY-EVIDENCE.json)
records the hash and scoped excerpts. This is static display-path inspection.
The full add helper and frontend-to-backend wiring remain to inspect; no application
search or Add operation was executed for this check.
