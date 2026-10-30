//! Helpers shared by the crate's unit tests: the markers of the strict JSON encoding, the vector
//! loader, the proptest strategies, JSON helpers, rules several tests check, and [`TempHome`].
pub(crate) mod json;
pub(crate) mod markers;
pub(crate) mod queue;
pub(crate) mod rmcp;
pub(crate) mod rules;
pub(crate) mod strategies;
pub(crate) mod vectors;

use std::path::{Path, PathBuf};

use proptest::test_runner::Config;
use serde_json::Value;

use crate::wire;

/// A home directory for one test, `<temp dir>/agentcat-<name>-<process ID>`, empty when made and
/// removed with its content on drop, so a test that writes `agentcat.log` never writes the
/// developer's `~/agentcat.log` and leaves nothing behind.
#[derive(Debug)]
pub(crate) struct TempHome(PathBuf);

impl TempHome {
    /// A new, empty home directory for the test `name`.
    ///
    /// # Panics
    ///
    /// When the directory cannot be made.
    pub(crate) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("agentcat-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the temp home");
        Self(dir)
    }

    /// The directory.
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The bytes of each side [`assert_same_json_text`] shows around the first difference.
const SHOWN_BYTES: usize = 40;

/// Asserts that `actual` is `expected` as `wire::stringify` writes it: [`wire::stringify`] of
/// `actual` equals [`wire::stringify`] of `expected` after [`markers::decode_utf16`] (each `$utf16`
/// marker, a string cut inside a surrogate pair, becomes that string without its half character).
///
/// # Panics
///
/// When the two texts differ, with `context`, the byte offset of the first difference and
/// [`SHOWN_BYTES`] bytes of each text around it.
pub(crate) fn assert_same_json_text(actual: &Value, expected: &Value, context: &str) {
    let actual = wire::stringify(actual);
    let expected = wire::stringify(&markers::decode_utf16(expected.clone()));
    if actual == expected {
        return;
    }
    let at = actual
        .bytes()
        .zip(expected.bytes())
        .position(|(a, e)| a != e)
        .unwrap_or_else(|| actual.len().min(expected.len()));
    panic!(
        "{context}: JSON texts differ at byte {at}\n  actual:   {}\n  expected: {}",
        around(&actual, at),
        around(&expected, at),
    );
}

/// About [`SHOWN_BYTES`] bytes of `text` around byte `at`, widened to character boundaries,
/// as a Rust string literal (so control characters show).
fn around(text: &str, at: usize) -> String {
    let mut start = at.saturating_sub(SHOWN_BYTES / 2);
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (start + SHOWN_BYTES).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    format!("{:?} (bytes {start}..{end})", &text[start..end])
}

/// The proptest settings for property tests: 256 cases unless `PROPTEST_CASES` says otherwise
/// (proptest reads that variable itself).
pub(crate) fn config() -> Config {
    Config::default()
}

/// The most shrink steps of a [`config_capped`] test: each step reruns the test on an input that
/// can be several hundred kilobytes.
const SHRINK_ITERS: u32 = 64;

/// The most time, in milliseconds, a [`config_capped`] test spends shrinking.
const SHRINK_MS: u32 = 60_000;

/// [`config`] with at most `max` cases, for a property test whose cases are slow or large (the
/// size-guard tests). Shrinking stops after [`SHRINK_ITERS`] steps or [`SHRINK_MS`]
/// milliseconds, so a failure reports a partly shrunk input in bounded time; a smaller limit
/// from `PROPTEST_MAX_SHRINK_ITERS` or `PROPTEST_MAX_SHRINK_TIME` still applies.
pub(crate) fn config_capped(max: u32) -> Config {
    let mut config = Config::with_cases(config().cases.min(max));
    config.max_shrink_iters = config.max_shrink_iters().min(SHRINK_ITERS);
    config.max_shrink_time = match config.max_shrink_time {
        0 => SHRINK_MS,
        ms => ms.min(SHRINK_MS),
    };
    config
}

mod tests {
    use proptest::test_runner::Config;
    use serde_json::json;

    use super::{around, assert_same_json_text, config, config_capped};

    /// The message of the panic of `f`.
    fn panic_message(f: impl FnOnce() + std::panic::UnwindSafe) -> String {
        let payload = std::panic::catch_unwind(f).unwrap_err();
        payload.downcast_ref::<String>().unwrap().clone()
    }

    #[test]
    fn assert_same_json_text_passes_equal_values() {
        let value = json!({"a": [1, "x", null], "b": {"c": true}});
        assert_same_json_text(&value, &value, "same");
    }

    #[test]
    fn assert_same_json_text_drops_the_half_character_of_a_utf16_marker() {
        assert_same_json_text(&json!(["a"]), &json!([{"$utf16": [97, 55357]}]), "cut");
    }

    #[test]
    fn assert_same_json_text_names_the_context_and_the_first_difference() {
        let message = panic_message(|| assert_same_json_text(&json!("abc"), &json!("abd"), "ctx"));
        let lines = [
            "ctx: JSON texts differ at byte 3",
            r#"  actual:   "\"abc\"" (bytes 0..5)"#,
            r#"  expected: "\"abd\"" (bytes 0..5)"#,
        ];
        assert_eq!(message, lines.join("\n"));
    }

    #[test]
    fn assert_same_json_text_puts_a_missing_tail_at_the_end_of_the_shorter_text() {
        let message = panic_message(|| assert_same_json_text(&json!(1), &json!(12), "tail"));
        assert!(
            message.starts_with("tail: JSON texts differ at byte 1\n"),
            "{message}"
        );
    }

    #[test]
    fn around_shows_about_40_bytes_widened_to_character_boundaries() {
        // '"' then 3-byte characters: byte 31 starts one; 31 - 20 = 11 and 10 + 40 = 50 do not.
        let text = format!("\"{}", "世".repeat(20));
        assert_eq!(
            around(&text, 31),
            format!("{:?} (bytes 10..52)", "世".repeat(14))
        );
        assert_eq!(around("ab", 1), format!("{:?} (bytes 0..2)", "ab"));
    }

    #[test]
    fn config_is_the_proptest_default() {
        assert_eq!(config().cases, Config::default().cases);
    }

    #[test]
    fn config_capped_takes_the_smaller_case_count() {
        assert_eq!(config_capped(1).cases, 1);
        assert_eq!(config_capped(u32::MAX).cases, Config::default().cases);
    }

    #[test]
    fn config_capped_caps_shrinking() {
        let config = config_capped(1_000);
        assert!(
            config.max_shrink_iters() <= 64,
            "{}",
            config.max_shrink_iters()
        );
        assert!(
            (1..=60_000).contains(&config.max_shrink_time),
            "{}",
            config.max_shrink_time
        );
    }
}
