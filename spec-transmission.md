---
feature: transmission
stage: spec
status: draft
version: 1
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-010, REQ-011, REQ-012, REQ-013, REQ-014, REQ-015]
---

# Spec: transmission

## 0a. Fundamental Design Principles

- **FDP-1: Same UX, different engine.** Adding a Transmission client should feel identical to adding a qBittorrent client. The user picks a type, fills in connection details, tests, saves. The download/import flow is indistinguishable after that.
- **FDP-2: Hash-based ownership.** Livrarr only manages torrents it added. It never touches, removes, or imports from torrents the user added independently in Transmission. Torrent hashes stored in grab records are the source of truth.
- **FDP-3: No version gating.** The integration must work with Transmission 3.x and 4.x. Features exclusive to 4.0+ (labels) are not relied upon.
- **FDP-4: Download directory as isolation boundary.** Transmission has no categories. A user-configured download directory serves the same role — keeping Livrarr's downloads physically separated from other Transmission activity.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | Transmission RPC spec | The server returns HTTP 409 with a new `X-Transmission-Session-Id` header when the session ID is missing or expired. The client must cache and resend. | Code that treats 409 as a fatal error or does not retry with the new session ID | Documented (rpc-spec.md) |
| ST-002 | Transmission RPC spec | `torrent-add` accepts either `filename` (magnet URI or URL) or `metainfo` (base64-encoded .torrent file), never both. Returns `torrent-added` with `id`, `name`, `hashString` on success; `torrent-duplicate` if already present. | Code that sends both `filename` and `metainfo` in a single request | Documented (rpc-spec.md) |
| ST-003 | Transmission RPC spec | Torrent status is an integer 0-6: 0=Stopped, 1=QueuedVerify, 2=Verifying, 3=QueuedDownload, 4=Downloading, 5=QueuedSeed, 6=Seeding | Code that assumes status values outside this range or uses string-based status matching | Documented (rpc-spec.md) |
| ST-004 | Transmission RPC spec | `torrent-get` requires a `fields` array in the request specifying which fields to return. Omitting `fields` returns nothing useful. | Code that calls `torrent-get` without specifying fields | Documented (rpc-spec.md) |
| ST-005 | Transmission RPC spec | Authentication uses HTTP Basic Auth (username/password). The session-ID header is a CSRF protection layer on top of Basic Auth. Both are required when auth is enabled. | Code that omits Basic Auth credentials or treats session-ID as the sole auth mechanism | Documented (rpc-spec.md) |
| ST-006 | Transmission RPC spec | `download-dir` in `torrent-add` overrides the session default. The path must exist and be writable by the Transmission daemon. | Code that assumes Livrarr can create the download directory remotely | Documented (rpc-spec.md) |
| ST-007 | Transmission RPC spec (v4.1.0+) | Transmission 4.1.0+ uses snake_case field names in JSON-RPC 2.0. Older versions use kebab-case in a bespoke protocol. Both are supported in Transmission 4.x. | Code that only handles one naming convention | Documented (rpc-spec.md) |

## 1. Problem Statement

Livrarr supports qBittorrent for torrent downloads and SABnzbd for usenet. Users who run Transmission as their torrent client cannot use Livrarr for automated torrent downloading — they must manually download and import files. This is a community-requested feature (GitHub issue #17).

Adding Transmission support enables all torrent operations (grab, queue monitoring, import detection) for Transmission users, matching the qBittorrent experience.

## 2. Requirements

### Connection & Configuration

- **REQ-001**: The system must support Transmission as a download client implementation type alongside qBittorrent and SABnzbd.
- **REQ-002**: Transmission client configuration must accept: name, host, port, SSL toggle, SSL validation skip, URL base, username, password, and download directory. The download directory specifies where Transmission should save Livrarr's downloads (equivalent to qBittorrent's category for isolation purposes).
- **REQ-003**: The system must authenticate to Transmission using HTTP Basic Auth (username/password) and handle the X-Transmission-Session-Id CSRF header transparently. On HTTP 409, the client must extract the new session ID from the response header and retry the request once. If the retry also returns 409, treat it as a fatal authentication error.
- **REQ-004**: A test connection function must verify: (a) successful authentication, (b) RPC version >= 14 (Transmission 2.80+, required for `free-space` support), and (c) that the configured download directory is accessible (via the `free-space` RPC method). If the download directory does not exist or is not writable, the test must fail with a clear error message.

### Torrent Operations

