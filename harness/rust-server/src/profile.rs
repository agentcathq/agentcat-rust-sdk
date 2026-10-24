//! The options profile (`HARNESS_OPTIONS`, harness/README.md "Options profile") as the SDK's
//! [`Options`]: the twin of `buildOptions` and `hook` in `harness/src/servers/common.mjs`.
//!
//! - Flags: `enableTracing`, `enableReportMissing`, `enableToolCallContext` and
//!   `enableAgentTracking` (booleans), `customContextDescription` (a string), each passed to its
//!   [`Options`] setter.
//! - Hooks: `resolveSessionId`, `identify`, `eventTags` and `eventProperties` become the SDK hooks
//!   with the canned behavior; `redactSensitiveInformation` becomes the SDK's string redaction
//!   hook (`replace` replaces each occurrence of the secret with `[REDACTED]`); `redactEvent`
//!   becomes the SDK's event redaction hook (`{"returns": null}` drops the event, `merge` sets the
//!   fields it names).
//!
//! A behavior is `{"returns": <value>}` or `{"throws": "<message>"}` (`redactSensitiveInformation`
//! also takes `{"replace": "<secret>"}`, `redactEvent` also takes `{"merge": {<field>: <value>}}`),
//! plus an optional `"sleepMs": <n>`. Where ts-v2 copies a value as given, the SDK's types need a
//! fit: a flag must have its type, a `returns` value must be one the hook's type can hold (see
//! [`session_id`], [`identity`], [`object`]), and a merge field must have the type of its
//! [`Event`] field (see [`merge`]). Anything else is an error, and the server exits 2.
//!
//! [`Canary`] (`HARNESS_CANARY`) perturbs the options for the L3 canary runs.
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use agentcat_sdk::{Event, HookError, Options, ToolCall, UserIdentity};
use serde_json::{Map, Value};

/// The context description of the perturbed configuration (`HARNESS_CANARY=1`), which the L3
/// canary run must catch.
const CANARY_DESCRIPTION: &str = "canary";
/// The resource name the event canary (`HARNESS_CANARY=event`) gives every event its hook sees.
const CANARY_RESOURCE: &str = "canary";

/// The kinds of behavior every hook takes.
const RETURNS_THROWS: &[&str] = &["returns", "throws"];
/// The kinds of behavior `redactSensitiveInformation` takes.
const RETURNS_THROWS_REPLACE: &[&str] = &["returns", "throws", "replace"];
/// The kinds of behavior `redactEvent` takes.
const RETURNS_THROWS_MERGE: &[&str] = &["returns", "throws", "merge"];
/// The name of the event redaction hook in the profile.
const REDACT_EVENT: &str = "redactEvent";
/// The fields a `redactEvent` merge may set, as `MERGE_FIELDS` in `common.mjs`.
const MERGE_FIELDS: &[&str] = &[
    "resourceName",
    "sessionId",
    "eventType",
    "userIntent",
    "properties",
];

/// The perturbation of an L3 canary run (`HARNESS_CANARY`, harness/README.md "L3 gate"): a
/// change the matrix must catch against an unperturbed Rust server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Canary {
    /// Unset or empty: no perturbation.
    None,
    /// `1`: the context description is [`CANARY_DESCRIPTION`], whatever the profile says.
    Context,
    /// `call`: an outer handler changes the issued mint-back text of each tool result (see
    /// `main.rs`); the options are the profile's.
    Call,
    /// `event`: a `redact_event` hook, in place of the profile's, sets the resource name of every
    /// event it sees to [`CANARY_RESOURCE`].
    Event,
}

impl Canary {
    /// The canary of the value of `HARNESS_CANARY`. The error is the line for stderr, without the
    /// server's name.
    pub(crate) fn from_env(value: Option<&str>) -> Result<Self, String> {
        match value {
            None | Some("") => Ok(Self::None),
            Some("1") => Ok(Self::Context),
            Some("call") => Ok(Self::Call),
            Some("event") => Ok(Self::Event),
            Some(_) => Err("HARNESS_CANARY: must be 1, call or event".to_owned()),
        }
    }
}

