# Validation — Goodreads autocomplete discovery (a21c643)

Live validation of the fix that moves Goodreads book discovery from the
WAF-blocked HTML `/search` page to the unguarded `/book/auto_complete` JSON
endpoint. Branch `wcc-stage5-green`; commit `a21c643`.

## Run 1 — core ladder (pre-fold build)

~90 books imported against a cleared DB.

- **89** books resolved via the autocomplete rung; **0** WAF blocks.
- **54+** works persisted with `gr_key` + rating + series (still enriching).
- Foreign-language enriched (previously impossible): El Ojo Del Mundo (es),
  El Problema De Los Tres Cuerpos (es), Le Petit Prince / L'étranger (fr),
  Pan Tadeusz (pl).
- Ladder degraded correctly: `Dune` (ISBN query empty) fell to the LLM-locator
  and resolved.
- 2 non-matches, both benign: HP Philosopher's Stone resolved via the
  LLM-locator fallback; `La Nuit Des Temps` had bad source tags (a Barjavel
  title mis-attributed to Jean Auel) — GR + the merge engine correctly
  rejected it.

## Run 2 — the folds (ISBN→title fallback + `.get()` safety)

Re-imported against a cleared DB on the folded build.

- **22** matched via autocomplete; **0** LLM-locator calls; **0** blocks; **0** misses.
- **ISBN→title fallback confirmed firing** — the path Run 1 could not exercise:

  ```
  12:26:29.932  GR autocomplete: no candidates   query=9798217176021
  12:26:30.185  GR autocomplete matched  title=Twelve Months  gr_key=230337028-twelve-months
  ```

  *Twelve Months'* ISBN is not in Goodreads' index; the ISBN query came up
  empty and the title query caught it 0.25s later — instead of dropping to the
  LLM-locator. Net effect: **zero** LLM-locator calls this run vs several in
  Run 1.

## Caveats (not regressions)

- Cross-language titles still lean on the LLM-disambiguator (e.g. a Spanish
  *edition* of an English-titled work). By design.
- Ambiguous source titles (a bare series name like "Wiedźmin") resolve to one
  specific book — a source-tagging issue, not the fix.

## Cross-family

- Codex authored the regression tests (autocomplete happy-path + the
  block→WillRetry invariant) — in `a21c643`.
- Gemini reviewed the diff; folded `.get()` index safety and the ISBN→title
  fallback (the latter validated by Run 2).

## Open / follow-up

- Rate-limiter (`fetcher.rs:73`, GR 1s) still above the polite floor — deferred.
- When `feat/metadata-modularization` merges (GR client moves to
  `livrarr-external-data`), carry this fix over.
