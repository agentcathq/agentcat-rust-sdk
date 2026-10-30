//! The SDK options of the tracked server, built from the scenario's options profile: flags, and
//! hooks with canned behaviors.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use agentcat_sdk::{Event, HookError, Options, ToolCall, UserIdentity};
use serde::Deserialize;
use serde_json::{Map, Value};

/// The text that `redact_sensitive_information`'s `replace` puts in place of the secret.
const REDACTED: &str = "[REDACTED]";

/// The SDK options of the options profile `profile` (the scenario's `options`). Each key is
/// applied with its [`Options`] setter:
///
/// - Flags: `enable_tracing`, `enable_report_missing`, `enable_tool_call_context` and
///   `enable_agent_tracking` (booleans), `custom_context_description` (a string).
/// - Hooks, each a canned behavior (see [`behavior`]): `resolve_session_id` (`returns` a string or
///   `null`), `identify` (`returns` `null` or `{"userId", "userName"?, "userData"?}`),
///   `event_tags` and `event_properties` (`returns` an object or `null`),
///   `redact_sensitive_information` (`replace` a secret with `[REDACTED]`, or `returns` a string)
///   and `redact_event` (`returns: null` drops the event; `merge` sets the event fields it names:
///   `resourceName`, `sessionId`, `eventType`, `userIntent`, `properties`). Any hook also takes
///   `throws`, an error with that message.
///
/// # Panics
///
/// When the profile has an unknown key or a value of the wrong shape.
pub(crate) fn options(profile: &Map<String, Value>) -> Options {
    let mut options = Options::default();
    for (key, value) in profile {
        options = match key.as_str() {
            "enable_tracing" => options.enable_tracing(flag(key, value)),
            "enable_report_missing" => options.enable_report_missing(flag(key, value)),
            "enable_tool_call_context" => options.enable_tool_call_context(flag(key, value)),
            "enable_agent_tracking" => options.enable_agent_tracking(flag(key, value)),
            "custom_context_description" => options.custom_context_description(
                value
                    .as_str()
                    .unwrap_or_else(|| panic!("{key}: must be a string")),
            ),
            "resolve_session_id" => options.resolve_session_id(hook(key, value, |id: String| id)),
            "identify" => options.identify(hook(key, value, Identity::user)),
            "event_tags" => options.event_tags(hook(key, value, |tags: Map<String, Value>| tags)),
            "event_properties" => {
                options.event_properties(hook(key, value, |properties: Map<String, Value>| {
                    properties
                }))
            }
            "redact_sensitive_information" => {
                options.redact_sensitive_information(redactor(key, value))
            }
            "redact_event" => options.redact_event(event_redactor(key, value)),
            other => panic!("the options profile has the unknown key {other:?}"),
        };
    }
    options
}

/// A boolean flag of the profile.
fn flag(key: &str, value: &Value) -> bool {
    value
        .as_bool()
        .unwrap_or_else(|| panic!("{key}: must be a boolean"))
}

/// A canned behavior: what the hook does, after it sleeps `sleep_ms` milliseconds when given.
struct Behavior {
    kind: Kind,
    sleep: Option<Duration>,
}

/// What a behavior does.
enum Kind {
    /// `{"returns": <value>}`: answers a fresh copy of the value.
    Returns(Value),
    /// `{"throws": "<message>"}`: fails with the message.
    Throws(String),
    /// `{"replace": "<secret>"}` (`redact_sensitive_information`): replaces each occurrence of the
    /// secret with [`REDACTED`].
    Replace(String),
    /// `{"merge": {...}}` (`redact_event`): sets the event fields it names.
    Merge(Merge),
}

/// The behavior `spec` of the hook `hook`: an object with exactly one of `returns`, `throws`,
/// `replace` and `merge`, and an optional `sleep_ms`.
///
/// # Panics
///
/// When `spec` is not such an object.
fn behavior(hook: &str, spec: &Value) -> Behavior {
    let fields = spec
        .as_object()
        .unwrap_or_else(|| panic!("{hook}: a behavior must be an object"));
    let mut kind = None;
    let mut sleep = None;
    for (key, value) in fields {
        let next = match key.as_str() {
            "sleep_ms" => {
                let ms = value
                    .as_u64()
                    .unwrap_or_else(|| panic!("{hook}: sleep_ms must be a non-negative integer"));
                sleep = Some(Duration::from_millis(ms));
                continue;
            }
            "returns" => Kind::Returns(value.clone()),
            "throws" => Kind::Throws(string(hook, key, value)),
            "replace" => Kind::Replace(string(hook, key, value)),
            "merge" => Kind::Merge(
                serde_json::from_value(value.clone())
                    .unwrap_or_else(|error| panic!("{hook}: merge: {error}")),
            ),
            other => panic!("{hook}: unknown key {other:?}"),
        };
        assert!(
            kind.replace(next).is_none(),
            "{hook}: a behavior has exactly one of returns, throws, replace and merge"
        );
    }
    let kind = kind
        .unwrap_or_else(|| panic!("{hook}: a behavior needs returns, throws, replace or merge"));
    Behavior { kind, sleep }
}

