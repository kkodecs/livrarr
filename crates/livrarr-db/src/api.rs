//! Per-entity database trait + request/DTO modules.
//!
//! Each module holds one (or a closely related few) `sqlite_*.rs` impl file's
//! trait contract plus its request/DTO structs — mirroring the entity split
//! the `sqlite_*.rs` files already use. Every public item is re-exported at
//! the crate root by `lib.rs`, so callers never reference `livrarr_db::api::*`
//! directly; the paths that existed before this split (`livrarr_db::WorkDb`,
//! `livrarr_db::CreateWorkDbRequest`, ...) are unchanged.

mod author;
mod bibliography;
mod bookmarks;
mod chapters;
mod config;
mod cross_format_state;
mod download_client;
mod external_id;
mod field_dissents;
mod grab;
mod history;
mod import;
mod import_intent;
mod indexer;
mod kash_link;
mod library_item;
mod list_import;
mod notification;
mod playback_progress;
mod provenance;
mod provider_cache;
mod provider_calls;
mod provider_policy;
mod remote_path_mapping;
mod retry_state;
mod root_folder;
mod series;
mod series_cache;
mod series_roster;
mod session;
mod user;
mod work;

pub use author::*;
pub use bibliography::*;
pub use bookmarks::*;
pub use chapters::*;
pub use config::*;
pub use cross_format_state::*;
pub use download_client::*;
pub use external_id::*;
pub use field_dissents::*;
pub use grab::*;
pub use history::*;
pub use import::*;
pub use import_intent::*;
pub use indexer::*;
pub use kash_link::*;
pub use library_item::*;
pub use list_import::*;
pub use notification::*;
pub use playback_progress::*;
pub use provenance::*;
pub use provider_cache::*;
pub use provider_calls::*;
pub use provider_policy::*;
pub use remote_path_mapping::*;
pub use retry_state::*;
pub use root_folder::*;
pub use series::*;
pub use series_cache::*;
pub use series_roster::*;
pub use session::*;
pub use user::*;
pub use work::*;
