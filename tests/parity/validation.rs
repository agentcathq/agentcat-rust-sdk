//! Parity: `src/tests/validation.test.ts`.

use super::support::capture_logs;
use crate::validation::validate_tags;
use serde_json::{Value, json};

fn s(k: &str, v: &str) -> (String, Value) {
    (k.to_owned(), Value::String(v.to_owned()))
}

fn p(k: &str, v: &str) -> (String, String) {
    (k.to_owned(), v.to_owned())
}

mod validatetags {
    use super::*;

    #[test]
    fn should_pass_through_valid_tags_unchanged() {
        let out = validate_tags([
            s("env", "production"),
            s("trace_id", "abc-123"),
            s("region", "us-east-1"),
        ]);
        assert_eq!(
            out,
            Some(vec![
                p("env", "production"),
                p("trace_id", "abc-123"),
                p("region", "us-east-1")
            ])
        );
    }

    #[test]
    fn should_return_null_for_empty_object() {
        assert_eq!(validate_tags(Vec::new()), None);
    }

    #[test]
    fn should_drop_keys_with_invalid_characters() {
        let (out, logs) = capture_logs(|| {
            validate_tags([
                s("valid_key", "value"),
                s("invalid!key", "value"),
                s("good.key", "value"),
            ])
        });
        assert_eq!(out, Some(vec![p("valid_key", "value"), p("good.key", "value")]));
        assert!(logs.iter().any(|l| l.contains("\"invalid!key\"")));
        assert!(logs.contains(
            &"Dropping invalid tag: \"invalid!key\" — key contains invalid characters or is empty".to_owned()
        ));
    }

    #[test]
    fn should_drop_keys_longer_than_32_characters() {
        let long = "a".repeat(33);
        let (out, logs) = capture_logs(|| validate_tags([s(&long, "value"), s("short", "value")]));
        assert_eq!(out, Some(vec![p("short", "value")]));
        assert!(logs.iter().any(|l| l.contains("exceeds max length")));
        assert!(logs.contains(&format!(
            "Dropping invalid tag: \"{long}\" — key exceeds max length of 32"
        )));
    }

    #[test]
    fn should_drop_values_longer_than_200_characters() {
        let (out, logs) = capture_logs(|| validate_tags([s("key1", &"a".repeat(201)), s("key2", "short")]));
        assert_eq!(out, Some(vec![p("key2", "short")]));
        assert!(logs.iter().any(|l| l.contains("exceeds max length")));
        assert!(
            logs.contains(&"Dropping invalid tag: \"key1\" — value exceeds max length of 200".to_owned())
        );
    }

    #[test]
    fn should_drop_values_containing_newlines() {
        let (out, logs) = capture_logs(|| validate_tags([s("key1", "has\nnewline"), s("key2", "clean")]));
        assert_eq!(out, Some(vec![p("key2", "clean")]));
        assert!(logs.iter().any(|l| l.contains("newline")));
        assert!(
            logs.contains(&"Dropping invalid tag: \"key1\" — value contains newline character".to_owned())
        );
    }

    #[test]
    fn should_drop_non_string_values_at_runtime() {
        let (out, logs) =
            capture_logs(|| validate_tags([("key1".to_owned(), json!(123)), s("key2", "valid")]));
        assert_eq!(out, Some(vec![p("key2", "valid")]));
        assert!(logs.iter().any(|l| l.contains("non-string")));
        assert!(logs.contains(&"Dropping invalid tag: \"key1\" — non-string value (got number)".to_owned()));
    }

    #[test]
    fn should_drop_empty_string_keys() {
        let out = validate_tags([s("", "empty-key-value"), s("valid", "value")]);
        assert_eq!(out, Some(vec![p("valid", "value")]));
    }

    #[test]
    fn should_keep_only_first_50_entries_when_exceeding_limit() {
        let tags: Vec<_> = (0..60)
            .map(|i| s(&format!("key{i:03}"), &format!("value{i}")))
            .collect();
        let (out, logs) = capture_logs(|| validate_tags(tags));
        let out = out.expect("some tags");
        assert_eq!(out.len(), 50);
        assert_eq!(out[0].0, "key000");
        assert_eq!(out[49].0, "key049");
        assert!(logs.iter().any(|l| l.contains("Dropping 10")));
        assert!(logs.contains(&"Dropping 10 tag(s) — exceeds maximum of 50 entries per event".to_owned()));
    }

    #[test]
    fn should_return_null_when_all_entries_are_invalid() {
        assert_eq!(validate_tags([s("!!!", "value"), s("###", "value")]), None);
    }

    #[test]
    fn should_allow_keys_with_periods_colons_dashes_spaces_and_dollar_signs() {
        let keys = ["my.tag", "my:tag", "my-tag", "my tag", "$ai_trace_id", "my$key"];
        let tags: Vec<_> = keys
            .iter()
            .map(|k| s(k, if *k == "$ai_trace_id" { "trace-1" } else { "v" }))
            .collect();
        let expected: Vec<_> = keys
            .iter()
            .map(|k| p(k, if *k == "$ai_trace_id" { "trace-1" } else { "v" }))
            .collect();
        assert_eq!(validate_tags(tags), Some(expected));
    }
}

mod rust_extras {
    use super::*;

    #[test]
    fn trailing_newline_in_a_key_is_invalid() {
        let (out, logs) = capture_logs(|| validate_tags([s("key\n", "v")]));
        assert_eq!(out, None);
        assert!(logs[0].ends_with("key contains invalid characters or is empty"));
    }

    #[test]
    fn carriage_return_in_a_value_is_kept() {
        assert_eq!(validate_tags([s("k", "a\rb")]), Some(vec![p("k", "a\rb")]));
    }

    #[test]
    fn non_ascii_keys_are_invalid() {
        assert_eq!(validate_tags([s("é", "v")]), None);
    }

    #[test]
    fn a_key_of_exactly_32_units_is_kept() {
        let key = "k".repeat(32);
        assert_eq!(validate_tags([s(&key, "v")]), Some(vec![p(&key, "v")]));
        assert_eq!(validate_tags([s(&"k".repeat(33), "v")]), None);
    }

    #[test]
    fn lengths_count_utf16_units() {
        assert!(validate_tags([s("k", &"😀".repeat(100))]).is_some());
        assert_eq!(validate_tags([s("k", &"😀".repeat(101))]), None);
    }

    #[test]
    fn duplicate_keys_keep_the_first_position_and_the_last_value() {
        let out = validate_tags([s("a", "1"), s("b", "2"), s("a", "3")]);
        assert_eq!(out, Some(vec![p("a", "3"), p("b", "2")]));
    }

    #[test]
    fn null_value_reports_object() {
        let (out, logs) = capture_logs(|| validate_tags([("k".to_owned(), Value::Null)]));
        assert_eq!(out, None);
        assert!(logs[0].ends_with("non-string value (got object)"));
    }
}
