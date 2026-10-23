use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};

use super::{Logger, configure, level, prefix, write};
use crate::config::LogDestination;

fn temp_home(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("agentcat-logging-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp home");
    dir
}

fn is_line(line: &str, message: &str) -> bool {
    // [YYYY-MM-DDTHH:MM:SS.mmmZ] [sdk=1.0.0 rust=X rmcp=3.Y] message
    let Some(rest) = line.strip_prefix('[') else {
        return false;
    };
    let Some((time, rest)) = rest.split_once("] ") else {
        return false;
    };
    let b = time.as_bytes();
    let shape = b.len() == 24
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[19] == b'.'
        && b[23] == b'Z'
        && time
            .bytes()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7 | 10 | 13 | 16 | 19 | 23) || c.is_ascii_digit());
    shape && rest == format!("{} {message}", prefix())
}

#[test]
fn file_logger_appends_prefixed_lines() {
    let home = temp_home("append");
    let logger = Logger::new(LogDestination::File, Some(home.clone()));
    logger.write("first message");
    logger.write("second message");
    let text = std::fs::read_to_string(home.join("agentcat.log")).expect("log file");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(is_line(lines[0], "first message"), "{}", lines[0]);
    assert!(is_line(lines[1], "second message"), "{}", lines[1]);
    assert!(prefix().starts_with("[sdk=1.0.0 rust="));
    assert!(prefix().contains(" rmcp=3."));
}

#[test]
fn no_home_is_silent() {
    // No file is named, so there is nothing to look at but the absence of a panic.
    Logger::new(LogDestination::File, None).write("nobody hears this");
}

#[test]
fn unwritable_home_is_ignored() {
    let missing = temp_home("missing").join("does-not-exist");
    Logger::new(LogDestination::File, Some(missing.clone())).write("lost");
    assert!(!missing.exists());
}

#[test]
fn level_follows_ts_severity() {
    assert_eq!(level("Failed to send"), Level::ERROR);
    assert_eq!(level("an ERROR"), Level::ERROR);
    assert_eq!(level("Warning: x"), Level::WARN);
    assert_eq!(level("Warning: send failed"), Level::ERROR);
    assert_eq!(level("Successfully sent"), Level::INFO);
}

#[derive(Default)]
struct Capture(Arc<Mutex<Vec<(String, Level, String)>>>);

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Subscriber for Capture {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        self.0.lock().expect("lock").push((
            event.metadata().target().to_owned(),
            *event.metadata().level(),
            message.0,
        ));
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

#[test]
fn tracing_destination_emits_events() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = Capture(Arc::clone(&events));
    tracing::subscriber::with_default(subscriber, || {
        let logger = Logger::new(LogDestination::Tracing, None);
        logger.write("Warning: slow");
        logger.write("Failed to send");
    });
    let got = events.lock().expect("lock").clone();
    assert_eq!(
        got,
        vec![
            (
                "agentcat".to_owned(),
                Level::WARN,
                "Warning: slow".to_owned()
            ),
            (
                "agentcat".to_owned(),
                Level::ERROR,
                "Failed to send".to_owned()
            ),
        ]
    );
}

#[test]
fn global_logger_is_silent_until_configured_and_first_call_wins() {
    // Relies on nextest running one process per test (controller ruling P4): `configure` is
    // process-global and sets once.
    let home = temp_home("global");
    write("before configure");
    assert!(!home.join("agentcat.log").exists());
    configure(LogDestination::File, Some(home.clone()));
    configure(LogDestination::File, None);
    write("after configure");
    let text = std::fs::read_to_string(home.join("agentcat.log")).expect("log file");
    assert_eq!(text.lines().count(), 1);
    assert!(is_line(
        text.lines().next().expect("line"),
        "after configure"
    ));
}