/// A checked profile.
#[derive(Debug)]
pub(crate) struct Profile {
    /// The SDK options the profile sets.
    pub(crate) options: Options,
}

/// A canned behavior: what the hook answers, after it sleeps.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Behavior {
    kind: Kind,
    sleep: Option<Duration>,
}

/// What a behavior answers.
#[derive(Clone, Debug, PartialEq)]
enum Kind {
    /// A fresh copy of the value.
    Returns(Value),
    /// An error with the message.
    Throws(String),
    /// The text with each occurrence of the secret replaced by `[REDACTED]`.
    Replace(String),
}

/// A checked `redactEvent` behavior: what the hook does with the event, after it sleeps.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EventBehavior {
    kind: EventKind,
    sleep: Option<Duration>,
}

/// What a `redactEvent` behavior does with the event.
#[derive(Clone, Debug, PartialEq)]
enum EventKind {
    /// `{"returns": null}`: drops the event.
    Drop,
    /// An error with the message.
    Throws(String),
    /// The event with the fields of the merge set.
    Merge(Merge),
}

/// The fields a `redactEvent` merge sets; `None` leaves the field as it is.
#[derive(Clone, Debug, Default, PartialEq)]
struct Merge {
    resource_name: Option<String>,
    session_id: Option<String>,
    event_type: Option<String>,
    user_intent: Option<String>,
    properties: Option<Map<String, Value>>,
}

impl Merge {
    /// `event` with the fields of the merge set, as `{...event, ...merge}` in `common.mjs`.
    fn apply(&self, event: &mut Event) {
        if let Some(name) = &self.resource_name {
            event.resource_name = Some(name.clone());
        }
        if let Some(id) = &self.session_id {
            event.session_id = Some(id.clone());
        }
        if let Some(event_type) = &self.event_type {
            event.event_type.clone_from(event_type);
        }
        if let Some(intent) = &self.user_intent {
            event.user_intent = Some(Value::String(intent.clone()));
        }
        if let Some(properties) = &self.properties {
            event.properties = Some(properties.clone());
        }
    }
}

/// The future of one call of the event redaction hook.
type EventFuture = Pin<Box<dyn Future<Output = Result<Option<Event>, HookError>> + Send>>;

/// A `returns` value that the hook's SDK type cannot hold.
struct Unfit;

/// A hook's answer, as its SDK type, and its sleep.
struct Canned<T> {
    answer: Result<Option<T>, String>,
    sleep: Option<Duration>,
}

/// The future of one call of a canned hook.
type CannedFuture<T> = Pin<Box<dyn Future<Output = Result<Option<T>, HookError>> + Send>>;

/// Reads `text` (the value of `HARNESS_OPTIONS`), then applies `canary` (see [`Canary`]). The
/// error is the line after `HARNESS_OPTIONS: `.
pub(crate) fn parse(text: &str, canary: Canary) -> Result<Profile, String> {
    let profile: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let Value::Object(entries) = profile else {
        return Err("must be a JSON object".to_owned());
    };
    let mut options = Options::default();
    for (key, value) in &entries {
        options = match key.as_str() {
            "enableTracing" => options.enable_tracing(flag(key, value)?),
            "enableReportMissing" => options.enable_report_missing(flag(key, value)?),
            "enableToolCallContext" => options.enable_tool_call_context(flag(key, value)?),
            "enableAgentTracking" => options.enable_agent_tracking(flag(key, value)?),
            "customContextDescription" => {
                let Some(description) = value.as_str() else {
                    return Err(format!("{key}: must be a string"));
                };
                options.custom_context_description(description)
            }
            "resolveSessionId" => {
                options.resolve_session_id(hook(key, value, session_id, "a string or null")?)
            }
            "identify" => options.identify(hook(
                key,
                value,
                identity,
                r#"null or {"userId": string, "userName"?: string, "userData"?: object}"#,
            )?),
            "eventTags" => options.event_tags(hook(key, value, object, "an object or null")?),
            "eventProperties" => {
                options.event_properties(hook(key, value, object, "an object or null")?)
            }
            "redactSensitiveInformation" => {
                let behavior = behavior(key, value, RETURNS_THROWS_REPLACE)?;
                if matches!(&behavior.kind, Kind::Returns(returned) if !returned.is_string()) {
                    return Err(format!("{key}: \"returns\" must be a string"));
                }
                options.redact_sensitive_information(redactor(behavior))
            }
            REDACT_EVENT => options.redact_event(event_redactor(event_behavior(value)?)),
            _ => return Err(format!("unknown key {}", Value::from(key.as_str()))),
        };
    }
    match canary {
        Canary::None | Canary::Call => {}
        Canary::Context => options = options.custom_context_description(CANARY_DESCRIPTION),
        Canary::Event => options = options.redact_event(rename_to_canary),
    }
    Ok(Profile { options })
}

