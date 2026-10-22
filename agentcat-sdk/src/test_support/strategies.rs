//! The input generators the property tests share. They lean on the inputs where JavaScript and
//! Rust differ: escapes, JavaScript whitespace, characters outside the BMP (two UTF-16 units),
//! array-index-like object keys, and numbers that `JSON.stringify` writes in its own way.
use std::ops::{Range, RangeInclusive};

use proptest::prelude::*;
use rmcp::model::Tool;
use serde_json::{Map, Number, Value, json};

use crate::pipeline::TsEvent;

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

/// A tool-call event as the TS SDK holds it when its pipeline runs ([`TsEvent`]), its keys in the
/// order TS sets them: `id` (`""`), `sessionId`, `projectId`, `eventType` (`"mcp:tools/call"`),
/// `timestamp` (ISO 8601 with milliseconds), `duration`, `inputTokens`, `resourceName`,
/// `parameters` (`{request, extra}`), `response` (`{content, structuredContent?, isError?}`, see
/// [`content_block`]), `userIntent`, `isError`, `error` (`{message, type, platform, frames?}`),
/// `identifyActorGivenId`, `identifyActorName`, `identifyActorData`, `sdkLanguage`,
/// `agentcatVersion`, `serverName`, `serverVersion`, `clientName`, `clientVersion`, `tags` and
/// `properties`. Each field but `id`, `sessionId`, `eventType` and `timestamp` is present or
/// absent; every string a customer, a client or the SDK sets is drawn from `text`, and the JSON
/// payloads are [`payload`]s.
///
/// The fields TS truncation limits with `truncateString` hold strings and `frames` is an array,
/// so no event meets RD-26 (an object with a `length` key) or RD-27 (an object with a `toString`
/// key in a joined array): the objects of the payloads only reach `normalize`.
pub(crate) fn ts_event(
    text: impl Strategy<Value = String> + Clone + 'static,
) -> impl Strategy<Value = TsEvent> + Clone {
    let string = || text.clone().prop_map(Value::String).boxed();
    let present = |strategy: BoxedStrategy<Value>| strategy.prop_map(Some).boxed();
    let maybe = |strategy: BoxedStrategy<Value>| prop::option::of(strategy).boxed();
    let count = || any::<u32>().prop_map(Value::from).boxed();
    let fields: Vec<(&'static str, BoxedStrategy<Option<Value>>)> = vec![
        ("id", Just(Some(Value::String(String::new()))).boxed()),
        ("sessionId", present(string())),
        ("projectId", maybe(string())),
        ("eventType", Just(Some(json!("mcp:tools/call"))).boxed()),
        ("timestamp", present(timestamp().boxed())),
        ("duration", maybe(count())),
        ("inputTokens", maybe(count())),
        ("resourceName", maybe(string())),
        ("parameters", maybe(parameters(text.clone()))),
        ("response", maybe(response(text.clone()))),
        ("userIntent", maybe(string())),
        (
            "isError",
            maybe(any::<bool>().prop_map(Value::Bool).boxed()),
        ),
        ("error", maybe(error_data(text.clone()))),
        ("identifyActorGivenId", maybe(string())),
        ("identifyActorName", maybe(string())),
        ("identifyActorData", maybe(object(text.clone()))),
        ("sdkLanguage", maybe(string())),
        ("agentcatVersion", maybe(string())),
        ("serverName", maybe(string())),
        ("serverVersion", maybe(string())),
        ("clientName", maybe(string())),
        ("clientVersion", maybe(string())),
        ("tags", maybe(tags(text.clone()))),
        ("properties", maybe(object(text))),
    ];
    let (names, values): (Vec<_>, Vec<_>) = fields.into_iter().unzip();
    values.prop_map(move |values| {
        names
            .iter()
            .zip(values)
            .filter_map(|(name, value)| Some(((*name).to_owned(), value?)))
            .collect()
    })
}

/// An ISO 8601 timestamp with milliseconds, as `Date.prototype.toISOString` writes it.
fn timestamp() -> impl Strategy<Value = Value> + Clone {
    (
        (2000u32..2100, 1u32..=12, 1u32..=28),
        (0u32..24, 0u32..60, 0u32..60, 0u32..1000),
    )
        .prop_map(|((year, month, day), (hour, minute, second, ms))| {
            Value::String(format!(
                "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{ms:03}Z"
            ))
        })
}

