//! Parity: `src/tests/ksuid.test.ts`.

use crate::ksuid::{Ksuid, KsuidError, new_prefixed};
use crate::util::{hex, unix_millis};
use regex::Regex;
use std::cmp::Ordering;
use std::collections::HashSet;

/// KSUID epoch in Unix milliseconds (14e11).
const EPOCH_MS: u64 = 1_400_000_000_000;

/// `Math.floor(t / 1000) * 1000`.
fn floor_to_second(t: u64) -> u64 {
    t / 1000 * 1000
}

mod ksuid {
    use super::*;

    mod basic_functionality {
        use super::*;

        #[test]
        fn should_create_a_ksuid_from_randomsync() {
            let k = Ksuid::random();
            assert_eq!(k.encode().len(), 27);
            assert_eq!(k.as_bytes().len(), 20);
        }

        #[test]
        fn should_create_ksuids_with_different_timestamps() {
            let time1 = unix_millis();
            let time2 = time1 + 1000;
            let k1 = Ksuid::random_at(time1).unwrap();
            let k2 = Ksuid::random_at(time2).unwrap();
            assert_eq!(k1.time_ms(), floor_to_second(time1));
            assert_eq!(k2.time_ms(), floor_to_second(time2));
            assert_eq!(k1.cmp(&k2), Ordering::Less);
        }
    }

    mod fromparts {
        use super::*;

        #[test]
        fn should_create_ksuid_from_timestamp_and_payload() {
            let time = unix_millis();
            let payload = [42u8; 16];
            let k = Ksuid::from_parts(time, &payload).unwrap();
            assert_eq!(k.time_ms(), floor_to_second(time));
            assert_eq!(k.payload(), payload);
        }

        #[test]
        fn should_throw_error_for_invalid_timestamp() {
            let payload = [0u8; 16];
            assert_eq!(Ksuid::from_parts(100, &payload), Err(KsuidError::Time));
            assert_eq!(
                Ksuid::from_parts(unix_millis() + 1_000_000_000_000_000, &payload),
                Err(KsuidError::Time)
            );
            assert_eq!(
                KsuidError::Time.to_string(),
                "Valid KSUID timestamps must be in milliseconds since 1970-01-01T00:00:00Z, \
                 no earlier than 2014-05-13T16:53:20Z and no later than 2150-06-19T23:21:35Z"
            );
        }

        #[test]
        fn should_throw_error_for_invalid_payload() {
            let time = unix_millis();
            assert_eq!(Ksuid::from_parts(time, &[42; 15]), Err(KsuidError::Payload));
            assert_eq!(Ksuid::from_parts(time, &[42; 17]), Err(KsuidError::Payload));
            assert_eq!(
                KsuidError::Payload.to_string(),
                "Valid KSUID payloads are 16 bytes"
            );
        }
    }

    mod string_encoding_parsing {
        use super::*;

        #[test]
        fn should_encode_and_parse_ksuid_strings_correctly() {
            let original = Ksuid::random();
            let encoded = original.encode();
            let parsed = Ksuid::parse(&encoded).unwrap();
            assert_eq!(parsed, original);
            assert_eq!(parsed.encode(), encoded);
        }

        #[test]
        fn should_handle_min_and_max_string_values() {
            let min = Ksuid::parse(Ksuid::MIN_STRING).unwrap();
            let max = Ksuid::parse(Ksuid::MAX_STRING).unwrap();
            assert_eq!(min.encode(), Ksuid::MIN_STRING);
            assert_eq!(max.encode(), Ksuid::MAX_STRING);
            assert_eq!(min.cmp(&max), Ordering::Less);
        }

        #[test]
        fn should_throw_error_for_invalid_string_length() {
            assert_eq!(Ksuid::parse("too_short"), Err(KsuidError::EncodedLength));
            assert_eq!(
                Ksuid::parse("way_too_long_string_for_ksuid"),
                Err(KsuidError::EncodedLength)
            );
            assert_eq!(
                KsuidError::EncodedLength.to_string(),
                "Valid encoded KSUIDs are 27 characters"
            );
        }
    }

    mod comparison_and_equality {
        use super::*;

        #[test]
        fn should_compare_ksuids_correctly() {
            let now = unix_millis();
            let k1 = Ksuid::random_at(now).unwrap();
            let k2 = Ksuid::random_at(now + 1000).unwrap();
            assert_eq!(k1.cmp(&k2), Ordering::Less);
            assert_eq!(k2.cmp(&k1), Ordering::Greater);
            assert_eq!(k1.cmp(&k1), Ordering::Equal);
        }

        #[test]
        fn should_check_equality_correctly() {
            let k1 = Ksuid::random();
            let k2 = Ksuid::parse(&k1.encode()).unwrap();
            let k3 = Ksuid::random();
            assert_eq!(k1, k1);
            assert_eq!(k1, k2);
            assert_ne!(k1, k3);
        }
    }

    mod buffer_validation {
        use super::*;

        #[test]
        fn should_validate_buffers_correctly() {
            assert!(Ksuid::from_bytes(&[0; 20]).is_ok());
            assert_eq!(Ksuid::from_bytes(&[0; 19]), Err(KsuidError::Buffer));
            assert_eq!(Ksuid::from_bytes(&[0; 21]), Err(KsuidError::Buffer));
        }

