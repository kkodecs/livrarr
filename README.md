# Livrarr

**Self-hosted ebook and audiobook library manager.** Built for the \*arr ecosystem — finds, grabs, and organizes your books the way Sonarr does for TV.

> ⚠️ **Alpha software.** Core workflows work. Rough edges exist. Feedback welcome.

---

> **Note:** alpha4 and earlier used an OpenLibrary User-Agent that wasn't fully policy-compliant, which could cause OL lookups (search, enrichment, author monitor, cover backfill) to fail. alpha5 ships a compliant UA — run alpha5 or later. See [Notes → OpenLibrary compliance](#openlibrary-compliance) for detail.

---

## What it does

- **Search** any Torznab or Newznab indexer (Prowlarr, NZBHydra2, Jackett, or direct) for ebooks and audiobooks
- **Grab** via qBittorrent or SABnzbd (forthcoming: support for other clients)
- **Import** to your library with automatic file organization
- **Enrich** metadata from Hardcover, OpenLibrary, Google Books, and Audnexus
- **Read** in your browser with a built-in ebook reader and audiobook player
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
    image: ghcr.io/kkodecs/livrarr:0.1.0-alpha5
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
| Docker | Yes | Multi-arch image — linux/amd64 and linux/arm64 |
| qBittorrent or SABnzbd | Yes | Download client |
| Torznab or Newznab indexer | Yes | Prowlarr, NZBHydra2, Jackett, or direct feed |
| Hardcover API key | No | Better metadata — free at hardcover.app |
| LLM integration | No | Better search and metadata |
| Calibre-Web Automated | No | Downstream ebook delivery |
| AudioBookShelf | No | Downstream audiobook delivery |

### Permissions (PUID / PGID)

Set `PUID`/`PGID` to your host user so Livrarr reads and writes your files with the right ownership — the same convention as Sonarr/Radarr. On startup the container briefly runs as root, fixes ownership of `/config`, then drops to `PUID:PGID` and runs the app as that user. It never leaves the app running as root, and it only ever changes ownership of `/config` — never your library.

```yaml
environment:
  - PUID=1000   # run `id -u` on the host
  - PGID=1000   # run `id -g` on the host
```

Mounted paths must be accessible by that user:

- `/config` — **writable** (database, covers, config file); ownership is auto-fixed to `PUID:PGID`
- `/books` — **writable** (Livrarr moves files here on import); you own this
- `/downloads` — **readable** (completed download directory); you own this

Livrarr auto-fixes `/config` only — make sure your library and downloads are already owned by (or accessible to) your `PUID:PGID`.

**Hardened / rootless:** to never run as root at all, delete the `cap_add:` block and set `user: "1000:1000"` (pre-`chown` your `./config` on the host first); in this mode `PUID`/`PGID` are ignored. Rootless Docker/Podman **must** use this mode. On Kubernetes use `securityContext.runAsUser/runAsGroup/fsGroup`. With SELinux, add `:z`/`:Z` to your bind mounts.

**Upgrading from an older build?** If you used the simple Quick Start compose, nothing changes. If you copied the hardened `docker-compose.yml` (with `cap_drop: ALL`), add the `cap_add:` block (or switch to `user: "1000:1000"`) — otherwise the container stops on start with a message telling you exactly what to do.

### Download path mapping

Livrarr and your download client must see completed downloads at the **same host path**. Example:

- qBittorrent saves to `/mnt/data/downloads` on the host
- Mount that same path into both containers: `- /mnt/data/downloads:/downloads`

`/books` and `/downloads` in the compose file are example container paths — you can rename them as long as you're consistent across all containers.

---

## Alpha Limitations

- Multi-user partially implemented — additional users can log in but share admin indexers/clients. Treat as single-user for alpha.
- Cover accuracy can still vary for some titles (especially Goodreads matches). A manual refresh usually fixes it.

---

## Stack

Built in Rust (backend) + React (frontend). Ships as a single Docker image — no database sidecar, no separate web server. For full workflows you'll still need a download client and at least one indexer. Starts in under a second.

---

## Notes

### OpenLibrary compliance

Per [OpenLibrary's published API policy](https://openlibrary.org/developers/api), bulk clients must use a User-Agent that identifies the app *and* includes a contact (email or URL). alpha4 and earlier sent a UA with the app name but no contact field (`Livrarr/0.1.0-alpha4`), which OL flagged as non-compliant bulk traffic — this could cause OpenLibrary search, enrichment, author monitor, and cover backfill to fail. The gap is tracked in [#83](https://github.com/kkodecs/livrarr/issues/83).

alpha5 ships a fully policy-compliant UA (app name + version + contact email + contact URL), which also earns OL's higher rate limit (3 req/s vs 1 req/s). If you're on an older build, upgrade:

```
docker compose pull livrarr && docker compose up -d
```

---

## Community

- **Discord:** [Join the Livrarr Discord](https://discord.gg/PJDsgjEvCV) — help, discussion, feature requests
- **GitHub Issues:** [Report bugs or request features](https://github.com/kkodecs/livrarr/issues)

---

## License

GPLv3
