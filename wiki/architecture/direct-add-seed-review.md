# Direct Add: seed persistence and test coverage

## Executive summary

Creating a new Work can discard supplied language, year and cover information.
This is the catalogue Add operation, not importing an ebook or audiobook file.
The isolated test reproduced it; a normal browser Add may later fill the gaps
from online data. See [cause and scope](#cause-and-scope),
[browser inspection and repeated Add](#browser-inspection-and-repeated-add),
and [the proposed correction](#constraints-on-the-proposed-correction).

## Confirmed result

The direct Add route drops supplied language, publication year and cover information
before creating a new Work. The September 9 static finding was reproduced through
the real authenticated route on September 18, using the deployed source in an
isolated test database. No fix is implemented.

A request containing French, 1943 and a cover URL succeeded, but language/year were
NULL both immediately and after background completion; the supplied cover was never
requested. Ordinary search-cover and explicit manual-cover requests reproduced the
same loss when no cached candidate or successful provider enrichment was available.
An existing-book control retained its saved metadata and returned the same Work.

## Cause and scope

The search page sends these values, and the handler builds `seed_add_box` from them.
It then passes only title/author identity and provider routes into settlement.
The settlement transaction has no descriptive-seed input. Background completion
receives the Work ID and optional candidate ID, so it cannot recover the discarded
request values. Successful cached/provider data may later hide the loss; this does
not prove preservation of the original selection.

The current search-card Add uses `coverManual=false`: its image is an automatic
cover candidate. An explicit manual choice is a separate API capability. A fix must
carry both meanings correctly without locking every search-result image.

## Constraints on the proposed correction

Keep the identity road as the sole creator. Persist the new Work's supplied scalar
metadata coherently with its creation; do not overwrite an existing dedup winner or
emit a second birth event. Carry cover URL/intent into the existing asynchronous
cover writer and account for interruption. A manual lock is earned only after image
bytes are saved; failed downloads must remain repairable.

The old `finish_created_work_fast` is not a drop-in solution: it records birth history
and attempts a synchronous cover download. Preserve the fast Add response and the
independent author follow-ups. Capture/handoff failures and enrichment failures have
different continuations, and each capture needs its own current identity generation.

The existing selected-cover service test does not drive this HTTP handler. Keep
real-router coverage with authentication, real SQLite and controlled external seams.
The new investigation establishes this failure case, not every cached-candidate,
provider-success, restart or cover-success branch.

[September 18 reproduction, evidence and proposed scope](../../build/reviews/wiki-follow-ups-2026-09-18/post-deploy/add-investigation/REPORT.md) ·
[September 9 static review](../../build/reviews/codebase-improvement-review-2026-09-09/focus-002/REVIEW.md) ·
[exact earlier guide](../../docs/design-history/wiki-followup-verification-2026-09-18/wiki/architecture/direct-add-seed-review.md).

## Browser inspection and repeated Add

[Browser steps](../../build/reviews/wiki-follow-ups-2026-09-18/post-deploy/add-investigation/REPORT.md#inspect-add-in-your-browser)
compare the Add request with its initial response. They do not establish a lasting
failure after successful enrichment, and have not been run against the live UI.

Search filters recognised existing Works. A repeat request reaching the backend
returns the existing Work with `created=false` and skips new-Work completion.
Author handling runs first and can enqueue linking work or attach a supplied
author route, so this is not a guaranteed no-change operation.
[Current behavior, rationale and discussion proposal](../../build/reviews/wiki-follow-ups-2026-09-18/post-deploy/add-investigation/REPORT.md#when-the-work-already-exists)
distinguish the observed behavior from a proposed **Already in your library** result
that leaves existing records alone.
