//! The panic text of the [`Fake`](super::Fake) assertions, and a fake with no handles.
use std::time::{Duration, Instant};

use super::{FLUSH, Fake, Recorded};

/// C43: a fake that built no handle has nothing to flush and nothing recorded. `events` answers
/// at once with no events, the record counts as settled, and `assert_nothing_tracked` passes.
#[tokio::test]
async fn fake_events_with_no_handles_is_empty() {
    let fake = Fake::new();
    let started = Instant::now();
    assert_eq!(fake.events().await, Vec::new());
    let recorded = fake.recorded().await;
    assert_eq!(recorded.events, Vec::new());
    assert!(recorded.settled, "no handle, so no flush timed out");
    fake.assert_nothing_tracked().await;
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
}

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