/// The string value of `key` in the behavior of `hook`.
fn string(hook: &str, key: &str, value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("{hook}: {key} must be a string"))
        .to_owned()
}

/// Sleeps `sleep`, when given.
async fn pause(sleep: Option<Duration>) {
    if let Some(duration) = sleep {
        tokio::time::sleep(duration).await;
    }
}

/// The future of one call of a hook.
type HookFuture<T> = Pin<Box<dyn Future<Output = Result<T, HookError>> + Send>>;

/// The SDK hook of the behavior `spec` of the hook `name`, which takes `returns` (a value that reads as
/// `T`, or `null` for none, then goes through `convert`) or `throws`.
///
/// # Panics
///
/// When the behavior is malformed, or its `returns` value does not read as `T`.
fn hook<R: serde::de::DeserializeOwned, T: Clone + Send + Sync + 'static>(
    name: &str,
    spec: &Value,
    convert: fn(R) -> T,
) -> impl Fn(ToolCall) -> HookFuture<Option<T>> + Send + Sync + use<R, T> {
    let Behavior { kind, sleep } = behavior(name, spec);
    let answer: Result<Option<T>, String> = match kind {
        Kind::Returns(value) => Ok(serde_json::from_value::<Option<R>>(value)
            .unwrap_or_else(|error| panic!("{name}: returns: {error}"))
            .map(convert)),
        Kind::Throws(message) => Err(message),
        Kind::Replace(_) | Kind::Merge(_) => panic!("{name}: takes returns or throws"),
    };
    move |_call: ToolCall| {
        let answer = answer.clone();
        Box::pin(async move {
            pause(sleep).await;
            answer.map_err(HookError::from)
        })
    }
}

/// An `identify` answer: `{"userId", "userName"?, "userData"?}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    #[serde(rename = "userId")]
    id: String,
    #[serde(rename = "userName")]
    name: Option<String>,
    #[serde(rename = "userData")]
    data: Option<Map<String, Value>>,
}

impl Identity {
    /// The SDK's identity.
    fn user(self) -> UserIdentity {
        let mut user = UserIdentity::new(self.id);
        if let Some(name) = self.name {
            user = user.user_name(name);
        }
        if let Some(data) = self.data {
            user = user.user_data(data);
        }
        user
    }
}

/// A text redaction.
type Redact = Box<dyn Fn(&str) -> String + Send + Sync>;

/// The string redaction hook of the behavior `spec`: after its sleep, the text with each
/// occurrence of the secret replaced (`replace`), the returned string (`returns`) or an error
/// (`throws`).
///
/// # Panics
///
/// When the behavior is malformed.
fn redactor(
    key: &str,
    spec: &Value,
) -> impl Fn(String) -> HookFuture<String> + Send + Sync + use<> {
    let Behavior { kind, sleep } = behavior(key, spec);
    let answer: Result<Redact, String> = match kind {
        Kind::Replace(secret) => Ok(Box::new(move |text| text.replace(&secret, REDACTED))),
        Kind::Returns(Value::String(fixed)) => Ok(Box::new(move |_text| fixed.clone())),
        Kind::Throws(message) => Err(message),
        Kind::Returns(_) | Kind::Merge(_) => {
            panic!("{key}: takes replace, throws or returns with a string")
        }
    };
    let answer = Arc::new(answer);
    move |text: String| {
        let answer = Arc::clone(&answer);
        Box::pin(async move {
            pause(sleep).await;
            match answer.as_ref() {
                Ok(redact) => Ok(redact(&text)),
                Err(message) => Err(HookError::from(message.clone())),
            }
        })
    }
}

/// The event fields a `redact_event` merge sets; `None` leaves a field as it is.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Merge {
    resource_name: Option<String>,
    session_id: Option<String>,
    event_type: Option<String>,
    user_intent: Option<String>,
    properties: Option<Map<String, Value>>,
}

impl Merge {
    /// Sets the fields of the merge on `event`.
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

/// The event redaction hook of the behavior `spec`: after its sleep, the event with the merge
/// applied (`merge`), no event (`returns: null`) or an error (`throws`).
///
/// # Panics
///
/// When the behavior is malformed.
fn event_redactor(
    key: &str,
    spec: &Value,
) -> impl Fn(Event) -> HookFuture<Option<Event>> + Send + Sync + use<> {
    let Behavior { kind, sleep } = behavior(key, spec);
    let answer: Result<Option<Merge>, String> = match kind {
        Kind::Returns(Value::Null) => Ok(None),
        Kind::Merge(merge) => Ok(Some(merge)),
        Kind::Throws(message) => Err(message),
        Kind::Returns(_) | Kind::Replace(_) => {
            panic!("{key}: takes merge, throws or returns with null")
        }
    };
    move |mut event: Event| {
        let answer = answer.clone();
        Box::pin(async move {
            pause(sleep).await;
            match answer {
                Ok(Some(merge)) => {
                    merge.apply(&mut event);
                    Ok(Some(event))
                }
                Ok(None) => Ok(None),
                Err(message) => Err(HookError::from(message)),
            }
        })
    }
}
