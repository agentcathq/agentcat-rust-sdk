//! The publish body schema of the `AgentCat` API (`publish-event.schema.json`, the request body of
//! `POST /v1/public/events/`) and a strict validator for it.
//!
//! The validator knows exactly the keywords that the schema uses: `type` (a name or a list of
//! names), `properties`, `required`, `additionalProperties` (a boolean or a schema), `enum` and
//! `format` (`date-time` per RFC 3339, `ipv4`), and the annotations `$schema`, `$comment`,
//! `description` and `examples`, which it skips. [`Schema::new`] panics on any other keyword, type
//! name or format, so a refreshed schema that needs more fails loudly instead of passing a body
//! unchecked. Every keyword applies on its own, as in JSON Schema: `format` checks strings only,
//! and `properties`, `required` and `additionalProperties` check objects only.
//!
//! An error names the JSON path of the value (as the golden comparison writes it: `$`, `.key`, or
//! `["key"]`), the keyword and the value.
use std::sync::OnceLock;

use chrono::DateTime;
use serde_json::{Map, Value, json};

use super::golden::{field_path, shown};
use super::mock_api::Publish;

/// The keywords the validator checks.
const KEYWORDS: [&str; 6] = [
    "type",
    "properties",
    "required",
    "additionalProperties",
    "enum",
    "format",
];

/// The annotations the validator skips.
const ANNOTATIONS: [&str; 4] = ["$schema", "$comment", "description", "examples"];

/// The type names of JSON Schema.
const TYPES: [&str; 7] = [
    "null", "boolean", "object", "array", "number", "string", "integer",
];

/// The formats the validator checks.
const FORMATS: [&str; 2] = ["date-time", "ipv4"];

/// A schema whose keywords the validator knows.
pub(crate) struct Schema {
    root: Map<String, Value>,
}

impl Schema {
    /// Checks every keyword, type name and format of `schema`.
    ///
    /// # Panics
    ///
    /// When `schema` holds a keyword, a type name or a format that the validator does not know,
    /// or a keyword whose value does not have the expected shape: the message names it and its
    /// place in the schema.
    pub(crate) fn new(schema: Value) -> Self {
        let Value::Object(root) = schema else {
            panic!("the schema is not an object: {}", shown(&schema));
        };
        check_keywords(&root, "#");
        Self { root }
    }

    /// The errors of `instance`, one line each; empty when `instance` is valid.
    pub(crate) fn errors(&self, instance: &Value) -> Vec<String> {
        let mut errors = Vec::new();
        validate(&self.root, instance, "$", &mut errors);
        errors
    }
}

/// The schema of a publish body, read once from `publish-event.schema.json`.
///
/// # Panics
///
/// When the file cannot be read, is not JSON, or does not pass [`Schema::new`].
pub(crate) fn publish_event() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/e2e/publish-event.schema.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let schema = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()));
        Schema::new(schema)
    })
}

/// Checks the body of every publish of the run `run` against [`publish_event`].
///
/// # Panics
///
/// When a body is not valid: the message lists each error of each invalid body, with the
/// publish's place in arrival order, its `event_type` and its `resource_name`.
pub(crate) fn assert_valid_bodies(publishes: &[Publish], run: &str) {
    let schema = publish_event();
    let mut lines = Vec::new();
    for (index, publish) in publishes.iter().enumerate() {
        let field = |key: &str| publish.body.get(key).unwrap_or(&Value::Null).to_string();
        for error in schema.errors(&publish.body) {
            lines.push(format!(
                "publish {index} ({} {}): {error}",
                field("event_type"),
                field("resource_name")
            ));
        }
    }
    assert!(
        lines.is_empty(),
        "publish bodies of {run} do not match the API schema (tests/e2e/publish-event.schema.json):\n{}",
        lines.join("\n")
    );
}

