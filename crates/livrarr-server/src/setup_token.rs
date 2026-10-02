//! The one-time first-run setup token and its file, `{data}/setup-token`.
//!
//! The database's pending-setup flag decides whether setup is open; the token
//! is only the proof a setup request must carry. While setup is pending, each
//! start reuses a well-formed token found in a regular file at the path (never
//! through a link) or generates a new one, then writes it privately to a new
//! temporary file and renames that onto the path. Where the no-follow open and
//! owner-only creation are unavailable (any non-Unix target), no file is
//! reused and issuing the token fails.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use tracing::warn;

/// File name of the setup token inside the data directory.
pub const SETUP_TOKEN_FILE_NAME: &str = "setup-token";

/// Random bytes in a setup token (128 bits).
const TOKEN_BYTES: usize = 16;

/// Length of a hex-encoded setup token.
const TOKEN_HEX_LEN: usize = TOKEN_BYTES * 2;

/// The active setup token and the file that holds it.
#[derive(Clone)]
pub struct SetupToken {
    value: String,
    file: PathBuf,
}

impl SetupToken {
    pub fn new(value: impl Into<String>, file: PathBuf) -> Self {
        Self {
            value: value.into(),
            file,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn file(&self) -> &Path {
        &self.file
    }
}

impl std::fmt::Debug for SetupToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupToken")
            .field("value", &"[REDACTED]")
            .field("file", &self.file)
            .finish()
    }
}

/// `{data}/setup-token`.
pub fn setup_token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SETUP_TOKEN_FILE_NAME)
}

/// Reuses a well-formed token from a regular file at `{data}/setup-token` or
/// generates a new one, then writes it to that path privately. An error means
/// the token file could not be written and setup must not be served.
pub fn issue_setup_token(data_dir: &Path) -> io::Result<SetupToken> {
    let path = setup_token_path(data_dir);
    let value = match reusable_token(&path) {
        Some(value) => value,
        None => generate_token()?,
    };
    write_private(&path, &value)?;
    Ok(SetupToken::new(value, path))
}

/// Removes the setup-token path if present: a file or a link is unlinked, a
/// directory is never removed. A failure is logged at WARN and otherwise
/// ignored.
pub fn remove_setup_token_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => warn!(
            "Could not remove the setup token file {}: {e}",
            path.display()
        ),
    }
}

/// 16 bytes from the operating system's random source, lowercase hex.
fn generate_token() -> io::Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::getrandom(&mut bytes).map_err(io::Error::other)?;
    Ok(data_encoding::HEXLOWER.encode(&bytes))
}

fn is_well_formed(token: &str) -> bool {
    token.len() == TOKEN_HEX_LEN
        && token
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The token held by a regular file at `path`, read without following a link,
/// whose whole contents are exactly 32 lowercase hex bytes. `None` for
/// anything else: missing, any other contents (including surrounding
/// whitespace or a newline), a link, a directory or any other file type.
fn reusable_token(path: &Path) -> Option<String> {
    let before = std::fs::symlink_metadata(path).ok()?;
    if !before.file_type().is_file() {
        return None;
    }
    let file = open_no_follow(path).ok()?;
    let opened = file.metadata().ok()?;
    if !opened.file_type().is_file() || !same_file(&before, &opened) {
        return None;
    }
    let mut contents = Vec::with_capacity(TOKEN_HEX_LEN + 1);
    file.take(TOKEN_HEX_LEN as u64 + 1)
        .read_to_end(&mut contents)
        .ok()?;
    let token = std::str::from_utf8(&contents).ok()?;
    is_well_formed(token).then(|| token.to_owned())
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// Without a no-follow open, an existing file is never read.
#[cfg(not(unix))]
fn open_no_follow(_path: &Path) -> io::Result<std::fs::File> {
    Err(unsupported())
}

#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(_a: &std::fs::Metadata, _b: &std::fs::Metadata) -> bool {
    false
}

/// Creates a new owner-only temporary file beside `path`, writes `token` to
/// it, then renames it onto `path`. The rename replaces whatever directory
/// entry was at `path` (a file or a link) and never writes into its target.
fn write_private(path: &Path, token: &str) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("the token path has no parent folder"))?;
    let mut suffix = [0u8; 6];
    getrandom::getrandom(&mut suffix).map_err(io::Error::other)?;
    let temp = dir.join(format!(
        ".{SETUP_TOKEN_FILE_NAME}.{}.tmp",
        data_encoding::HEXLOWER.encode(&suffix)
    ));

    let mut file = create_private(&temp)?;
    let result = file
        .write_all(token.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| std::fs::rename(&temp, path));
    drop(file);
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(unix)]
fn create_private(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// Without owner-only creation, the token file is never written, so startup
/// stops before serving setup.
#[cfg(not(unix))]
fn create_private(_path: &Path) -> io::Result<std::fs::File> {
    Err(unsupported())
}

#[cfg(not(unix))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "private setup token files are supported only on Unix",
    )
}
