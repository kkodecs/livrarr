-- Non-unique support indexes are safe before authority activation. The final
-- idx_works_identity_v2 unique index is created only by the activation
-- transaction after blocker and collision checks succeed.

CREATE INDEX idx_identity_routes_owner
    ON identity_routes(user_id, resolved_work_id, state);
CREATE INDEX idx_identity_routes_typed_lookup
    ON identity_routes(user_id, provider, kind, provider_scoped_id);
CREATE INDEX idx_identity_routes_one_active_owner
    ON identity_routes(user_id, provider, kind, provider_scoped_id)
    WHERE state = 'active';
CREATE INDEX idx_editions_work ON editions(user_id, work_id, state);
CREATE INDEX idx_work_contributors_order ON work_contributors(user_id, work_id, ordinal);
CREATE INDEX idx_identity_review_cards_pending
    ON identity_review_cards(user_id, status, generation);
CREATE INDEX idx_identity_conflicts_v2_pending
    ON identity_conflicts_v2(user_id, status, current_work_id);
CREATE INDEX idx_identity_provider_attempts_route
    ON identity_provider_attempts(user_id, work_id, provider, route_kind, route_value);

CREATE VIEW identity_conflicts AS
SELECT id, user_id, current_work_id, class, candidate_provider, candidate_kind,
       candidate_value, proposed_owner_type, proposed_owner_id, status,
       resolution, audit_id, expected_generation
  FROM identity_conflicts_v2;

INSERT INTO _livrarr_meta (key, value) VALUES ('schema_version', '83')
ON CONFLICT (key) DO UPDATE SET value = excluded.value;
