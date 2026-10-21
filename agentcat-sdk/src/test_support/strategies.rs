//! The input generators the property tests share. They lean on the inputs where JavaScript and
//! Rust differ: escapes, JavaScript whitespace, characters outside the BMP (two UTF-16 units),
//! array-index-like object keys, and numbers that `JSON.stringify` writes in its own way.
use std::ops::Range;

use proptest::prelude::*;
use serde_json::{Map, Number, Value};

/// The JavaScript whitespace set (what `String.prototype.trim` removes) and its neighbors
/// U+0085, U+180E and U+200B, which are not JavaScript whitespace.
const WHITESPACE: [char; 28] = [
    '\u{9}', '\u{A}', '\u{B}', '\u{C}', '\u{D}', '\u{20}', '\u{85}', '\u{A0}', '\u{1680}',
    '\u{180E}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}',
    '\u{2007}', '\u{2008}', '\u{2009}', '\u{200A}', '\u{200B}', '\u{2028}', '\u{2029}', '\u{202F}',
    '\u{205F}', '\u{3000}', '\u{FEFF}',
];

/// Object keys a strategy never gives: `__proto__` (RD-24 has its own vector cases) and the
/// marker keys of the strict JSON encoding.
const RESERVED_KEYS: [&str; 4] = ["__proto__", "$utf16", "$f64", "$repeat"];

/// The characters of [`js_char`] that are in the BMP, in their weights.
fn bmp_mix() -> impl Strategy<Value = char> + Clone {
    prop_oneof![
        6 => prop_oneof![
            prop::char::range('a', 'z'),
            prop::char::range('A', 'Z'),
            prop::char::range('0', '9'),
        ],
        1 => prop::sample::select(vec!['"', '\\']),
        1 => prop_oneof![prop::char::range('\u{0}', '\u{1F}'), Just('\u{7F}')],
        2 => prop::sample::select(WHITESPACE.to_vec()),
        1 => prop::sample::select(vec!['é', '世']),
        1 => prop::char::range('\u{E000}', '\u{F8FF}'),
    ]
}

/// One character: a weighted mix of ASCII letters and digits, `"` and `\`, U+0000 to U+001F,
/// U+007F, the JavaScript whitespace set and its neighbors, BMP letters, the private use area
/// U+E000 to U+F8FF, characters outside the BMP, and any `char`.
pub(crate) fn js_char() -> impl Strategy<Value = char> + Clone {
    prop_oneof![
        12 => bmp_mix(),
        2 => prop::sample::select(vec!['\u{1F600}', '\u{1D11E}']),
        1 => any::<char>(),
    ]
}

/// A string of 0 to `max_len` characters of [`js_char`].
pub(crate) fn js_string(max_len: usize) -> impl Strategy<Value = String> + Clone {
    prop::collection::vec(js_char(), 0..=max_len).prop_map(String::from_iter)
}

/// A string of `len` characters, all in the BMP, so its UTF-16 length is its length in
/// characters: the characters of [`js_char`] without those outside the BMP, and any BMP `char`.
pub(crate) fn bmp_string(len: Range<usize>) -> impl Strategy<Value = String> + Clone {
    let bmp_char = prop_oneof![
        12 => bmp_mix(),
        1 => prop::char::range('\u{0}', '\u{FFFF}'),
    ];
    prop::collection::vec(bmp_char, len).prop_map(String::from_iter)
}

/// An object key: array-index-like keys (`"0"` to `"20"`, `"4294967294"`, `"4294967295"`, and
/// the non-index spellings `"01"`, `"-1"`, `"1.5"`), short words, `"..."`, `"context"`,
/// `"type"`, `"text"`, and [`js_string`] of up to 12 characters. Never `__proto__`, `$utf16`,
/// `$f64` or `$repeat`.
pub(crate) fn object_key() -> impl Strategy<Value = String> + Clone {
    let fixed = |keys: &[&str]| {
        prop::sample::select(keys.iter().map(|&k| k.to_owned()).collect::<Vec<_>>())
    };
    prop_oneof![
        3 => (0u32..=20).prop_map(|n| n.to_string()),
        1 => fixed(&["4294967294", "4294967295", "01", "-1", "1.5"]),
        2 => fixed(&["a", "b", "id", "key", "name", "value", "data", "items"]),
        2 => fixed(&["...", "context", "type", "text"]),
        3 => js_string(12),
    ]
    .prop_filter("a reserved key", |key| {
        !RESERVED_KEYS.contains(&key.as_str())
    })
}

