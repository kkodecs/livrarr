# Metadata lessons

Current guidance with links to the full dated evidence. Read implementation claims
against the named source revision; accepted design is not proof of runtime behavior.

<a id="12-never-use-openlibrary-for-foreign-language-metadata"></a>
<a id="lesson-12"></a>
## 12. Foreign discovery differs from enrichment

OpenLibrary may assist foreign-language discovery with a language filter; it is not a foreign descriptive-enrichment provider. Google Books is the documented primary foreign metadata source. Do not turn the shorthand “never OL for foreign” into a ban on its discovery role.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/metadata.md#12-never-use-openlibrary-for-foreign-language-metadata).

<a id="16-foreign-works-skip-english-centric-providers-via"></a>
<a id="lesson-16"></a>
## 16. Language policy has one merge guard

Use Work language for applicability. Foreign descriptive metadata excludes OpenLibrary and Hardcover at the shared merge boundary as well as dispatch. The old metadata_source column was removed; it cannot gate current refresh behavior.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/metadata.md#16-foreign-works-skip-english-centric-providers-via).

<a id="32-notconfigured-is-terminal-but-resettable"></a>
<a id="lesson-32"></a>
## 32. Configuration failures can recover

NotConfigured is terminal for ordinary retry selection but resettable when configuration changes. The settings change must clear the relevant stored outcomes so a newly configured provider is actually tried.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/metadata.md#32-notconfigured-is-terminal-but-resettable).

<a id="42-author-monitor-has-three-independent-controls"></a>
<a id="lesson-42"></a>
## 42. Author monitoring controls

Author monitored chooses participation; monitor_new_items chooses auto-add versus notification; monitor_since limits publication age. Work ebook/audiobook monitoring is separate. Do not collapse these policies into one boolean.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/metadata.md#42-author-monitor-has-three-independent-controls).

<a id="50-sprint-b-evidence-round-2026-06-10-corrected-four"></a>
<a id="lesson-50"></a>
## 50. Verify the actual failure source

The June metadata incident involved wrong-book adoption, not a failed foreign-language guard. Logging existed but the status page pointed at a stale file. Cache and anchor ownership claims were also wrong. The durable rule is to follow the actual writer, payload, log file and cache seam before designing a fix; the dated incident inventory is not current topology.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/metadata.md#50-sprint-b-evidence-round-2026-06-10-corrected-four).

<a id="51-sprint-b-metadata-correctness"></a>
<a id="lesson-51"></a>
## 51. Identity, descriptive merge and observation are separate

Descriptive merge must not write identity anchors or generic cover state. Missing/empty values preserve populated metadata. Apply language rejection and provider dissents at the common merge boundary, including cached reuse. Retry/call records need real user/Work fixtures where foreign keys apply.

Enrichment is anchor-grounded; later route-native search returns identity evidence through settlement, not a text fallback into field merge. Materialization records actual dimensions and observes content change before retag. Chapter extraction crosses its domain seam.

Call observation uses a bounded sink, batching, drop accounting and shutdown drain; historical defaults were capacity 4096, batches 64/2s, retention 30d/100k. Provider health must come from real recent outcomes. Log setup failures are visible and the status page points at the active rotated file. The June code-gate and uncommitted-state narrative is archived; it does not describe today’s acceptance.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/metadata.md#51-sprint-b-metadata-correctness).

<a id="52-test-stage-fix-wave-2026-06-11-night"></a>
<a id="lesson-52"></a>
## 52. Coherent fixtures and tolerant provider fields

Provider evidence must carry enough independent title/author information to evaluate identity; stamping the seed onto an unverified response manufactures self-confirmation. Parse fields independently where the provider contract allows missing/malformed optional data.

The old LLM-based Goodreads selection and quorum descriptions were superseded by deterministic capture. Cover ownership depends on an actual saved artifact, and later cover-gate policy supersedes the historical trust flags. Real old-format EPUB fixtures exposed a parser dropping metadata after a self-closing element; test realistic payloads rather than only ideal examples.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/metadata.md#52-test-stage-fix-wave-2026-06-11-night).

<a id="55-refreshs-long-pole-is-the-enrichment-scatter-not-the"></a>
<a id="lesson-55"></a>
## 55. Measure refresh bottlenecks

Provider scatter was already parallel before a design proposed parallelizing it. Removing a legacy re-chase saved only part of refresh latency. That gate was later deleted; retain the measurement lesson, not the old mechanism. More Work-level concurrency cannot exceed shared provider pacing, and any new performance proposal needs a fresh baseline.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/metadata.md#55-refreshs-long-pole-is-the-enrichment-scatter-not-the).

<a id="56-mergeenginemerge-is-the-payload-policy-chokepoint"></a>
<a id="lesson-56"></a>
## 56. One payload-policy choke point

Both live network merging and cached-payload reuse pass through MergeEngine::merge. Put language compatibility and field-offer rules there. Empty strings/lists are no offer, not authority to erase stored values. The old separate Goodreads cover strip was deleted; cover policy belongs at its dedicated gate.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/metadata.md#56-mergeenginemerge-is-the-payload-policy-chokepoint).

<a id="76-mergeoutputwork_update-is-a-last-known-good-echo"></a>
<a id="lesson-76"></a>
## 76. Merge output can echo old values

A populated work_update may contain last-known-good values unchanged from before. Detect real content differences, excluding always-stamped bookkeeping, before reporting changed or retagging. Historical NoChange/Deferred enum variants had no producers; never infer runtime outcomes from enum presence alone.

[Full dated record and corrections](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/metadata.md#76-mergeoutputwork_update-is-a-last-known-good-echo).

## Source and history

[Exact revision before cleanup](../../docs/design-history/wiki-before-cleanup-2026-09-09/wiki/insights/metadata.md). Historical implementation claims retain
their original dates and source limits; the root principles and newer corrections take precedence.

<!-- Preserved section IDs for existing bookmarks and historical references. -->
<a id="metadata-insights"></a>