/// Checks the keywords of the schema object `schema` at `at` (a JSON pointer into the schema)
/// and of every schema under it.
fn check_keywords(schema: &Map<String, Value>, at: &str) {
    for (keyword, value) in schema {
        let here = format!("{at}/{keyword}");
        match keyword.as_str() {
            "type" => {
                let names = match value {
                    Value::String(name) => vec![name.as_str()],
                    Value::Array(names) if !names.is_empty() => names
                        .iter()
                        .map(|name| {
                            name.as_str()
                                .unwrap_or_else(|| panic!("{here}: a type name is not a string"))
                        })
                        .collect(),
                    _ => panic!("{here}: `type` is not a name or a list of names"),
                };
                for name in names {
                    assert!(TYPES.contains(&name), "unsupported type `{name}` at {here}");
                }
            }
            "properties" => {
                let Value::Object(properties) = value else {
                    panic!("{here}: `properties` is not an object");
                };
                for (name, property) in properties {
                    let Value::Object(property) = property else {
                        panic!("{here}/{name}: a property schema is not an object");
                    };
                    check_keywords(property, &format!("{here}/{name}"));
                }
            }
            "required" => {
                let names_only = value
                    .as_array()
                    .is_some_and(|names| names.iter().all(Value::is_string));
                assert!(names_only, "{here}: `required` is not a list of names");
            }
            "additionalProperties" => match value {
                Value::Bool(_) => {}
                Value::Object(schema) => check_keywords(schema, &here),
                _ => panic!("{here}: `additionalProperties` is not a boolean or a schema"),
            },
            "enum" => assert!(value.is_array(), "{here}: `enum` is not a list"),
            "format" => {
                let Value::String(format) = value else {
                    panic!("{here}: `format` is not a name");
                };
                assert!(
                    FORMATS.contains(&format.as_str()),
                    "unsupported format `{format}` at {here}"
                );
            }
            other => assert!(
                ANNOTATIONS.contains(&other),
                "unsupported keyword `{other}` at {here} (the validator knows {KEYWORDS:?} and \
                 skips {ANNOTATIONS:?})"
            ),
        }
    }
}

/// Adds to `errors` the errors of `instance`, at `path`, against the schema object `schema`.
fn validate(schema: &Map<String, Value>, instance: &Value, path: &str, errors: &mut Vec<String>) {
    if let Some(types) = schema.get("type") {
        let names: Vec<&str> = match types {
            Value::Array(names) => names.iter().filter_map(Value::as_str).collect(),
            other => other.as_str().into_iter().collect(),
        };
        if !names.iter().any(|name| has_type(instance, name)) {
            errors.push(format!(
                "{path}: type: {} expected, got {}",
                names.join(" or "),
                shown(instance)
            ));
        }
    }
    if let Some(Value::Array(allowed)) = schema.get("enum")
        && !allowed.contains(instance)
    {
        errors.push(format!(
            "{path}: enum: {} is not one of the {} allowed values",
            shown(instance),
            allowed.len()
        ));
    }
    if let (Some(Value::String(format)), Value::String(text)) = (schema.get("format"), instance)
        && !format_holds(format, text)
    {
        errors.push(format!(
            "{path}: format: {} is not a valid {format}",
            shown(instance)
        ));
    }
    let Value::Object(object) = instance else {
        return;
    };
    if let Some(Value::Array(required)) = schema.get("required") {
        for name in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(name) {
                errors.push(format!("{}: required: missing", field_path(path, name)));
            }
        }
    }
    let properties = schema.get("properties").and_then(Value::as_object);
    for (key, value) in object {
        let at = field_path(path, key);
        match properties.and_then(|properties| properties.get(key)) {
            Some(Value::Object(property)) => validate(property, value, &at, errors),
            Some(_) => {}
            None => match schema.get("additionalProperties") {
                Some(Value::Bool(false)) => errors.push(format!(
                    "{at}: additionalProperties: not allowed, got {}",
                    shown(value)
                )),
                Some(Value::Object(additional)) => validate(additional, value, &at, errors),
                _ => {}
            },
        }
    }
}

/// Whether `instance` has the JSON Schema type `name`. An integer is a number with no fraction,
/// so `1.0` is one.
fn has_type(instance: &Value, name: &str) -> bool {
    match (name, instance) {
        ("null", Value::Null)
        | ("boolean", Value::Bool(_))
        | ("object", Value::Object(_))
        | ("array", Value::Array(_))
        | ("number", Value::Number(_))
        | ("string", Value::String(_)) => true,
        ("integer", Value::Number(number)) => {
            number.is_i64()
                || number.is_u64()
                || number.as_f64().is_some_and(|float| float.fract() == 0.0)
        }
        _ => false,
    }
}

/// Whether `text` is a valid `format`: a `date-time` is an RFC 3339 date and time with its
/// offset, an `ipv4` is four decimal octets with no leading zero.
fn format_holds(format: &str, text: &str) -> bool {
    match format {
        "date-time" => DateTime::parse_from_rfc3339(text).is_ok(),
        "ipv4" => text.parse::<std::net::Ipv4Addr>().is_ok(),
        other => panic!("unsupported format `{other}`"),
    }
}

/// A body that the schema accepts: the schema's own example.
fn example_body() -> Value {
    publish_event()
        .root
        .get("examples")
        .and_then(|examples| examples.get(0))
        .cloned()
        .expect("the schema has an example")
}

/// The errors of `example_body` with `key` set to `value`.
fn errors_with(key: &str, value: Value) -> Vec<String> {
    let mut body = example_body();
    body[key] = value;
    publish_event().errors(&body)
}

