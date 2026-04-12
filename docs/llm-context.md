# Livrarr — AI Assistant Context

> This file provides context for AI assistants helping Livrarr users with setup, configuration, and troubleshooting. It is referenced automatically by the in-app "Get AI Help" feature.

## What is Livrarr?

Livrarr is a self-hosted ebook and audiobook library manager, similar to Sonarr (TV) and Radarr (movies) but for books. It automates searching, downloading, organizing, and tagging book files.

**Key traits:**
- Works-first model — a "work" = a book title, independent of format or edition
- Manages both ebooks and audiobooks in one app
- Multi-user with per-user library isolation
- Integrates with the *arr ecosystem: Prowlarr, qBittorrent, SABnzbd
- Integrates with downstream readers: Calibre-Web Automated (CWA), Audiobookshelf, Kavita
- Metadata from OpenLibrary + Hardcover (English) and Goodreads via LLM scraping (foreign languages)

**Stack:** Rust backend, React/TypeScript frontend, SQLite database, Docker deployment

## How It Works

1. **Search** — User searches by title. Livrarr queries metadata providers (OpenLibrary, Hardcover for English; Goodreads via LLM for foreign languages).
2. **Add** — User adds the work. Livrarr enriches it with description, genres, series info, covers, ratings.
3. **Find releases** — User searches indexers (Torznab/Newznab) for downloadable files.
4. **Download** — Livrarr sends the grab to qBittorrent (torrents) or SABnzbd (usenet).
5. **Import** — When download completes, poller detects it, copies files to organized library, writes metadata tags (EPUB/M4B/MP3), creates DB records, optionally hardlinks to CWA.

## Supported File Formats

| Type | Formats |
|------|---------|
| Ebook | EPUB, MOBI, AZW3, PDF |
| Audiobook | M4B, MP3, M4A, FLAC, OGG, WMA |

## Setup Requirements

All configured through the web UI at `http://<host>:8789`:

| Component | Where | Required? |
|-----------|-------|-----------|
| Root folders | Settings > Media Management | Yes — at least one ebook or audiobook root |
| Download client | Settings > Download Clients | Yes — qBittorrent or SABnzbd |
| Indexers | Settings > Indexers | Yes — Torznab/Newznab URLs (or import from Prowlarr) |
| Hardcover token | Settings > Metadata | Recommended — free API token from hardcover.app |
| LLM endpoint | Settings > Metadata | Optional — OpenAI-compatible API for foreign language support |
| Audnexus | Settings > Metadata | Optional — audiobook narrator data (default public instance) |

## Detailed Setup Guide

After first launch, visit `http://<host>:8789` and complete the setup wizard (create admin account). Then configure in this order:

### Step 1: Root Folders (Settings > Media Management)

Root folders tell Livrarr where to store imported files. You need at least one.

- **Ebook root folder** — e.g., `/books` (inside container). Imported ebooks go to `{root}/{Author}/{Title}.epub`.
- **Audiobook root folder** — e.g., `/audiobooks` (inside container). Imported audiobooks go to `{root}/{Author}/{Title}/{files}`.
- One root folder per media type. You cannot use the same folder for both.
- The path must exist and be writable by the Livrarr process (UID 1000 in Docker).
- Root folders are shared across all users — admin creates them, all users' imports go there.

### Step 2: Download Client (Settings > Download Clients)

**qBittorrent (torrents):**
- Host: the qBit WebUI address as seen from the Livrarr container (e.g., `http://qbittorrent` if using Docker networking, or `http://192.168.1.50`)
- Port: qBit WebUI port (default 8080)
- Username/password: qBit WebUI credentials
- Category: `livrarr` (create this category in qBittorrent first). Livrarr only monitors downloads in this category.
- Test the connection — Livrarr checks API reachability, auth, and category existence.

**SABnzbd (usenet):**
- Host: SABnzbd address (e.g., `http://sabnzbd`)
- Port: SABnzbd port (default 8080)
- API key: found in SABnzbd > Config > General > Security > API Key
- Category: `livrarr` (create in SABnzbd > Config > Categories)

