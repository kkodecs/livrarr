CREATE TABLE author_provider_routes (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    author_id           INTEGER NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    provider            TEXT NOT NULL CHECK(provider IN ('open_library', 'goodreads', 'hardcover')),
    route_value         TEXT NOT NULL,
    state               TEXT NOT NULL CHECK(state IN ('active', 'removed')),
    provenance          TEXT NOT NULL CHECK(provenance IN (
                            'legacy_unguarded',
                            'tier1_inherited',
                            'readarr_guarded',
                            'user_picked',
                            'merge_coalesced'
                        )),
    evidence_work_id    INTEGER REFERENCES works(id) ON DELETE SET NULL,
    created_at          TEXT NOT NULL,
    verified_at         TEXT,
    removed_at          TEXT,
    removed_by_user_id  INTEGER REFERENCES users(id) ON DELETE SET NULL,
    UNIQUE(user_id, provider, route_value)
);

CREATE INDEX idx_author_provider_routes_author
    ON author_provider_routes(user_id, author_id, state, provider, id);

CREATE TABLE author_name_variants (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    author_id           INTEGER NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    canonical_name      TEXT NOT NULL,
    source              TEXT NOT NULL CHECK(source IN (
                            'user',
                            'goodreads',
                            'hardcover',
                            'google_books',
                            'open_library',
                            'readarr',
                            'import',
                            'legacy'
                        )),
    source_route_id     INTEGER REFERENCES author_provider_routes(id) ON DELETE SET NULL,
    open_library_role  TEXT CHECK(open_library_role IN ('primary', 'alias')),
    user_selected_at   TEXT,
    observed_at         TEXT NOT NULL,
    UNIQUE(user_id, author_id, source, canonical_name)
);

CREATE INDEX idx_author_name_variants_author
    ON author_name_variants(user_id, author_id, source, observed_at, id);

CREATE TABLE author_link_candidates (
    id                          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id                     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    author_id                   INTEGER NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    provider                    TEXT NOT NULL CHECK(provider IN ('open_library', 'goodreads', 'hardcover')),
    route_value                 TEXT NOT NULL,
    candidate_name              TEXT NOT NULL,
    reason                      TEXT NOT NULL CHECK(reason IN (
                                    'tier2_name_search',
                                    'name_guard_failed',
                                    'readarr_name_guard_failed',
                                    'tombstoned',
                                    'legacy_contradiction',
                                    'ownership_collision',
                                    'invalid_legacy_route'
                                )),
    name_verdict                TEXT NOT NULL CHECK(name_verdict IN ('agree', 'grey', 'disagree', 'abstain')),
    primary_name_verdict        TEXT NOT NULL CHECK(primary_name_verdict IN ('agree', 'grey', 'disagree', 'abstain')),
    top_work_preview            TEXT,
    catalog_evidence_state      TEXT NOT NULL CHECK(catalog_evidence_state IN (
                                    'pending',
                                    'partial',
                                    'retrying',
                                    'complete',
                                    'unavailable'
                                )),
    corroborated_title_count    INTEGER NOT NULL DEFAULT 0,
    settled_work_count          INTEGER NOT NULL DEFAULT 0,
    previously_removed          INTEGER NOT NULL DEFAULT 0,
    status                      TEXT NOT NULL CHECK(status IN ('pending', 'dismissed', 'picked', 'superseded')),
    evidence_generation         INTEGER NOT NULL,
    observed_at                 TEXT NOT NULL,
    resolved_at                 TEXT,
    UNIQUE(user_id, author_id, provider, route_value, reason, evidence_generation)
);

CREATE INDEX idx_author_link_candidates_review
    ON author_link_candidates(user_id, author_id, evidence_generation, status, id);

CREATE TABLE author_link_candidate_alternate_name_evidence (
    candidate_id    INTEGER NOT NULL REFERENCES author_link_candidates(id) ON DELETE CASCADE,
    ordinal         INTEGER NOT NULL,
    name            TEXT NOT NULL,
    canonical_name  TEXT NOT NULL,
    verdict         TEXT NOT NULL CHECK(verdict IN ('agree', 'grey', 'disagree', 'abstain')),
    PRIMARY KEY(candidate_id, ordinal),
    UNIQUE(candidate_id, canonical_name)
);

