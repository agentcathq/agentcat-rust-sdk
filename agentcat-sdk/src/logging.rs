//! The SDK's own log lines.
//!
//! The destination is process-wide and the first call of [`configure`] decides it. `File` appends
//! `[<ISO time>] [sdk=… rust=… rmcp=…] <message>` lines to `<home>/agentcat.log`, one open per
//! line, and ignores every error. `Tracing` emits `tracing` events with target `agentcat`.
//!
//! The delivery worker of each handle writes the lines, so `Tracing` events come from the
//! worker's thread: a subscriber that `tracing::subscriber::with_default` sets on another thread
//! does not see them. The one exception is the line about a delivery thread that cannot be
//! spawned, which the caller's thread writes.
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;

use tracing::Level;

use crate::config::LogDestination;
use crate::wire::iso_millis;

/// A log destination with its resolved file path.
#[derive(Debug)]
pub(crate) struct Logger {
    destination: LogDestination,
    path: Option<PathBuf>,
}

impl Logger {
    /// A logger for `destination`; `home` is the home directory, `None` when there is none.
    pub(crate) fn new(destination: LogDestination, home: Option<PathBuf>) -> Self {
        Self {
            destination,
            path: home.map(|dir| dir.join("agentcat.log")),
        }
    }

    /// Writes one line. Failures are ignored; a missing home directory makes the file route silent.
    pub(crate) fn write(&self, message: &str) {
        match self.destination {
            LogDestination::Tracing => emit(message),
            LogDestination::File => self.write_file(message),
        }
    }

    fn write_file(&self, message: &str) {
        let Some(path) = &self.path else { return };
        let line = format!(
            "[{}] {} {message}\n",
            iso_millis(SystemTime::now()),
            prefix()
        );
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            // A failed write is ignored.
            let _ = file.write_all(line.as_bytes());
        }
    }
}

fn emit(message: &str) {
    match level(message) {
        Level::ERROR => tracing::error!(target: "agentcat", "{message}"),
        Level::WARN => tracing::warn!(target: "agentcat", "{message}"),
        _ => tracing::info!(target: "agentcat", "{message}"),
    }
}

static GLOBAL: OnceLock<Logger> = OnceLock::new();

/// Sets the process-wide logger; the first call wins.
pub(crate) fn configure(destination: LogDestination, home: Option<PathBuf>) {
    let _ = GLOBAL.set(Logger::new(destination, home));
}

/// Writes through the process-wide logger; nothing happens before [`configure`].
pub(crate) fn write(message: &str) {
    if let Some(logger) = GLOBAL.get() {
        logger.write(message);
    }
}

/// `[sdk=<crate version> rust=<rustc version> rmcp=<rmcp version>]`, built once.
pub(crate) fn prefix() -> &'static str {
    static PREFIX: OnceLock<String> = OnceLock::new();
    PREFIX.get_or_init(|| {
        format!(
            "[sdk={} rust={} rmcp={}]",
            env!("CARGO_PKG_VERSION"),
            env!("AGENTCAT_RUSTC_VERSION"),
            rmcp::model::Implementation::default().version
        )
    })
}

/// The severity of a line: ERROR for `fail` or `error` in any case, checked first, then
/// WARN for `Warning:`, else INFO.
pub(crate) fn level(message: &str) -> Level {
    let lower = message.to_lowercase();
    if lower.contains("fail") || lower.contains("error") {
        Level::ERROR
    } else if message.contains("Warning:") {
        Level::WARN
    } else {
        Level::INFO
    }
}

#[cfg(test)]
mod tests;
