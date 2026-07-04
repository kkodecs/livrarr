# Principles

Project-specific fundamental design principles for Livrarr. Non-negotiable guardrails that guide every decision and every line of code. Loaded into every reviewer prompt, every block invocation. The tiebreaker for ambiguous decisions.

**Authority:** Highest tier. Principles override spec requirements, IR contracts, tests, and implementation when they conflict.

---

## 1. Work-first, not author-first
The user wants a book. Authors are metadata, not the entry point. The Work is the primary entity in the data model, the UI, and every workflow.
**Rationale:** Prevents fragmented UX where the same book appears in multiple places.

## 2. One app, both formats
Livrarr manages ebooks and audiobooks together. A work represents a title — independent of format, edition, or packaging. Users search for works, add works, then find and grab releases in whichever media type they want.
**Rationale:** Users think in titles, not file formats.

## 3. Ecosystem citizen
Integrate with Prowlarr, qBittorrent, Audiobookshelf, Kavita, Calibre-Web Automated. Follow Servarr conventions for API shape and terminology. Don't reinvent what's solved.
**Rationale:** Self-hosted users already have a stack. Livrarr fits in, not replaces.

## 4. Multi-user from day one
Every user-scoped table has `user_id`. Infrastructure (root folders, download clients) is shared and admin-managed. User data (works, library items, grabs) is isolated at the query layer — no unscoped queries, ever.
**Rationale:** Retrofitting multi-user is architectural surgery. Build it in from the start.

## 5. The file is the artifact
Metadata belongs inside the file. A correctly tagged EPUB or M4B is self-contained — it works in any tool without Livrarr. Tag writing happens at import time. Files already in the library are not modified without explicit user action, except as deferred completion of a user-initiated workflow (e.g., async LLM metadata resolution updating tags after the original add).
**Rationale:** Files outlive applications. Metadata must travel with the file.

## 6. Enrich eagerly, degrade gracefully
Enrichment is synchronous where the user is watching (Add Work, manual-import review) and converges in the background where they are not (list/Readarr import, series/author monitors). No Hardcover token? Open Library only. Audnexus miss? Skip audiobook fields. Provider down? Serve what you have. Nothing blocks — interactive creation returns fully-formed; background paths may create identity-pending works that converge to the same identity (M9, REQ-022) and surface a terminal needs-review state when unresolvable.
**Rationale:** Partial metadata is always better than a loading spinner or an error; eventual consistency is confined to the paths the user does not watch.

## 7. Opinionated filesystem, tolerant consumers
Livrarr owns the layout. Ebooks flat: `{root}/{user_id}/{Author}/{Title}.ext`. Audiobooks in directories: `{root}/{user_id}/{Author}/{Title}/{files}`. Separate roots per media type. Per-user subdirectories ensure file isolation within shared root folders. Downstream tools adapt to Livrarr's output.
**Rationale:** Consistent layout enables automation and prevents cross-user file access.

## 8. Copy to library
Import copies files from the download directory to the organized library. The original stays in the download directory for torrent seeding. Tag writing modifies only the library copy. No hardlinking for library import — the library copy is always independent of the source. Exception: CWA downstream integration uses hardlink first with copy fallback, since the CWA copy is identical to the tagged library copy and is never modified.
**Rationale:** Independence from the download client. No broken links when torrents are removed.

## 9. Automated discovery, automated organization
Author monitoring auto-adds new works. RSS sync auto-grabs matching releases for monitored works. After the grab, the system handles everything: download, import, organize, tag. The user sets policy (what to monitor, match thresholds); the system executes.
**Rationale:** Full automation for the common path. Manual intervention only for ambiguous cases.

## 10. Failure isolation
External dependency failures degrade capability, never corrupt state. CWA copy fails? Log warning, main import succeeds. Provider times out? Create the work with available data. Download client unreachable? Surface the error, don't lose the grab intent.
**Rationale:** Partial success is better than total failure. Never lose user intent.

## 11. LLM as metadata advisor
LLM assists with metadata repair (e.g., extracting fields from provider HTML that deterministic parsing misses). It never selects matches, never triggers downloads, never mutates library state, never auto-accepts. Deterministic matching decides; ambiguity goes to the user, not an LLM. Fully functional without an LLM configured.
**Rationale:** LLMs are probabilistic. They advise; they don't decide.

## 12. Secure by default
Self-hosted doesn't mean insecure. Passwords hashed with argon2id. Session tokens and API keys stored as SHA-256 hashes — plaintext shown once, never retrievable. No anonymous access, no network-based auth bypass. Download client passwords are the exception — stored plaintext per Servarr convention, redacted in API responses.
**Rationale:** Self-hosted users are exposed to their local network. Secure defaults protect them.

## 13. Self-hosted, Linux-only, Docker-first
Users control their own hardware and data. Target platform is Linux. Single-container Docker deployment. Use cross-platform Rust APIs but only test and support Linux.
**Rationale:** Focus. One platform done well beats three done poorly.

## 14. Opinionated simplicity
Fewer features done well over comprehensive coverage done poorly. Sensible defaults, works out of the box for most users. When in doubt, leave it out.
**Rationale:** Complexity is the enemy of reliability.

## 15. Fast
Responsive on modest hardware. A Raspberry Pi 4 is the performance floor. No operation should feel slow to the user.
**Rationale:** Self-hosted means running on whatever hardware people have.
