# Search-results rendering

## Executive summary

How one search result row is drawn and how its Add button reaches the add helper, read
from the source reviewed on 2026-09-08. The file inspected is the archived copy of
`frontend/src/pages/search/SearchPage.tsx` in the
[merge-containment source archive](../../build/reviews/merge-containment/pre-containment-source/)
(base `af865bdb`; the containment patch does not touch this file, and the archived copy's
SHA-256 `3cb8c506…` matches the [review evidence](../../build/reviews/architecture-review-2026-09-07/handoff-016/SEARCH-DISPLAY-EVIDENCE.json)).
Line numbers below refer to that archived copy, not to today's main. This is static
display-path inspection; see [limits](#limits).

## The row and its caller

Start at `OlResult` (line 382 of the archived
[SearchPage.tsx](../../build/reviews/merge-containment/pre-containment-source/), extracted
from `source.tar` at `frontend/src/pages/search/SearchPage.tsx`). It receives one
`WorkSearchResult` and an `onAdd` callback. It renders a cover (or `?` placeholder), title,
conditional series and provider labels, author, conditional rating, and an Add button.

The page requests results through `lookupWorks(query, selectedLang, showRaw)` at
line 95 and reads the response's `results` at line 101. The list mapping at line 288
creates an `OlResult` for each `filteredOlResults` entry. Its supplied callback
calls `addWorkWithCover(work, work.coverUrl, false)`; the row's button invokes
`onAdd` at line 423. The row handles presentation and delegates the Add action.

In the review browser, use Ctrl+P to open the file and Ctrl+G 382 for the row,
or Ctrl+G 288 for its caller.

## Limits

Verified 2026-09-08 against the reviewed containment worktree; the
[source evidence](../../build/reviews/architecture-review-2026-09-07/handoff-016/SEARCH-DISPLAY-EVIDENCE.json)
records the hash and scoped excerpts. The full add helper and frontend-to-backend wiring
remain to inspect; no application search or Add operation was executed for this check.
The worktree path the evidence names no longer exists; the archive above is the surviving copy.