#[test]
fn the_schema_example_is_a_valid_body() {
    assert_eq!(
        publish_event().errors(&example_body()),
        Vec::<String>::new()
    );
    let mut stateless = example_body();
    stateless["session_id"] = Value::Null;
    stateless["duration"] = json!(150.0);
    assert_eq!(publish_event().errors(&stateless), Vec::<String>::new());
}

#[test]
fn a_wrong_type_fails_with_its_path() {
    assert_eq!(
        errors_with("duration", json!("150")),
        ["$.duration: type: integer expected, got \"150\""]
    );
    assert_eq!(
        errors_with("duration", json!(1.5)),
        ["$.duration: type: integer expected, got 1.5"]
    );
    assert_eq!(
        errors_with("session_id", json!(7)),
        ["$.session_id: type: string or null expected, got 7"]
    );
    assert_eq!(
        errors_with("tags", json!({"env": "prod", "build": 42})),
        ["$.tags.build: type: string expected, got 42"]
    );
}

#[test]
fn a_value_outside_the_enum_fails() {
    assert_eq!(
        errors_with("event_type", json!("mcp:tools/run")),
        ["$.event_type: enum: \"mcp:tools/run\" is not one of the 19 allowed values"]
    );
}

#[test]
fn a_missing_project_id_fails() {
    let mut body = example_body();
    body.as_object_mut()
        .expect("the example is an object")
        .remove("project_id");
    assert_eq!(
        publish_event().errors(&body),
        ["$.project_id: required: missing"]
    );
}

#[test]
fn an_extra_property_fails_where_additional_properties_forbids_it() {
    let schema = Schema::new(json!({
        "type": "object",
        "properties": {"error": {"type": "object", "additionalProperties": false, "properties": {
            "message": {"type": "string"},
        }}},
    }));
    assert_eq!(
        schema.errors(&json!({"error": {"message": "boom", "stack": "at main"}, "extra": 1})),
        ["$.error.stack: additionalProperties: not allowed, got \"at main\""]
    );
    // The publish schema allows extra properties at the top level.
    assert_eq!(
        errors_with("io.modelcontextprotocol/extra", json!(1)),
        Vec::<String>::new()
    );
}

#[test]
fn a_bad_date_time_fails() {
    for (text, valid) in [
        ("2026-10-29T14:24:23.948Z", true),
        ("2026-10-29T14:24:23+02:00", true),
        ("2026-10-29T14:24:23", false),
        ("2026-02-30T14:24:23Z", false),
        ("2026-10-29", false),
        ("yesterday", false),
    ] {
        let errors = errors_with("timestamp", json!(text));
        if valid {
            assert_eq!(errors, Vec::<String>::new(), "{text}");
        } else {
            assert_eq!(
                errors,
                [format!(
                    "$.timestamp: format: \"{text}\" is not a valid date-time"
                )],
                "{text}"
            );
        }
    }
}

#[test]
fn a_bad_ipv4_fails() {
    for (text, valid) in [
        ("192.168.1.1", true),
        ("0.0.0.0", true),
        ("192.168.1", false),
        ("256.1.1.1", false),
        ("01.2.3.4", false),
        ("::1", false),
    ] {
        let errors = errors_with("ip_address", json!(text));
        if valid {
            assert_eq!(errors, Vec::<String>::new(), "{text}");
        } else {
            assert_eq!(
                errors,
                [format!(
                    "$.ip_address: format: \"{text}\" is not a valid ipv4"
                )],
                "{text}"
            );
        }
    }
}

#[test]
fn a_property_with_a_non_plain_key_is_quoted_in_the_path() {
    let schema = Schema::new(json!({
        "type": "object",
        "additionalProperties": {"type": "string"},
    }));
    assert_eq!(
        schema.errors(&json!({"io.example/key": 1})),
        ["$[\"io.example/key\"]: type: string expected, got 1"]
    );
}

#[test]
#[should_panic(expected = "unsupported keyword `minimum` at #/properties/duration/minimum")]
fn an_unknown_keyword_panics() {
    Schema::new(json!({
        "type": "object",
        "properties": {"duration": {"type": "integer", "minimum": 0}},
    }));
}

#[test]
#[should_panic(expected = "unsupported format `email` at #/properties/contact/format")]
fn an_unknown_format_panics() {
    Schema::new(json!({
        "properties": {"contact": {"type": "string", "format": "email"}},
    }));
}

#[test]
#[should_panic(expected = "unsupported type `int` at #/additionalProperties/type")]
fn an_unknown_type_name_panics() {
    Schema::new(json!({"additionalProperties": {"type": "int"}}));
}
