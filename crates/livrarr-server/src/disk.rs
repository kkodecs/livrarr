/// Return (free_bytes, total_bytes) for the filesystem containing `path`.
/// Returns `(None, None)` if the path is invalid or the syscall fails.
pub fn disk_space(path: &str) -> (Option<i64>, Option<i64>) {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::mem::MaybeUninit;

        let c_path = match CString::new(path) {
            Ok(p) => p,
            Err(_) => return (None, None),
        };

        unsafe {
            let mut stat = MaybeUninit::<libc::statvfs>::uninit();
            if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) != 0 {
                return (None, None);
            }
            let stat = stat.assume_init();
            let free = (stat.f_bavail as i64) * (stat.f_frsize as i64);
            let total = (stat.f_blocks as i64) * (stat.f_frsize as i64);
            (Some(free), Some(total))
        }
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        (None, None)
    }
}
