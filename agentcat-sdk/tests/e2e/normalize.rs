//! The normalized document of a run: `{"transcript": [...], "events": [...]}`.
//!
//! - `transcript`: the client's entries, in step order (see `driver`).
//! - `events`: one `{"content_type", "query", "body"}` per publish request: its `content-type`
//!   header, its query string and its body, the event as the SDK published it.
//!
//! The document keeps every value as the run produced it, except the values that differ between
//! two runs of the same tree. [`RULES`] lists each path whose values the normalizer replaces, once,
//! with the reason. A value that does not have the expected shape stays as it is, so the
//! comparison with the golden file shows it.
//!
//! The events are in time order: by the time of the raw `timestamp`, then the raw text; in the
//! same millisecond, a tool call (an event with a `duration` that is not a custom event: custom
//! events publish during the call) before the rest, then by `resource_name`, then by the
//! normalized body with every placeholder unnumbered and its keys sorted. The SDK publishes
//! several events at once, so the arrival order changes from run to run; the driver keeps the
//! steps of a session in different milliseconds. A scenario with `"eventOrder": "multiset"`
//! orders the events by that masked body alone.
use std::collections::HashMap;

use chrono::DateTime;
use serde_json::{Map, Value, json};

use super::mock_api::Publish;
use super::scenario::EventOrder;

/// One normalization: the values at `path` and how they are replaced.
struct Rule {
    /// The JSON path: `.key` a field, `[*]` every item of an array, `.**` every value at any
    /// depth.
    path: &'static str,
    /// Why the values differ between two runs.
    reason: &'static str,
    /// Replaces one value at the path.
    rewrite: fn(&mut Value, &mut Placeholders),
}

/// The normalizations, applied in this order.
const RULES: &[Rule] = &[
    Rule {
        path: "$.**",
        reason: "the SDK mints evt_, ses_ and agt_ IDs from the clock and random bytes",
        rewrite: replace_ids,
    },
    Rule {
        path: "$.events[*].body.timestamp",
        reason: "the wall-clock time of the event",
        rewrite: replace_timestamp,
    },
    Rule {
        path: "$.events[*].body.duration",
        reason: "the measured time of the call",
        rewrite: replace_duration,
    },
    Rule {
        path: "$.events[*].body.tags",
        reason: "the publish request holds the tags in a hash map: the key order changes",
        rewrite: sort_keys,
    },
    Rule {
        path: "$.events[*].body.parameters.extra.sessionId",
        reason: "rmcp gives each HTTP session a random ID",
        rewrite: replace_http_session,
    },
    Rule {
        path: "$.events[*].body.parameters.extra.requestInfo.headers.mcp-session-id",
        reason: "rmcp gives each HTTP session a random ID",
        rewrite: replace_http_session,
    },
    Rule {
        path: "$.events[*].body.parameters.extra.requestInfo.headers.host",
        reason: "the HTTP server listens on a free port",
        rewrite: replace_port,
    },
    Rule {
        path: "$.events[*].body.parameters.extra.requestInfo.url",
        reason: "the HTTP server listens on a free port",
        rewrite: replace_port,
    },
];

/// The prefixes of the IDs the SDK mints, each followed by 27 base62 characters (a KSUID).
const ID_PREFIXES: [&str; 3] = ["evt_", "ses_", "agt_"];

/// The number of base62 characters after an ID prefix.
const KSUID_CHARS: usize = 27;

/// The KSUID epoch, in seconds since the Unix epoch.
const KSUID_EPOCH_S: u64 = 1_400_000_000;

/// 2024-03-01T00:00:00Z in seconds: a `ses_` ID with an earlier KSUID time is derived from a
/// string (its time lies between 2024-01-01 and about 2024-02-19), the same in every run, and
/// stays. A minted ID has the time it was minted.
const DERIVED_BEFORE_S: u64 = 1_709_251_200;

/// The base62 alphabet of a KSUID.
const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// The normalized document of a run.
pub(crate) fn document(
    transcript: Vec<Value>,
    publishes: Vec<Publish>,
    order: EventOrder,
) -> Value {
    let mut events: Vec<(SortKey, Value)> = publishes
        .into_iter()
        .map(|publish| {
            let entry = object([
                (
                    "content_type",
                    publish.content_type.map_or(Value::Null, Value::String),
                ),
                ("query", Value::String(publish.query)),
                ("body", publish.body),
            ]);
            (SortKey::of(&entry), entry)
        })
        .collect();
    events.sort_by(|(a, _), (b, _)| match order {
        EventOrder::Time => a.cmp(b),
        EventOrder::Multiset => a.masked.cmp(&b.masked),
    });
    let events = events.into_iter().map(|(_, entry)| entry).collect();
    let mut document = object([
        ("transcript", Value::Array(transcript)),
        ("events", Value::Array(events)),
    ]);
    normalize(&mut document, &mut Placeholders::numbered());
    document
}

