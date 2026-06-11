//! Truthful tracing-init surface (REQ-003): the log directory is prepared
//! with failures captured — never swallowed — and the active daily rolling
//! path is computed from the naming `tracing_appender::rolling::daily`
//! actually produces.

use std::path::{Path, PathBuf};

use livrarr_domain::LogSurfaceStatus;

/// The daily rolling file the appender writes for the current UTC date
/// (`livrarr.log.YYYY-MM-DD`).
pub fn active_log_path(log_dir: &Path) -> PathBuf {
    log_dir.join(format!(
        "livrarr.log.{}",
        chrono::Utc::now().format("%Y-%m-%d")
    ))
}

/// Create the log directory and probe writability. A failure is surfaced on
/// stderr (tracing has no file layer yet at this point in startup) and
/// captured in the returned status for the status page (#102's vector);
/// the server still boots — console + ring-buffer logging keep working.
pub fn prepare_log_surface(log_dir: &Path) -> LogSurfaceStatus {
    let init_error = match std::fs::create_dir_all(log_dir) {
        Err(e) => Some(format!(
            "log directory creation failed ({}): {e}",
            log_dir.display()
        )),
        Ok(()) => {
            let probe = log_dir.join(".livrarr-write-probe");
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&probe)
            {
                Ok(_) => {
                    let _ = std::fs::remove_file(&probe);
                    None
                }
                Err(e) => Some(format!(
                    "log directory not writable ({}): {e}",
                    log_dir.display()
                )),
            }
        }
    };

    if let Some(ref e) = init_error {
        eprintln!("WARNING: file logging disabled — {e}");
    }

    LogSurfaceStatus {
        active_path: active_log_path(log_dir),
        init_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IR directive (AC-004 slice): unwritable log dir → init_error
    /// populated, no panic. A file in the parent path makes create_dir_all
    /// fail deterministically.
    #[test]
    fn unwritable_dir_is_captured_not_swallowed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"not a dir").expect("write blocker");

        let status = prepare_log_surface(&blocker.join("logs"));

        let err = status.init_error.expect("init_error populated");
        assert!(err.contains("log directory creation failed"));
    }

    #[test]
    fn writable_dir_yields_no_error_and_dated_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let log_dir = tmp.path().join("logs");

        let status = prepare_log_surface(&log_dir);

        assert_eq!(status.init_error, None);
        let name = status.active_path.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("livrarr.log."));
        assert!(!log_dir.join(".livrarr-write-probe").exists());
    }
}
