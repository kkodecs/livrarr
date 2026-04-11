# Livrarr — AI Assistant Context

> This file provides context for AI assistants helping Livrarr users with setup, configuration, and troubleshooting. It is referenced automatically by the in-app "Get AI Help" feature.

## What is Livrarr?

Livrarr is a self-hosted ebook and audiobook library manager, similar to Sonarr (TV) and Radarr (movies) but for books. It automates searching, downloading, organizing, and tagging book files.

**Key traits:**
- Works-first model — a "work" = a book title, independent of format or edition
- Manages both ebooks (EPUB, PDF, MOBI, AZW3) and audiobooks (M4B, MP3) in one app
- Multi-user with per-user library isolation
- Integrates with the *arr ecosystem: Prowlarr, qBittorrent, SABnzbd
- Integrates with downstream readers: Calibre-Web Automated, Audiobookshelf, Kavita
- Metadata from OpenLibrary + Hardcover (English) and Goodreads via LLM scraping (8 foreign languages)

**Stack:** Rust backend, React/TypeScript frontend, SQLite database, Docker deployment (Linux only)

## How It Works

1. **Search** — User searches for a book by title. Livrarr queries metadata providers (OpenLibrary for English, Goodreads via LLM for foreign languages).
2. **Add** — User adds the work to their library. Livrarr enriches it with description, genres, series info, covers, ratings from Hardcover/Goodreads.
3. **Find releases** — User searches indexers (Torznab/Newznab) for downloadable files.
4. **Download** — Livrarr sends the grab to qBittorrent or SABnzbd.
5. **Import** — When download completes, Livrarr copies the file to the organized library, writes metadata tags (EPUB/M4B), and optionally hardlinks to CWA.

## Setup Requirements

All configured through the web UI at `http://<host>:8789`:

| Component | Where | Required? |
|-----------|-------|-----------|
| Root folders | Settings > Media Management | Yes — at least one ebook or audiobook root |
| Download client | Settings > Download Clients | Yes — qBittorrent or SABnzbd |
| Indexers | Settings > Indexers | Yes — Torznab/Newznab URLs (or import from Prowlarr) |
| Hardcover token | Settings > Metadata | Recommended — free API token from hardcover.app for rich metadata |
| LLM endpoint | Settings > Metadata | Optional — any OpenAI-compatible API for foreign language search and result cleaning |
| Audnexus | Settings > Metadata | Optional — audiobook narrator data (default public instance works) |

## Configuration

The only file-level config is `config.toml` in the data directory:

```toml
[server]
host = "0.0.0.0"
port = 8789
data_dir = "/data"

[log]
level = "info"   # trace | debug | info | warn | error
```

Everything else is configured through the web UI and stored in the SQLite database.

## Docker Deployment

```yaml
services:
  livrarr:
    image: ghcr.io/kkodecs/livrarr:latest
    container_name: livrarr
    ports:
      - "8789:8789"
    volumes:
      - ./data:/data          # Config, database, covers
      - /path/to/ebooks:/ebooks
      - /path/to/audiobooks:/audiobooks
      - /path/to/downloads:/downloads
    restart: unless-stopped
```

**Volume mapping notes:**
- `/data` — persistent storage for config.toml, livrarr.db, and cover images
- Library root folders and download paths must be accessible inside the container
- Remote path mappings (Settings > Download Clients) may be needed if the download client and Livrarr see different mount paths for the same files

## File Organization

Livrarr organizes imported files into a strict layout:

- **Ebooks:** `{root}/{user_id}/{Author Name}/{Title}.epub`
- **Audiobooks:** `{root}/{user_id}/{Author Name}/{Title}/{files}`

Separate root folders for ebooks and audiobooks. Per-user subdirectories ensure file isolation.

## Common Issues and Solutions

### No search results
- **Check indexers** — Settings > Indexers. Test each one with the bolt icon.
- **Wrong categories** — Book categories are typically 7000-7999 for Torznab. Ensure at least one book category is selected.
- **Prowlarr users** — Import indexers from Prowlarr first, then test individually.

### Enrichment failed / partial
- **"failed"** — All providers failed. Check Hardcover API token validity in Settings > Metadata.
- **"partial"** — Some providers worked. Usually means Hardcover failed but OpenLibrary succeeded. Less metadata but functional.
- **"skipped"** — Foreign language work without a detail URL. Normal for works added before enrichment was available.

### Downloads not importing
- **No root folder** — Must have at least one root folder matching the media type (ebook or audiobook).
- **Path mismatch** — The download path the client reports must be accessible to Livrarr. Use remote path mappings if paths differ between containers.
- **qBittorrent category** — Livrarr only monitors downloads in its configured category (default: "livrarr").

### Blurry covers
- Search thumbnails are intentionally small. Full-resolution covers are downloaded when the work is added.
- For foreign language works, high-res covers come from the detail page enrichment.
- Use "Refresh" on the work detail page to re-download covers.

### Foreign language search issues
- Requires LLM configured in Settings > Metadata (any OpenAI-compatible endpoint).
- Language must be enabled in the language list on the Metadata settings page.
- Supported languages: French, German, Spanish, Dutch, Italian, Japanese, Korean, Polish.

### CWA (Calibre-Web Automated) integration
- Set the CWA ingest path in Settings > Media Management.
- Livrarr hardlinks (or copies as fallback) imported files to the CWA ingest directory.
- CWA picks up new files automatically from its ingest folder.

## API Reference

REST API at `/api/v1/`. Authenticate with `X-Api-Key: <key>` header or session cookie.

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/system/status` | GET | Version, OS, uptime |
| `/system/logs/tail?lines=N` | GET | Recent log lines (admin only) |
| `/health` | GET | Health check |
| `/work/lookup?term=...&lang=en` | GET | Search metadata providers |
| `/work` | POST | Add work to library |
| `/work/{id}` | GET | Work detail |
| `/work/{id}/refresh` | POST | Re-enrich from providers |
| `/release?workId=N` | GET | Search indexers for releases |
| `/release/grab` | POST | Send release to download client |

## Architecture (for advanced troubleshooting)

- **10 Rust crates:** livrarr-server (composition root), livrarr-db, livrarr-domain, livrarr-metadata, livrarr-http, livrarr-download, livrarr-organize, livrarr-tagwrite, livrarr-jobs
- **Database:** SQLite via sqlx with migration files in `crates/livrarr-db/migrations/`
- **Enrichment pipeline:** Hardcover GraphQL → OpenLibrary JSON → Audnexus REST (English); Goodreads HTML → LLM extraction (foreign)
- **Background jobs:** download poller, enrichment retry, session cleanup, author monitor
- **Tag writing:** EPUB metadata via epub crate, M4B via mp4ameta crate

## Links

- Repository: https://github.com/kkodecs/livrarr
- Issues: https://github.com/kkodecs/livrarr/issues
- License: GPL-3.0
