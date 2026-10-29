//! The markers of the strict JSON encoding of the vector files:
//!
//! - `{"$utf16": [<UTF-16 code units>]}`: a string that is not well-formed UTF-16, which a Rust
//!   `String` cannot hold.
//! - `{"$f64": "<16 lowercase hex digits>"}`: `-0`, or a number that `serde_json` (without
//!   `float_roundtrip`) would not parse to the same `f64`; its IEEE-754 bits, big-endian.
//! - `{"$repeat": ["<unit>", <count>]}`: in vector case inputs only, `unit` repeated `count` times.
//!
//! The encoder rejects any other object with one of these keys, so an object with a marker key is
//! a marker, and a malformed one is an error.
use serde_json::{Map, Number, Value};

const UTF16: &str = "$utf16";
const F64: &str = "$f64";
const REPEAT: &str = "$repeat";

/// `value` with every `$f64` marker replaced by its number, at any depth.
///
/// # Panics
///
/// On a malformed `$f64` marker.
pub(crate) fn decode_f64(value: Value) -> Value {
    try_decode_f64(value).unwrap_or_else(|e| panic!("{e}"))
}

/// `value` with every `$utf16` marker replaced by its string, every unpaired surrogate dropped
/// (RD-22: a Rust cut drops the whole character where TS keeps half of it), at any depth.
///
/// # Panics
///
/// On a malformed `$utf16` marker.
pub(crate) fn d09(value: Value) -> Value {
    try_d09(value).unwrap_or_else(|e| panic!("{e}"))
}

/// `value` with every `$repeat` macro replaced by its string, at any depth.
///
/// # Panics
///
/// On a malformed `$repeat` macro.
pub(crate) fn expand(value: Value) -> Value {
    try_expand(value).unwrap_or_else(|e| panic!("{e}"))
}

/// [`decode_f64`], with an error for a malformed marker.
pub(super) fn try_decode_f64(value: Value) -> Result<Value, String> {
    rewrite(value, F64, &|marker| {
        let bits = marker
            .as_str()
            .filter(|hex| {
                hex.len() == 16 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            })
            .and_then(|hex| u64::from_str_radix(hex, 16).ok())
            .ok_or("16 lowercase hex digits")?;
        Number::from_f64(f64::from_bits(bits))
            .map(Value::Number)
            .ok_or_else(|| "a finite number".to_owned())
    })
}

/// [`d09`], with an error for a malformed marker.
fn try_d09(value: Value) -> Result<Value, String> {
    rewrite(value, UTF16, &|marker| {
        let units = marker
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .map(|unit| unit.as_u64().and_then(|n| u16::try_from(n).ok()))
                    .collect::<Option<Vec<u16>>>()
            })
            .ok_or("an array of UTF-16 code units")?;
        Ok(Value::String(
            char::decode_utf16(units).filter_map(Result::ok).collect(),
        ))
    })
}

/// [`expand`], with an error for a malformed macro.
pub(super) fn try_expand(value: Value) -> Result<Value, String> {
    rewrite(
        value,
        REPEAT,
        &|marker| match marker.as_array().map(Vec::as_slice) {
            Some([Value::String(unit), count]) => count
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .map(|n| Value::String(unit.repeat(n)))
                .ok_or_else(|| "a non-negative integer count".to_owned()),
            _ => Err("[\"<unit>\", <count>]".to_owned()),
        },
    )
}

/// `value` with every object that has the key `marker` replaced by `decode` of that key's value.
/// Objects keep their key order.
fn rewrite(
    value: Value,
    marker: &str,
    decode: &dyn Fn(&Value) -> Result<Value, String>,
) -> Result<Value, String> {
    match value {
        Value::Array(items) => items
            .into_iter()
            .map(|item| rewrite(item, marker, decode))
            .collect::<Result<_, _>>()
            .map(Value::Array),
        Value::Object(map) if map.contains_key(marker) => {
            let content = match map.len() {
                1 => map.get(marker),
                _ => None,
            };
            content
                .ok_or_else(|| "it has other keys".to_owned())
                .and_then(decode)
                .map_err(|why| malformed(marker, &map, &why))
        }
        Value::Object(map) => map
            .into_iter()
            .map(|(key, item)| Ok((key, rewrite(item, marker, decode)?)))
            .collect::<Result<Map<_, _>, String>>()
            .map(Value::Object),
        other => Ok(other),
    }
}