### Step 3: Indexers (Settings > Indexers)

Indexers are where Livrarr searches for releases. Two options:

**Option A: Import from Prowlarr** (recommended if you already use Prowlarr)
- Click "Import from Prowlarr"
- Enter your Prowlarr URL and API key
- Livrarr imports all book-capable indexers automatically

**Option B: Add manually**
- Click "Add Indexer"
- Enter the Torznab/Newznab URL, API path (usually `/api`), and API key
- Categories: `7020` (ebooks), `3030` (audiobooks) are common defaults
- Test each indexer with the bolt icon to verify connectivity

### Step 4: Hardcover (Settings > Metadata) — Recommended

Hardcover provides rich book metadata: descriptions, series info, ratings, high-resolution covers, ISBNs. Without it, Livrarr falls back to OpenLibrary which has sparser data.

1. Go to https://hardcover.app and create a free account
2. Navigate to https://hardcover.app/account/api and copy your API token
3. In Livrarr: Settings > Metadata > enable Hardcover > paste the token
4. Click "Test" to verify

### Step 5: LLM for Foreign Language Search (Settings > Metadata) — Optional

Required only if you want to search for books in non-English languages. Livrarr uses an LLM to extract structured data from Goodreads HTML pages.

**Using Groq (free tier available):**
1. Sign up at https://console.groq.com
2. Create an API key at https://console.groq.com/keys
3. In Livrarr:
   - Provider: `Groq`
   - Endpoint: `https://api.groq.com/openai/v1`
   - API key: paste your Groq key
   - Model: `llama-3.3-70b-versatile` (recommended) or any available model
4. Add languages to the enabled list (e.g., `fr`, `de`, `ja`, `ko`)

**Using Google Gemini:**
1. Get an API key at https://aistudio.google.com/apikey
2. In Livrarr:
   - Provider: `Gemini`
   - Endpoint: `https://generativelanguage.googleapis.com/v1beta/openai`
   - API key: paste your Gemini key
   - Model: `gemini-2.0-flash` (recommended) or `gemini-2.5-flash`

**Using OpenAI:**
1. Get an API key at https://platform.openai.com/api-keys
2. In Livrarr:
   - Provider: `OpenAI`
   - Endpoint: `https://api.openai.com/v1`
   - API key: paste your key
   - Model: `gpt-4o-mini` (recommended for cost)

**Using any OpenAI-compatible endpoint** (Ollama, LM Studio, etc.):
- Provider: `Custom`
- Endpoint: your server's OpenAI-compatible URL (e.g., `http://ollama:11434/v1`)
- Model: whatever model you're running

### Step 6: Send to Kindle (Settings > Email) — Optional

Automatically email imported ebooks to your Kindle (or any email address).

