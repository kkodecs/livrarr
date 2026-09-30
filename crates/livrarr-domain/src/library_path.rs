//! Item-to-disk rules for library files: where a library item lives under its
//! root folder, and when it may be removed.
//!
//! Both rules start from the same step, resolving the root folder path, and
//! report which side failed with the original filesystem error.
//!
//! - [`resolve_for_read`] follows links and requires the item inside the root.
//! - [`remove_library_file`] never follows a link at the item's own entry. It
//!   requires the item's resolved parent folder inside the resolved root and
//!   removes the entry only when it is a regular file. It never removes a
//!   folder.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// The shared first step: the root folder path, resolved.
fn resolve_root(root: &Path) -> io::Result<PathBuf> {
    root.canonicalize()
}

/// Why a library item could not be resolved for reading.
#[derive(Debug)]
pub enum ReadPathError {
    /// The root folder path could not be resolved.
    Root(io::Error),
    /// The item path could not be resolved.
    Item(io::Error),
    /// The item resolves outside the root folder.
    OutsideRoot,
}

/// Resolves `relative` under `root` for reading, following links, and
/// requires the result inside the resolved root.
///
/// An item that cannot be resolved is reported before a root that cannot be
/// resolved, so a missing root folder reads as a missing item.
pub fn resolve_for_read(root: &Path, relative: &str) -> Result<PathBuf, ReadPathError> {
    let resolved_root = resolve_root(root);
    let item = root
        .join(relative)
        .canonicalize()
        .map_err(ReadPathError::Item)?;
    let resolved_root = resolved_root.map_err(ReadPathError::Root)?;
    if !item.starts_with(&resolved_root) {
        return Err(ReadPathError::OutsideRoot);
    }
    Ok(item)
}

/// What removing a library file did when it succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveOutcome {
    /// The regular file was removed.
    Removed,
    /// The item, or its parent folder, is absent under a usable root, or the
    /// file vanished during removal.
    Absent,
}

/// What kind of entry sits at an item's path when it is not a regular file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Link,
    Folder,
    RootFolder,
    Other,
}

/// Why a library file was not removed.
#[derive(Debug)]
pub enum RemoveError {
    /// The root folder path could not be resolved.
    Root(io::Error),
    /// The item's parent folder resolves outside the root folder.
    OutsideRoot,
    /// The entry at the item's path is not a regular file.
    NotRegularFile(EntryKind),
    /// Any other filesystem error.
    Io(io::Error),
}

impl fmt::Display for RemoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoveError::Root(e) => write!(f, "root folder cannot be resolved: {e}"),
            RemoveError::OutsideRoot => write!(f, "resolves outside the root folder"),
            RemoveError::NotRegularFile(EntryKind::Link) => {
                write!(f, "is a link, not a regular file")
            }
            RemoveError::NotRegularFile(EntryKind::Folder) => {
                write!(f, "is a folder, not a regular file")
            }
            RemoveError::NotRegularFile(EntryKind::RootFolder) => {
                write!(f, "is the root folder, not a regular file")
            }
            RemoveError::NotRegularFile(EntryKind::Other) => write!(f, "is not a regular file"),
            RemoveError::Io(e) => write!(f, "could not be removed: {e}"),
        }
    }
}

/// Removes the library file at `relative` under `root`.
///
/// The item's parent folder is resolved and must sit inside the resolved
/// root. The item's own entry in that folder is then read without following
/// links, and only a regular file is removed. No folder is ever removed.
pub fn remove_library_file(root: &Path, relative: &str) -> Result<RemoveOutcome, RemoveError> {
    let resolved_root = resolve_root(root).map_err(RemoveError::Root)?;

    let item = root.join(relative);
    let (Some(parent), Some(name)) = (item.parent(), item.file_name()) else {
        return Err(RemoveError::NotRegularFile(EntryKind::Folder));
    };

    let resolved_parent = match parent.canonicalize() {
        Ok(p) => p,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(RemoveOutcome::Absent),
        Err(e) => return Err(RemoveError::Io(e)),
    };
    let entry = resolved_parent.join(name);
    if entry == resolved_root {
        return Err(RemoveError::NotRegularFile(EntryKind::RootFolder));
    }
    if !resolved_parent.starts_with(&resolved_root) {
        return Err(RemoveError::OutsideRoot);
    }

    let file_type = match std::fs::symlink_metadata(&entry) {
        Ok(meta) => meta.file_type(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(RemoveOutcome::Absent),
        Err(e) => return Err(RemoveError::Io(e)),
    };
    if file_type.is_symlink() {
        return Err(RemoveError::NotRegularFile(EntryKind::Link));
    }
    if file_type.is_dir() {
        return Err(RemoveError::NotRegularFile(EntryKind::Folder));
    }
    if !file_type.is_file() {
        return Err(RemoveError::NotRegularFile(EntryKind::Other));
    }

    match std::fs::remove_file(&entry) {
        Ok(()) => Ok(RemoveOutcome::Removed),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(RemoveOutcome::Absent),
        Err(e) => Err(RemoveError::Io(e)),
    }
}
