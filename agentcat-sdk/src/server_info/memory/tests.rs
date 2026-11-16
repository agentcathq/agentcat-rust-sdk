//! The memory of the heartbeat with times given by the test: when an attempt is due, what a
//! dropped build gives back, and the limit on the keys.
use std::time::Duration;

use tokio::time::Instant;

use super::{INTERVAL, LASTING_AGE, MAX_DROPPED, MAX_DROPPED_LISTINGS, MAX_KEYS, Memory, Trigger};

const PROJECT: &str = "proj_memory";
const SERVER: &str = "demo-server";

/// `seconds` seconds.
const fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

/// A memory whose first check ran at `start`, [`LASTING_AGE`] ago at `start + LASTING_AGE`.
fn started(start: Instant) -> Memory {
    let mut memory = Memory::new();
    assert!(!memory.claim(Trigger::ToolCall, "", "", start));
    memory
}

#[test]
fn the_limits_have_their_values() {
    assert_eq!(INTERVAL, secs(3600));
    assert_eq!(LASTING_AGE, secs(60));
    assert_eq!(MAX_KEYS, 1000);
    assert_eq!(MAX_DROPPED, 5);
    assert_eq!(MAX_DROPPED_LISTINGS, 20);
}

#[test]
fn v19_a_tool_call_is_due_from_an_age_of_60_seconds() {
    let start = Instant::now();
    let mut memory = Memory::new();
    assert!(!memory.claim(Trigger::ToolCall, PROJECT, SERVER, start));
    assert!(!memory.claim(Trigger::ToolCall, PROJECT, SERVER, start + secs(59)));
    assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, start + secs(60)));
}

#[test]
fn v17_a_listing_and_a_feedback_note_are_due_in_a_new_memory() {
    let now = Instant::now();
    assert!(Memory::new().claim(Trigger::Listing, PROJECT, SERVER, now));
    assert!(Memory::new().claim(Trigger::FeedbackNote, PROJECT, SERVER, now));
    assert!(!Memory::new().claim(Trigger::ToolCall, PROJECT, SERVER, now));
}

#[test]
fn the_first_check_sets_the_age_whatever_starts_it() {
    let start = Instant::now();
    let mut memory = Memory::new();
    assert!(memory.claim(Trigger::Listing, "other", SERVER, start));
    assert!(!memory.claim(Trigger::ToolCall, PROJECT, SERVER, start + secs(59)));
    assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, start + secs(60)));
}

#[test]
fn v7_the_next_attempt_is_due_3600_seconds_after_the_last_one_started() {
    let start = Instant::now();
    let mut memory = started(start);
    let first = start + LASTING_AGE;
    assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, first));
    assert!(!memory.claim(Trigger::ToolCall, PROJECT, SERVER, first));
    assert!(!memory.claim(Trigger::Listing, PROJECT, SERVER, first + secs(1)));
    assert!(!memory.claim(Trigger::FeedbackNote, PROJECT, SERVER, first + secs(3599)));
    assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, first + secs(3600)));
    assert!(!memory.claim(Trigger::ToolCall, PROJECT, SERVER, first + secs(7199)));
    assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, first + secs(7200)));
}

#[test]
fn v8_each_project_and_server_name_has_its_own_attempt() {
    let now = Instant::now();
    let mut memory = Memory::new();
    assert!(memory.claim(Trigger::Listing, PROJECT, SERVER, now));
    assert!(!memory.claim(Trigger::Listing, PROJECT, SERVER, now));
    assert!(memory.claim(Trigger::Listing, PROJECT, "other-server", now));
    assert!(memory.claim(Trigger::Listing, "proj_other", SERVER, now));
    assert!(!memory.claim(Trigger::Listing, PROJECT, "other-server", now));
    assert!(!memory.claim(Trigger::Listing, "proj_other", SERVER, now));
}

#[test]
fn the_twentieth_dropped_build_of_listings_and_feedback_notes_in_a_row_keeps_its_attempt() {
    let now = Instant::now();
    let mut memory = Memory::new();
    for drop in 1..MAX_DROPPED_LISTINGS {
        let trigger = if drop % 2 == 0 {
            Trigger::Listing
        } else {
            Trigger::FeedbackNote
        };
        assert!(memory.claim(trigger, PROJECT, SERVER, now));
        assert!(!memory.release(trigger, PROJECT, SERVER), "{drop}");
    }
    assert!(memory.claim(Trigger::Listing, PROJECT, SERVER, now));
    assert!(memory.release(Trigger::Listing, PROJECT, SERVER));
    assert!(!memory.claim(Trigger::Listing, PROJECT, SERVER, now));
    assert!(!memory.claim(Trigger::FeedbackNote, PROJECT, SERVER, now + secs(3599)));

    // An hour later the count starts again.
    let later = now + INTERVAL;
    for _ in 1..MAX_DROPPED_LISTINGS {
        assert!(memory.claim(Trigger::Listing, PROJECT, SERVER, later));
        assert!(!memory.release(Trigger::Listing, PROJECT, SERVER));
    }
    assert!(memory.claim(Trigger::Listing, PROJECT, SERVER, later));
    assert!(memory.release(Trigger::Listing, PROJECT, SERVER));
}

