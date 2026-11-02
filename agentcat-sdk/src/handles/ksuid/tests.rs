//! `ksuid`: the cases of `tests/vectors/ksuid.json` and `tests/vectors/ksuid_cases.json`,
//! property tests of the encoding (it reads back, and it keeps the order), and tests of `random`.
use proptest::prelude::*;
use serde_json::{Map, Value};

use super::{EPOCH_MS, MAX_TIME_MS, encode, from_parts, random, random_at};
use crate::test_support::rules::base62_bytes;
use crate::test_support::{config, vectors};

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

/// Checks one `from-parts:*` case: the ID, or `None` where the case expects an error.
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
        (None, Some(_)) => assert_eq!(actual, None, "{name}: the case expects an error"),
        _ => panic!("{name}: a case has exactly one of output and error"),
    }
}

#[test]
fn fixture_cases_hold() {
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
fn min_and_max_ids_read_back() {
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

/// Times for `from_parts`: a third from the whole range, a third around `EPOCH_MS` and a third
/// around `MAX_TIME_MS`, each margin reaching 2 s on both sides of the limit, so the in-range and
/// out-of-range edges are both common.
fn times() -> impl Strategy<Value = u64> {
    prop_oneof![
        (EPOCH_MS - 2_000)..=(MAX_TIME_MS + 2_000),
        (EPOCH_MS - 2_000)..=(EPOCH_MS + 2_000),
        (MAX_TIME_MS - 2_000)..=(MAX_TIME_MS + 2_000),
    ]
}

/// The cases of `ksuid_cases.json`: random times around the edges of the range and inside it,
/// with random payloads.
#[test]
fn from_parts_matches_ksuid_cases() {
    let cases = vectors::cases("ksuid_cases");
    assert!(cases.len() >= 100, "ksuid_cases.json has its cases");
    for case in &cases {
        check_from_parts(case);
    }
}

/// The seconds since [`EPOCH_MS`] of an in-range `time_ms`, as a KSUID holds them.
fn seconds_of(time_ms: u64) -> u32 {
    u32::try_from((time_ms - EPOCH_MS) / 1_000).unwrap()
}

/// Times inside `EPOCH_MS..=MAX_TIME_MS`: a third from the whole range, a third in its first 2 s
/// and a third in its last 2 s.
fn in_range_times() -> impl Strategy<Value = u64> {
    prop_oneof![
        EPOCH_MS..=MAX_TIME_MS,
        EPOCH_MS..=(EPOCH_MS + 2_000),
        (MAX_TIME_MS - 2_000)..=MAX_TIME_MS,
    ]
}

proptest! {
    #![proptest_config(config())]

    /// An in-range time gives `evt_` and 27 base62 characters that read back to the whole
    /// seconds since the epoch (4 bytes, big-endian) and the payload; any other time gives
    /// `None`.
    #[test]
    fn from_parts_reads_back_to_its_seconds_and_payload(
        time_ms in times(),
        payload in prop::array::uniform16(any::<u8>()),
    ) {
        let actual = from_parts("evt", time_ms, &payload);
        if !(EPOCH_MS..=MAX_TIME_MS).contains(&time_ms) {
            prop_assert_eq!(actual, None);
            return Ok(());
        }
        let id = actual.expect("an in-range time gives an ID");
        let text = id.strip_prefix("evt_").expect("the prefix and `_`");
        prop_assert_eq!(text.len(), 27);
        prop_assert!(is_base62(text), "{}", id);
        let bytes = base62_bytes(text);
        prop_assert_eq!(&bytes[..4], &seconds_of(time_ms).to_be_bytes()[..]);
        prop_assert_eq!(&bytes[4..], &payload[..]);
    }

    /// Two IDs with the same prefix sort as their seconds, then their payloads, sort.
    #[test]
    fn from_parts_keeps_the_order_of_seconds_then_payload(
        first in (in_range_times(), prop::array::uniform16(any::<u8>())),
        second in (in_range_times(), prop::array::uniform16(any::<u8>())),
    ) {
        let ids = [&first, &second].map(|(time_ms, payload)| from_parts("evt", *time_ms, payload).unwrap());
        let parts = [&first, &second].map(|(time_ms, payload)| (seconds_of(*time_ms), *payload));
        prop_assert_eq!(ids[0].cmp(&ids[1]), parts[0].cmp(&parts[1]));
    }
}

/// `encode` gives what `from_parts` gives for every in-range time.
#[test]
fn encode_equals_from_parts_in_range() {
    let payload = [0xab; 16];
    for time_ms in [EPOCH_MS, EPOCH_MS + 999, EPOCH_MS + 1_000, MAX_TIME_MS] {
        assert_eq!(
            from_parts("ses", time_ms, &payload),
            Some(encode("ses", (time_ms - EPOCH_MS) / 1_000, &payload)),
            "{time_ms}"
        );
    }
}

#[test]
fn random_has_the_shape_and_differs() {
    // Different prefixes, and the two this SDK uses.
    for prefix in ["event", "user", "task", "session", "req", "ses", "evt"] {
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

/// Unique prefixed KSUIDs: 10 `event_` IDs, all distinct.
#[test]
fn ten_event_ids_are_distinct() {
    let ids: Vec<String> = (0..10).map(|_| random("event").unwrap()).collect();
    let distinct: std::collections::BTreeSet<&String> = ids.iter().collect();
    assert_eq!(distinct.len(), ids.len());
    for id in &ids {
        assert!(id.starts_with("event_") && encoded(id).len() == 27, "{id}");
    }
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
