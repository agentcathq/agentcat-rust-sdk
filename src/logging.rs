//! SDK logging to `~/agentcat.log`, mirrored to `tracing` (target
//! `agentcat`) and to the diagnostics sink.
//!
//! The SDK never writes to stdout or stderr: for stdio MCP servers stdout is
//! the JSON-RPC wire, and one stray line corrupts the protocol stream.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

static FILE_LOGGING: AtomicBool = AtomicBool::new(true);
static LOG_FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static PREFIX: OnceLock<String> = OnceLock::new();

/// Turns writing to `~/agentcat.log` on or off for the whole process.
/// `tracing` events and diagnostics are unaffected.
pub(crate) fn set_file_logging(enabled: bool) {
    FILE_LOGGING.store(enabled, Ordering::Relaxed);
}

fn log_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AGENTCAT_LOG_FILE").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    #[allow(deprecated, reason = "fixed and undeprecated in Rust 1.86; MSRV is 1.88")]
    let home = std::env::home_dir().filter(|h| !h.as_os_str().is_empty())?;
    Some(home.join("agentcat.log"))
}

fn log_file() -> &'static Mutex<Option<File>> {
    LOG_FILE.get_or_init(|| {
        // Containers with an arbitrary UID may have no home directory: drop
        // file logging silently rather than falling back to stdout.
        let file = log_path().and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());
        Mutex::new(file)
    })
}

/// `[sdk=… lang=rust mcp=…]`, stamped on every line so one pasted line
/// identifies the SDK and MCP library versions that produced it.
pub(crate) fn version_prefix() -> &'static str {
    PREFIX.get_or_init(|| {
        let mcp = crate::runtime::mcp_library_version().unwrap_or("unknown");
        format!("[sdk={} lang=rust mcp={mcp}]", crate::constants::SDK_VERSION)
    })
}

#[cfg(test)]
thread_local! {
    static CAPTURED: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` with this thread's SDK log lines captured: the message only,
/// with no timestamp or version prefix. While capturing, the lines go
/// nowhere else.
#[cfg(test)]
pub(crate) fn capture_logs<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    struct Restore(Option<Vec<String>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            CAPTURED.with_borrow_mut(|lines| *lines = previous);
        }
    }

    let restore = Restore(CAPTURED.with_borrow_mut(|lines| lines.replace(Vec::new())));
    let value = f();
    let lines = CAPTURED.with_borrow_mut(Option::take).unwrap_or_default();
    drop(restore);
    (value, lines)
}

/// True when `capture_logs` took the line.
#[cfg(test)]
fn captured(message: &str) -> bool {
    CAPTURED
        .try_with(|lines| {
            lines
                .borrow_mut()
                .as_mut()
                .map(|lines| lines.push(message.to_owned()))
                .is_some()
        })
        .unwrap_or(false)
}

/// Writes one line to the SDK log. Never fails and never panics.
pub(crate) fn write_to_log(message: &str) {
    #[cfg(test)]
    if captured(message) {
        return;
    }
    let _ = std::panic::catch_unwind(|| {
        let timestamp = crate::util::iso_millis(&chrono::Utc::now());
        let entry = format!("[{timestamp}] {} {message}", version_prefix());

        crate::diagnostics::capture(&entry);
        tracing::debug!(target: "agentcat", "{message}");

        if !FILE_LOGGING.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(mut guard) = log_file().lock()
            && let Some(file) = guard.as_mut()
            && writeln!(file, "{entry}").is_err()
        {
            *guard = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_logs_collects_only_this_threads_lines() {
        let ((), lines) = capture_logs(|| {
            write_to_log("on this thread");
            std::thread::spawn(|| write_to_log("on another thread"))
                .join()
                .expect("the spawned thread finishes");
        });
        assert_eq!(lines, ["on this thread"]);
    }
}
