# Livrarr — AI Assistant Context

> This file provides context for AI assistants helping Livrarr users with setup, configuration, and troubleshooting. It is referenced automatically by the in-app "Get AI Help" feature.

## What is Livrarr?

Livrarr is a self-hosted ebook and audiobook library manager, similar to Sonarr (TV) and Radarr (movies) but for books. It automates searching, downloading, organizing, and tagging book files.

**Key traits:**
- Works-first model — a "work" = a book title, independent of format or edition
- Manages both ebooks and audiobooks in one app
- Multi-user with per-user library isolation
- Integrates with the *arr ecosystem: Prowlarr, qBittorrent, Transmission, SABnzbd
- Integrates with downstream readers: Calibre-Web Automated (CWA), Audiobookshelf, Kavita
- Metadata from OpenLibrary + Hardcover + Google Books (English and foreign languages); Goodreads via LLM scraping (foreign languages, when available)

**Stack:** Rust backend, React/TypeScript frontend, SQLite database, Docker deployment

## How It Works

1. **Search** — User searches by title. Livrarr queries metadata providers (OpenLibrary, Hardcover for English; Google Books + Goodreads via LLM for foreign languages).
2. **Add** — User adds the work. Livrarr enriches it with description, genres, series info, covers, ratings.
3. **Find releases** — User searches indexers (Torznab/Newznab) for downloadable files.
4. **Download** — Livrarr sends the grab to qBittorrent or Transmission (torrents), or SABnzbd (usenet).
5. **Import** — When download completes, poller detects it, copies files to organized library, writes metadata tags (EPUB only — see note below), creates DB records, optionally hardlinks to CWA.

## Supported File Formats

| Type | Formats | Tag writing |
|------|---------|-------------|
| Ebook | EPUB, MOBI, AZW3, PDF | EPUB only (OPF metadata rewrite) |
| Audiobook | M4B, MP3, M4A, FLAC, OGG, WMA | Disabled — see note |

> **Note on audiobook tag writing:** M4B and MP3 tag writing is currently disabled. The upstream tag writers buffer the shifted media region in RAM, which causes OOM crashes on large audiobook files. Audiobook players (Audiobookshelf, Plex) use their own metadata databases and do not rely on embedded tags, so imports work correctly without them.

## Setup Requirements

All configured through the web UI at `http://<host>:8789`:

| Component | Where | Required? |
|-----------|-------|-----------|
| Root folders | Settings > Media Management | Yes — at least one ebook or audiobook root |
| Download client | Settings > Download Clients | Yes — qBittorrent, Transmission or SABnzbd |
| Indexers | Settings > Indexers | Yes — Torznab/Newznab URLs (or import from Prowlarr) |
| Hardcover token | Settings > Metadata | Recommended — free API token from hardcover.app |
| Google Books API key | Settings > Metadata | Recommended — required for foreign language enrichment |
| LLM endpoint | Settings > Metadata | Optional — OpenAI-compatible API for foreign language support |
| Audnexus | Settings > Metadata | Optional — audiobook narrator data (default public instance) |

## Detailed Setup Guide

After first launch, visit `http://<host>:8789` and complete the setup wizard (create admin account). The wizard asks for a setup token: "Livrarr printed a one-time setup token when it started. Run `docker logs livrarr`, or open the file `setup-token` in your config folder." A missing or wrong token is refused with: "The setup token is missing or wrong. Find it in Livrarr's startup output, or in the file setup-token in its data folder (/config/setup-token in Docker)." Then configure in this order:

### Step 1: Root Folders (Settings > Media Management)

Root folders tell Livrarr where to store imported files. You need at least one.

- **Ebook root folder** — e.g., `/books` (inside container). Imported ebooks go to `{root}/{user_id}/{Author}/{Title}.epub`.
- **Audiobook root folder** — e.g., `/audiobooks` (inside container). Imported audiobooks go to `{root}/{user_id}/{Author}/{Title}/{files}`.
- One root folder per media type. You cannot use the same folder for both.
- The path must exist and be writable by the Livrarr process: the container's `PUID:PGID` user (default 1000:1000), or the `user:` you set.
- Root folders are shared across all users — admin creates them, all users' imports go there.

### Step 2: Download Client (Settings > Download Clients)

**qBittorrent (torrents):**
- Host: the qBit WebUI address as seen from the Livrarr container (e.g., `http://qbittorrent` if using Docker networking, or `http://192.168.1.50`)
- Port: qBit WebUI port (default 8080)
- Username/password: qBit WebUI credentials
- Category: `livrarr` (create this category in qBittorrent first). Livrarr only monitors downloads in this category.
- Test the connection — Livrarr checks API reachability and authentication.

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

