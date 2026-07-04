//! The covers startup sequence: three one-shot passes over the covers
//! directory whose correctness depends on strict order, so they run
//! sequentially from one caller — never as parallel tasks.
//!
//! 1. Layout migration first — it moves legacy root-level files into
//!    per-user directories and renames the legacy `_audiobook` suffix; the
//!    recovery pass and every serving path only understand the per-user
//!    layout, so nothing else may observe the tree until it is settled.
//! 2. Gate-write recovery second — it converges rows and pending candidate
//!    files to a consistent state, taking the same per-slot locks live
//!    writers use.
//! 3. Provenance backfill last — it derives missing cover sources from the
//!    now-converged rows; running it against rows recovery is still healing
//!    would race the heal and could stamp a source derived from a URL the
//!    heal is about to replace.

use std::path::Path;

use livrarr_db::WorkDb;

pub async fn run_cover_startup_passes<D: WorkDb + Sync>(db: &D, covers_root: &Path) {
    crate::cover_layout_migration::run_cover_layout_migration(db, covers_root).await;
    crate::cover_write_gate_recovery::recover_pending_cover_writes(db, covers_root).await;
    crate::cover_provenance_backfill::run_cover_provenance_backfill(db).await;
}