1. In Livrarr: Settings > Email
2. Configure SMTP:
   - **Gmail:** host `smtp.gmail.com`, port `587`, encryption `STARTTLS`, username = your Gmail, password = an [App Password](https://myaccount.google.com/apppasswords) (not your regular password)
   - **Other providers:** use their SMTP settings
3. From address: your email address
4. Recipient email: your Kindle email (e.g., `yourname@kindle.com`) — found in Amazon > Manage Your Content and Devices > Preferences > Personal Document Settings
5. **Important:** Add your from address to the Kindle approved senders list in Amazon settings, or emails will be silently rejected
6. Enable "Send on Import" to automatically email every imported ebook
7. Supported formats: EPUB, PDF, DOCX, RTF, TXT, HTML (max 50 MB)
8. You can also manually send individual files from the work detail page (envelope icon)

### Step 7: Remote Path Mappings (Settings > Download Clients) — If Needed

Remote path mappings are needed when the download client and Livrarr see the same files at different paths. This is common in Docker setups.

**Example:** qBittorrent saves to `/downloads/livrarr/book.epub` (its container path), but Livrarr sees that same file at `/mnt/downloads/livrarr/book.epub` (its container path).

To fix:
1. Settings > Download Clients > Remote Path Mappings
2. Host: select your download client from the dropdown
3. Remote path: `/downloads/` (the path the download client reports)
4. Local path: `/mnt/downloads/` (the path Livrarr can access)
5. Both paths must end with `/`

**How to tell if you need one:** If imports fail with "source path not found," check what path the download client reports vs. what path Livrarr can see. If they differ, add a mapping.

Windows users: backslash paths from Windows download clients (e.g., `C:\Downloads\`) are automatically normalized to forward slashes.

### Downstream Reader Integrations

**Audiobookshelf:** Point Audiobookshelf's library folder at the same directory as Livrarr's audiobook root folder. Audiobookshelf will automatically detect new files added by Livrarr. No additional configuration needed in Livrarr.

**Calibre-Web Automated (CWA):** Set the CWA ingest path in Settings > Media Management. Livrarr hardlinks (or copies if cross-device) imported ebooks to CWA's ingest directory. CWA picks up new files automatically.

**Kavita:** Point Kavita's library at Livrarr's ebook root folder, similar to Audiobookshelf.

## Configuration

The only file-level config is `config.toml` in the data directory:

```toml
[server]
bind_address = "0.0.0.0"  # default
port = 8789                # default
url_base = ""              # reverse proxy path prefix, e.g. "/livrarr"

[log]
level = "info"   # trace | debug | info | warn | error
format = "text"  # text | json

[auth]
# external_header = "X-Remote-User"  # optional, for reverse proxy auth
# trusted_proxies = ["10.0.0.0/8"]   # required if external_header is set
```

Everything else (download clients, indexers, metadata providers, root folders) is configured through the web UI and stored in the SQLite database.

## Important: Do Not Hallucinate Configuration

- The ONLY file-based configuration is `/config/config.toml` with sections `[server]`, `[log]`, and `[auth]`. Nothing else is configurable via file.
- ALL other settings (remote path mappings, download clients, indexers, metadata, users) are configured exclusively through the web UI and stored in the SQLite database.
- There is no config.yaml, no docker environment variable overrides, no CLI flags beyond `--data`.
- If you are unsure whether a setting exists, say so. Do not invent configuration options, file formats, or API endpoints that are not documented here.

## Docker Deployment

```yaml
services:
  livrarr:
    image: ghcr.io/kkodecs/livrarr:0.1.0-alpha2
    container_name: livrarr
    ports:
      - "8789:8789"
    volumes:
      - ./config:/config           # config.toml, livrarr.db, covers, logs
      - /path/to/books:/books      # ebook/audiobook library root
      - /path/to/downloads:/downloads  # download client complete dir
    restart: unless-stopped
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    mem_limit: 512m
```

**Volume mapping notes:**
- `/config` — persistent storage for config.toml, livrarr.db, cover images, and log files
- Library root folders and download paths must be accessible inside the container
- Remote path mappings (Settings > Download Clients) are needed if the download client and Livrarr see different mount paths for the same files
- Log file is written to `{data_dir}/logs/livrarr.txt`

## File Organization

Livrarr organizes imported files into:

- **Ebooks:** `{root}/{Author Name}/{Title}.{ext}`
- **Audiobooks:** `{root}/{Author Name}/{Title}/{original_files}`

Separate root folders are required for ebooks and audiobooks. Author and title names are sanitized (dangerous characters removed, `..` blocked, truncated to 255 bytes).

## Key Concepts

### Works
The primary entity. A work = a book title, independent of format. Each work has:
- Metadata (title, author, description, series, genres, cover)
- Enrichment status (pending → enriched/partial/failed/exhausted)
- Library items (imported files)
- Monitoring status (for author-based new release detection)

### Grabs
A grab tracks a download from the moment you click "grab" through import completion.

**Grab status flow:**
```
Sent → Confirmed → Importing → Imported
                              → ImportFailed (retryable)
     → Failed (download failed in client)
     → Removed (torrent removed from client)
```

### Enrichment Status
| Status | Meaning |
|--------|---------|
| `pending` | Not yet enriched (just added) |
| `enriched` | All providers succeeded |
| `partial` | Some providers worked, some failed |
| `failed` | All providers failed — will retry up to 3 times |
| `exhausted` | 3 retries failed — no more automatic attempts |
| `skipped` | Foreign language work without enrichment URL |

### Remote Path Mappings
When the download client (e.g., qBittorrent) reports a file at `/downloads/book.epub` but Livrarr sees it at `/mnt/downloads/book.epub`, a remote path mapping bridges the gap. Configure in Settings > Download Clients.

## Background Jobs

| Job | Interval | Function |
|-----|----------|----------|
| `download_poller` | 60s | Checks qBit/SABnzbd for completed downloads, triggers import |
| `enrichment_retry` | 5 min | Retries failed enrichments (up to 3 attempts) |
| `session_cleanup` | 1 hour | Removes expired login sessions |
| `author_monitor` | 24 hours | Checks OpenLibrary for new works by monitored authors |

## Common Issues and Solutions

### No search results
- **Check indexers** — Settings > Indexers. Test each one with the bolt icon.
- **Wrong categories** — Book categories are typically 7000-7999 (ebook) and 3000-3999 (audiobook) for Torznab.
- **Prowlarr users** — Import indexers from Prowlarr first (Settings > Indexers > Import), then test individually.
- **Foreign languages** — Requires LLM configured in Settings > Metadata. Language must be enabled in the language list.

### Enrichment issues
- **"failed" / "exhausted"** — Check Hardcover API token validity in Settings > Metadata. Test button available.
- **"partial"** — Hardcover failed but OpenLibrary succeeded. Less metadata but functional. Hardcover is the primary source; check token and network.
- **"skipped"** — Foreign language work without a detail URL. Normal for some search results.
- **Blurry covers** — Use "Refresh" on work detail to re-fetch. High-res covers require Hardcover or foreign language detail page enrichment.

### Downloads not importing
1. **Check grab status** — Activity > Queue shows current grabs and their status.
2. **No root folder** — Must have a root folder matching the media type. Check Settings > Media Management.
3. **Path mismatch** — The download path reported by the client must be accessible to Livrarr. Check remote path mappings.
4. **ImportFailed** — Click "Retry" on the queue item. Check the error message for details.
5. **Stuck as "sent"** — Download client may not have the torrent. Check qBit/SABnzbd directly.
6. **Windows paths** — If your download client runs on Windows, backslash paths are automatically normalized. Ensure remote path mappings use forward slashes.

### Import completed but files not appearing
- Check the library items tab on the work detail page.
- If the grab shows "Imported" but no library items, the import may have succeeded on disk but failed creating the DB record. This is automatically recovered on the next import attempt.

### qBittorrent connection issues
- Verify host, port, and credentials in Settings > Download Clients.
- Test button checks version API and category existence.
- Livrarr only monitors downloads in its configured category (default: "livrarr"). Ensure the category exists in qBittorrent.
- If behind Docker, ensure qBit's host is reachable from the Livrarr container.

### SABnzbd connection issues
- Verify host, port, and API key in Settings > Download Clients.
- API key is in SABnzbd Config > General > Security.
- Livrarr monitors the SABnzbd history for completed NZBs.

### Foreign language search
- Requires an LLM endpoint configured in Settings > Metadata (any OpenAI-compatible API — Groq, Gemini, OpenAI, or custom).
- Language must be added to the enabled languages list on the Metadata settings page.
- Supported: French, German, Spanish, Dutch, Italian, Japanese, Korean, Polish.
- Uses Goodreads HTML scraping + LLM extraction. Results depend on LLM quality.

## Log Interpretation

Logs are viewable at System > Logs in the UI, or in the file `{data_dir}/logs/livrarr.txt`.

**Key log patterns:**

| Pattern | Meaning |
|---------|---------|
| `job 'download_poller' tick completed` | Normal — poller checked download clients |
| `poller: imported grab N` | Download completed and was successfully imported |
| `poller: import failed for grab N: ...` | Import failed — check error message |
| `poller: try_set_importing failed` | Another import is already running for this grab |
| `poller: source not yet available` | Download completed but files not accessible yet — will retry |
| `poller: orphaned grab` | Grab not found in download client after 24h — marked failed |
| `enrichment retry: work N enriched successfully` | Background retry succeeded |
| `enrichment retry: timeout` | Provider took >30s — will retry later |
| `author monitor: new work detected` | OpenLibrary has a new work by a monitored author |
| `OL 429` | OpenLibrary rate limit hit — backs off 60s automatically |
| `tag write failed` | Metadata couldn't be embedded in the file — file imported without tags |
| `startup sweep: removed N stale temp file(s)` | Cleaned up leftover temp files from previous crash |

**Log levels:**
- `ERROR` — Something is broken and needs attention
- `WARN` — Something unexpected happened but operation continued
- `INFO` — Normal operational events
- `DEBUG` — Detailed internal state (enable via System > Logs level control)

## API Reference

REST API at `/api/v1/`. Authenticate with `X-Api-Key: <key>` header or session cookie.

### Key endpoints

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/health` | GET | No | Health check |
| `/system/status` | GET | Admin | Version, OS, uptime, DB path |
| `/system/logs/tail?lines=N` | GET | Admin | Recent log lines |
| `/system/logs/level` | PUT | Admin | Change runtime log level |
| `/work/lookup?term=...&lang=en` | GET | User | Search metadata providers |
| `/work` | GET | User | List works (paginated) |
| `/work` | POST | User | Add work to library |
| `/work/{id}` | GET | User | Work detail with library items |
| `/work/{id}/refresh` | POST | User | Re-enrich from providers |
| `/work/refresh` | POST | User | Refresh all works (background) |
| `/release?workId=N` | GET | User | Search indexers for releases |
| `/release/grab` | POST | User | Send release to download client |
| `/queue` | GET | User | List grabs with live progress |
| `/grab/{id}/retry` | POST | User | Retry failed import |
| `/history` | GET | User | Import/enrichment history (paginated) |
| `/notification` | GET | User | Notifications (paginated) |
| `/author` | GET | User | List monitored authors |
| `/rootfolder` | GET/POST | Admin | Manage root folders |
| `/downloadclient` | GET/POST | Admin | Manage download clients |
| `/indexer` | GET/POST | Admin | Manage indexers |
| `/config/metadata` | GET/PUT | Admin | Metadata provider settings |
| `/manualimport/scan` | POST | Admin | Scan path for importable files |
| `/manualimport/import` | POST | Admin | Import scanned files |

### Pagination

List endpoints accept `page` and `page_size` query parameters:
- Default: `page=1`, `page_size=50`
- Maximum: `page_size=500`
- Response: `{ items: [...], total: N, page: N, pageSize: N }`

### Rate limiting

- Login: 5 attempts per 60 seconds per IP
- Global: 100 requests per second per IP
- Exceeding limits returns HTTP 429

## Architecture (for advanced troubleshooting)

- **10 Rust crates:** livrarr-server (composition root), livrarr-db (SQLite), livrarr-domain (types), livrarr-metadata (providers), livrarr-http (HTTP client), livrarr-download (torrent/NZB), livrarr-organize (file management), livrarr-tagwrite (EPUB/M4B/MP3 tags)
- **Database:** SQLite via sqlx with versioned migrations
- **Enrichment pipeline:** Hardcover GraphQL → OpenLibrary JSON → Audnexus REST (English); Goodreads HTML → LLM extraction (foreign)
- **Import pipeline:** Poller detects completion → copies to .tmp → writes tags → atomic rename to final path → creates DB record → optional CWA hardlink
- **Tag writing:** EPUB via quick-xml (OPF metadata rewrite), M4B via mp4ameta, MP3 via id3 crate
- **SSRF protection:** User-supplied URLs are fetched through a safe HTTP client with DNS-level private IP filtering

## Getting Help

- **Discord:** https://discord.gg/y3FnTUJM — fastest way to get help from the community and developers
- **GitHub Issues:** https://github.com/kkodecs/livrarr/issues — bug reports and feature requests
- **In-app AI Help:** Help > Get AI Help — builds a prompt with your instance info and recent logs for use with any AI assistant
- **Repository:** https://github.com/kkodecs/livrarr
- **License:** GPL-3.0