Required only if you want to search for books in non-English languages. Livrarr uses an LLM to extract structured data from Goodreads HTML pages and to disambiguate foreign language search results.

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
trusted_proxies = []       # default; e.g. ["10.0.0.0/8"]. Per-IP rate limits trust X-Real-IP / X-Forwarded-For only from these addresses. An entry that is not an IP address or range (a host name, a port, a prefix above /32 or /128) is ignored and reported as a warning in the log and on System > Status.
# url_base: serving Livrarr under a sub-path (e.g. "/livrarr") is not supported yet. The key is accepted and has no effect.

[log]
level = "info"   # trace | debug | info | warn | error
format = "text"  # text | json
```

Everything else (download clients, indexers, metadata providers, root folders) is configured through the web UI and stored in the SQLite database.

## Important: Do Not Hallucinate Configuration

- The ONLY file-based configuration is `/config/config.toml` with sections `[server]`, `[log]`, `[convergence]`, `[metadata_cache]` and `[author_link]`. Nothing else is configurable via file.
- ALL other settings (remote path mappings, download clients, indexers, metadata, users) are configured exclusively through the web UI and stored in the SQLite database.
- There is no config.yaml and no environment-variable override of `config.toml`. The container reads `PUID` and `PGID`, and `RUST_LOG` replaces the log filter. Command-line options: `--data`, `--ui-dir` and the `identity-cutover` subcommand.
- If you are unsure whether a setting exists, say so. Do not invent configuration options, file formats, or API endpoints that are not documented here.

## Docker Deployment

```yaml
services:
  livrarr:
    image: ghcr.io/kkodecs/livrarr:0.1.0-alpha6
    container_name: livrarr
    environment:
      - PUID=1000            # your host user id  (run `id -u`)
      - PGID=1000            # your host group id (run `id -g`)
      - TZ=Etc/UTC           # e.g. America/New_York
    ports:
      - 8789:8789
    volumes:
      - ./config:/config           # config.toml, livrarr.db, covers, logs
      - /path/to/books:/books      # ebook/audiobook library root
      - /path/to/downloads:/downloads  # download client complete dir
    restart: unless-stopped
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    cap_add:                 # minimal set: fix /config ownership, then drop root → PUID:PGID
      - CHOWN
      - SETUID
      - SETGID
      - DAC_OVERRIDE
    mem_limit: 512m
    cpus: 2.0
    healthcheck:
      test: ["CMD-SHELL", "wget --no-verbose --tries=1 --spider http://127.0.0.1:8789/api/v1/health || exit 1"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 10s
```

**Volume mapping notes:**
- `/config` — persistent storage for config.toml, livrarr.db, cover images, and log files
- Library root folders and download paths must be accessible inside the container
- Remote path mappings (Settings > Download Clients) are needed if the download client and Livrarr see different mount paths for the same files
- Log file is written to `{data_dir}/logs/livrarr.log.<YYYY-MM-DD>`, a new file each day
- After each change of day Livrarr keeps today's log file and the 30 most recent earlier files, and after a restart later the same day it keeps today's file and the 29 most recent earlier ones, ordered by file creation time and counting every file whose name starts with `livrarr.log` (so an old undated `livrarr.log` is deleted like a dated one); days Livrarr did not run leave no file and use none of the 30
- The image's health check calls `http://127.0.0.1:8789/api/v1/health` inside the container, so if you change `port`, or set `bind_address` to one specific address, override the health check (in compose, with your own `healthcheck` block) to use the new address

## File Organization

Livrarr organizes imported files into:

- **Ebooks:** `{root}/{user_id}/{Author Name}/{Title}.{ext}`
- **Audiobooks:** `{root}/{user_id}/{Author Name}/{Title}/{original_files}`

Separate root folders are required for ebooks and audiobooks. Author and title names are sanitized (dangerous characters removed, `..` blocked, truncated to 255 bytes).

## Key Concepts

### Works
The primary entity. A work = a book title, independent of format. Each work has:
- Metadata (title, author, description, series, genres, cover)
- Enrichment status (see below)
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
| `unenriched` | Not yet enriched (just added, or crash recovery state) |
| `enriched` | Enrichment completed successfully |
| `failed` | Transient enrichment error — background job will retry |
| `conflict` | LLM identity validation rejected all provider matches; terminal until user resets via manual refresh |
| `identity_pending` | Identity could not be resolved at add-time; background job will retry |
| `needs_review` | Resolution exhausted for a work with no resolving identifier; user must act |

### Remote Path Mappings
When the download client (e.g., qBittorrent) reports a file at `/downloads/book.epub` but Livrarr sees it at `/mnt/downloads/book.epub`, a remote path mapping bridges the gap. Configure in Settings > Download Clients.

## Background Jobs

| Job | Interval | Function |
|-----|----------|----------|
| `download_poller` | 60s | Checks qBittorrent, Transmission and SABnzbd for completed downloads, triggers import |
| `rss_sync` | 60s | Polls all enabled RSS-capable indexers for new releases matching monitored works |
| `enrichment_retry` | 5 min | Retries failed/pending enrichments |
| `tag_convergence` | 60s | Writes missing tags to already-imported files |
| `state_map_cleanup` | 30 min | Evicts expired in-memory state entries |
| `session_cleanup` | 1 hour | Removes expired login sessions |
| `author_monitor` | 24 hours | Checks OpenLibrary for new works by monitored authors |

## Common Issues and Solutions

### No search results
- **Check indexers** — Settings > Indexers. Test each one with the bolt icon.
- **Wrong categories** — Book categories are typically 7000-7999 (ebook) and 3000-3999 (audiobook) for Torznab.
- **Prowlarr users** — Import indexers from Prowlarr first (Settings > Indexers > Import), then test individually.
- **Foreign languages** — Requires LLM configured in Settings > Metadata. Language must be enabled in the language list.

### Enrichment issues
- **`failed`** — Transient error; background job retries automatically. Check Hardcover API token and network in Settings > Metadata.
- **`conflict`** — LLM rejected all provider matches for this work. Use "Manual Refresh" on the work detail page to reset and retry.
- **`identity_pending`** — Normal transitional state; the background job will resolve it shortly.
- **`needs_review`** — No provider could be matched confidently. Open the work and use the search/manual match flow.
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
- Test button checks API reachability and authentication only — it does not verify the category exists.
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
- Foreign language enrichment uses Google Books as the primary metadata source, with Goodreads HTML scraping + LLM extraction as a secondary path. Results depend on LLM quality and provider availability.

## Log Interpretation

Logs are viewable at System > Logs in the UI, or in the file `{data_dir}/logs/livrarr.log.<YYYY-MM-DD>` (a new file each day).

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
| `enrichment_retry: enrich_work(...) timed out` | Enrichment exceeded 30s — will retry later |
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

REST API at `/api/v1/`. Authenticate with an `X-Api-Key: <key>` header, or with `Authorization: Bearer <token>` using the session token from login.

### Key endpoints

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/health` | GET | No | Health check: reads the database within 2 seconds; 200 with `database` / `ok`, or 503 with `database` / `error` and the message "database check failed" |
| `/system/health` | GET | Admin | Health Checks list on System > Status: the database check with its failure detail, then config warnings (ignored `trusted_proxies` entries, unknown config keys) |
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

### Login protection

Login is rate limited per IP: a burst of 5 attempts, then one more every 12 seconds. Every API route also has a per-IP limit. A username locks for 15 minutes after 5 failed logins.

## Architecture (for advanced troubleshooting)

- **17 Rust crates:** livrarr-server (composition root), livrarr-handlers (route handlers, compile-walled), livrarr-jobs (job triggering trait), livrarr-db (SQLite), livrarr-domain (types/traits), livrarr-metadata (orchestration), livrarr-external-data (provider clients), livrarr-identity, livrarr-enrichment, livrarr-materialize, livrarr-http (HTTP client), livrarr-download (torrent/NZB), livrarr-matching (file matching), livrarr-library (import/layout), livrarr-tagwrite (EPUB tags), livrarr-behavioral (test harness), livrarr-cli (stub)
- **Database:** SQLite via sqlx with versioned migrations
- **Enrichment pipeline:** Hardcover GraphQL → OpenLibrary JSON → Audnexus REST (English); Google Books → Goodreads HTML → LLM extraction (foreign)
- **Import pipeline:** Poller detects completion → copies to .tmp → writes tags (EPUB only) → atomic rename to final path → creates DB record → optional CWA hardlink
- **Tag writing:** EPUB via quick-xml (OPF metadata rewrite). M4B and MP3 tag writing is currently disabled due to OOM constraints with large audiobook files.
- **SSRF protection:** Admin-configured infrastructure (download clients, indexers, LLM endpoints) uses an unrestricted HTTP client; runtime-derived URLs (cover fetches, scraped content) use a safe HTTP client with DNS-level private IP filtering

## Getting Help

- **Discord:** https://discord.gg/PJDsgjEvCV — fastest way to get help from the community and developers
- **GitHub Issues:** https://github.com/kkodecs/livrarr/issues — bug reports and feature requests
- **In-app AI Help:** Help > Get AI Help — builds a prompt with your instance info and recent logs for use with any AI assistant
- **Repository:** https://github.com/kkodecs/livrarr
- **License:** GPL-3.0
