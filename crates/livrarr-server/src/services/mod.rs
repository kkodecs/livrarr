//! Business-logic service layer.
//!
//! Each service is a plain struct that owns its dependencies extracted from
//! [`AppState`]. Handlers are thin wrappers: validate input → call service
//! method → convert result to HTTP response.
//!
//! No service traits — concrete structs only.

pub mod manual_import_service;
pub mod readarr_import_service;
pub mod release_service;
