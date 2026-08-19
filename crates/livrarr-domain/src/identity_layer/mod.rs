//! Identity-layer-rewrite (F2) additive domain vocabulary.
//!
//! Structural authority: `ir-v1-identity-layer-rewrite.yaml` `modules` (the
//! `livrarr-domain` entry) and `shared_types`. This module is deliberately
//! additive and separate from `crate::identity`/`crate::identity_matching`/
//! `crate::identity_edit` and from `crate::services::work_identity`: several
//! IR v1 names (`IdentityStatus`, `CapturedIdentity`, `IdentityConflict`,
//! `WorkIdentityRepository`) already exist in this crate with an
//! incompatible pre-cutover shape that must keep compiling until the
//! migration 082-084 cutover activates the new authority (IR v1
//! `migration_plan`). The new, amended shapes live here instead of
//! overwriting the legacy ones — reachable only via
//! `livrarr_domain::identity_layer::*`, never re-exported at the crate root.

pub mod conflict;
pub mod contributor;
pub mod cover;
pub mod door;
pub mod edition;
pub mod matching;
pub mod migration;
pub mod review;
pub mod route;
pub mod services;
pub mod shared;
pub mod status;
pub mod title;

pub use conflict::*;
pub use contributor::*;
pub use cover::*;
pub use door::*;
pub use edition::*;
pub use matching::*;
pub use migration::*;
pub use review::*;
pub use route::*;
pub use services::*;
pub use shared::*;
pub use status::*;
pub use title::*;