/// A JSON number: any `i64`, any `u64`, any finite `f64`, and the specials `0.0`, `-0.0`, `1e21`,
/// `1e-7`, `1e-6`, `0.1 + 0.2`, and the integers 2^53 - 1 and 2^53 + 1.
pub(crate) fn number() -> impl Strategy<Value = Number> + Clone {
    let finite = prop::num::f64::POSITIVE
        | prop::num::f64::NEGATIVE
        | prop::num::f64::NORMAL
        | prop::num::f64::SUBNORMAL
        | prop::num::f64::ZERO;
    let floats = [0.0, -0.0, 1e21, 1e-7, 1e-6, 0.1 + 0.2];
    let specials: Vec<Number> = floats
        .into_iter()
        .filter_map(Number::from_f64)
        .chain([
            Number::from((1i64 << 53) - 1),
            Number::from((1i64 << 53) + 1),
        ])
        .collect();
    prop_oneof![
        2 => any::<i64>().prop_map(Number::from),
        1 => any::<u64>().prop_map(Number::from),
        2 => finite.prop_filter_map("not finite", Number::from_f64),
        2 => prop::sample::select(specials),
    ]
}

/// A JSON value nested at most `depth` arrays or objects deep, each with at most `width` items:
/// `null`, booleans, [`number`]s, [`js_string`]s of up to 16 characters, arrays, and objects
/// with [`object_key`] keys.
pub(crate) fn json_value(depth: u32, width: usize) -> impl Strategy<Value = Value> + Clone {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        number().prop_map(Value::Number),
        js_string(16).prop_map(Value::String),
    ];
    let branch = u32::try_from(width).unwrap_or(u32::MAX);
    leaf.prop_recursive(
        depth,
        branch.saturating_mul(8).max(16),
        branch.max(1),
        move |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..=width).prop_map(Value::Array),
                prop::collection::vec((object_key(), inner), 0..=width)
                    .prop_map(|entries| Value::Object(entries.into_iter().collect::<Map<_, _>>())),
            ]
        },
    )
}

mod tests {
    use proptest::prelude::*;
    use serde_json::Value;

    use super::{bmp_string, js_string, json_value, number, object_key};
    use crate::test_support::config;

    const RESERVED: [&str; 4] = ["__proto__", "$utf16", "$f64", "$repeat"];

    /// The deepest nesting of arrays and objects in `value` (a scalar is 0), and checks every
    /// container has at most `width` items and every number is finite.
    fn depth(value: &Value, width: usize) -> u32 {
        match value {
            Value::Array(items) => {
                assert!(items.len() <= width, "{value}");
                1 + items.iter().map(|v| depth(v, width)).max().unwrap_or(0)
            }
            Value::Object(map) => {
                assert!(map.len() <= width, "{value}");
                for key in map.keys() {
                    assert!(!RESERVED.contains(&key.as_str()), "{key}");
                }
                1 + map.values().map(|v| depth(v, width)).max().unwrap_or(0)
            }
            Value::Number(n) => {
                assert!(n.as_f64().is_some_and(f64::is_finite), "{n}");
                0
            }
            _ => 0,
        }
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn object_keys_are_never_reserved(key in object_key()) {
            prop_assert!(!RESERVED.contains(&key.as_str()), "{key}");
        }

        #[test]
        fn js_strings_have_at_most_max_len_chars(s in js_string(7)) {
            prop_assert!(s.chars().count() <= 7, "{s:?}");
        }

        #[test]
        fn bmp_strings_have_the_length_and_no_astral_char(s in bmp_string(3..9)) {
            let n = s.chars().count();
            prop_assert!((3..9).contains(&n), "{s:?}");
            prop_assert_eq!(s.encode_utf16().count(), n);
        }

        #[test]
        fn numbers_are_finite(n in number()) {
            prop_assert!(n.as_f64().is_some_and(f64::is_finite), "{n}");
        }

        #[test]
        fn json_values_keep_to_depth_and_width(value in json_value(3, 4)) {
            prop_assert!(depth(&value, 4) <= 3, "{value}");
        }
    }
}
