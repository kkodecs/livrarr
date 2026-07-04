//! Startup wrapper for the covers startup sequence. The three passes and
//! their strict ordering live in `livrarr_metadata::cover_startup`; this
//! module just wires the sequence into the server's startup job spawn.

use std::path::PathBuf;

use livrarr_db::sqlite::SqliteDb;

pub async fn run_cover_startup_passes(db: SqliteDb, covers_root: PathBuf) {
    livrarr_metadata::cover_startup::run_cover_startup_passes(&db, &covers_root).await;
}
