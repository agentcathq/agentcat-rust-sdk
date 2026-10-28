use std::path::Path;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};

use super::{Logger, configure, level, prefix, write};
use crate::config::LogDestination;
use crate::test_support::TempHome;

/// The temp home of the logging test `name`.
fn temp_home(name: &str) -> TempHome {
    TempHome::new(&format!("logging-{name}"))
}

/// The size of `<dir>/agentcat.log`, `None` when there is no such file.
fn log_size(dir: &Path) -> Option<u64> {
    std::fs::metadata(dir.join("agentcat.log"))
        .ok()
        .map(|file| file.len())
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
    let logger = Logger::new(LogDestination::File, Some(home.path().to_path_buf()));
    logger.write("first message");
    logger.write("second message");
    let text = std::fs::read_to_string(home.path().join("agentcat.log")).expect("log file");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(is_line(lines[0], "first message"), "{}", lines[0]);
    assert!(is_line(lines[1], "second message"), "{}", lines[1]);
    assert!(prefix().starts_with("[sdk=1.0.0 rust="));
    assert!(prefix().contains(" rmcp=3."));
}

/// TS writes nothing when there is no home directory: the file logger names no file, and its
/// line reaches no `agentcat.log` in the temp root or in the working directory (where a relative
/// path would put it), each the same size as before, or still absent.
#[test]
fn no_home_is_silent() {
    let logger = Logger::new(LogDestination::File, None);
    assert_eq!(logger.path, None);
    let places = [
        std::env::temp_dir(),
        std::env::current_dir().expect("a working directory"),
    ];
    let before: Vec<Option<u64>> = places.iter().map(|dir| log_size(dir)).collect();
    logger.write("nobody hears this");
    let after: Vec<Option<u64>> = places.iter().map(|dir| log_size(dir)).collect();
    assert_eq!(after, before, "{places:?}");
}

#[test]
fn unwritable_home_is_ignored() {
    let home = temp_home("missing");
    let missing = home.path().join("does-not-exist");
    Logger::new(LogDestination::File, Some(missing.clone())).write("lost");
    assert!(!missing.exists());
}

/// The helper of these tests: a temp home is removed with its content when it drops, so no test
/// leaves a directory behind.
#[test]
fn temp_home_is_removed_on_drop() {
    let home = temp_home("drop");
    let dir = home.path().to_path_buf();
    Logger::new(LogDestination::File, Some(dir.clone())).write("a line");
    assert!(dir.join("agentcat.log").exists());
    drop(home);
    assert!(!dir.exists(), "{dir:?}");
}

/// Checks that `text` is the entries of `messages` in order, each `[<ISO time>] <prefix>
/// <message>\n` as TS's `writeToLog` writes it, line breaks inside a message included.
fn assert_entries(text: &str, messages: &[&str]) {
    let mut rest = text;
    for message in messages {
        // `[`, the 24-byte time, `] `, the prefix, a space, the message and the line break.
        let length = 1 + 24 + 2 + prefix().len() + 1 + message.len() + 1;
        let entry = rest
            .get(..length)
            .unwrap_or_else(|| panic!("an entry for {message:?} in {text:?}"));
        let line = entry
            .strip_suffix('\n')
            .expect("the entry ends with a line break");
        assert!(is_line(line, message), "{entry:?}");
        rest = &rest[length..];
    }
    assert!(rest.is_empty(), "nothing after the entries: {rest:?}");
}

/// TS's edge-runtime "should not throw when called" (C42): writing `Test message`, an empty
/// message and `Special chars: \n\t\r` returns normally, and the file gets each entry whole, in
/// TS's `[<ISO time>] <prefix> <message>` form, the empty message and the control characters
/// included.
#[test]
fn write_returns_normally_and_keeps_each_message_whole() {
    let messages = ["Test message", "", "Special chars: \n\t\r"];
    let home = temp_home("whole");
    let logger = Logger::new(LogDestination::File, Some(home.path().to_path_buf()));
    for message in messages {
        logger.write(message);
    }
    let text = std::fs::read_to_string(home.path().join("agentcat.log")).expect("log file");
    assert_entries(&text, &messages);
}

/// TS's edge-runtime "should handle rapid successive calls" (C42): 100 writes in a row, `Message
/// 0` to `Message 99`, each land as one entry, in order.
#[test]
fn rapid_successive_writes_all_land_in_order() {
    let messages: Vec<String> = (0..100).map(|i| format!("Message {i}")).collect();
    let home = temp_home("rapid");
    let logger = Logger::new(LogDestination::File, Some(home.path().to_path_buf()));
    for message in &messages {
        logger.write(message);
    }
    let text = std::fs::read_to_string(home.path().join("agentcat.log")).expect("log file");
    let messages: Vec<&str> = messages.iter().map(String::as_str).collect();
    assert_entries(&text, &messages);
}

/// TS's "caches the prefix so repeated logging stays consistent": the prefix is built once, so
/// each call gives the same text at the same address, and two lines in a row carry that prefix.
#[test]
fn prefix_is_computed_once() {
    let first = prefix();
    assert!(
        std::ptr::eq(first, prefix()),
        "a second call built it again"
    );
    let home = temp_home("prefix");
    let logger = Logger::new(LogDestination::File, Some(home.path().to_path_buf()));
    logger.write("first");
    logger.write("second");
    let text = std::fs::read_to_string(home.path().join("agentcat.log")).expect("log file");
    let prefixes: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_once("] ")?.1.rsplit_once(' '))
        .map(|(line_prefix, _)| line_prefix)
        .collect();
    assert_eq!(prefixes, [first, first], "{text}");
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
    assert!(!home.path().join("agentcat.log").exists());
    configure(LogDestination::File, Some(home.path().to_path_buf()));
    configure(LogDestination::File, None);
    write("after configure");
    let text = std::fs::read_to_string(home.path().join("agentcat.log")).expect("log file");
    assert_eq!(text.lines().count(), 1);
    assert!(is_line(
        text.lines().next().expect("line"),
        "after configure"
    ));
}
