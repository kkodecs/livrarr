# Project document site

The project desk provides a browser view of Livrarr's Markdown documents and build
records. Open the site at `http://<your-server-address>:8792/` from a device that can reach Oasis.

The site uses a Tokyo Night–inspired dark palette with light text. Printing uses
dark text on a light background; embedded reports retain their original styling.

Use the dashboard for the latest [briefing](../../HANDOFF.md), then follow the
links to detailed results and decisions. Build panels read the selected feature
from `build/state/ACTIVE` and its state and status files. Dates identify when those
records changed; they do not indicate that an agent is currently running.

## Browse and search

- Search document titles, paths and contents from any page.
- Use Wiki, Reference, Build state, Reviews, Reports or History to narrow the list.
- Open a document to read its formatted contents, follow its section links, or
  view the original text.
- Historical documents remain available and are labeled as history. The current
  handoff identifies the latest accepted work.

The site reads the existing files. Edit the Markdown and build records in their
usual locations; no separate publishing step is needed. New documents appear in
the index within five seconds, and the dashboard refreshes every thirty seconds.

## Operate the service

The standalone service is called `livrarr-docs.service`. Its source and operating
guide are in `build/ops/document-site/`, separate from the Livrarr application.

```bash
systemctl --user status livrarr-docs.service
systemctl --user restart livrarr-docs.service
systemctl --user stop livrarr-docs.service
journalctl --user -u livrarr-docs.service -n 50
```

Stopping or restarting this document site does not restart Livrarr. The site has
no document editing, build, deployment or application-control endpoints.

## Scope

The site serves Markdown and saved HTML reports from the documented project
collections, plus selected build YAML. It excludes application configuration,
databases, binaries, logs, symlinks and runtime backups. JSON evidence and source
code are outside its initial browsing scope. Saved HTML reports are displayed
with scripts and external requests disabled.

At the user's request, the installed service listens on `0.0.0.0:8792` for remote
access. Its Host checks accept the server's LAN address, `oasis`, and the local addresses.
The port remains 8792; no Livrarr application setting changed.

[Initial setup](../../build/reviews/document-site-2026-09-18/RESULT.md) ·
[Remote-access verification](../../build/reviews/document-site-network-2026-09-18/RECEIPT.md).
