//! Parity: `src/tests/posthog-uuidv7.test.ts`.

use crate::exporters::posthog_uuid_v7 as to_uuid_v7;
use crate::ksuid::{Ksuid, new_event_id, new_session_id};
use crate::util::unix_millis;
use regex::Regex;

/// The UUID without dashes.
fn raw(uuid: &str) -> String {
    uuid.replace('-', "")
}

fn assert_valid_v7(uuid: &str) {
    let re = Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$").unwrap();
    assert!(re.is_match(uuid), "not a UUIDv7: {uuid}");
}

/// The first 48 bits as milliseconds.
fn embedded_ms(uuid: &str) -> u64 {
    u64::from_str_radix(&raw(uuid)[..12], 16).unwrap()
}

mod touuidv7 {
    use super::*;

    #[test]
    fn should_produce_a_valid_uuidv7() {
        assert_valid_v7(&to_uuid_v7(&new_session_id()));
    }

    #[test]
    fn should_be_deterministic_same_input_always_produces_same_output() {
        let id = new_session_id();
        let a = to_uuid_v7(&id);
        assert_eq!(a, to_uuid_v7(&id));
        assert_eq!(a, to_uuid_v7(&id));
    }

    #[test]
    fn should_produce_different_uuids_for_different_session_ids() {
        assert_ne!(to_uuid_v7(&new_session_id()), to_uuid_v7(&new_session_id()));
    }

    #[test]
    fn should_embed_the_ksuid_timestamp_in_the_uuidv7() {
        let known_time = 1_749_988_800_000_u64;
        let ksuid = Ksuid::from_parts(known_time, &[0xab; 16]).unwrap();
        let result = to_uuid_v7(&format!("ses_{ksuid}"));
        assert_eq!(result, "01977375-f600-7355-b83a-0dea9024f86c");
        assert!(embedded_ms(&result).abs_diff(known_time) < 1000);
        assert_eq!(embedded_ms(&result), 0x0197_7375_f600);
    }

    #[test]
    fn should_set_version_bits_to_7_0111() {
        let hex = raw(&to_uuid_v7(&new_session_id()));
        assert_eq!(hex.as_bytes()[12], b'7');
    }

    #[test]
    fn should_set_variant_bits_to_10xx() {
        let hex = raw(&to_uuid_v7(&new_session_id()));
        let nibble = (hex.as_bytes()[16] as char).to_digit(16).unwrap();
        assert!((8..=0xb).contains(&nibble));
    }

    #[test]
    fn should_handle_evt_prefix() {
        assert_valid_v7(&to_uuid_v7(&new_event_id()));
    }

    #[test]
    fn should_handle_invalid_ksuid_gracefully_with_fallback_timestamp() {
        let result = to_uuid_v7("ses_invalid_ksuid_string");
        assert_valid_v7(&result);
        assert_eq!(result, "8a9bdc3e-a187-7187-bbe0-cee6b92a5751");
    }

    #[test]
    fn should_be_deterministic_for_non_ksuid_input_fallback_path() {
        let first = to_uuid_v7("my-customer-task-id");
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(first, to_uuid_v7("my-customer-task-id"));
        assert_eq!(first, "634902a7-d0c7-7220-91c6-0fa56fad8828");
    }

    #[test]
    fn should_produce_different_uuids_for_different_non_ksuid_inputs() {
        assert_ne!(to_uuid_v7("task-a"), to_uuid_v7("task-b"));
        assert_eq!(to_uuid_v7("task-a"), "2084b1c4-8b52-77bb-bbed-5fd87cbc8c1b");
        assert_eq!(to_uuid_v7("task-b"), "8c37e6bc-5830-7977-9a3c-7aa32912c971");
    }

    #[test]
    fn should_be_deterministic_for_empty_string_input() {
        let first = to_uuid_v7("");
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(to_uuid_v7(""), first);
        assert_valid_v7(&first);
        assert_eq!(first, "f4c8996f-b924-73b0-8442-98fc1c149afb");
    }

    #[test]
    fn should_produce_timestamps_that_are_before_or_equal_to_current_time() {
        let id = new_session_id();
        let now = unix_millis();
        assert!(embedded_ms(&to_uuid_v7(&id)) <= now);
    }

    #[test]
    fn golden_vectors_match() {
        for (input, expected) in [
            (
                "ses_session45600000000000000000",
                "6c350afa-fa75-7081-b0c6-8ad3272daa6c",
            ),
            ("evt_test123", "16abe8cf-1d05-7811-87f2-fd2f9686af87"),
        ] {
            assert_eq!(to_uuid_v7(input), expected, "{input}");
        }
    }

    #[test]
    fn non_ksuid_input_matches_ts() {
        let cases: Vec<(String, &str)> = vec![
            (
                format!("ses_{}", "-".repeat(27)),
                "0145f680-b000-786d-98cc-d5d01e28dec2",
            ),
            (
                format!("ses_{}", "é".repeat(27)),
                "88417185-4ad5-75bc-9149-acad1fa28640",
            ),
            (
                format!("ses_{}a", "😀".repeat(13)),
                "0145f680-b000-7343-b240-b7c50fc2c863",
            ),
            (
                format!("ses_{}", "z".repeat(27)),
                "824c5521-0b85-7994-816a-e1cc13ecbe70",
            ),
            (
                "ses_aWgEPTl1tmebfsQzFP4bxwgy80V".into(),
                "052df680-ac18-70fb-a3ad-b18287c0f14b",
            ),
            (
                "ses_aWgEPTl1tmebfsQzFP4bxwgy80W".into(),
                "69af7543-b7cc-7b64-a50a-27922b46ef70",
            ),
            (
                format!("ses_{}", " ".repeat(27)),
                "0145f680-b000-7647-ba2b-7a4d7eca1c91",
            ),
            (
                "Ses_2yXlSmKnrVgSWKGxWkkGlLaGu7v".into(),
                "4d232166-b827-7896-8f84-4a5e15317ae1",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(to_uuid_v7(&input), expected, "{input:?}");
        }
    }
}
