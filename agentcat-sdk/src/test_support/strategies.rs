//! The input generators the property tests share. They lean on the inputs where JavaScript and
//! Rust differ: escapes, JavaScript whitespace, characters outside the BMP (two UTF-16 units),
//! array-index-like object keys, and numbers that `JSON.stringify` writes in its own way.
use std::ops::{Range, RangeInclusive};

use proptest::prelude::*;
use serde_json::{Map, Number, Value};

/// One part of a character mix: its weight, and the characters it draws from, evenly.
type Part = (u32, &'static [RangeInclusive<char>]);

/// The BMP characters of [`js_char`] in their weights, then any BMP character (the last part, for
/// [`bmp_string`] only). The third part lists the JavaScript whitespace set (what
/// `String.prototype.trim` removes) and its neighbors U+0085, U+180E and U+200B, which are not
/// JavaScript whitespace.
#[expect(
    clippy::single_range_in_vec_init,
    reason = "a part is a list of ranges, and the private use area is one range"
)]
static BMP_PARTS: [Part; 7] = [
    (6, &['a'..='z', 'A'..='Z', '0'..='9']),
    (1, &['"'..='"', '\\'..='\\']),
    (
        2,
        &[
            '\u{9}'..='\u{D}',
            '\u{20}'..='\u{20}',
            '\u{85}'..='\u{85}',
            '\u{A0}'..='\u{A0}',
            '\u{1680}'..='\u{1680}',
            '\u{180E}'..='\u{180E}',
            '\u{2000}'..='\u{200B}',
            '\u{2028}'..='\u{2029}',
            '\u{202F}'..='\u{202F}',
            '\u{205F}'..='\u{205F}',
            '\u{3000}'..='\u{3000}',
            '\u{FEFF}'..='\u{FEFF}',
        ],
    ),
    (1, &['\u{0}'..='\u{1F}', '\u{7F}'..='\u{7F}']),
    (1, &['é'..='é', '世'..='世']),
    (1, &['\u{E000}'..='\u{F8FF}']),
    (1, &['\u{0}'..='\u{D7FF}', '\u{E000}'..='\u{FFFF}']),
];

/// The draws per unit of weight: at least the size of the largest part (any BMP character,
/// 63 488), so every character of a part can come up.
const SLOTS: u32 = 1 << 16;

/// One character of `parts`. A single integer draw mapped through the table, not a union of
/// strategies: a 400 000-character [`bmp_string`] holds one small value tree per character.
fn mix(parts: &'static [Part]) -> impl Strategy<Value = char> + Clone {
    let draws: u32 = parts.iter().map(|&(weight, _)| weight * SLOTS).sum();
    (0..draws).prop_map(move |n| pick(parts, n))
}

/// The character of `draw`: the parts take `weight * SLOTS` draws each, in order, and a part
/// gives its characters in turn. Draw 0 is `'a'`, where a shrunk character ends.
fn pick(parts: &[Part], draw: u32) -> char {
    let mut n = draw;
    let len =
        |range: &RangeInclusive<char>| u32::from(*range.end()) - u32::from(*range.start()) + 1;
    for &(weight, ranges) in parts {
        if n < weight * SLOTS {
            let mut k = n % ranges.iter().map(len).sum::<u32>();
            for range in ranges {
                if k < len(range) {
                    let code = u32::from(*range.start()) + k;
                    return char::from_u32(code).expect("no range holds a surrogate");
                }
                k -= len(range);
            }
        }
        n -= weight * SLOTS;
    }
    panic!("draw {draw} is past the end of the mix")
}

/// Object keys a strategy never gives: `__proto__` (RD-24 has its own vector cases) and the
/// marker keys of the strict JSON encoding.
const RESERVED_KEYS: [&str; 4] = ["__proto__", "$utf16", "$f64", "$repeat"];

/// One character: a weighted mix of ASCII letters and digits, `"` and `\`, U+0000 to U+001F,
/// U+007F, the JavaScript whitespace set and its neighbors, BMP letters, the private use area
/// U+E000 to U+F8FF, characters outside the BMP, and any `char`.
pub(crate) fn js_char() -> impl Strategy<Value = char> + Clone {
    prop_oneof![
        12 => mix(&BMP_PARTS[..BMP_PARTS.len() - 1]),
        2 => prop::sample::select(vec!['\u{1F600}', '\u{1D11E}']),
        1 => any::<char>(),
    ]
}

/// A string of 0 to `max_len` characters of [`js_char`].
pub(crate) fn js_string(max_len: usize) -> impl Strategy<Value = String> + Clone {
    prop::collection::vec(js_char(), 0..=max_len).prop_map(String::from_iter)
}

/// A string of `len` characters, all in the BMP, so its UTF-16 length is its length in
/// characters: the characters of [`js_char`] without those outside the BMP, and any BMP `char`
/// (weight 1 against 12). Fast enough for strings of several hundred thousand characters.
pub(crate) fn bmp_string(len: Range<usize>) -> impl Strategy<Value = String> + Clone {
    prop::collection::vec(mix(&BMP_PARTS), len).prop_map(String::from_iter)
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

    use super::{BMP_PARTS, SLOTS, bmp_string, js_string, json_value, number, object_key, pick};
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

    /// Every draw of the whole mix: each part gets `weight * SLOTS` draws, every character of a
    /// part comes up, no draw leaves its part, and no character is outside the BMP.
    #[test]
    fn the_mix_gives_each_part_its_weight_and_every_character() {
        let mut n = 0;
        for &(weight, ranges) in &BMP_PARTS {
            let mut seen = std::collections::BTreeSet::new();
            for _ in 0..weight * SLOTS {
                let c = pick(&BMP_PARTS, n);
                assert!(
                    ranges.iter().any(|r| r.contains(&c)),
                    "draw {n}: {c:?} not in {ranges:?}"
                );
                assert!(u32::from(c) <= 0xFFFF, "draw {n}: {c:?}");
                seen.insert(c);
                n += 1;
            }
            let size: usize = ranges.iter().map(|r| r.clone().count()).sum();
            assert_eq!(seen.len(), size, "{ranges:?}");
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
