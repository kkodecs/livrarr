# Livrarr — current handoff

## Executive summary

**Next feature: prerelease-trust-pass, approved and at the spec stage, with nothing dispatched yet. It waits on a fresh PM session.** There is no push hold: main was pushed on 2026-10-01 and equals origin/main (`59b7e9db`). The only unpushed change is the documentation commit that carries this handoff. errors-and-delete-pass, small-bugs-pass and the retro fixes are delivered and on origin; GitHub #178, #180, #182 and #183 are closed. Nothing beyond the new feature's scope is approved; scope additions need the PO's explicit word. See [the next feature](#next-feature), [what is delivered](#what-is-delivered), [recorded limits](#recorded-limits), [open items](#open-items) and [where to read next](#read-next).

## Next feature

**prerelease-trust-pass** — initialized 2026-10-01 at stage spec (PO "ok go"). Scope, three parts:

- fix the 16 wrong claims in the docs, including the context file that feeds the in-app AI help;
- the two documented settings that do nothing (the reverse-proxy sub-path and the proxy-login keys);
- a bootstrap token for first-run setup, so no one else on the network can claim the admin account first.

One PO call is open for the spec: make each do-nothing setting work, or remove it from the docs and validation; the spec seat presents both with a recommendation. Opus writing this feature's tests needs the PO's approval at the tests stage. Source: [state](build/state/prerelease-trust-pass.yaml), [status](build/state/STATUS-prerelease-trust-pass.md), [session log](build/state/session-log-prerelease-trust-pass.md). Handoff for the fresh PM session: [handoff-prerelease-trust-pass.md](build/state/handoff-prerelease-trust-pass.md).

## What is delivered

- **Push and issues, 2026-10-01** — livrarr `43b369c2..59b7e9db` pushed to origin/main (small-bugs-pass, errors-and-delete-pass, retro commit `59b7e9db`); kk-build `arch-step-dogfood` pushed (`09d311e..94de78a`). GitHub #178, #180, #182 and #183 closed, each citing `c48f9b02`.
- **errors-and-delete-pass retro follow-ups** — done 2026-10-01: the close gate passes without an override; the Stop-hook summarizer runs with no tools; review records count severities; the miner ignores summarizer runs; root [CLAUDE.md](CLAUDE.md) has the retro lessons. Source: [state](build/state/errors-and-delete-pass.yaml) (`retro_followups`), [retro](build/state/retro-errors-and-delete-pass.md).
- **errors-and-delete-pass** — delivered 2026-09-30, now pushed: silent failures show an error, a book delete can also remove its files (opt-in, unticked every time), "Delete File" removes the file, and book search says so when every source fails. Source: [spec v6](spec-errors-and-delete-pass.md#executive-summary), [live check](build/reviews/errors-and-delete-pass/deploy-20260930T221942Z/LIVE-CHECK.md).
- **small-bugs-pass** (2026-09-30) and **identity-upgrade-inplace** (2026-09-29) — delivered and pushed. Earlier features: see the [steward continuation](build/ops/document-steward/CONTINUE.md).
- Standing PO directions: Gemini and Grok benched, Astra sole reviewer (2026-09-24); let the user edit what they want (2026-09-25); no push without the PO's word; the steward pass runs at every feature close.

## Recorded limits

Both are recorded in [spec v6, limits](spec-errors-and-delete-pass.md#limits) and are not known defects:

- **PDF paths are unproven end to end.** The PDF reader cannot open a PDF in a built app, and the PO said not now; the PDF place-saving test stays skipped.
- **The check-then-remove race is accepted.** File removal checks the path, then removes it; a folder swapped for a link in between could redirect it. Recorded under Security in TODO.

## Open items

Unapproved candidates are in [TODO](build/plans/TODO.md): the PDF reader fix, the delete-with-files folder-swap race (Security), and the "Noticed during errors-and-delete-pass" list. Also open: files on the parked branch `wip/identity-conflict-authority-stopped` ([proposal](build/reviews/steward-upkeep-2026-09-26/WIP-BRANCH-PROPOSAL.md)).

## Read next

PM: `~/Projects/kk-build/templates/claude-md/pm-context.md`, then [CLAUDE.md](CLAUDE.md), the [prerelease-trust-pass handoff](build/state/handoff-prerelease-trust-pass.md) and its [state](build/state/prerelease-trust-pass.yaml), then the sources its scope names in [TODO](build/plans/TODO.md) (Docs, Bugs, Security) and the [docs, operations and security scan](build/reports/docs-ops-security-scan-2026-09-26.md). Document upkeep: [steward continuation](build/ops/document-steward/CONTINUE.md) and this pass's [receipt](build/reviews/steward-upkeep-2026-10-01/RECEIPT.md), which lists contradictions for the PM.

The previous root handoff is preserved unchanged at `build/reviews/steward-upkeep-2026-10-01/predecessors/HANDOFF.md`.
