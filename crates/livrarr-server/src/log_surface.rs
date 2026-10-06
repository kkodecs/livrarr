//! Truthful tracing-init surface (REQ-003): the log directory is prepared
//! with failures captured — never swallowed — the file appender is built with
//! a file cap, and the active daily rolling path is computed from the naming
//! that appender produces.

use std::path::{Path, PathBuf};

use livrarr_domain::LogSurfaceStatus;
use tracing_appender::rolling::{InitError, RollingFileAppender, Rotation};

/// Prefix of every log file name; the appender adds `.YYYY-MM-DD`.
const LOG_FILE_PREFIX: &str = "livrarr.log";

/// Files the appender keeps. It prunes to one fewer before opening a new
/// file, so after a change of day the folder holds today's file and the 30
/// most recent earlier files. Pruning counts every file whose name starts
/// with `livrarr.log`, oldest creation time first.
const MAX_LOG_FILES: usize = 31;

/// The daily rolling file appender startup writes the log through, or the
/// library's error when it cannot create the initial file.
pub fn build_file_appender(log_dir: &Path) -> Result<RollingFileAppender, InitError> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .max_log_files(MAX_LOG_FILES)
        .build(log_dir)
}

/// The daily rolling file the appender writes for the current UTC date
/// (`livrarr.log.YYYY-MM-DD`).
pub fn active_log_path(log_dir: &Path) -> PathBuf {
    log_dir.join(format!(
        "{LOG_FILE_PREFIX}.{}",
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

    // Retention matrix: the file appender as startup constructs it, on real
    // folders seeded with files created oldest first.

    use std::collections::BTreeSet;
    use std::io::Write;
    use std::time::Duration;

    use chrono::NaiveDate;
    use tracing_appender::rolling::RollingFileAppender;

    /// The file appender for `log_dir` as startup constructs it, or its build
    /// error.
    fn file_appender(log_dir: &Path) -> Result<RollingFileAppender, String> {
        build_file_appender(log_dir).map_err(|e| e.to_string())
    }

    /// Gap between seeded creations, so creation times order them.
    const SEED_GAP: Duration = Duration::from_millis(15);

    fn utc_today() -> NaiveDate {
        chrono::Utc::now().date_naive()
    }

    /// Runs `attempt` with the UTC date it starts on and returns its result
    /// when it also ends on that date. An attempt that spans UTC midnight is
    /// discarded and run once more, on a fresh folder and the new date.
    fn within_one_utc_day<T>(mut attempt: impl FnMut(NaiveDate) -> T) -> T {
        for _ in 0..2 {
            let day = utc_today();
            let outcome = attempt(day);
            if utc_today() == day {
                return outcome;
            }
        }
        panic!("the UTC date changed during both attempts");
    }

    /// The log file name for `day`.
    fn log_name(day: NaiveDate) -> String {
        format!("livrarr.log.{}", day.format("%Y-%m-%d"))
    }

    /// The dated log file name `days_before` days before `today`.
    fn dated_name(today: NaiveDate, days_before: i64) -> String {
        log_name(today - chrono::Duration::days(days_before))
    }

    fn seed_file(log_dir: &Path, name: &str) {
        std::fs::write(log_dir.join(name), b"seeded\n").expect("seed file");
        std::thread::sleep(SEED_GAP);
    }

    fn seed_dir(log_dir: &Path, name: &str) {
        std::fs::create_dir(log_dir.join(name)).expect("seed directory");
        std::thread::sleep(SEED_GAP);
    }

    /// Dated files for `offsets` (days before `today`), created oldest first.
    /// Returns their names, oldest first.
    fn seed_dated(
        log_dir: &Path,
        today: NaiveDate,
        offsets: impl IntoIterator<Item = i64>,
    ) -> Vec<String> {
        let mut offsets: Vec<i64> = offsets.into_iter().collect();
        offsets.sort_unstable_by(|a, b| b.cmp(a));
        offsets
            .into_iter()
            .map(|days| {
                let name = dated_name(today, days);
                seed_file(log_dir, &name);
                name
            })
            .collect()
    }

    fn entries(log_dir: &Path) -> BTreeSet<String> {
        std::fs::read_dir(log_dir)
            .expect("read log folder")
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    /// The last `n` of `oldest_first`.
    fn newest(oldest_first: &[String], n: usize) -> BTreeSet<String> {
        oldest_first.iter().rev().take(n).cloned().collect()
    }

    /// What one retention attempt left in its folder, and what it should hold.
    struct Retained {
        case: String,
        expected: BTreeSet<String>,
        actual: BTreeSet<String>,
    }

    impl Retained {
        fn assert_exact(&self) {
            assert_eq!(
                self.actual,
                self.expected,
                "{}: expected {} entries, found {}; unexpected {:?}; missing {:?}",
                self.case,
                self.expected.len(),
                self.actual.len(),
                self.actual.difference(&self.expected).collect::<Vec<_>>(),
                self.expected.difference(&self.actual).collect::<Vec<_>>()
            );
        }
    }

    /// One AC-311 row: `seeded` dated files (none when `folder_exists` is
    /// false) before the appender is built.
    fn retention_row(folder_exists: bool, seeded: i64) {
        within_one_utc_day(|today| {
            let tmp = tempfile::tempdir().expect("tempdir");
            let log_dir = tmp.path().join("logs");
            let mut names = Vec::new();
            if folder_exists {
                std::fs::create_dir(&log_dir).expect("create log folder");
                names = seed_dated(&log_dir, today, 1..=seeded);
                assert_eq!(entries(&log_dir).len() as i64, seeded, "control: seeded");
            }

            let appender = file_appender(&log_dir).expect("control: the appender builds");
            let actual = entries(&log_dir);
            drop(appender);

            let mut expected = newest(&names, 30);
            expected.insert(log_name(today));
            Retained {
                case: format!("{seeded} dated files"),
                expected,
                actual,
            }
        })
        .assert_exact();
    }

    #[test]
    fn retention_without_a_log_folder_keeps_only_todays_file() {
        retention_row(false, 0);
    }

    #[test]
    fn retention_in_an_empty_folder_keeps_only_todays_file() {
        retention_row(true, 0);
    }

    #[test]
    fn retention_with_29_dated_files_keeps_all_and_todays_file() {
        retention_row(true, 29);
    }

    #[test]
    fn retention_with_30_dated_files_keeps_all_and_todays_file() {
        retention_row(true, 30);
    }

    #[test]
    fn retention_with_31_dated_files_keeps_the_30_newest_and_todays_file() {
        retention_row(true, 31);
    }

    #[test]
    fn retention_with_85_dated_files_keeps_the_30_newest_and_todays_file() {
        retention_row(true, 85);
    }

    #[test]
    fn retention_counts_undated_log_files_and_ignores_other_names() {
        let (retained, dir_kept) = within_one_utc_day(|today| {
            let tmp = tempfile::tempdir().expect("tempdir");
            let log_dir = tmp.path().to_path_buf();
            seed_file(&log_dir, "livrarr.log");
            seed_file(&log_dir, "notes.txt");
            seed_file(&log_dir, "livrarr.log.bak");
            seed_dir(&log_dir, "livrarr.log.2020-01-01");
            let dated = seed_dated(&log_dir, today, 1..=30);
            assert_eq!(entries(&log_dir).len(), 34, "control: seeded");

            let appender = file_appender(&log_dir).expect("control: the appender builds");
            let actual = entries(&log_dir);
            let dir_kept = log_dir.join("livrarr.log.2020-01-01").is_dir();
            drop(appender);

            let mut expected: BTreeSet<String> = dated.into_iter().collect();
            expected.insert("notes.txt".into());
            expected.insert("livrarr.log.2020-01-01".into());
            expected.insert(log_name(today));
            let retained = Retained {
                case: "undated and other files".into(),
                expected,
                actual,
            };
            (retained, dir_kept)
        });
        retained.assert_exact();
        assert!(dir_kept, "the seeded directory stays a directory");
    }

    #[test]
    fn retention_counts_files_not_calendar_days() {
        within_one_utc_day(|today| {
            let tmp = tempfile::tempdir().expect("tempdir");
            let log_dir = tmp.path().to_path_buf();
            let names = seed_dated(&log_dir, today, (0..40).map(|i| 2 * i + 1));
            assert_eq!(entries(&log_dir).len(), 40, "control: seeded");

            let appender = file_appender(&log_dir).expect("control: the appender builds");
            let actual = entries(&log_dir);
            drop(appender);

            let mut expected = newest(&names, 30);
            expected.insert(log_name(today));
            Retained {
                case: "40 files on alternate days".into(),
                expected,
                actual,
            }
        })
        .assert_exact();
    }

    #[test]
    fn appender_writes_the_file_active_log_path_names() {
        let marker = "ac314-active-file-marker";
        let written = within_one_utc_day(|_| {
            let tmp = tempfile::tempdir().expect("tempdir");
            let log_dir = tmp.path().to_path_buf();
            let mut appender = file_appender(&log_dir).expect("control: the appender builds");
            writeln!(appender, "{marker}").expect("write through the appender");
            appender.flush().expect("flush the appender");
            drop(appender);
            std::fs::read_to_string(active_log_path(&log_dir))
        });

        let written = written.expect("the file active_log_path names exists");
        assert!(
            written.contains(marker),
            "the appender writes the file active_log_path names: {written:?}"
        );
    }

    #[test]
    fn same_day_restart_keeps_todays_file_and_the_29_newest_earlier_files() {
        within_one_utc_day(|today| {
            let tmp = tempfile::tempdir().expect("tempdir");
            let log_dir = tmp.path().to_path_buf();
            let earlier = seed_dated(&log_dir, today, 1..=35);
            seed_file(&log_dir, &log_name(today));
            assert_eq!(entries(&log_dir).len(), 36, "control: seeded");

            let appender = file_appender(&log_dir).expect("control: the appender builds");
            let actual = entries(&log_dir);
            drop(appender);

            let mut expected = newest(&earlier, 29);
            expected.insert(log_name(today));
            Retained {
                case: "same-day restart".into(),
                expected,
                actual,
            }
        })
        .assert_exact();
    }

    /// How one call to the constructor ended.
    enum Build {
        Panicked(String),
        Built,
        Failed(String),
    }

    /// Calls the constructor, catching a panic.
    fn build(log_dir: &Path) -> Build {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            file_appender(log_dir).map(drop)
        }));
        match outcome {
            Err(payload) => Build::Panicked(
                payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_else(|| "non-text panic".into()),
            ),
            Ok(Ok(())) => Build::Built,
            Ok(Err(e)) => Build::Failed(e),
        }
    }

    /// Requires a returned build error naming the failed file creation.
    fn assert_returned_creation_error(outcome: Build) {
        match outcome {
            Build::Panicked(message) => {
                panic!("the constructor panicked instead of returning an error: {message}")
            }
            Build::Built => panic!("the constructor returned an appender; expected an error"),
            Build::Failed(error) => assert!(
                error.contains("failed to create initial log file"),
                "error names the failed file creation: {error}"
            ),
        }
    }

    #[test]
    fn a_directory_named_as_todays_file_is_a_returned_error() {
        let outcome = within_one_utc_day(|today| {
            let tmp = tempfile::tempdir().expect("tempdir");
            let log_dir = tmp.path().to_path_buf();
            let probe = log_dir.join("control-probe");
            std::fs::write(&probe, b"").expect("control: the folder is writable");
            std::fs::remove_file(&probe).expect("control: remove probe");
            std::fs::create_dir(log_dir.join(log_name(today))).expect("seed directory");
            build(&log_dir)
        });

        assert_returned_creation_error(outcome);
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_is_a_returned_error() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().expect("tempdir");
        let log_dir = tmp.path().join("logs");
        std::fs::create_dir(&log_dir).expect("create log folder");
        std::fs::set_permissions(&log_dir, std::fs::Permissions::from_mode(0o555))
            .expect("make the folder read-only");
        let refused = std::fs::write(log_dir.join("control-probe"), b"").is_err();

        let outcome = build(&log_dir);

        std::fs::set_permissions(&log_dir, std::fs::Permissions::from_mode(0o755))
            .expect("restore folder permissions");
        assert!(refused, "control: the read-only folder refuses a new file");
        assert_returned_creation_error(outcome);
    }
}