/// The event canary's hook: the event with the resource name [`CANARY_RESOURCE`].
async fn rename_to_canary(mut event: Event) -> Result<Option<Event>, HookError> {
    event.resource_name = Some(CANARY_RESOURCE.to_owned());
    Ok(Some(event))
}

/// A boolean flag.
fn flag(key: &str, value: &Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| format!("{key}: must be a boolean"))
}

/// Checks the behavior `spec` of hook `name`, whose kinds are `kinds`.
fn behavior(name: &str, spec: &Value, kinds: &[&str]) -> Result<Behavior, String> {
    let (kind, sleep) = checked(name, spec, kinds, |kind, value| {
        Ok(match kind {
            "throws" => Kind::Throws(thrown(name, value)?),
            "replace" => Kind::Replace(
                value
                    .as_str()
                    .filter(|secret| !secret.is_empty())
                    .ok_or_else(|| format!("{name}: \"replace\" must be a non-empty string"))?
                    .to_owned(),
            ),
            // "returns"
            _ => Kind::Returns(value.clone()),
        })
    })?;
    Ok(Behavior { kind, sleep })
}

/// Checks the `redactEvent` behavior `spec`: `returns` must be `null`, and `merge` an object of
/// [`MERGE_FIELDS`], each with the type of its [`Event`] field (see [`merge`]).
fn event_behavior(spec: &Value) -> Result<EventBehavior, String> {
    let name = REDACT_EVENT;
    let (kind, sleep) = checked(name, spec, RETURNS_THROWS_MERGE, |kind, value| {
        Ok(match kind {
            "throws" => EventKind::Throws(thrown(name, value)?),
            "merge" => EventKind::Merge(merge(value)?),
            // "returns"
            _ if value.is_null() => EventKind::Drop,
            _ => return Err(format!("{name}: \"returns\" must be null")),
        })
    })?;
    Ok(EventBehavior { kind, sleep })
}

/// The fields of a `redactEvent` merge. ts-v2 checks only the field names; the SDK's [`Event`]
/// also needs a string for each text field and an object for `properties`.
fn merge(value: &Value) -> Result<Merge, String> {
    let name = REDACT_EVENT;
    let Value::Object(fields) = value else {
        return Err(format!("{name}: \"merge\" must be an object"));
    };
    if let Some(field) = fields
        .keys()
        .find(|field| !MERGE_FIELDS.contains(&field.as_str()))
    {
        return Err(format!(
            "{name}: unknown merge field {} (use {})",
            Value::from(field.as_str()),
            MERGE_FIELDS.join(", ")
        ));
    }
    let text = |field: &str| match fields.get(field) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!(
            "{name}: merge field {} must be a string",
            Value::from(field)
        )),
    };
    let properties = match fields.get("properties") {
        None => None,
        Some(Value::Object(properties)) => Some(properties.clone()),
        Some(_) => {
            return Err(format!(
                "{name}: merge field \"properties\" must be an object"
            ));
        }
    };
    Ok(Merge {
        resource_name: text("resourceName")?,
        session_id: text("sessionId")?,
        event_type: text("eventType")?,
        user_intent: text("userIntent")?,
        properties,
    })
}