/// A JSON value like [`json_value`] (at most 3 levels, 4 items per container) whose strings are
/// drawn from `text`.
fn payload(text: impl Strategy<Value = String> + Clone + 'static) -> BoxedStrategy<Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        number().prop_map(Value::Number),
        text.prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..=4).prop_map(Value::Array),
            prop::collection::vec((object_key(), inner), 0..=4)
                .prop_map(|entries| Value::Object(entries.into_iter().collect())),
        ]
    })
    .boxed()
}

/// An object of up to 4 [`payload`]s under [`object_key`]s.
fn object(text: impl Strategy<Value = String> + Clone + 'static) -> BoxedStrategy<Value> {
    prop::collection::vec((object_key(), payload(text)), 0..=4)
        .prop_map(|entries| Value::Object(entries.into_iter().collect()))
        .boxed()
}

/// Up to 5 tags: [`object_key`]s with strings drawn from `text`.
fn tags(text: impl Strategy<Value = String> + Clone + 'static) -> BoxedStrategy<Value> {
    prop::collection::vec((object_key(), text.prop_map(Value::String)), 0..=5)
        .prop_map(|entries| Value::Object(entries.into_iter().collect()))
        .boxed()
}

/// `{request, extra}`: an MCP `tools/call` request with [`payload`] arguments, and a payload.
fn parameters(text: impl Strategy<Value = String> + Clone + 'static) -> BoxedStrategy<Value> {
    (text.clone(), payload(text.clone()), payload(text))
        .prop_map(|(name, arguments, extra)| {
            json!({
                "request": {"method": "tools/call", "params": {"name": name, "arguments": arguments}},
                "extra": extra,
            })
        })
        .boxed()
}

/// `{content, structuredContent?, isError?}`, with up to 4 [`content_block`]s.
fn response(text: impl Strategy<Value = String> + Clone + 'static) -> BoxedStrategy<Value> {
    (
        prop::collection::vec(content_block(text.clone()), 0..=4),
        prop::option::of(payload(text)),
        prop::option::of(any::<bool>()),
    )
        .prop_map(|(content, structured, is_error)| {
            let mut response = Map::new();
            response.insert("content".to_owned(), Value::Array(content));
            if let Some(structured) = structured {
                response.insert("structuredContent".to_owned(), structured);
            }
            if let Some(is_error) = is_error {
                response.insert("isError".to_owned(), Value::Bool(is_error));
            }
            Value::Object(response)
        })
        .boxed()
}

/// One content block of a tool result: `text`, `image`, `audio`, `resource` with `text` or with
/// `blob`, `resource_link`, or the unknown type `"custom"` with a [`payload`].
fn content_block(text: impl Strategy<Value = String> + Clone + 'static) -> BoxedStrategy<Value> {
    let pair = || (text.clone(), text.clone());
    prop_oneof![
        text.clone().prop_map(|text| json!({"type": "text", "text": text})),
        text.clone().prop_map(|data| json!({"type": "image", "data": data, "mimeType": "image/png"})),
        text.clone().prop_map(|data| json!({"type": "audio", "data": data, "mimeType": "audio/wav"})),
        pair().prop_map(|(uri, text)| {
            json!({"type": "resource", "resource": {"uri": uri, "mimeType": "text/plain", "text": text}})
        }),
        pair().prop_map(|(uri, blob)| json!({"type": "resource", "resource": {"uri": uri, "blob": blob}})),
        pair().prop_map(|(uri, name)| json!({"type": "resource_link", "uri": uri, "name": name})),
        payload(text.clone()).prop_map(|value| json!({"type": "custom", "value": value})),
    ]
    .boxed()
}

/// `{message, type, platform: "rust", frames?}`, with up to 60 frames.
fn error_data(text: impl Strategy<Value = String> + Clone + 'static) -> BoxedStrategy<Value> {
    let frame = (text.clone(), text.clone(), any::<u32>(), any::<bool>()).prop_map(
        |(filename, function, lineno, in_app)| {
            json!({"filename": filename, "function": function, "lineno": lineno, "in_app": in_app})
        },
    );
    (
        text.clone(),
        text,
        prop::option::of(prop::collection::vec(frame, 0..=60)),
    )
        .prop_map(|(message, kind, frames)| {
            let mut error = Map::new();
            error.insert("message".to_owned(), Value::String(message));
            error.insert("type".to_owned(), Value::String(kind));
            error.insert("platform".to_owned(), json!("rust"));
            if let Some(frames) = frames {
                error.insert("frames".to_owned(), Value::Array(frames));
            }
            Value::Object(error)
        })
        .boxed()
}