/// An object of `fields`, in order.
fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

/// Applies [`RULES`] to `document`.
fn normalize(document: &mut Value, placeholders: &mut Placeholders) {
    for rule in RULES {
        let path = parse(rule.path);
        visit(document, &path, &mut |value| {
            (rule.rewrite)(value, placeholders);
        });
    }
}

/// What orders an event.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SortKey {
    /// The time of the raw `timestamp` in milliseconds; [`i64::MAX`] (last) when it is not one.
    time: i64,
    /// The raw `timestamp`, as JSON.
    timestamp: String,
    /// `false` for a tool call, which comes first.
    untimed: bool,
    /// The raw `resource_name`, as JSON.
    resource: String,
    /// The normalized entry with unnumbered placeholders and sorted keys.
    masked: String,
}

impl SortKey {
    /// The key of the publish entry `entry`.
    fn of(entry: &Value) -> Self {
        let body = &entry["body"];
        let timestamp = &body["timestamp"];
        let time = timestamp
            .as_str()
            .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
            .map_or(i64::MAX, |time| time.timestamp_millis());
        let custom = body["event_type"].as_str() == Some("agentcat:custom");
        let mut masked = json!({"events": [entry]});
        normalize(&mut masked, &mut Placeholders::unnumbered());
        Self {
            time,
            timestamp: timestamp.to_string(),
            untimed: !body["duration"].is_number() || custom,
            resource: body["resource_name"].to_string(),
            masked: canonical(&masked["events"][0]),
        }
    }
}

/// The placeholders of the run: each random value gets `<kind#n>`, numbered by first appearance
/// in the document, so a value that appears twice gets the same placeholder. Unnumbered (for the
/// event order), every value of a kind gets `<kind>`.
struct Placeholders {
    numbered: bool,
    given: HashMap<String, String>,
    counts: HashMap<&'static str, usize>,
}

impl Placeholders {
    fn numbered() -> Self {
        Self {
            numbered: true,
            given: HashMap::new(),
            counts: HashMap::new(),
        }
    }

    fn unnumbered() -> Self {
        Self {
            numbered: false,
            ..Self::numbered()
        }
    }

    /// The placeholder of `value`, a value of `kind`.
    fn of(&mut self, kind: &'static str, value: &str) -> String {
        if !self.numbered {
            return format!("<{kind}>");
        }
        if let Some(placeholder) = self.given.get(value) {
            return placeholder.clone();
        }
        let count = self.counts.entry(kind).or_default();
        *count += 1;
        let placeholder = format!("<{kind}#{count}>");
        self.given.insert(value.to_owned(), placeholder.clone());
        placeholder
    }
}

/// A step of a rule's path.
enum Step {
    /// `.key`: the field `key` of an object.
    Key(&'static str),
    /// `[*]`: every item of an array.
    Each,
    /// `.**`: the value and every value below it.
    Deep,
}

/// The steps of the path `path` (`$` then `.key`, `[*]` and `.**`).
///
/// # Panics
///
/// When `path` is not such a path.
fn parse(path: &'static str) -> Vec<Step> {
    let mut rest = path
        .strip_prefix('$')
        .unwrap_or_else(|| panic!("{path}: a path starts with $"));
    let mut steps = Vec::new();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("[*]") {
            steps.push(Step::Each);
            rest = after;
        } else if let Some(after) = rest.strip_prefix(".**") {
            steps.push(Step::Deep);
            rest = after;
        } else if let Some(after) = rest.strip_prefix('.') {
            let end = after.find(['.', '[']).unwrap_or(after.len());
            assert!(end > 0, "{path}: an empty key");
            steps.push(Step::Key(&after[..end]));
            rest = &after[end..];
        } else {
            panic!("{path}: cannot read {rest:?}");
        }
    }
    steps
}

