-- Livrarr initial schema
-- Source: spec-livrarr-v2.md Section 7 + v2.1 extensions

-- ============================================================================
-- Authentication
-- ============================================================================

CREATE TABLE users (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT NOT NULL,
    password_hash   TEXT NOT NULL,
    role            TEXT NOT NULL CHECK(role IN ('admin', 'user')),
    api_key_hash    TEXT NOT NULL UNIQUE,
    setup_pending   INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_users_username_ci ON users(LOWER(username));

CREATE TABLE sessions (
    token_hash      TEXT PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    persistent      INTEGER NOT NULL,
    created_at      TEXT NOT NULL,
    expires_at      TEXT NOT NULL
);

-- Placeholder admin (setup_pending=1, empty hashes until setup wizard runs)
INSERT INTO users (id, username, password_hash, role, api_key_hash, setup_pending, created_at, updated_at)
VALUES (1, 'admin', '', 'admin', '', 1,
        strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
        strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

-- ============================================================================
-- Core: Authors and Works
-- ============================================================================

CREATE TABLE authors (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    sort_name           TEXT,
    ol_key              TEXT,
    monitored           INTEGER NOT NULL DEFAULT 0,
    monitor_new_items   INTEGER NOT NULL DEFAULT 0,
    monitor_since       TEXT,
    added_at            TEXT NOT NULL
);
CREATE INDEX idx_authors_user_id ON authors(user_id);

CREATE TABLE works (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id                 INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title                   TEXT NOT NULL,
    sort_title              TEXT,
    subtitle                TEXT,
    original_title          TEXT,
    author_name             TEXT NOT NULL,
    author_id               INTEGER REFERENCES authors(id) ON DELETE SET NULL,
    description             TEXT,
    year                    INTEGER,
    series_name             TEXT,
    series_position         REAL,
    genres                  TEXT,       -- JSON array
    language                TEXT,
    page_count              INTEGER,
    duration_seconds        INTEGER,
    publisher               TEXT,
    publish_date            TEXT,
    ol_key                  TEXT,
    hardcover_id            TEXT,
    isbn_13                 TEXT,
    asin                    TEXT,
    narrator                TEXT,       -- JSON array
    narration_type          TEXT,
    abridged                INTEGER DEFAULT 0,
    rating                  REAL,
    rating_count            INTEGER,
    enrichment_status       TEXT NOT NULL DEFAULT 'pending',
    enriched_at             TEXT,
    enrichment_source       TEXT,
    cover_url               TEXT,
    cover_manual            INTEGER NOT NULL DEFAULT 0,
    monitored               INTEGER NOT NULL DEFAULT 1,
    added_at                TEXT NOT NULL,
    enrichment_retry_count  INTEGER NOT NULL DEFAULT 0  -- v2.1 extension
);
CREATE INDEX idx_works_user_id ON works(user_id);

CREATE TABLE external_ids (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    work_id     INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    id_type     TEXT NOT NULL,
    id_value    TEXT NOT NULL,
    UNIQUE(work_id, id_type, id_value)
);

-- ============================================================================
-- Library
-- ============================================================================

CREATE TABLE root_folders (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    path        TEXT NOT NULL UNIQUE,
    media_type  TEXT NOT NULL CHECK(media_type IN ('ebook', 'audiobook'))
);
CREATE UNIQUE INDEX idx_root_folders_media_type ON root_folders(media_type);

CREATE TABLE library_items (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id         INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    root_folder_id  INTEGER NOT NULL REFERENCES root_folders(id) ON DELETE RESTRICT,
    path            TEXT NOT NULL,
    media_type      TEXT NOT NULL CHECK(media_type IN ('ebook', 'audiobook')),
    file_size       INTEGER NOT NULL,
    imported_at     TEXT NOT NULL,
    UNIQUE(user_id, root_folder_id, path)
);
CREATE INDEX idx_library_items_user_id ON library_items(user_id);

-- ============================================================================
-- Configuration (singleton rows)
-- ============================================================================

CREATE TABLE naming_config (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    author_folder_format    TEXT NOT NULL DEFAULT '{Author Name}',
    book_folder_format      TEXT NOT NULL DEFAULT '{Book Title}',
    rename_files            INTEGER NOT NULL DEFAULT 0,
    replace_illegal_chars   INTEGER NOT NULL DEFAULT 1
);
INSERT INTO naming_config (id) VALUES (1);

CREATE TABLE media_management_config (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    cwa_ingest_path TEXT
);
INSERT INTO media_management_config (id) VALUES (1);

CREATE TABLE prowlarr_config (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    url     TEXT,
    api_key TEXT,
    enabled INTEGER NOT NULL DEFAULT 0
);
INSERT INTO prowlarr_config (id) VALUES (1);

CREATE TABLE metadata_config (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    hardcover_api_token TEXT,
    llm_provider        TEXT,
    llm_endpoint        TEXT,
    llm_api_key         TEXT,
    llm_model           TEXT,
    audnexus_url        TEXT NOT NULL DEFAULT 'https://api.audnex.us',
    languages           TEXT NOT NULL DEFAULT '["en"]'
);
INSERT INTO metadata_config (id) VALUES (1);

-- ============================================================================
-- Download Clients
-- ============================================================================

CREATE TABLE download_clients (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    name                TEXT NOT NULL,
    implementation      TEXT NOT NULL DEFAULT 'qBittorrent',
    host                TEXT NOT NULL,
    port                INTEGER NOT NULL DEFAULT 8080,
    use_ssl             INTEGER NOT NULL DEFAULT 0,
    skip_ssl_validation INTEGER NOT NULL DEFAULT 0,
    url_base            TEXT,
    username            TEXT,
    password            TEXT,
    category            TEXT NOT NULL DEFAULT 'livrarr',
    enabled             INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE remote_path_mappings (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    host        TEXT NOT NULL,
    remote_path TEXT NOT NULL,
    local_path  TEXT NOT NULL
);

-- ============================================================================
-- Grabs (download tracking)
-- ============================================================================

CREATE TABLE grabs (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id             INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    download_client_id  INTEGER NOT NULL REFERENCES download_clients(id),
    title               TEXT NOT NULL,
    indexer             TEXT NOT NULL,
    guid                TEXT NOT NULL,
    size                INTEGER,
    download_url        TEXT NOT NULL,
    download_id         TEXT,
    status              TEXT NOT NULL DEFAULT 'sent',
    import_error        TEXT,
    grabbed_at          TEXT NOT NULL,
    UNIQUE(user_id, guid, indexer)
);
CREATE INDEX idx_grabs_user_id ON grabs(user_id);

-- ============================================================================
-- History and Notifications
-- ============================================================================

CREATE TABLE history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id     INTEGER REFERENCES works(id) ON DELETE SET NULL,
    event_type  TEXT NOT NULL,
    data        TEXT NOT NULL DEFAULT '{}',
    date        TEXT NOT NULL
);
CREATE INDEX idx_history_user_id ON history(user_id);
CREATE INDEX idx_history_date ON history(date);

CREATE TABLE notifications (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    type        TEXT NOT NULL,
    ref_key     TEXT,
    message     TEXT NOT NULL,
    data        TEXT NOT NULL DEFAULT '{}',
    read        INTEGER NOT NULL DEFAULT 0,
    dismissed   INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_notifications_user_id ON notifications(user_id);
CREATE UNIQUE INDEX idx_notifications_dedup ON notifications(user_id, type, ref_key);
