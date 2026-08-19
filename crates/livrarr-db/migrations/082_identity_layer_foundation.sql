-- Identity-layer v2 additive foundation. Activation and its unique Work key
-- remain runtime-controlled so a non-empty legacy database cannot enter a
-- mixed-authority state merely by applying migrations.

ALTER TABLE works ADD COLUMN normalized_identity_main TEXT NOT NULL DEFAULT '';
ALTER TABLE works ADD COLUMN normalized_identity_subtitle TEXT NOT NULL DEFAULT '';
ALTER TABLE works ADD COLUMN normalized_identity_volume TEXT NOT NULL DEFAULT '';
ALTER TABLE works ADD COLUMN text_distinction TEXT NOT NULL DEFAULT 'common';
ALTER TABLE works ADD COLUMN identity_status_v2 TEXT NOT NULL DEFAULT 'not_connected'
    CHECK (identity_status_v2 IN ('user_confirmed', 'connected', 'not_connected'));
ALTER TABLE works ADD COLUMN primary_author_id INTEGER REFERENCES authors(id) ON DELETE SET NULL;
ALTER TABLE works ADD COLUMN identity_title_provenance TEXT NOT NULL DEFAULT '"Migrated"';
ALTER TABLE works ADD COLUMN identity_volume TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_works_user_id_id ON works(user_id, id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_authors_user_id_id ON authors(user_id, id);

CREATE TABLE identity_routes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    owner_type TEXT NOT NULL CHECK (owner_type IN ('work', 'edition')),
    work_id INTEGER,
    edition_id INTEGER,
    resolved_work_id INTEGER NOT NULL,
    provider TEXT NOT NULL,
    kind TEXT NOT NULL,
    provider_scoped_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('active', 'retired')),
    provenance TEXT NOT NULL,
    user_confirmed INTEGER NOT NULL DEFAULT 0 CHECK (user_confirmed IN (0, 1)),
    observed_at TEXT NOT NULL,
    CHECK ((owner_type = 'work' AND work_id IS NOT NULL AND edition_id IS NULL)
        OR (owner_type = 'edition' AND work_id IS NULL AND edition_id IS NOT NULL)),
    FOREIGN KEY (user_id, resolved_work_id) REFERENCES works(user_id, id) ON DELETE CASCADE,
    FOREIGN KEY (user_id, work_id) REFERENCES works(user_id, id) ON DELETE CASCADE,
    FOREIGN KEY (user_id, edition_id) REFERENCES editions(user_id, id) ON DELETE CASCADE
);

CREATE TABLE editions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER NOT NULL,
    format TEXT NOT NULL,
    language TEXT,
    subtitle TEXT,
    subtitle_provenance TEXT,
    source_provider TEXT,
    provider_edition_id TEXT,
    state TEXT NOT NULL CHECK (state IN ('active', 'archived')),
    FOREIGN KEY (user_id, work_id) REFERENCES works(user_id, id) ON DELETE CASCADE,
    UNIQUE (user_id, id),
    UNIQUE (user_id, work_id, id)
);

CREATE TABLE work_contributors (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER NOT NULL,
    author_id INTEGER NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    FOREIGN KEY (user_id, work_id) REFERENCES works(user_id, id) ON DELETE CASCADE,
    FOREIGN KEY (user_id, author_id) REFERENCES authors(user_id, id) ON DELETE RESTRICT,
    PRIMARY KEY (user_id, work_id, author_id),
    UNIQUE (user_id, work_id, ordinal)
);

CREATE TABLE work_contributor_roles (
    user_id INTEGER NOT NULL,
    work_id INTEGER NOT NULL,
    author_id INTEGER NOT NULL,
    role TEXT NOT NULL,
    provenance TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    FOREIGN KEY (user_id, work_id, author_id)
        REFERENCES work_contributors(user_id, work_id, author_id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, work_id, author_id, role, provenance)
);

CREATE TABLE work_relationships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    from_work_id INTEGER NOT NULL,
    relationship_kind TEXT NOT NULL CHECK (relationship_kind IN ('contains', 'part_of')),
    target_work_id INTEGER,
    target_route_provider TEXT,
    target_route_kind TEXT,
    target_route_value TEXT,
    provenance TEXT NOT NULL,
    FOREIGN KEY (user_id, from_work_id) REFERENCES works(user_id, id) ON DELETE CASCADE,
    CHECK (target_work_id IS NOT NULL OR target_route_value IS NOT NULL)
);

CREATE TABLE work_subjects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER NOT NULL,
    subject_kind TEXT NOT NULL CHECK (subject_kind IN ('person', 'place', 'time', 'topic')),
    value TEXT NOT NULL,
    provenance TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    FOREIGN KEY (user_id, work_id) REFERENCES works(user_id, id) ON DELETE CASCADE,
    UNIQUE (user_id, work_id, subject_kind, value, provenance)
);

CREATE TABLE work_default_editions (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER NOT NULL,
    format TEXT NOT NULL,
    edition_id INTEGER NOT NULL,
    provenance TEXT NOT NULL,
    FOREIGN KEY (user_id, work_id, edition_id)
        REFERENCES editions(user_id, work_id, id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, work_id, format)
);

