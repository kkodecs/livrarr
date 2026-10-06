# Lessons to consult

## Executive summary

Use this index to find the rules relevant to your next change. Start with [identity lessons](#identity) for Work lookup and [tests and fixtures](#tests-and-fixtures) for verification; the dated sources preserve earlier decisions and corrections.

Scan this index, then open the lessons relevant to the next decision. Read the
current rule first; each entry links its full earlier account and corrections.
Historical test counts, old source locations and superseded mechanisms are evidence,
not live task instructions. Root principles and later accepted decisions take precedence.

The original lesson numbers and incoming section links are preserved. New lessons
need both a theme entry and a short index link. Feature progress belongs in build/state.

## Architecture

- [1. Workspace boundaries](insights/architecture.md#lesson-1)
- [2. Core entities](insights/architecture.md#lesson-2)
- [3. Work is primary](insights/architecture.md#lesson-3)
- [4. One Work spans both formats](insights/architecture.md#lesson-4)
- [6. Collections and roots](insights/architecture.md#lesson-6)
- [38. Crate navigation](insights/architecture.md#lesson-38)
- [48. Canonical architecture model](insights/architecture.md#lesson-48)
- [64. Discovery has separate ownership](insights/architecture.md#lesson-64)
- [66. Add returns before background completion](insights/architecture.md#lesson-66)

## Coding patterns

- [7. Service contracts and doubles](insights/coding-patterns.md#lesson-7)
- [8. Async traits](insights/coding-patterns.md#lesson-8)
- [9. SQL and business decisions](insights/coding-patterns.md#lesson-9)
- [9b. Compile wall](insights/coding-patterns.md#lesson-9b)
- [9c. Shared service instances](insights/coding-patterns.md#lesson-9c)
- [9d. Explicit construction](insights/coding-patterns.md#lesson-9d)
- [9e. Boundary types](insights/coding-patterns.md#lesson-9e)
- [9f. Accessor adapters](insights/coding-patterns.md#lesson-9f)
- [9g. Background task ownership](insights/coding-patterns.md#lesson-9g)
- [9h. Narrow handler capabilities](insights/coding-patterns.md#lesson-9h)
- [9i. Credentials are separate capabilities](insights/coding-patterns.md#lesson-9i)
- [10. Blocking I/O](insights/coding-patterns.md#lesson-10)
- [11. Datetime convention](insights/coding-patterns.md#lesson-11)
- [36. Run guards and shutdown](insights/coding-patterns.md#lesson-36)
- [39. Settings contracts](insights/coding-patterns.md#lesson-39)
- [40. Import utilities versus orchestration](insights/coding-patterns.md#lesson-40)
- [41. Composite handler contracts](insights/coding-patterns.md#lesson-41)
- [103. Checked invocation facade](insights/coding-patterns.md#lesson-103)
- [106. A TOML file is a table, not a value](insights/coding-patterns.md#lesson-106)
- [107. An errored query with no data is not in error while it refetches](insights/coding-patterns.md#lesson-107)
- [110. Cancel in-flight reads before storing a save's reply](insights/coding-patterns.md#lesson-110)
- [111. A pop-up's content is drawn once; role-dependent parts read the store](insights/coding-patterns.md#lesson-111)

## Covers

- [44. OpenLibrary cover identifiers](insights/covers.md#lesson-44)
- [63. Cover ownership and recovery](insights/covers.md#lesson-63)

## Data and state

- [5. SQLite writer admission](insights/data-and-state.md#lesson-5)
- [17. Missing versus wanted](insights/data-and-state.md#lesson-17)
- [18. Persistent state after reload](insights/data-and-state.md#lesson-18)
- [19. Immutable migrations](insights/data-and-state.md#lesson-19)
- [20. SQLite upserts](insights/data-and-state.md#lesson-20)
- [21. Independent monitoring](insights/data-and-state.md#lesson-21)
- [81. Pagination is not the whole library](insights/data-and-state.md#lesson-81)

## History and review

- [77. History records moments of truth](insights/history-and-review.md#lesson-77)
- [86. Separate source and approval storage](insights/history-and-review.md#lesson-86)
- [87. Migration reports are contracts](insights/history-and-review.md#lesson-87)
- [88. Nonempty databases can be ready](insights/history-and-review.md#lesson-88)
- [89. Migration compatibility is one contract](insights/history-and-review.md#lesson-89)
- [91. Review generation is observed at decision time](insights/history-and-review.md#lesson-91)
- [100. Identity audit findings need their baseline](insights/history-and-review.md#lesson-100)
- [101. Review questions and dismissal have shared owners](insights/history-and-review.md#lesson-101)

## Identity

- [13. LLMs never choose identity](insights/identity.md#lesson-13)
- [15. User choice survives identity completion](insights/identity.md#lesson-15)
- [53. One seed policy across creation paths](insights/identity.md#lesson-53)
- [54. Background creation must converge](insights/identity.md#lesson-54)
- [57. Completion must agree with retry selection](insights/identity.md#lesson-57)
- [59. Shared identity comparison](insights/identity.md#lesson-59)
- [60. Historical review surface](insights/identity.md#lesson-60)
- [67. Author adoption and merge](insights/identity.md#lesson-67)
- [72. Subtitle trust needs its cause](insights/identity.md#lesson-72)
- [73. Provider editions and parsing can mislead matching](insights/identity.md#lesson-73)
- [78. Reconstruct the full migration history](insights/identity.md#lesson-78)
- [80. Observe generation before making the decision](insights/identity.md#lesson-80)
- [82. Author routes and contributor evidence](insights/identity.md#lesson-82)
- [85. Presentation can degrade without weakening identity](insights/identity.md#lesson-85)
- [90. Captured identity replaces frozen badges](insights/identity.md#lesson-90)
- [92. Review must not leave a hidden duplicate](insights/identity.md#lesson-92)
- [93. No-op, selection and attempt accounting](insights/identity.md#lesson-93)
- [94. Project identifiers from active routes](insights/identity.md#lesson-94)
- [95. Bounded search fallback](insights/identity.md#lesson-95)
- [96. Goodreads Book and Work IDs are disjoint](insights/identity.md#lesson-96)
- [97. Search eligibility is per provider](insights/identity.md#lesson-97)
- [98. Compare complete identities across stored key versions](insights/identity.md#lesson-98)
- [99. A correctly called seam can still be unwired](insights/identity.md#lesson-99)

## Metadata

- [12. Foreign discovery differs from enrichment](insights/metadata.md#lesson-12)
- [16. Language policy has one merge guard](insights/metadata.md#lesson-16)
- [32. Configuration failures can recover](insights/metadata.md#lesson-32)
- [42. Author monitoring controls](insights/metadata.md#lesson-42)
- [50. Verify the actual failure source](insights/metadata.md#lesson-50)
- [51. Identity, descriptive merge and observation are separate](insights/metadata.md#lesson-51)
- [52. Coherent fixtures and tolerant provider fields](insights/metadata.md#lesson-52)
- [55. Measure refresh bottlenecks](insights/metadata.md#lesson-55)
- [56. One payload-policy choke point](insights/metadata.md#lesson-56)
- [76. Merge output can echo old values](insights/metadata.md#lesson-76)

## Process

- [14. Public metadata assistance has limits](insights/process.md#lesson-14)
- [23. Build persistence realistically](insights/process.md#lesson-23)
- [24. Fix the originating fault](insights/process.md#lesson-24)
- [25. Prototype external contracts](insights/process.md#lesson-25)
- [26. Review structural duplication](insights/process.md#lesson-26)
- [27. Comments describe the code](insights/process.md#lesson-27)
- [35. Path prefix boundaries](insights/process.md#lesson-35)
- [37. HTTP trust follows URL provenance](insights/process.md#lesson-37)
- [46. Trace every entry into the workflow](insights/process.md#lesson-46)
- [47. State real system facts](insights/process.md#lesson-47)
- [49. Performance evidence and live data](insights/process.md#lesson-49)
- [74. Reader, notification and transfer edge cases](insights/process.md#lesson-74)
- [75. Dev build is not frontend typechecking](insights/process.md#lesson-75)

## Providers and transport

- [22. Provider deadlines differ](insights/providers-and-transport.md#lesson-22)
- [28. SABnzbd history search](insights/providers-and-transport.md#lesson-28)
- [29. Encode query credentials](insights/providers-and-transport.md#lesson-29)
- [30. One outbound queue](insights/providers-and-transport.md#lesson-30)
- [31. Download matching is client-scoped](insights/providers-and-transport.md#lesson-31)
- [33. Map provider errors at the adapter](insights/providers-and-transport.md#lesson-33)
- [43. OpenLibrary bibliography shape](insights/providers-and-transport.md#lesson-43)
- [45. Avoid per-record bulk API harvest](insights/providers-and-transport.md#lesson-45)
- [58. Shared breaker state needs one test guard](insights/providers-and-transport.md#lesson-58)
- [62. Unreadable is not empty](insights/providers-and-transport.md#lesson-62)
- [68. Provider cache policy belongs at its seam](insights/providers-and-transport.md#lesson-68)
- [69. Share clients; measure concurrency](insights/providers-and-transport.md#lesson-69)
- [70. One qBittorrent state classifier](insights/providers-and-transport.md#lesson-70)
- [71. Indexer pacing and breaker scopes](insights/providers-and-transport.md#lesson-71)
- [79. Queue dispatcher lifetime](insights/providers-and-transport.md#lesson-79)

## Tests and fixtures

- [34. See failures beyond the first binary](insights/tests-and-fixtures.md#lesson-34)
- [61. Frozen comparison fixtures are intentional](insights/tests-and-fixtures.md#lesson-61)
- [65. Registered, tracked and executed are different](insights/tests-and-fixtures.md#lesson-65)
- [83. Use reliable tracing capture](insights/tests-and-fixtures.md#lesson-83)
- [84. Stub HTTP response queues have a sticky tail](insights/tests-and-fixtures.md#lesson-84)
- [102. B0 red-test facts](insights/tests-and-fixtures.md#lesson-102)
- [104. Cancellation test diagnostics](insights/tests-and-fixtures.md#lesson-104)
- [105. Finish SQLite fixture setup before pausing Tokio time](insights/tests-and-fixtures.md#lesson-105)
- [108. Strip colour codes before searching the live log](insights/tests-and-fixtures.md#lesson-108)
- [109. A passing connection test writes no log line](insights/tests-and-fixtures.md#lesson-109)
- [112. A new web call needs every whole-app test stub that renders its page](insights/tests-and-fixtures.md#lesson-112)

## Source and history

[Exact revision before cleanup](../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="active-insights"></a>
<a id="data--state"></a>
