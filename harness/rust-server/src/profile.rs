//! The options profile (`HARNESS_OPTIONS`, harness/README.md "Options profile") as the SDK's
//! [`Options`]: the twin of `buildOptions` and `hook` in `harness/src/servers/common.mjs`.
//!
//! - Flags: `enableTracing`, `enableReportMissing`, `enableToolCallContext` and
//!   `enableAgentTracking` (booleans), `customContextDescription` (a string), each passed to its
//!   [`Options`] setter.
//! - Hooks: `resolveSessionId`, `identify`, `eventTags` and `eventProperties` become the SDK hooks
//!   with the canned behavior; `redactSensitiveInformation` becomes the SDK's string redaction
//!   hook (`replace` replaces each occurrence of the secret with `[REDACTED]`).
//!
//! A behavior is `{"returns": <value>}` or `{"throws": "<message>"}` (`redactSensitiveInformation`
//! also takes `{"replace": "<secret>"}`), plus an optional `"sleepMs": <n>`. Where ts-v2 copies a
//! value as given, the SDK's types need a fit: a flag must have its type, and a `returns` value
//! must be one the hook's type can hold (see [`session_id`], [`identity`], [`object`]). Anything
//! else is an error, and the server exits 2.
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use agentcat_sdk::{HookError, Options, ToolCall, UserIdentity};
use serde_json::{Map, Value};

/// The context description of the perturbed configuration (`HARNESS_CANARY=1`), which the L3
/// canary run must catch.
const CANARY_DESCRIPTION: &str = "canary";

/// The kinds of behavior every hook takes.
const RETURNS_THROWS: &[&str] = &["returns", "throws"];
/// The kinds of behavior `redactSensitiveInformation` takes.
const RETURNS_THROWS_REPLACE: &[&str] = &["returns", "throws", "replace"];

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

/// A `returns` value that the hook's SDK type cannot hold.
struct Unfit;

/// A hook's answer, as its SDK type, and its sleep.
struct Canned<T> {
    answer: Result<Option<T>, String>,
    sleep: Option<Duration>,
}

/// The future of one call of a canned hook.
type CannedFuture<T> = Pin<Box<dyn Future<Output = Result<Option<T>, HookError>> + Send>>;

/// Reads `text` (the value of `HARNESS_OPTIONS`). With `canary`, the context description is
/// [`CANARY_DESCRIPTION`] whatever the profile says. The error is the line after `HARNESS_OPTIONS: `.
pub(crate) fn parse(text: &str, canary: bool) -> Result<Profile, String> {
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
            _ => return Err(format!("unknown key {}", Value::from(key.as_str()))),
        };
    }
    if canary {
        options = options.custom_context_description(CANARY_DESCRIPTION);
    }
    Ok(Profile { options })
}

/// A boolean flag.
fn flag(key: &str, value: &Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| format!("{key}: must be a boolean"))
}

/// Checks the behavior `spec` of hook `name`, whose kinds are `kinds`.
fn behavior(name: &str, spec: &Value, kinds: &[&str]) -> Result<Behavior, String> {
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
    let kind = match kind {
        "throws" => Kind::Throws(
            value
                .as_str()
                .ok_or_else(|| format!("{name}: \"throws\" must be a string"))?
                .to_owned(),
        ),
        "replace" => Kind::Replace(
            value
                .as_str()
                .filter(|secret| !secret.is_empty())
                .ok_or_else(|| format!("{name}: \"replace\" must be a non-empty string"))?
                .to_owned(),
        ),
        // "returns"
        _ => Kind::Returns(value.clone()),
    };
    let sleep = match spec.get("sleepMs") {
        None => None,
        Some(ms) => Some(Duration::from_millis(ms.as_u64().ok_or_else(|| {
            format!("{name}: \"sleepMs\" must be a non-negative integer")
        })?)),
    };
    Ok(Behavior { kind, sleep })
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
        parse(text, false).expect_err(text)
    }

    #[test]
    fn the_basic_profile_parses_and_sets_the_redaction() {
        let profile = parse(
            r#"{"identify": {"returns": {"userId": "user-1", "userName": "User One", "userData": {"plan": "pro"}}},
                "eventTags": {"returns": {"team": "core"}}, "eventProperties": {"returns": {"region": "eu"}},
                "redactSensitiveInformation": {"replace": "SECRET_TOKEN"}, "enableTracing": true,
                "resolveSessionId": {"returns": null, "sleepMs": 5}, "customContextDescription": "why"}"#,
            false,
        )
        .expect("the profile parses");
        let _ = profile;
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