CREATE TABLE edition_cover_candidates (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    edition_id INTEGER NOT NULL,
    candidate_id TEXT NOT NULL,
    source TEXT NOT NULL,
    media_type TEXT NOT NULL CHECK (media_type IN ('ebook', 'audiobook')),
    proxy_url TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    passes_quality_gate INTEGER NOT NULL CHECK (passes_quality_gate IN (0, 1)),
    FOREIGN KEY (user_id, edition_id) REFERENCES editions(user_id, id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, edition_id, candidate_id)
);

CREATE TABLE work_cover_selections (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER NOT NULL,
    format TEXT NOT NULL CHECK (format IN ('ebook', 'audiobook')),
    candidate_id TEXT,
    source TEXT,
    fallback_from_format TEXT,
    provenance TEXT,
    computed_at_generation INTEGER NOT NULL,
    FOREIGN KEY (user_id, work_id) REFERENCES works(user_id, id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, work_id, format)
);

CREATE TABLE embedded_cover_inspections (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    library_item_id INTEGER NOT NULL,
    revision_size_bytes INTEGER NOT NULL,
    revision_modified_ns TEXT NOT NULL,
    revision_sha256 BLOB NOT NULL CHECK (length(revision_sha256) = 32),
    outcome TEXT NOT NULL CHECK (outcome IN ('extracted', 'verified_no_cover', 'could_not_inspect', 'file_gone')),
    cover_candidate_id INTEGER,
    sanitized_error_code TEXT,
    inspected_at TEXT NOT NULL,
    PRIMARY KEY (user_id, library_item_id, revision_size_bytes, revision_modified_ns, revision_sha256)
);

CREATE TABLE machine_subtitle_projections (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER NOT NULL,
    value TEXT,
    edition_id INTEGER,
    provenance TEXT,
    computed_at_generation INTEGER NOT NULL,
    FOREIGN KEY (user_id, work_id) REFERENCES works(user_id, id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, work_id)
);

CREATE TABLE identity_conflicts_v2 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    current_work_id INTEGER NOT NULL,
    class TEXT NOT NULL,
    candidate_provider TEXT NOT NULL,
    candidate_kind TEXT NOT NULL,
    candidate_value TEXT NOT NULL,
    proposed_owner_type TEXT NOT NULL,
    proposed_owner_id INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'resolved')),
    resolution TEXT,
    audit_id INTEGER,
    expected_generation INTEGER NOT NULL,
    FOREIGN KEY (user_id, current_work_id) REFERENCES works(user_id, id) ON DELETE CASCADE
);

CREATE TABLE identity_review_cards (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER,
    kind TEXT NOT NULL,
    generation INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'resolved', 'cancelled')),
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL,
    resolved_at TEXT,
    FOREIGN KEY (user_id, work_id) REFERENCES works(user_id, id) ON DELETE CASCADE
);

CREATE TABLE identity_route_archives (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    kind TEXT NOT NULL,
    route_value TEXT NOT NULL,
    former_owner_type TEXT NOT NULL,
    former_owner_id INTEGER NOT NULL,
    reason TEXT NOT NULL,
    audit_id INTEGER,
    archived_at TEXT NOT NULL
);

CREATE TABLE identity_merge_archives (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    winner_work_id INTEGER NOT NULL,
    loser_work_id INTEGER NOT NULL,
    preserved_fields TEXT NOT NULL,
    audit_id INTEGER,
    archived_at TEXT NOT NULL
);

CREATE TABLE identity_audit_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER,
    event_kind TEXT NOT NULL,
    actor TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE identity_provider_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER NOT NULL,
    provider TEXT NOT NULL,
    route_kind TEXT NOT NULL,
    route_value TEXT NOT NULL,
    attempt_key TEXT NOT NULL,
    outcome TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    FOREIGN KEY (user_id, work_id) REFERENCES works(user_id, id) ON DELETE CASCADE,
    UNIQUE (user_id, work_id, provider, route_kind, route_value, attempt_key)
);

CREATE TABLE identity_cutover_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mode TEXT NOT NULL,
    branch TEXT NOT NULL,
    source_schema_version INTEGER NOT NULL,
    source_fingerprint BLOB NOT NULL,
    canonical_output_fingerprint BLOB NOT NULL,
    status TEXT NOT NULL,
    report_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE identity_cutover_reports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES identity_cutover_runs(id) ON DELETE CASCADE,
    source_schema_version INTEGER NOT NULL,
    source_fingerprint BLOB NOT NULL,
    canonical_output_fingerprint BLOB NOT NULL,
    mapped_route_count INTEGER NOT NULL,
    edition_count INTEGER NOT NULL,
    blocker_count INTEGER NOT NULL,
    index_ready INTEGER NOT NULL CHECK (index_ready IN (0, 1)),
    trivially_empty INTEGER NOT NULL CHECK (trivially_empty IN (0, 1)),
    UNIQUE (run_id)
);

INSERT INTO _livrarr_meta (key, value) VALUES ('schema_version', '82')
ON CONFLICT (key) DO UPDATE SET value = excluded.value;