        #[test]
        fn should_throw_error_for_invalid_buffer_in_constructor() {
            for len in [19, 21] {
                let err = Ksuid::from_bytes(&vec![0; len]).unwrap_err();
                assert_eq!(err.to_string(), "Valid KSUID buffers are 20 bytes");
            }
        }
    }

    mod serialization {
        use super::*;

        #[test]
        fn should_serialize_to_json_correctly() {
            let k = Ksuid::random();
            assert_eq!(serde_json::to_string(&k).unwrap(), format!("\"{k}\""));
            assert_eq!(k.to_string(), k.encode());
        }

        #[test]
        fn should_have_proper_tostring_representation() {
            let k = Ksuid::random();
            assert_eq!(format!("{k:?}"), format!("KSUID {{ {k} }}"));
        }
    }

    mod properties {
        use super::*;

        #[test]
        fn should_have_correct_timestamp_property() {
            let time = unix_millis();
            let k = Ksuid::random_at(time).unwrap();
            assert_eq!(u64::from(k.timestamp()), (time - EPOCH_MS) / 1000);
        }

        #[test]
        fn should_have_correct_date_property() {
            let time = unix_millis();
            let k = Ksuid::random_at(time).unwrap();
            assert_eq!(k.time_ms(), floor_to_second(time));
        }

        #[test]
        fn should_have_16_byte_payload() {
            assert_eq!(Ksuid::random().payload().len(), 16);
        }
    }

    mod edge_cases {
        use super::*;

        #[test]
        fn should_handle_ksuid_epoch_boundary() {
            let k = Ksuid::random_at(EPOCH_MS).unwrap();
            assert_eq!(k.timestamp(), 0);
            assert_eq!(k.time_ms(), EPOCH_MS);
        }

        #[test]
        fn should_create_unique_ksuids() {
            let strings: Vec<String> = (0..100).map(|_| Ksuid::random().encode()).collect();
            let unique: HashSet<&String> = strings.iter().collect();
            assert_eq!(unique.len(), strings.len());
        }
    }

    mod prefix_functionality {
        use super::*;

        fn assert_prefixed(prefix: &str, id: &str) {
            let pattern = Regex::new(&format!("^{prefix}_[0-9A-Za-z]{{27}}$")).unwrap();
            assert!(pattern.is_match(id), "{id}");
        }

        #[test]
        fn should_create_prefixed_ksuids_with_randomsync() {
            assert_prefixed("event", &new_prefixed("event"));
        }

        #[test]
        fn should_create_prefixed_ksuids_with_random_async() {
            assert_prefixed("user", &new_prefixed("user"));
        }

        #[test]
        fn should_create_prefixed_ksuids_with_fromparts() {
            let k = Ksuid::from_parts(unix_millis(), &[42; 16]).unwrap();
            assert_prefixed("task", &format!("task_{k}"));
        }

        #[test]
        fn should_handle_different_prefixes() {
            for prefix in ["event", "user", "task", "session", "req"] {
                assert_prefixed(prefix, &new_prefixed(prefix));
            }
        }

        #[test]
        fn should_create_unique_prefixed_ksuids() {
            let ids: Vec<String> = (0..10).map(|_| new_prefixed("event")).collect();
            let unique: HashSet<&String> = ids.iter().collect();
            assert_eq!(unique.len(), ids.len());
            for id in &ids {
                assert_prefixed("event", id);
            }
        }
    }
}

/// The TS decoder has no alphabet check: any 27-UTF-16-unit string decodes,
/// unless its value overflows 20 bytes. PostHog UUIDs depend on this.
#[test]
fn ts_decoder_semantics_for_non_base62_input() {
    let decodes_to = |input: &str, bytes: &str| {
        let k = Ksuid::parse(input).unwrap_or_else(|e| panic!("{input:?}: {e}"));
        assert_eq!(hex(k.as_bytes()), bytes, "{input:?}");
        k
    };
    let overflow = "Fixed length of 20 is too small, expected at least 21";

    let dashes = decodes_to(&"-".repeat(27), "000000000000000000000000000000000000003f");
    assert_eq!(dashes.timestamp(), 0);
    let spaces = decodes_to(&" ".repeat(27), "0000000000000000000000000000000000000050");
    assert_eq!(spaces.timestamp(), 0);
    decodes_to(
        &format!("{}a", "\u{1F600}".repeat(13)),
        "000000000000000024533fc4f1c940234e857024",
    );

    let accents = Ksuid::parse(&"\u{e9}".repeat(27)).unwrap_err();
    assert_eq!(
        accents,
        KsuidError::Decode {
            expected_at_least: 21
        }
    );
    assert_eq!(accents.to_string(), overflow);
    let above_max = Ksuid::parse("aWgEPTl1tmebfsQzFP4bxwgy80W").unwrap_err();
    assert_eq!(above_max.to_string(), overflow);

    assert_eq!(
        Ksuid::parse(&"\u{1F600}".repeat(14)),
        Err(KsuidError::EncodedLength)
    );

    let max = decodes_to(Ksuid::MAX_STRING, &"ff".repeat(20));
    assert_eq!(max.time_ms(), 5_694_967_295_000);
}