#[test]
fn the_fifth_dropped_build_of_tool_calls_in_a_row_keeps_its_attempt() {
    let start = Instant::now();
    let mut memory = started(start);
    let now = start + LASTING_AGE;
    for _ in 0..4 {
        assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, now));
        assert!(!memory.release(Trigger::ToolCall, PROJECT, SERVER));
    }
    assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, now));
    assert!(memory.release(Trigger::ToolCall, PROJECT, SERVER));
    assert!(!memory.claim(Trigger::ToolCall, PROJECT, SERVER, now));
    assert!(!memory.claim(Trigger::Listing, PROJECT, SERVER, now + secs(3599)));

    // An hour later the count starts again: four more builds may be dropped.
    let later = now + INTERVAL;
    for _ in 0..4 {
        assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, later));
        memory.release(Trigger::ToolCall, PROJECT, SERVER);
    }
    assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, later));
    memory.release(Trigger::ToolCall, PROJECT, SERVER);
    assert!(!memory.claim(Trigger::ToolCall, PROJECT, SERVER, later));
}

#[test]
fn dropped_listings_and_dropped_tool_calls_are_counted_apart() {
    let start = Instant::now();
    let mut memory = started(start);
    let now = start + LASTING_AGE;
    for _ in 0..4 {
        assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, now));
        assert!(!memory.release(Trigger::ToolCall, PROJECT, SERVER));
    }
    for _ in 1..MAX_DROPPED_LISTINGS {
        assert!(memory.claim(Trigger::Listing, PROJECT, SERVER, now));
        assert!(!memory.release(Trigger::Listing, PROJECT, SERVER));
    }
    assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, now));
    assert!(memory.release(Trigger::ToolCall, PROJECT, SERVER));
    // The fifth dropped tool call recorded the attempt; the count of listings starts again.
    let later = now + INTERVAL;
    assert!(memory.claim(Trigger::Listing, PROJECT, SERVER, later));
    assert!(!memory.release(Trigger::Listing, PROJECT, SERVER));
}

#[test]
fn a_build_that_ends_starts_both_counts_again() {
    let now = Instant::now();
    let mut memory = Memory::new();
    for _ in 1..MAX_DROPPED_LISTINGS {
        assert!(memory.claim(Trigger::Listing, PROJECT, SERVER, now));
        assert!(!memory.release(Trigger::Listing, PROJECT, SERVER));
    }
    assert!(memory.claim(Trigger::Listing, PROJECT, SERVER, now));
    memory.settle(PROJECT, SERVER);
    let later = now + INTERVAL;
    for _ in 1..MAX_DROPPED_LISTINGS {
        assert!(memory.claim(Trigger::Listing, PROJECT, SERVER, later));
        assert!(!memory.release(Trigger::Listing, PROJECT, SERVER));
    }
}

#[test]
fn a_build_that_ends_keeps_its_attempt_and_starts_the_count_again() {
    let start = Instant::now();
    let mut memory = started(start);
    let now = start + LASTING_AGE;
    for _ in 0..4 {
        assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, now));
        memory.release(Trigger::ToolCall, PROJECT, SERVER);
    }
    assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, now));
    memory.settle(PROJECT, SERVER);
    assert!(!memory.claim(Trigger::ToolCall, PROJECT, SERVER, now + secs(3599)));

    let later = now + INTERVAL;
    for _ in 0..4 {
        assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, later));
        memory.release(Trigger::ToolCall, PROJECT, SERVER);
    }
    assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, later));
}

#[test]
fn the_dropped_count_is_kept_for_each_key() {
    let start = Instant::now();
    let mut memory = started(start);
    let now = start + LASTING_AGE;
    for _ in 0..4 {
        assert!(memory.claim(Trigger::ToolCall, PROJECT, SERVER, now));
        memory.release(Trigger::ToolCall, PROJECT, SERVER);
    }
    assert!(memory.claim(Trigger::ToolCall, PROJECT, "other-server", now));
    memory.release(Trigger::ToolCall, PROJECT, "other-server");
    assert!(memory.claim(Trigger::ToolCall, PROJECT, "other-server", now));
}

#[test]
fn a_key_the_memory_does_not_hold_is_left_alone() {
    let mut memory = Memory::new();
    assert!(!memory.release(Trigger::ToolCall, PROJECT, SERVER));
    memory.settle(PROJECT, SERVER);
    assert_eq!(memory.keys, 0);
    assert_eq!(memory.entries.len(), 0);
}

#[test]
fn the_memory_empties_itself_at_1000_keys() {
    let now = Instant::now();
    let mut memory = Memory::new();
    for index in 0..MAX_KEYS {
        assert!(memory.claim(Trigger::Listing, PROJECT, &format!("server-{index}"), now));
    }
    assert_eq!(memory.keys, MAX_KEYS);
    assert!(!memory.claim(Trigger::Listing, PROJECT, "server-0", now));
    assert_eq!(memory.keys, MAX_KEYS);

    // One more key empties the memory, so every key is due again; the age stays.
    assert!(memory.claim(Trigger::Listing, PROJECT, "one-more", now));
    assert_eq!(memory.keys, 1);
    assert!(memory.claim(Trigger::Listing, PROJECT, "server-0", now));
    assert_eq!(memory.keys, 2);
    assert!(!memory.claim(Trigger::ToolCall, PROJECT, "server-1", now + secs(59)));
}

#[test]
fn the_process_memory_is_emptied_and_aged_for_a_test() {
    super::forget();
    assert!(super::process().started.is_none());
    assert!(super::process().claim(Trigger::Listing, PROJECT, SERVER, Instant::now()));
    super::start_aged(LASTING_AGE);
    assert!(super::process().claim(Trigger::ToolCall, PROJECT, SERVER, Instant::now()));
    super::start_aged(secs(59));
    assert!(!super::process().claim(Trigger::ToolCall, PROJECT, SERVER, Instant::now()));
}