/// A tool listing of 1 to 6 tools in the shapes of the `seeded:` cases of
/// `oracle/src/vectors/gen/inject.mjs`, each one rmcp's `Tool` holds. Names come from a small set
/// with `get_more_tools` and `unknown`, so they repeat. An input schema has, in any key order: a
/// `type` or none; `properties` that is an object of customer properties (some named `session_id`,
/// `agent_id` or `context`, with truthy or falsy schemas, and index keys), absent, falsy, a string,
/// number or boolean, or an array; `required` absent, a list or another value;
/// `additionalProperties`; and a composition key with a truthy or falsy value. A tool sometimes has
/// a description, an output schema ([`output_schema`]) and annotations.
pub(crate) fn tool_list() -> impl Strategy<Value = Vec<Tool>> + Clone {
    let name = prop::sample::select(vec!["echo", "search", "get_more_tools", "own", "unknown"]);
    let description = prop::option::of(prop::sample::select(vec!["Search", "", "d"]));
    let annotations =
        prop::option::weighted(0.15, Just(json!({"title": "T", "readOnlyHint": true})));
    let tool = (
        name,
        description,
        input_schema(),
        prop::option::weighted(0.4, output_schema()),
        annotations,
    )
        .prop_map(|(name, description, input, output, annotations)| {
            let mut tool = Map::new();
            tool.insert("name".to_owned(), json!(name));
            if let Some(description) = description {
                tool.insert("description".to_owned(), json!(description));
            }
            tool.insert("inputSchema".to_owned(), input);
            if let Some(output) = output {
                tool.insert("outputSchema".to_owned(), output);
            }
            if let Some(annotations) = annotations {
                tool.insert("annotations".to_owned(), annotations);
            }
            serde_json::from_value::<Tool>(Value::Object(tool)).expect("rmcp's Tool holds the tool")
        });
    prop::collection::vec(tool, 1..=6)
}

/// An input schema of [`tool_list`].
fn input_schema() -> impl Strategy<Value = Value> + Clone {
    let select = |values: Vec<Value>| prop::sample::select(values);
    let property = select(vec![
        json!({"type": "string"}),
        json!({"type": "number", "description": "n"}),
        json!({}),
        json!(false),
        Value::Null,
        json!(0),
        json!(""),
    ]);
    let key = prop::sample::select(vec![
        "text",
        "q",
        "session_id",
        "agent_id",
        "context",
        "10",
        "2",
    ]);
    let customer = prop::collection::vec((key, property), 0..=4).prop_map(|entries| {
        Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        )
    });
    let properties = prop_oneof![
        3 => customer.prop_map(Some),
        1 => Just(None),
        1 => select(vec![Value::Null, json!(false), json!(0), json!("")]).prop_map(Some),
        1 => select(vec![json!("abc"), json!(5), json!(true)]).prop_map(Some),
        1 => select(vec![json!([]), json!([{"type": "string"}])]).prop_map(Some),
    ];
    let names = prop::sample::subsequence(vec!["text", "session_id", "agent_id", "context"], 0..=4);
    let required = prop_oneof![
        2 => Just(None),
        2 => names.prop_map(|names| Some(json!(names))),
        1 => select(vec![json!("text"), Value::Null, json!({})]).prop_map(Some),
    ];
    let additional = prop::option::weighted(
        0.4,
        select(vec![
            json!(false),
            json!(false),
            json!(true),
            json!({"type": "string"}),
        ]),
    );
    let composition = prop::option::weighted(
        0.25,
        (
            prop::sample::select(vec!["oneOf", "allOf", "anyOf"]),
            select(vec![
                json!([]),
                json!([{"required": ["q"]}]),
                json!({}),
                Value::Null,
                json!(0),
                json!(false),
                json!(""),
            ]),
        ),
    );
    (
        prop::bool::weighted(0.7),
        properties,
        required,
        additional,
        composition,
    )
        .prop_flat_map(|(typed, properties, required, additional, composition)| {
            let mut entries = Vec::new();
            if typed {
                entries.push(("type".to_owned(), json!("object")));
            }
            let keyed = [
                ("properties", properties),
                ("required", required),
                ("additionalProperties", additional),
            ];
            for (key, value) in keyed {
                if let Some(value) = value {
                    entries.push((key.to_owned(), value));
                }
            }
            if let Some((key, value)) = composition {
                entries.push((key.to_owned(), value));
            }
            Just(entries).prop_shuffle()
        })
        .prop_map(|entries| Value::Object(entries.into_iter().collect()))
}