/// Calls `rewrite` with each value of `value` at `path`; `.**` visits a value before the values
/// below it, in document order.
fn visit(value: &mut Value, path: &[Step], rewrite: &mut dyn FnMut(&mut Value)) {
    let Some((step, rest)) = path.split_first() else {
        rewrite(value);
        return;
    };
    match (step, value) {
        (Step::Key(key), Value::Object(fields)) => {
            if let Some(field) = fields.get_mut(*key) {
                visit(field, rest, rewrite);
            }
        }
        (Step::Each, Value::Array(items)) => {
            for item in items {
                visit(item, rest, rewrite);
            }
        }
        (Step::Deep, value) => {
            visit(value, rest, rewrite);
            match value {
                Value::Array(items) => {
                    for item in items {
                        visit(item, path, rewrite);
                    }
                }
                Value::Object(fields) => {
                    for field in fields.values_mut() {
                        visit(field, path, rewrite);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// In a string: each minted ID (a prefix of [`ID_PREFIXES`] and 27 base62 characters, on word
/// boundaries) becomes `<evt#n>`, `<ses#n>` or `<agt#n>`. A derived `ses_` ID stays.
fn replace_ids(value: &mut Value, placeholders: &mut Placeholders) {
    let Value::String(text) = value else { return };
    let mut replaced = String::with_capacity(text.len());
    let mut changed = false;
    let mut rest = text.as_str();
    let mut previous: Option<char> = None;
    while let Some(id) = next_id(rest, previous) {
        let (before, after) = rest.split_at(id.start);
        let found = &after[..id.len];
        replaced.push_str(before);
        if found.starts_with("ses_") && derived(found) {
            replaced.push_str(found);
        } else {
            replaced.push_str(&placeholders.of(id.kind, found));
            changed = true;
        }
        previous = found.chars().last();
        rest = &after[id.len..];
    }
    replaced.push_str(rest);
    if changed {
        *text = replaced;
    }
}

/// Where an ID starts in a text, its length and its kind.
struct Found {
    start: usize,
    len: usize,
    kind: &'static str,
}

/// The first ID in `text`, whose character before is `previous`.
fn next_id(text: &str, previous: Option<char>) -> Option<Found> {
    let bytes = text.as_bytes();
    let len = 4 + KSUID_CHARS;
    for start in 0..bytes.len() {
        let before = match start {
            0 => previous.is_some_and(is_word),
            _ => is_word(char::from(bytes[start - 1])),
        };
        let Some(prefix) = ID_PREFIXES
            .iter()
            .find(|p| bytes[start..].starts_with(p.as_bytes()))
        else {
            continue;
        };
        let body = bytes.get(start + 4..start + len);
        let after = bytes.get(start + len).copied().map(char::from);
        let is_id = !before
            && body.is_some_and(|body| body.iter().all(u8::is_ascii_alphanumeric))
            && !after.is_some_and(is_word);
        if is_id {
            let kind = &prefix[..3];
            return Some(Found { start, len, kind });
        }
    }
    None
}

/// Whether `c` is a word character: an ID starts and ends on a word boundary.
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Whether the `ses_` ID `id` was derived from a string (its KSUID time is before 2024-03-01).
fn derived(id: &str) -> bool {
    ksuid_seconds(&id[4..]).is_some_and(|time| time + KSUID_EPOCH_S < DERIVED_BEFORE_S)
}

/// The time field of the KSUID with the base62 text `text`: the first 4 of its 20 bytes, in
/// seconds since [`KSUID_EPOCH_S`]. None when `text` is not base62 or exceeds 20 bytes.
fn ksuid_seconds(text: &str) -> Option<u64> {
    // Big-endian, one byte more than a KSUID to see an overflow.
    let mut bytes = [0_u8; 21];
    for character in text.bytes() {
        let mut carry = u32::try_from(BASE62.iter().position(|&c| c == character)?).ok()?;
        for byte in bytes.iter_mut().rev() {
            let value = u32::from(*byte) * 62 + carry;
            let [.., low] = value.to_be_bytes();
            *byte = low;
            carry = value >> 8;
        }
    }
    let [0, a, b, c, d, ..] = bytes else {
        return None;
    };
    Some(u64::from(u32::from_be_bytes([a, b, c, d])))
}

/// An ISO 8601 UTC timestamp with whole seconds or milliseconds becomes `<timestamp>`.
fn replace_timestamp(value: &mut Value, _: &mut Placeholders) {
    if value.as_str().is_some_and(is_utc_timestamp) {
        *value = json!("<timestamp>");
    }
}

/// Whether `text` is `YYYY-MM-DDTHH:MM:SS` with an optional `.mmm`, then `Z`.
fn is_utc_timestamp(text: &str) -> bool {
    let shape = match text.len() {
        20 => "dddd-dd-ddTdd:dd:ddZ",
        24 => "dddd-dd-ddTdd:dd:dd.dddZ",
        _ => return false,
    };
    text.bytes().zip(shape.bytes()).all(|(byte, want)| {
        if want == b'd' {
            byte.is_ascii_digit()
        } else {
            byte == want
        }
    })
}

/// A non-negative integer of milliseconds becomes `<duration>`.
fn replace_duration(value: &mut Value, _: &mut Placeholders) {
    if value.is_u64() {
        *value = json!("<duration>");
    }
}

/// An object gets its keys in sorted order.
fn sort_keys(value: &mut Value, _: &mut Placeholders) {
    if let Value::Object(fields) = value {
        let mut sorted: Vec<(String, Value)> = std::mem::take(fields).into_iter().collect();
        sorted.sort_by(|(a, _), (b, _)| a.cmp(b));
        *fields = sorted.into_iter().collect::<Map<String, Value>>();
    }
}

/// A string becomes `<http-session#n>`.
fn replace_http_session(value: &mut Value, placeholders: &mut Placeholders) {
    if let Value::String(id) = value {
        *value = Value::String(placeholders.of("http-session", id));
    }
}

/// In a `host:port` or a URL, the port becomes `<port>`.
fn replace_port(value: &mut Value, _: &mut Placeholders) {
    let Value::String(text) = value else { return };
    let start = text.find("://").map_or(0, |scheme| scheme + 3);
    let end = text[start..]
        .find(['/', '?', '#'])
        .map_or(text.len(), |end| start + end);
    let Some(colon) = text[start..end].rfind(':').map(|colon| start + colon) else {
        return;
    };
    let port = &text[colon + 1..end];
    if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) {
        text.replace_range(colon + 1..end, "<port>");
    }
}

/// `value` as JSON text with the keys of every object sorted.
fn canonical(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", items.join(","))
        }
        Value::Object(fields) => {
            let mut fields: Vec<(&String, &Value)> = fields.iter().collect();
            fields.sort_by_key(|(key, _)| *key);
            let fields: Vec<String> = fields
                .into_iter()
                .map(|(key, value)| format!("{}:{}", Value::from(key.as_str()), canonical(value)))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        other => other.to_string(),
    }
}

#[test]
fn rules_list_each_path_once() {
    let mut paths = std::collections::HashSet::new();
    for rule in RULES {
        assert!(!rule.reason.is_empty(), "{} has no reason", rule.path);
        assert!(!parse(rule.path).is_empty(), "{} names no value", rule.path);
        assert!(paths.insert(rule.path), "{} is listed twice", rule.path);
    }
}

#[test]
fn minted_ids_get_placeholders_and_derived_ones_stay() {
    let minted = "ses_3LN9sNQXZmvwvVJXsJUQym6w950";
    let derived = "ses_2amkZriNZjjGeN3vIElOkPlgm91";
    let glued = format!("x{minted}");
    let mut document = json!({
        "transcript": [format!("session_id: {minted}"), derived, glued, "evt_3LN9sLtcphiPbE9SCvZL3nlwfxr"],
        "events": [{"body": {"session_id": minted}}],
    });
    normalize(&mut document, &mut Placeholders::numbered());
    let expected = json!({
        "transcript": ["session_id: <ses#1>", derived, glued, "<evt#1>"],
        "events": [{"body": {"session_id": "<ses#1>"}}],
    });
    assert_eq!(document, expected);
}

#[test]
fn derived_ids_are_those_before_march_2024() {
    assert!(derived("ses_2d3ykafzYwGnkMqnex3Bf5k5Uyd"));
    assert!(!derived("ses_2d3ykafzYwGnkMqnex3Bf5k5Uye"));
    assert!(!derived("ses_zzzzzzzzzzzzzzzzzzzzzzzzzzz"));
}

#[test]
fn clock_and_port_values_get_placeholders() {
    let mut document = json!({"events": [{"body": {
        "timestamp": "2026-10-29T14:24:23.948Z",
        "duration": 3,
        "tags": {"team": "core", "agentcat_session_id_source": "minted"},
        "parameters": {"extra": {
            "sessionId": "eecb4dfc",
            "requestInfo": {
                "headers": {"host": "127.0.0.1:45449", "mcp-session-id": "eecb4dfc"},
                "url": "http://127.0.0.1:45449/mcp",
            },
        }},
    }}, {"body": {"timestamp": "2026-10-29 14:24:23", "duration": -1}}]});
    normalize(&mut document, &mut Placeholders::numbered());
    let expected = json!({"events": [{"body": {
        "timestamp": "<timestamp>",
        "duration": "<duration>",
        "tags": {"agentcat_session_id_source": "minted", "team": "core"},
        "parameters": {"extra": {
            "sessionId": "<http-session#1>",
            "requestInfo": {
                "headers": {"host": "127.0.0.1:<port>", "mcp-session-id": "<http-session#1>"},
                "url": "http://127.0.0.1:<port>/mcp",
            },
        }},
    }}, {"body": {"timestamp": "2026-10-29 14:24:23", "duration": -1}}]});
    assert_eq!(document, expected);
    let tags: Vec<&String> = document["events"][0]["body"]["tags"]
        .as_object()
        .map(|tags| tags.keys().collect())
        .unwrap_or_default();
    assert_eq!(tags, ["agentcat_session_id_source", "team"]);
}