fn malformed(marker: &str, map: &Map<String, Value>, why: &str) -> String {
    let text = Value::Object(map.clone()).to_string();
    let shown: String = text.chars().take(200).collect();
    format!("malformed {marker} marker {shown}: expected {why}")
}

mod tests {
    use serde_json::{Value, json};

    use super::{d09, decode_f64, expand};

    fn f64_of(value: &Value) -> f64 {
        value.as_f64().unwrap()
    }

    #[test]
    fn decode_f64_gives_negative_zero() {
        let out = decode_f64(json!({"$f64": "8000000000000000"}));
        assert!(out.is_f64(), "{out}");
        assert_eq!(f64_of(&out).to_bits(), (-0.0f64).to_bits());
    }

    #[test]
    fn decode_f64_gives_the_exact_bits() {
        let out = decode_f64(json!({"$f64": "3fd3333333333334"}));
        assert_eq!(f64_of(&out).to_bits(), (0.1f64 + 0.2).to_bits());
    }

    #[test]
    fn decode_f64_recurses_and_leaves_other_values_alone() {
        let out = decode_f64(json!({
            "a": [1, {"$f64": "8000000000000000"}, "x"],
            "b": {"c": {"$f64": "3fd3333333333334"}},
            "u": {"$utf16": [97, 55357]},
            "r": {"$repeat": ["ab", 2]},
        }));
        assert_eq!(f64_of(&out["a"][1]).to_bits(), (-0.0f64).to_bits());
        assert_eq!(f64_of(&out["b"]["c"]).to_bits(), (0.1f64 + 0.2).to_bits());
        assert_eq!(out["a"][0], json!(1));
        assert_eq!(out["a"][2], json!("x"));
        assert_eq!(out["u"], json!({"$utf16": [97, 55357]}));
        assert_eq!(out["r"], json!({"$repeat": ["ab", 2]}));
    }

    #[test]
    fn d09_drops_an_unpaired_surrogate() {
        assert_eq!(d09(json!({"$utf16": [97, 55357]})), json!("a"));
    }

    #[test]
    fn d09_keeps_a_surrogate_pair() {
        assert_eq!(d09(json!({"$utf16": [55357, 56832, 98]})), json!("😀b"));
    }

    #[test]
    fn d09_drops_every_unpaired_surrogate_whatever_its_position() {
        // A low surrogate first, a high one before a non-surrogate, and a high one last.
        let units = [56832, 97, 55357, 98, 55357, 56832, 55357];
        assert_eq!(d09(json!({"$utf16": units})), json!("ab😀"));
    }

    #[test]
    fn d09_recurses_and_leaves_other_values_alone() {
        let out = d09(json!({
            "a": [{"$utf16": [97, 55357]}, 2],
            "b": {"c": {"$utf16": [56832]}},
            "f": {"$f64": "8000000000000000"},
        }));
        assert_eq!(
            out,
            json!({"a": ["a", 2], "b": {"c": ""}, "f": {"$f64": "8000000000000000"}})
        );
    }

    #[test]
    fn expand_repeats_the_unit() {
        assert_eq!(
            expand(json!({"a": [{"$repeat": ["ab", 3]}]})),
            json!({"a": ["ababab"]})
        );
    }

    #[test]
    fn expand_recurses_and_leaves_other_values_alone() {
        let out = expand(json!([
            {"$repeat": ["😀", 2]},
            {"b": {"$repeat": ["x", 0]}},
            {"$f64": "8000000000000000"},
            "plain",
        ]));
        assert_eq!(
            out,
            json!(["😀😀", {"b": ""}, {"$f64": "8000000000000000"}, "plain"])
        );
    }

    #[test]
    fn decoding_keeps_the_key_order() {
        let input = json!({"z": {"$f64": "8000000000000000"}, "a": {"$utf16": [97]}, "m": {"$repeat": ["q", 1]}});
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        for out in [decode_f64(input.clone()), d09(input.clone()), expand(input)] {
            assert_eq!(keys(&out), ["z", "a", "m"]);
        }
    }

    #[test]
    #[should_panic(expected = "$f64")]
    fn a_malformed_f64_marker_panics() {
        let _ = decode_f64(json!({"$f64": "-0"}));
    }

    #[test]
    #[should_panic(expected = "$utf16")]
    fn a_malformed_utf16_marker_panics() {
        let _ = d09(json!({"$utf16": [65536]}));
    }

    #[test]
    #[should_panic(expected = "$repeat")]
    fn a_malformed_repeat_marker_panics() {
        let _ = expand(json!({"$repeat": ["ab", -1]}));
    }
}
