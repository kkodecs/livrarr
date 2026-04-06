# Librarr

**Self-hosted ebook and audiobook library manager.** Built for the \*arr ecosystem — finds, grabs, and organizes your books the way Sonarr does for TV.

> ⚠️ **Alpha software.** Core workflows work. Rough edges exist. Feedback welcome.

---

## What it does

- **Search** any Torznab or Newznab indexer for ebooks and audiobooks
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
  librarr:
    image: ghcr.io/kkodecs/librarr:latest
    container_name: librarr
    ports:
      - 8788:8787
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

Navigate to `http://your-server:8788`. You'll be prompted to create your admin account on first launch — no pre-seeding required.

**4. Configure**

Go to **Settings** and add:
- A root folder (where books land after import)
- A download client (qBittorrent or SABnzbd)
- At least one indexer (Torznab or Newznab URL + API key)

---

## Configuration

All settings live in the UI. Optionally create `/config/config.toml` for advanced options:

```toml
[server]
port = 8787          # internal port (map externally in compose)
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
| Torznab or Newznab indexer | Yes | Torznab / Newznab feed |
| Hardcover API key | No | Better metadata — free at hardcover.app |
| LLM integration | No | Better search and metadata |
| Calibre-Web Automated | No | Downstream ebook delivery |
| AudioBookShelf | No | Downstream audiobook delivery |

### Permissions

Librarr runs as UID/GID 1000 inside the container. All mounted paths must be accessible by that user:

- `/config` — must be **writable** (database, covers, config file)
- `/books` — must be **writable** (Librarr moves files here on import)
- `/downloads` — must be **readable** (completed download directory)

If you're on a different UID, `chown 1000:1000` the host directories before starting.

### Download path mapping

Librarr and your download client must see completed downloads at the **same host path**. Example:

- qBittorrent saves to `/mnt/data/downloads` on the host
- Mount that same path into both containers: `- /mnt/data/downloads:/downloads`

`/books` and `/downloads` in the compose file are example container paths — you can rename them as long as you're consistent across all containers.

---

## Alpha Limitations

- Multi-user partially implemented — additional users can log in but share admin indexers/clients; Settings not visible to non-admin users. Treat as single-user for alpha.
- PUID/PGID not configurable — runs as UID/GID 1000 (fix in beta)
- No mobile-optimized UI
- Readarr import not yet supported

---

## Stack

Built in Rust (backend) + React (frontend). Ships as a single Docker image — no database sidecar, no separate web server. For full workflows you'll still need a download client and at least one indexer. Starts in under a second.

---

## License

GPLv3
