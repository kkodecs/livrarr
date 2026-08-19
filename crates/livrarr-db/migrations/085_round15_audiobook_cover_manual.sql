-- Round 15: audiobook cover choices need the same durable user-sovereignty
-- bit as ebook choices before the Goodreads damage heal can safely reselect
-- machine covers. Existing audiobook covers were written without such a bit
-- and therefore remain machine-selected by default.
ALTER TABLE works
    ADD COLUMN audiobook_cover_manual INTEGER NOT NULL DEFAULT 0
    CHECK (audiobook_cover_manual IN (0, 1));

-- Durable worklist for the marker-gated repair. A crash after clearing a bad
-- slot but before downloading/re-materializing its replacement must not make
-- that work disappear from the next startup's predicate.
CREATE TABLE identity_round15_gr_cover_reselect_queue (
    user_id INTEGER NOT NULL,
    work_id INTEGER NOT NULL,
    ebook INTEGER NOT NULL CHECK (ebook IN (0, 1)),
    audiobook INTEGER NOT NULL CHECK (audiobook IN (0, 1)),
    PRIMARY KEY (user_id, work_id),
    FOREIGN KEY (work_id) REFERENCES works(id) ON DELETE CASCADE
);