/// An output schema of [`tool_list`]: a plain object schema (sometimes with `required` and
/// `additionalProperties: false`), one without `properties`, `{"type": "array"}`, a composition, one
/// that declares `mcp_session` (truthy or `false`), and `properties` that is a string, a number or an
/// array.
fn output_schema() -> impl Strategy<Value = Value> + Clone {
    let select = |values: Vec<Value>| prop::sample::select(values);
    let count = || json!({"count": {"type": "number"}});
    prop_oneof![
        2 => any::<bool>().prop_map(move |strict| if strict {
            json!({"type": "object", "properties": count(), "required": ["count"], "additionalProperties": false})
        } else {
            json!({"type": "object", "properties": count()})
        }),
        1 => select(vec![json!({"type": "object"}), json!({})]),
        1 => Just(json!({"type": "array"})),
        1 => (
            prop::sample::select(vec!["oneOf", "allOf", "anyOf"]),
            select(vec![json!([{"type": "object"}]), json!([]), Value::Null]),
        )
            .prop_map(|(key, value)| Value::Object(Map::from_iter([(key.to_owned(), value)]))),
        1 => select(vec![json!({"type": "object"}), json!(false)]).prop_map(|own| {
            json!({"type": "object", "properties": {"count": {"type": "number"}, "mcp_session": own}})
        }),
        1 => select(vec![json!("x"), json!(3), json!([])])
            .prop_map(|properties| json!({"type": "object", "properties": properties})),
    ]
}

mod tests {
    use proptest::prelude::*;
    use serde_json::Value;

    use super::{
        BMP_PARTS, SLOTS, bmp_string, js_string, json_value, number, object_key, pick, ts_event,
    };
    use crate::test_support::config;

    const RESERVED: [&str; 4] = ["__proto__", "$utf16", "$f64", "$repeat"];

    /// The keys of a [`ts_event`], in order.
    const EVENT_KEYS: [&str; 24] = [
        "id",
        "sessionId",
        "projectId",
        "eventType",
        "timestamp",
        "duration",
        "inputTokens",
        "resourceName",
        "parameters",
        "response",
        "userIntent",
        "isError",
        "error",
        "identifyActorGivenId",
        "identifyActorName",
        "identifyActorData",
        "sdkLanguage",
        "agentcatVersion",
        "serverName",
        "serverVersion",
        "clientName",
        "clientVersion",
        "tags",
        "properties",
    ];

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

        #[test]
        fn ts_events_have_the_ts_key_order_and_the_fixed_fields(event in ts_event(js_string(8))) {
            let keys: Vec<&str> = event.keys().map(String::as_str).collect();
            let mut order = EVENT_KEYS.iter();
            for key in &keys {
                prop_assert!(order.any(|known| known == key), "{keys:?}");
            }
            for key in ["id", "sessionId", "eventType", "timestamp"] {
                prop_assert!(event.contains_key(key), "{key}");
            }
            prop_assert_eq!(&event["id"], "");
            prop_assert_eq!(&event["eventType"], "mcp:tools/call");
            let timestamp = event["timestamp"].as_str().unwrap_or_default();
            prop_assert_eq!(timestamp.len(), "2025-01-15T12:00:00.000Z".len(), "{}", timestamp);
            if let Some(Value::Object(error)) = event.get("error") {
                prop_assert!(error.get("frames").is_none_or(Value::is_array));
            }
            for key in ["userIntent", "resourceName", "serverName", "serverVersion", "clientName", "clientVersion"] {
                prop_assert!(event.get(key).is_none_or(Value::is_string), "{key}");
            }
        }
    }
}