- **REQ-005**: The system must add torrents to Transmission via the `torrent-add` RPC method, using `filename` for magnet URIs and `metainfo` (base64-encoded) for .torrent file uploads. The configured download directory must be passed as `download-dir` in every add request.
- **REQ-006**: When `torrent-add` returns `torrent-duplicate` (torrent already exists in Transmission), the system must treat this as a successful add — extract the hash from the duplicate response and proceed with grab tracking. The torrent may exist in a different download directory than configured; the poller still matches by hash regardless of directory. This matches qBittorrent behavior where re-adding an existing torrent is idempotent.
- **REQ-007**: The system must retrieve torrent status from Transmission via the `torrent-get` RPC method. The following fields must be requested: `hashString`, `name`, `status`, `percentDone`, `totalSize`, `sizeWhenDone`, `downloadDir`, `eta`, `error`, `errorString`, `isFinished`, `leftUntilDone`.

### Download Polling & Import

- **REQ-008**: The download poller must poll Transmission clients alongside qBittorrent and SABnzbd clients. For each enabled Transmission client, the poller must retrieve all torrents from Transmission and match them against active grab records by torrent hash (source of truth). Torrents not matching any grab record are ignored — Livrarr never interacts with torrents it didn't add.
- **REQ-009**: A Transmission torrent is considered "completed" when `percentDone == 1.0` (or equivalently `leftUntilDone == 0`) AND `status` is 0 (Stopped), 5 (QueuedSeed), or 6 (Seeding). Both conditions are required — status 0 alone could mean a manually-stopped incomplete torrent. The poller must trigger the import pipeline for completed torrents. The content path is derived from the torrent's `downloadDir` and `name` fields as returned by the Transmission API (not constructed from user config).
- **REQ-010**: Torrent progress reporting must map Transmission's `percentDone` (0.0-1.0) to the queue display's percentage format. ETA must be derived from the `eta` field (-1 = not available, -2 = unknown).
- **REQ-011**: Transmission torrents with `error > 0` must be reported in the queue display with the `errorString` shown to the user. Error severity must be mapped: `error == 1` (tracker warning) is displayed as a warning but does not block download progress; `error == 2` (tracker error) or `error == 3` (local error) are displayed as errors.

### Queue & Removal

- **REQ-012**: The queue endpoint must include Transmission torrents alongside qBittorrent torrents. Each queue item must show: title, status, progress, size, ETA, download client name, and indexer (from grab record).
- **REQ-013**: Removing a grab from the Livrarr queue must NOT remove the torrent from Transmission (consistent with qBittorrent behavior — the torrent stays in the client, Livrarr just stops tracking it).

### Data Model

- **REQ-014**: The `DownloadClient` entity must support a `download_dir` field (optional string). This field is used by Transmission to specify the download directory. For qBittorrent and SABnzbd, this field is unused (null). The existing `category` field continues to serve qBittorrent and SABnzbd.
- **REQ-015**: Transmission must map to `DownloadProtocol::Torrent`. A user may have both a qBittorrent client and a Transmission client configured. The `is_default_for_protocol` field determines which torrent client receives grabs by default.

## 3. UI/Interface Design

The settings UI follows the exact same pattern as the existing download client form.

- **Download Client Type dropdown**: adds "Transmission" as a third option alongside "qBittorrent" and "SABnzbd"
- **When Transmission is selected**:
  - Show: Name, Host, Port, SSL toggle, Skip SSL Validation, URL Base, Username, Password, Download Directory
  - Hide: Category (not applicable), API Key (not applicable)
  - The Download Directory field has placeholder text: e.g., `/downloads/livrarr`
  - A help tip explains: "Directory where Transmission saves Livrarr downloads. Must exist and be writable by Transmission."
- **When qBittorrent is selected**: form unchanged (Category shown, Download Directory hidden)
- **When SABnzbd is selected**: form unchanged (Category shown, Download Directory hidden)
- **Test button**: same behavior — calls test endpoint, shows success/failure toast

No new pages. No new modals. The existing download client settings page handles everything.

## 4. Non-Requirements

