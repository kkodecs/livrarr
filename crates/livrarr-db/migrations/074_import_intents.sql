-- Import-intent crash-consistency record (Unit D2). Persisted before any
-- staging-file I/O begins for one imported file; cleared only after the
-- LibraryItem row is durably finalized. The exact ordered sequence:
-- persist intent -> write+fsync staging (tempfile in the destination dir,
-- standards.md:295) -> atomic rename -> fsync parent dir -> finalize
-- LibraryItem -> clear intent (standards.md:81). A crash at any point
-- leaves a reconcilable row: startup recovery (recover_interrupted_state,
-- before anything can be in flight) checks the target path on disk and
-- either completes the finalize+clear or rolls back the staging file.
--
-- state: 'staging' (persisted before the staging file is written; the
-- rename may or may not have happened yet — recovery verifies against the
-- filesystem) or 'renamed' (the atomic rename + parent-dir fsync are
-- durably complete; only the LibraryItem finalize + intent clear remain).
CREATE TABLE IF NOT EXISTS import_intents (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id         INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    root_folder_id  INTEGER NOT NULL REFERENCES root_folders(id) ON DELETE RESTRICT,
    media_type      TEXT NOT NULL CHECK(media_type IN ('ebook', 'audiobook')),
    target_relative TEXT NOT NULL,
    staging_path    TEXT NOT NULL,
    expected_size   INTEGER NOT NULL,
    import_id       TEXT,
    state           TEXT NOT NULL CHECK(state IN ('staging', 'renamed')),
    created_at      TEXT NOT NULL,
    -- Moving the LibraryItem row's creation to AFTER the rename (the
    -- crash-safety fix itself) removes the old collision arbiter: the
    -- library_items UNIQUE constraint used to reject a second work's
    -- concurrent import to the same target path before either one touched
    -- the file. This constraint reinstates that guarantee one step
    -- earlier — whichever concurrent import inserts its intent first wins
    -- the path; the other's create_import_intent fails atomically before
    -- any staging I/O begins.
    UNIQUE(user_id, root_folder_id, target_relative)
);

-- Recovery groups outstanding intents by work to take the same per-(user,
-- work) import lock the live import path holds.
CREATE INDEX IF NOT EXISTS idx_import_intents_user_work
    ON import_intents(user_id, work_id);
