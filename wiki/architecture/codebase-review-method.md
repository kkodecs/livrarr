# Reviewing a codebase for improvements

Start with a concrete user or maintainer problem: what fails, causes delay, creates
confusion or makes an ordinary change difficult? Establish current behavior and
impact before proposing a redesign. The [codebase improvement review](../../build/reviews/codebase-improvement-review-2026-09-09/PLAN.md)
and [backlog](../../build/reviews/codebase-improvement-review-2026-09-09/BACKLOG.md)
apply this method; improvements remain proposals until approved.

Reuse corrected crate reports as leads, with their source paths, hashes and PM
corrections. Validate claims afresh against the designated source, including real
callers and relevant tests/configurations. Do not substitute the parked checkout
for the reviewed containment worktree. The [earlier review lessons](architecture-review-simplification.md)
explain index-scope and census pitfalls; an old report is not current proof.

There is no LOC quota. A small cleanup list does not establish an efficient overall
architecture, and absence of a demonstrated large reduction does not prove
impossibility. Evaluate simpler ownership or boundaries by their actual effect on
behavior and maintenance. Moving code earns zero net reduction; count all replacement
helpers, adapters and callers before estimating savings.

The [later workflow sketch](../../build/reviews/architecture-review-2026-09-07/closure-024/CLOSURE.md)
illustrates the limit: wrapping retained engines suggested clearer ownership but
left reduction unmeasured. A follow-up must name eliminated mechanisms and preserve
optional data, independent jobs, failure follow-ups and fresh identity generations.
A selected slice is not a whole-workflow or project denominator.

Administrative closure carries unresolved product/principle questions forward; it
does not authorize implementation or certify a software gate. Keep evidence,
proposals, accepted decisions and validation limits distinct.
