-- Index for efficient expired session cleanup queries
CREATE INDEX idx_sessions_expires_at ON sessions(expires_at);
