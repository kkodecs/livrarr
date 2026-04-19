pub mod accessors;
pub mod auth;
pub mod author;
pub mod config;
pub mod context;
pub mod coverproxy;
pub mod download_client;
pub mod filesystem;
pub mod history;
pub mod indexer;
pub mod list_import;
pub mod manual_import;
pub mod mediacover;
pub mod middleware;
pub mod notification;
pub mod opds;
pub mod profile;
pub mod queue;
pub mod readarr_import;
pub mod release;
pub mod remote_path_mapping;
pub mod root_folder;
pub mod series;
pub mod setup;
pub mod system;
pub mod types;
pub mod user;
pub mod work;
pub mod workfile;

// Re-export all types at crate root for convenience.
pub use types::api_error::*;
pub use types::auth::*;
pub use types::author::*;
pub use types::config::*;
pub use types::download_client::*;
pub use types::history::*;
pub use types::indexer::*;
pub use types::notification::*;
pub use types::pagination::*;
pub use types::queue::*;
pub use types::release::*;
pub use types::remote_path_mapping::*;
pub use types::root_folder::*;
pub use types::scan::*;
pub use types::series::*;
pub use types::system::*;
pub use types::work::*;

pub fn deserialize_optional_secret<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Option::<Option<String>>::deserialize(deserializer)
}