/// The message of a `throws` behavior of hook `name`.
fn thrown(name: &str, value: &Value) -> Result<String, String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{name}: \"throws\" must be a string"))
}

/// Checks the shape of the behavior `spec` of hook `name`, whose kinds are `kinds`, in the order
/// of `hook` in `common.mjs`: an object, no unknown key, exactly one kind, the kind's value
/// (`read`, given the kind and its value), then `sleepMs`.
fn checked<K>(
    name: &str,
    spec: &Value,
    kinds: &[&str],
    read: impl FnOnce(&str, &Value) -> Result<K, String>,
) -> Result<(K, Option<Duration>), String> {
    let Value::Object(spec) = spec else {
        return Err(format!("{name}: a behavior must be an object"));
    };
    if let Some(key) = spec
        .keys()
        .find(|key| *key != "sleepMs" && !kinds.contains(&key.as_str()))
    {
        return Err(format!("{name}: unknown key {}", Value::from(key.as_str())));
    }
    let mut given = kinds
        .iter()
        .filter_map(|kind| spec.get(*kind).map(|value| (*kind, value)));
    let (Some((kind, value)), None) = (given.next(), given.next()) else {
        return Err(format!("{name}: needs exactly one of {}", kinds.join(", ")));
    };
    let kind = read(kind, value)?;
    let sleep = match spec.get("sleepMs") {
        None => None,
        Some(ms) => Some(Duration::from_millis(ms.as_u64().ok_or_else(|| {
            format!("{name}: \"sleepMs\" must be a non-negative integer")
        })?)),
    };
    Ok((kind, sleep))
}

/// The SDK hook for the behavior `spec` of hook `name`: `read` turns a `returns` value into the
/// hook's answer; a value that does not fit is an error that names `shape`.
fn hook<T: Clone + Send + Sync + 'static>(
    name: &str,
    spec: &Value,
    read: fn(&Value) -> Result<Option<T>, Unfit>,
    shape: &str,
) -> Result<impl Fn(ToolCall) -> CannedFuture<T> + Send + Sync + 'static, String> {
    let Behavior { kind, sleep } = behavior(name, spec, RETURNS_THROWS)?;
    let answer = match kind {
        Kind::Returns(value) => {
            Ok(read(&value).map_err(|Unfit| format!("{name}: \"returns\" must be {shape}"))?)
        }
        Kind::Throws(message) => Err(message),
        // `behavior` takes "replace" only from `RETURNS_THROWS_REPLACE`.
        Kind::Replace(_) => return Err(format!("{name}: unknown key \"replace\"")),
    };
    Ok(canned(Canned { answer, sleep }))
}

/// A hook that sleeps, then gives a fresh copy of the canned answer (an error for `throws`).
fn canned<T: Clone + Send + Sync + 'static>(
    canned: Canned<T>,
) -> impl Fn(ToolCall) -> CannedFuture<T> + Send + Sync + 'static {
    let Canned { answer, sleep } = canned;
    move |_call: ToolCall| {
        let answer = answer.clone();
        Box::pin(async move {
            if let Some(duration) = sleep {
                tokio::time::sleep(duration).await;
            }
            answer.map_err(HookError::from)
        })
    }
}

/// The string redaction hook of the behavior: after its sleep, the text with each occurrence of
/// the secret replaced (`replace`), the returned string (`returns`) or an error (`throws`).
fn redactor(
    behavior: Behavior,
) -> impl Fn(String) -> Pin<Box<dyn Future<Output = Result<String, HookError>> + Send>>
+ Send
+ Sync
+ 'static {
    let Behavior { kind, sleep } = behavior;
    move |text: String| {
        let kind = kind.clone();
        Box::pin(async move {
            if let Some(duration) = sleep {
                tokio::time::sleep(duration).await;
            }
            match kind {
                Kind::Replace(secret) => Ok(text.replace(&secret, "[REDACTED]")),
                Kind::Throws(message) => Err(HookError::from(message)),
                Kind::Returns(value) => Ok(value.as_str().unwrap_or_default().to_owned()),
            }
        })
    }
}