CREATE TABLE author_link_progress (
    author_id                   INTEGER PRIMARY KEY REFERENCES authors(id) ON DELETE CASCADE,
    user_id                     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    state                       TEXT NOT NULL CHECK(state IN (
                                    'queued',
                                    'running',
                                    'parked_no_settled_work',
                                    'parked_no_evidence',
                                    'needs_review',
                                    'linked',
                                    'retryable_failure'
                                )),
    tier                        INTEGER,
    cursor                      TEXT,
    evaluated_fingerprint       TEXT,
    evidence_generation         INTEGER NOT NULL DEFAULT 0,
    display_name_generation     INTEGER NOT NULL DEFAULT 0,
    display_name_dirty          INTEGER NOT NULL DEFAULT 0,
    attempt_count               INTEGER NOT NULL DEFAULT 0,
    next_attempt_at             TEXT NOT NULL,
    claim_token                 TEXT,
    lease_until                 TEXT,
    last_error                  TEXT,
    would_have_linked_at_090    INTEGER NOT NULL DEFAULT 0,
    trigger                     TEXT NOT NULL CHECK(trigger IN (
                                    'legacy_backfill',
                                    'author_created',
                                    'author_adopted',
                                    'user_re_resolve',
                                    'evidence_fingerprint_changed',
                                    'display_name_dirty',
                                    'retry_due'
                                )),
    updated_at                  TEXT NOT NULL,
    UNIQUE(user_id, author_id)
);

CREATE INDEX idx_author_link_progress_due
    ON author_link_progress(next_attempt_at, author_id);

CREATE INDEX idx_author_link_progress_lease
    ON author_link_progress(lease_until, author_id);

CREATE TABLE author_link_key_attempts (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id             INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    author_id           INTEGER NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    evidence_generation INTEGER NOT NULL,
    work_id             INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    provider            TEXT NOT NULL CHECK(provider IN ('open_library', 'goodreads', 'hardcover')),
    work_route          TEXT NOT NULL,
    state               TEXT NOT NULL CHECK(state IN (
                            'pending',
                            'running',
                            'succeeded',
                            'retryable',
                            'skipped_not_configured',
                            'skipped_permanent',
                            'parked_layout_drift'
                        )),
    claim_token         TEXT,
    attempt_count       INTEGER NOT NULL DEFAULT 0,
    next_attempt_at     TEXT,
    last_error          TEXT,
    diagnostic_code     TEXT,
    updated_at          TEXT NOT NULL,
    UNIQUE(user_id, author_id, evidence_generation, work_id, provider, work_route)
);

CREATE INDEX idx_author_link_key_attempts_due
    ON author_link_key_attempts(user_id, author_id, evidence_generation, state, next_attempt_at, id);

CREATE TABLE author_route_legacy_staging (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    author_id   INTEGER NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    provider    TEXT NOT NULL CHECK(provider IN ('open_library', 'goodreads', 'hardcover')),
    raw_value   TEXT NOT NULL,
    status      TEXT NOT NULL CHECK(status IN ('pending', 'ingested', 'invalid')),
    diagnostic  TEXT,
    staged_at   TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    UNIQUE(user_id, author_id, provider)
);

CREATE INDEX idx_author_route_legacy_staging_status
    ON author_route_legacy_staging(status, id);

CREATE TABLE author_name_legacy_staging (
    author_id       INTEGER PRIMARY KEY REFERENCES authors(id) ON DELETE CASCADE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    canonical_name  TEXT,
    status          TEXT NOT NULL CHECK(status IN ('pending', 'ingested', 'invalid')),
    diagnostic      TEXT,
    staged_at       TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TRIGGER author_link_work_evidence_changed
AFTER UPDATE OF identity_status, ol_key, gr_key, hc_key ON works
WHEN OLD.identity_status IS NOT NEW.identity_status
  OR OLD.ol_key IS NOT NEW.ol_key
  OR OLD.gr_key IS NOT NEW.gr_key
  OR OLD.hc_key IS NOT NEW.hc_key
BEGIN
    UPDATE author_link_progress
       SET state = 'queued',
           trigger = 'evidence_fingerprint_changed',
           next_attempt_at = MIN(
               next_attempt_at,
               strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
           ),
           claim_token = NULL,
           lease_until = NULL,
           updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
     WHERE user_id = NEW.user_id
       AND author_id = NEW.author_id;
END;
