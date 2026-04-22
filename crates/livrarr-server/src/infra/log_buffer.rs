use std::collections::VecDeque;

// =============================================================================
// Log Buffer — in-memory ring buffer for recent log lines
// =============================================================================

pub const MAX_LOG_LINES: usize = 200;

/// Thread-safe ring buffer that stores recent log lines for the help page.
pub struct LogBuffer {
    lines: std::sync::Mutex<VecDeque<String>>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self {
            lines: std::sync::Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES)),
        }
    }
}

impl LogBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a log line. Drops oldest if at capacity.
    pub fn push(&self, line: String) {
        let mut buf = self.lines.lock().unwrap();
        if buf.len() >= MAX_LOG_LINES {
            buf.pop_front();
        }
        buf.push_back(line);
    }

    /// Get the last `n` log lines.
    pub fn tail(&self, n: usize) -> Vec<String> {
        let buf = self.lines.lock().unwrap();
        buf.iter().rev().take(n).rev().cloned().collect()
    }
}

// =============================================================================
// Log Level Handle — dynamic tracing filter reload at runtime
// =============================================================================

/// Handle for dynamically reloading the tracing EnvFilter at runtime.
pub struct LogLevelHandle {
    inner: tracing_subscriber::reload::Handle<
        tracing_subscriber::EnvFilter,
        tracing_subscriber::Registry,
    >,
    current_level: std::sync::Mutex<String>,
}

impl LogLevelHandle {
    pub fn new(
        handle: tracing_subscriber::reload::Handle<
            tracing_subscriber::EnvFilter,
            tracing_subscriber::Registry,
        >,
        initial_level: &str,
    ) -> Self {
        Self {
            inner: handle,
            current_level: std::sync::Mutex::new(initial_level.to_string()),
        }
    }

    pub fn set_level(&self, level: &str) -> Result<(), String> {
        let filter =
            tracing_subscriber::EnvFilter::try_new(format!("livrarr={level},tower_http={level}"))
                .map_err(|e| format!("invalid log level: {e}"))?;
        self.inner
            .reload(filter)
            .map_err(|e| format!("reload failed: {e}"))?;
        *self.current_level.lock().unwrap() = level.to_string();
        Ok(())
    }

    pub fn current_level(&self) -> String {
        self.current_level.lock().unwrap().clone()
    }
}
