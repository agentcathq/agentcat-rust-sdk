//! `ksuid` against the TS KSUID library: the vectors of `tests/vectors/ksuid.json`, a property
//! test that runs the same inputs through the JS oracle, and Rust-only tests of `random`.
use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::{EPOCH_MS, MAX_TIME_MS, from_parts, random, random_at};
use crate::test_support::{config, oracle, vectors};

/// The 16 payload bytes of a case's `payload_hex`.
fn payload_of(hex_text: &str) -> [u8; 16] {
    hex::decode(hex_text)
        .ok()
        .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
        .unwrap_or_else(|| panic!("payload_hex {hex_text:?} is not 16 bytes"))
}

/// The 27 characters after the prefix of `id`.
fn encoded(id: &str) -> &str {
    id.split_once('_').map_or(id, |(_, rest)| rest)
}

fn is_base62(text: &str) -> bool {
    text.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Checks one `from-parts:*` case: the ID, or `None` where TS throws.
fn check_from_parts(case: &Map<String, Value>) {
    let name = vectors::name(case);
    let input = &case["input"];
    let (Some(prefix), Some(time_ms), Some(payload_hex)) = (
        input["prefix"].as_str(),
        input["time_ms"].as_u64(),
        input["payload_hex"].as_str(),
    ) else {
        panic!("{name}: input is not {{prefix, time_ms, payload_hex}}");
    };
    let actual = from_parts(prefix, time_ms, &payload_of(payload_hex));
    match (case.get("output"), case.get("error")) {
        (Some(output), None) => assert_eq!(actual.as_deref(), output.as_str(), "{name}"),
        (None, Some(_)) => assert_eq!(actual, None, "{name}: TS throws"),
        _ => panic!("{name}: a case has exactly one of output and error"),
    }
}

#[test]
fn vectors_match_ts() {
    let cases = vectors::cases("ksuid");
    let mut from_parts_cases = 0;
    for case in cases
        .iter()
        .filter(|case| vectors::name(case).starts_with("from-parts:"))
    {
        check_from_parts(case);
        from_parts_cases += 1;
    }
    assert!(
        from_parts_cases >= 49,
        "{from_parts_cases} from-parts cases"
    );
}

#[test]
fn min_and_max_match_ts() {
    let cases = vectors::cases("ksuid");
    let output_of = |name: &str| {
        cases
            .iter()
            .find(|case| vectors::name(case) == name)
            .and_then(|case| case["output"].as_str().map(str::to_owned))
            .unwrap_or_else(|| panic!("no case {name}"))
    };
    let min = from_parts("ses", EPOCH_MS, &[0; 16]).unwrap();
    let max = from_parts("ses", MAX_TIME_MS, &[0xff; 16]).unwrap();
    assert_eq!(encoded(&min), output_of("min-max:min"));
    assert_eq!(encoded(&max), output_of("min-max:max"));
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn from_parts_matches_ts(
        time_ms in (EPOCH_MS - 2_000)..=(MAX_TIME_MS + 2_000),
        payload in prop::array::uniform16(any::<u8>()),
    ) {
        let args = [json!("evt"), json!(time_ms), json!(hex::encode(payload))];
        let actual = from_parts("evt", time_ms, &payload);
        // An oracle error is TS throwing, and nothing else may fail.
        let js = oracle::call("ksuid.fromParts", &args).unwrap_or_else(|e| {
            assert!(e.message.starts_with("Valid KSUID timestamps"), "{}", e.message);
            None
        });
        prop_assert_eq!(actual.map(Value::from), js);
    }
}

#[test]
fn random_has_the_shape_and_differs() {
    for prefix in ["ses", "evt", "event", "user", "req"] {
        let first = random(prefix).unwrap();
        let second = random(prefix).unwrap();
        let (head, rest) = first.split_once('_').unwrap();
        assert_eq!(head, prefix);
        assert_eq!(rest.len(), 27, "{first}");
        assert!(is_base62(rest), "{first}");
        assert_ne!(first, second);
    }
    let ids: std::collections::BTreeSet<String> =
        (0..100).map(|_| random("ses").unwrap()).collect();
    assert_eq!(ids.len(), 100);
}

#[test]
fn random_at_is_none_outside_the_range_or_without_randomness() {
    let ok = |buf: &mut [u8; 16]| {
        buf.fill(7);
        Ok(())
    };
    assert!(random_at("ses", Some(EPOCH_MS), ok).is_some());
    assert_eq!(random_at("ses", None, ok), None);
    assert_eq!(random_at("ses", Some(EPOCH_MS - 1), ok), None);
    assert_eq!(random_at("ses", Some(MAX_TIME_MS + 1), ok), None);
    let failing = |_: &mut [u8; 16]| Err(getrandom::Error::UNSUPPORTED);
    assert_eq!(random_at("ses", Some(EPOCH_MS), failing), None);
}
