-- The ranked candidates behind a work's NeedsReview park (REQ-010): the
-- resolver could not confidently pick one, so the candidate set a reviewer
-- would choose from is persisted queryably per work instead of discarded.
-- One row per work; a fresh park replaces the prior row wholesale.
CREATE TABLE IF NOT EXISTS work_identity_review_candidates (
    work_id         INTEGER PRIMARY KEY REFERENCES works(id) ON DELETE CASCADE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    candidates_json TEXT    NOT NULL,
    recorded_at     TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_work_identity_review_candidates_user
    ON work_identity_review_candidates(user_id);
