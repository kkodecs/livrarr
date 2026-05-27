# Livrarr

**Self-hosted ebook and audiobook library manager.** Built for the \*arr ecosystem — finds, grabs, and organizes your books the way Sonarr does for TV.

> ⚠️ **Alpha software.** Core workflows work. Rough edges exist. Feedback welcome.

---

## 🚨 OpenLibrary lookups affected on alpha4 and earlier — upgrade to alpha5

**TL;DR — if you're having trouble adding works, upgrade to alpha5 immediately. Honestly, you should do that regardless.**

### What happened

OpenLibrary started returning HTTP 403 to Livrarr's User-Agent. Search, enrichment, author monitor, and cover backfill all silently broke. Many reports of "OL is down" were actually "OL is blocking us."

### Diagnosis

Per [OpenLibrary's published API policy](https://openlibrary.org/developers/api), bulk clients must use a User-Agent that identifies the app *and* includes a contact (email or URL). Livrarr's UA on alpha4 (`Livrarr/0.1.0-alpha4`) had the app name but no contact field — flagged as identifiable bulk traffic without policy compliance, penalized harder than fully-anonymous requests.

### What we did

- Filed [#83](https://github.com/kkodecs/livrarr/issues/83) with the empirical evidence and the two-line fix
- Sent an apology email to OpenLibrary's contact address explaining the gap and confirming our intent to comply
- No response yet — but as of 2026-05-27, OL appears to have lifted the block on the old UA

### Current status

- OL is currently returning 200 to alpha4's UA from multiple test hosts. So lookups are working again **for now**.
- But every alpha4 request is still **technically out of compliance** with OL's policy. The block could come back at any time, and the next round of enforcement may be more aggressive (IP-level instead of UA-level).

### The fix

**alpha5 ships a fully policy-compliant UA** — app name + version + contact email + contact URL — earning the higher rate limit (3 req/s vs 1 req/s) and getting out of the penalty bucket entirely.

**Recommendation: upgrade to alpha5 as soon as it's released.** It also bundles a stack of metadata fixes (audiobook cover pipeline, OpenLibrary cover extraction, UI cache invalidation on metadata refresh, PID-deadlock-on-restart bug) that are worth the upgrade on their own.

---

## What it does

- **Search** any Torznab or Newznab indexer (Prowlarr, NZBHydra2, Jackett, or direct) for ebooks and audiobooks
- **Grab** via qBittorrent or SABnzbd (forthcoming: support for other clients)
- **Import** to your library with automatic file organization
- **Enrich** metadata from Hardcover, OpenLibrary, and Audnexus
- **Push** to Calibre-Web Automated (CWA) or AudioBookShelf (ABS)

---

## Design Philosophy

- Ebooks and audiobooks unified in a single instance — no separate installations
- No closed-source metadata proxy — all providers are open, pluggable, and federated
- Single container, single SQLite database — nothing else to manage
- AI-assisted metadata disambiguation when exact matches fail

---

## Quick Start

**1. Copy the compose file**

```yaml
services:
  livrarr:
    image: ghcr.io/kkodecs/livrarr:0.1.0-alpha4
    container_name: livrarr
    ports:
      - 8789:8789
    volumes:
      - ./config:/config
      - /path/to/books:/books
      - /path/to/downloads:/downloads
    restart: unless-stopped
```

**2. Set permissions and start**

```bash
mkdir config
sudo chown 1000:1000 config
docker compose up -d
```

**3. Open the UI**

Navigate to `http://your-server:8789`. You'll be prompted to create your admin account on first launch — no pre-seeding required.

**4. Configure**

Go to **Settings** and add:
- A root folder (where books land after import)
- A download client (qBittorrent or SABnzbd)
- At least one indexer (any Torznab/Newznab source: Prowlarr, NZBHydra2, Jackett, or direct indexer URL + API key)

---

## Configuration

All settings live in the UI. Optionally create `/config/config.toml` for advanced options:

```toml
[server]
port = 8789          # internal port (map externally in compose)
bind_address = "0.0.0.0"

[log]
level = "info"       # trace | debug | info | warn | error
```

---

## Requirements

| Component | Required | Notes |
|---|---|---|
| Docker | Yes | linux/amd64 only (ARM coming later) |
| qBittorrent or SABnzbd | Yes | Download client |
| Torznab or Newznab indexer | Yes | Prowlarr, NZBHydra2, Jackett, or direct feed |
| Hardcover API key | No | Better metadata — free at hardcover.app |
| LLM integration | No | Better search and metadata |
| Calibre-Web Automated | No | Downstream ebook delivery |
| AudioBookShelf | No | Downstream audiobook delivery |

### Permissions

Livrarr runs as UID/GID 1000 inside the container. All mounted paths must be accessible by that user:

- `/config` — must be **writable** (database, covers, config file)
- `/books` — must be **writable** (Livrarr moves files here on import)
- `/downloads` — must be **readable** (completed download directory)

If you're on a different UID, `chown 1000:1000` the host directories before starting.

### Download path mapping

Livrarr and your download client must see completed downloads at the **same host path**. Example:

- qBittorrent saves to `/mnt/data/downloads` on the host
- Mount that same path into both containers: `- /mnt/data/downloads:/downloads`

`/books` and `/downloads` in the compose file are example container paths — you can rename them as long as you're consistent across all containers.

---

## Alpha Limitations

- Multi-user partially implemented — additional users can log in but share admin indexers/clients. Treat as single-user for alpha.
- PUID/PGID not configurable — runs as UID/GID 1000 (fix in beta)
- No mobile-optimized UI
- Cover quality varies — Goodreads matching can return incorrect covers for some titles. Manual refresh usually fixes it. Full cover trust model coming in alpha4.

---

## Stack

Built in Rust (backend) + React (frontend). Ships as a single Docker image — no database sidecar, no separate web server. For full workflows you'll still need a download client and at least one indexer. Starts in under a second.

---

## Community

- **Discord:** [Join the Livrarr Discord](https://discord.gg/PJDsgjEvCV) — help, discussion, feature requests
- **GitHub Issues:** [Report bugs or request features](https://github.com/kkodecs/livrarr/issues)

---

## License

GPLv3