- **No torrent removal from Transmission.** Livrarr does not delete torrents from the client — only stops tracking them. Consistent with qBittorrent behavior.
- **No Transmission labels support.** Labels are Transmission 4.0+ only. We use download directory + hash tracking instead.
- **No Transmission settings management.** Livrarr does not configure Transmission's global settings (speed limits, peer limits, encryption, etc.).
- **No per-torrent speed/priority control.** Livrarr adds torrents with default priority. User can adjust in Transmission's own UI.
- **No Transmission daemon management.** Livrarr does not start/stop/restart the Transmission daemon.
- **No alternative RPC transports.** Only HTTP/HTTPS RPC. No Unix socket support.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Should we support both kebab-case (Transmission 3.x) and snake_case (Transmission 4.x) field names? | resolved | Yes. The client must handle both formats. Transmission 4.x still accepts kebab-case, so we send kebab-case requests (maximum compatibility) and handle both response formats. |
| Q-002 | What happens when the download directory doesn't exist? | resolved | Test connection fails with a clear error. Add-torrent will also fail at runtime with a Transmission error. The UI help tip tells the user the directory must exist. |
| Q-003 | How to determine content path for multi-file torrents? | resolved | Use `downloadDir` + torrent `name` from `torrent-get`. For single-file torrents, this is the file. For multi-file, this is the directory containing the files. Same approach as qBittorrent's `content_path`. |
| Q-004 | Should the download directory be required or optional? | resolved | Required when implementation is Transmission. The UI validates this. Without it, Livrarr can't isolate its downloads. |
| Q-005 | Protocol field name format — kebab-case or snake_case in requests? | resolved | Send kebab-case (`download-dir`, `hash-string`) for maximum compatibility with Transmission 3.x and 4.x. Parse responses flexibly. |

## 6. Acceptance Criteria

### Connection & Configuration
- [ ] **AC-001** (REQ-001): "Transmission" appears in the download client type dropdown alongside "qBittorrent" and "SABnzbd".
- [ ] **AC-002** (REQ-002): Saving a Transmission client with host, port, username, password, and download directory succeeds. The client appears in the download clients list.
- [ ] **AC-003** (REQ-002): The Download Directory field is shown when Transmission is selected and hidden for qBittorrent/SABnzbd. The Category field is hidden for Transmission.
- [ ] **AC-004** (REQ-003): Connecting to a Transmission instance that returns 409 on the first request succeeds — the client automatically retries with the new session ID.
- [ ] **AC-005** (REQ-004): Test connection to a valid Transmission instance succeeds. Test connection with wrong credentials fails with an auth error. Test connection with a non-existent download directory fails with a directory error.

### Torrent Operations
- [ ] **AC-006** (REQ-005): Grabbing a torrent release with Transmission as the default torrent client adds the torrent to Transmission in the configured download directory.
- [ ] **AC-007** (REQ-005): Grabbing a magnet link uses the `filename` field. Grabbing a .torrent file uses the `metainfo` field with base64 encoding.
- [ ] **AC-008** (REQ-006): Re-grabbing a release that already exists in Transmission succeeds (torrent-duplicate response handled gracefully).
- [ ] **AC-009** (REQ-007): The queue page shows Transmission torrent progress (percentage, ETA, size) updating on each poll cycle.

### Download Polling & Import
- [ ] **AC-010** (REQ-008): A completed torrent in Transmission (seeding, in the configured download directory, matching a grab hash) triggers automatic import.
- [ ] **AC-011** (REQ-009): A torrent that is still downloading shows as "Downloading" in the queue with accurate progress.
- [ ] **AC-012** (REQ-009): A torrent that finishes downloading and starts seeding transitions from "Downloading" to "Completed" and triggers import.
- [ ] **AC-013** (REQ-010): ETA displays correctly: positive values show time remaining, -1 shows "unknown".
- [ ] **AC-014** (REQ-011): A torrent with an error in Transmission shows the error message in the queue.

### Queue & Removal
- [ ] **AC-015** (REQ-012): The queue page shows torrents from both qBittorrent and Transmission clients (if both are configured and enabled).
- [ ] **AC-016** (REQ-013): Removing a grab from the Livrarr queue does not remove the torrent from Transmission.

### Data Model
- [ ] **AC-017** (REQ-014): The database migration adds a `download_dir` column to the download clients table. Existing clients have null for this field.
- [ ] **AC-018** (REQ-015): When both a qBittorrent and Transmission client exist, only the one marked `is_default_for_protocol` receives torrent grabs.

### Multi-client Coexistence
- [ ] **AC-019** (REQ-008, REQ-015): The download poller correctly polls both a qBittorrent client (by category) and a Transmission client (by download directory + hash) in the same tick without cross-contamination.
- [ ] **AC-020** (REQ-008): Torrents added directly by the user in Transmission (not through Livrarr) are ignored by the poller — they don't appear in the queue and don't trigger import.