/// The event redaction hook of the behavior: after its sleep, the event with the merge applied
/// (`merge`), no event (`returns: null`) or an error (`throws`).
fn event_redactor(
    behavior: EventBehavior,
) -> impl Fn(Event) -> EventFuture + Send + Sync + 'static {
    let EventBehavior { kind, sleep } = behavior;
    move |mut event: Event| {
        let kind = kind.clone();
        Box::pin(async move {
            if let Some(duration) = sleep {
                tokio::time::sleep(duration).await;
            }
            match kind {
                EventKind::Drop => Ok(None),
                EventKind::Throws(message) => Err(HookError::from(message)),
                EventKind::Merge(merge) => {
                    merge.apply(&mut event);
                    Ok(Some(event))
                }
            }
        })
    }
}

/// A `resolveSessionId` answer: a string, or `null` for none.
fn session_id(value: &Value) -> Result<Option<String>, Unfit> {
    match value {
        Value::Null => Ok(None),
        Value::String(id) => Ok(Some(id.clone())),
        _ => Err(Unfit),
    }
}

/// An `identify` answer: `null` for none, or `{"userId", "userName"?, "userData"?}`.
fn identity(value: &Value) -> Result<Option<UserIdentity>, Unfit> {
    let fields = match value {
        Value::Null => return Ok(None),
        Value::Object(fields) => fields,
        _ => return Err(Unfit),
    };
    if fields
        .keys()
        .any(|key| !["userId", "userName", "userData"].contains(&key.as_str()))
    {
        return Err(Unfit);
    }
    let text = |key: &str| match fields.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(Unfit),
    };
    let mut user = UserIdentity::new(text("userId")?.ok_or(Unfit)?);
    if let Some(name) = text("userName")? {
        user = user.user_name(name);
    }
    if let Some(data) = fields.get("userData") {
        user = user.user_data(data.as_object().ok_or(Unfit)?.clone());
    }
    Ok(Some(user))
}

