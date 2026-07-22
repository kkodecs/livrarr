-- Admin-approved Readarr origins (Unit B3 Part 1 — origin trust boundary).
--
-- Per standards.md:172-180, a user-submitted URL is untrusted and must pass
-- SSRF validation before connection; only admin-configured infrastructure
-- gets the private-address exception. A raw user-supplied Readarr origin is
-- allowed WITHOUT an entry here only when the SSRF-safe classifier judges it
-- PUBLIC (no internal-probe risk). Private/loopback/special-use origins
-- require an explicit admin-approved row here first.
--
-- `origin` is the NORMALIZED form (`livrarr_http::normalized_origin` —
-- lowercased `scheme://host[:port]`, default port omitted, no path) so
-- lookup at connect/preview/start time is a plain equality check.
CREATE TABLE IF NOT EXISTS readarr_origins (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    origin     TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);
