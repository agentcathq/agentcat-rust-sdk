//! The panic text of the [`Fake`](super::Fake) assertions.
use super::{FLUSH, Recorded};

/// The note on a flush that timed out names the flush timeout; without it there is no note.
#[test]
fn recorded_names_the_flush_timeout_when_a_flush_timed_out() {
    let late = Recorded {
        events: Vec::new(),
        settled: false,
    };
    let text = late.to_string();
    assert!(text.starts_with("recorded: []"), "{text}");
    assert!(
        text.contains(&format!("within the {} s flush", FLUSH.as_secs())),
        "{text}"
    );
    let settled = Recorded {
        events: Vec::new(),
        settled: true,
    };
    assert_eq!(settled.to_string(), "recorded: []");
}