/// An `eventTags` or `eventProperties` answer: an object, or `null` for none.
fn object(value: &Value) -> Result<Option<Map<String, Value>>, Unfit> {
    match value {
        Value::Null => Ok(None),
        Value::Object(map) => Ok(Some(map.clone())),
        _ => Err(Unfit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(text: &str) -> String {
        parse(text, Canary::None).expect_err(text)
    }

    /// The options as their `Debug` text shows them: the one view of which hooks are set.
    fn options_of(text: &str, canary: Canary) -> String {
        format!("{:?}", parse(text, canary).expect(text).options)
    }

    #[test]
    fn the_basic_profile_parses_and_sets_the_redaction() {
        let options = options_of(
            r#"{"identify": {"returns": {"userId": "user-1", "userName": "User One", "userData": {"plan": "pro"}}},
                "eventTags": {"returns": {"team": "core"}}, "eventProperties": {"returns": {"region": "eu"}},
                "redactSensitiveInformation": {"replace": "SECRET_TOKEN"}, "enableTracing": false,
                "resolveSessionId": {"returns": null, "sleepMs": 5}, "customContextDescription": "why"}"#,
            Canary::None,
        );
        for set in [
            "tracing: false",
            r#"custom_context_description: "why""#,
            r#"resolve_session_id: Some("<hook>")"#,
            r#"identify: Some("<hook>")"#,
            r#"event_tags: Some("<hook>")"#,
            r#"event_properties: Some("<hook>")"#,
            r#"string_redactor: Some("<hook>")"#,
            "event_redactor: None",
        ] {
            assert!(options.contains(set), "{set}: {options}");
        }
        let empty = options_of("{}", Canary::None);
        for unset in ["identify: None", "string_redactor: None", "tracing: true"] {
            assert!(empty.contains(unset), "{unset}: {empty}");
        }
    }

    #[test]
    fn redact_event_profile_parses() {
        for spec in [
            r#"{"returns": null}"#,
            r#"{"throws": "no", "sleepMs": 1}"#,
            r#"{"merge": {"resourceName": "renamed", "sessionId": "merged", "eventType": "x", "userIntent": "why", "properties": {"a": 1}}}"#,
            r#"{"merge": {}}"#,
        ] {
            let options = options_of(&format!(r#"{{"redactEvent": {spec}}}"#), Canary::None);
            assert!(
                options.contains(r#"event_redactor: Some("<hook>")"#),
                "{spec}: {options}"
            );
        }
        let behavior = event_behavior(&serde_json::json!({"merge": {
            "resourceName": "renamed", "sessionId": "merged", "eventType": "x", "userIntent": "why",
            "properties": {"a": 1}
        }, "sleepMs": 2}))
        .expect("merge");
        assert_eq!(
            behavior,
            EventBehavior {
                kind: EventKind::Merge(Merge {
                    resource_name: Some("renamed".to_owned()),
                    session_id: Some("merged".to_owned()),
                    event_type: Some("x".to_owned()),
                    user_intent: Some("why".to_owned()),
                    properties: Some(Map::from_iter([("a".to_owned(), serde_json::json!(1))])),
                }),
                sleep: Some(Duration::from_millis(2)),
            }
        );
        assert_eq!(
            event_behavior(&serde_json::json!({"returns": null})).map(|b| b.kind),
            Ok(EventKind::Drop)
        );
    }

    #[test]
    fn unknown_merge_field_is_refused() {
        for (profile, message) in [
            (
                r#"{"redactEvent": {"merge": {"resourceName": "x", "tags": {}}}}"#,
                r#"redactEvent: unknown merge field "tags" (use resourceName, sessionId, eventType, userIntent, properties)"#,
            ),
            (
                r#"{"redactEvent": {"merge": {"resourceName": 1}}}"#,
                r#"redactEvent: merge field "resourceName" must be a string"#,
            ),
            (
                r#"{"redactEvent": {"merge": {"userIntent": null}}}"#,
                r#"redactEvent: merge field "userIntent" must be a string"#,
            ),
            (
                r#"{"redactEvent": {"merge": {"properties": []}}}"#,
                r#"redactEvent: merge field "properties" must be an object"#,
            ),
            (
                r#"{"redactEvent": {"merge": ["resourceName"]}}"#,
                r#"redactEvent: "merge" must be an object"#,
            ),
            (
                r#"{"redactEvent": {"returns": {}}}"#,
                r#"redactEvent: "returns" must be null"#,
            ),
            (
                r#"{"redactEvent": {"merge": {}, "returns": null}}"#,
                "redactEvent: needs exactly one of returns, throws, merge",
            ),
            (
                r#"{"redactEvent": {"replace": "x"}}"#,
                r#"redactEvent: unknown key "replace""#,
            ),
            (
                r#"{"identify": {"merge": {}}}"#,
                r#"identify: unknown key "merge""#,
            ),
        ] {
            assert_eq!(error(profile), message, "{profile}");
        }
    }

    #[test]
    fn canary_values_are_one_call_or_event() {
        assert_eq!(Canary::from_env(None), Ok(Canary::None));
        assert_eq!(Canary::from_env(Some("")), Ok(Canary::None));
        assert_eq!(Canary::from_env(Some("1")), Ok(Canary::Context));
        assert_eq!(Canary::from_env(Some("call")), Ok(Canary::Call));
        assert_eq!(Canary::from_env(Some("event")), Ok(Canary::Event));
        for bad in ["2", "Call", " event", "calls"] {
            assert_eq!(
                Canary::from_env(Some(bad)),
                Err("HARNESS_CANARY: must be 1, call or event".to_owned()),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_context_and_event_canaries_replace_the_profile() {
        let context = options_of(r#"{"customContextDescription": "why"}"#, Canary::Context);
        assert!(
            context.contains(r#"custom_context_description: "canary""#),
            "{context}"
        );
        let event = options_of("{}", Canary::Event);
        assert!(
            event.contains(r#"event_redactor: Some("<hook>")"#),
            "{event}"
        );
        // The call canary changes the handler, not the options.
        let call = options_of(r#"{"customContextDescription": "why"}"#, Canary::Call);
        assert!(
            call.contains(r#"custom_context_description: "why""#),
            "{call}"
        );
        assert!(call.contains("event_redactor: None"), "{call}");
    }

    #[tokio::test]
    async fn the_redactor_replaces_returns_or_throws() {
        let replace = behavior(
            "redactSensitiveInformation",
            &serde_json::json!({"replace": "SECRET"}),
            RETURNS_THROWS_REPLACE,
        )
        .expect("replace");
        let hook = redactor(replace);
        assert_eq!(
            hook("a SECRET b SECRET".to_owned()).await.ok().as_deref(),
            Some("a [REDACTED] b [REDACTED]")
        );
        let returns = behavior(
            "redactSensitiveInformation",
            &serde_json::json!({"returns": "fixed"}),
            RETURNS_THROWS_REPLACE,
        )
        .expect("returns");
        assert_eq!(
            redactor(returns)("x".to_owned()).await.ok().as_deref(),
            Some("fixed")
        );
        let throws = behavior(
            "redactSensitiveInformation",
            &serde_json::json!({"throws": "no"}),
            RETURNS_THROWS_REPLACE,
        )
        .expect("throws");
        assert!(redactor(throws)("x".to_owned()).await.is_err());
    }

    #[test]
    fn a_profile_that_is_not_an_object_or_has_an_unknown_key_is_refused() {
        assert_eq!(error("[]"), "must be a JSON object");
        assert_eq!(error(r#"{"nope": 1}"#), r#"unknown key "nope""#);
        assert!(error("{").contains("EOF"));
    }

    #[test]
    fn flags_need_their_type() {
        assert_eq!(
            error(r#"{"enableTracing": 0}"#),
            "enableTracing: must be a boolean"
        );
        assert_eq!(
            error(r#"{"customContextDescription": false}"#),
            "customContextDescription: must be a string"
        );
    }

    #[test]
    fn malformed_behaviors_are_refused() {
        for (profile, message) in [
            (
                r#"{"identify": 1}"#,
                "identify: a behavior must be an object",
            ),
            (
                r#"{"identify": {"returns": null, "throws": "x"}}"#,
                "identify: needs exactly one of returns, throws",
            ),
            (
                r#"{"identify": {"sleepMs": 1}}"#,
                "identify: needs exactly one of returns, throws",
            ),
            (
                r#"{"eventTags": {"replace": "x"}}"#,
                r#"eventTags: unknown key "replace""#,
            ),
            (
                r#"{"eventTags": {"throws": 1}}"#,
                r#"eventTags: "throws" must be a string"#,
            ),
            (
                r#"{"eventTags": {"returns": {}, "sleepMs": -1}}"#,
                r#"eventTags: "sleepMs" must be a non-negative integer"#,
            ),
            (
                r#"{"redactSensitiveInformation": {"replace": ""}}"#,
                r#"redactSensitiveInformation: "replace" must be a non-empty string"#,
            ),
            (
                r#"{"redactSensitiveInformation": {"returns": 1}}"#,
                r#"redactSensitiveInformation: "returns" must be a string"#,
            ),
        ] {
            assert_eq!(error(profile), message, "{profile}");
        }
    }

    #[test]
    fn a_returns_value_the_hook_type_cannot_hold_is_refused() {
        for (profile, message) in [
            (
                r#"{"resolveSessionId": {"returns": 5}}"#,
                r#"resolveSessionId: "returns" must be a string or null"#,
            ),
            (
                r#"{"identify": {"returns": {"userName": "x"}}}"#,
                r#"identify: "returns" must be null or {"userId": string, "userName"?: string, "userData"?: object}"#,
            ),
            (
                r#"{"identify": {"returns": {"userId": "u", "extra": 1}}}"#,
                r#"identify: "returns" must be null or {"userId": string, "userName"?: string, "userData"?: object}"#,
            ),
            (
                r#"{"eventProperties": {"returns": []}}"#,
                r#"eventProperties: "returns" must be an object or null"#,
            ),
        ] {
            assert_eq!(error(profile), message, "{profile}");
        }
    }
}
