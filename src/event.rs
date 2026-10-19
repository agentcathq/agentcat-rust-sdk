//! The event model.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One analytics event, as the `redact_event` hook sees it.
///
/// Tool calls publish `mcp:tools/call` events; `publish_custom_event`
/// publishes `agentcat:custom` events. Field names match the AgentCat API.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Event {
    /// `evt_` KSUID. Empty until the event leaves the redaction stage.
    pub id: String,
    /// `ses_` session ID; empty when the event has no session.
    pub session_id: String,
    /// AgentCat project ID; `None` in telemetry-only mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// `mcp:tools/call` or `agentcat:custom`.
    pub event_type: String,
    /// Request start time.
    pub timestamp: DateTime<Utc>,
    /// Duration in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<u64>,
    /// Estimated tokens the model spent on the call's arguments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    /// Estimated tokens of the result text the model reads back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(missing_docs)]
    pub ip_address: Option<String>,
    /// Always `"Rust"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdk_language: Option<String>,
    /// This SDK's version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agentcat_version: Option<String>,
    /// The MCP server's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    /// The MCP server's version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    /// The MCP client's name, resolved per request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    /// The MCP client's version, resolved per request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
    /// The actor's ID from `identify`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identify_actor_given_id: Option<String>,
    /// The actor's name from `identify`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identify_actor_name: Option<String>,
    /// The actor's extra data from `identify` (`{}` when anonymous).
    #[serde(rename = "identify_data", skip_serializing_if = "Option::is_none")]
    pub identify_actor_data: Option<Value>,
    /// The tool name, or the custom event's resource name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_name: Option<String>,
    /// For tool calls: `{ "request": <the raw tools/call request>, "extra": <request context> }`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    /// For tool calls: the customer's original result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Value>,
    /// The agent's stated intent (the injected `context` argument).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_intent: Option<String>,
    /// True when the call failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// Error details (see [`ErrorData`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    /// Indexed string tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<BTreeMap<String, String>>,
    /// Free-form JSON properties.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Map<String, Value>>,
    /// Legacy actor ID; the API also derives it from `identify_actor_given_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    /// Legacy custom event ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
}

/// Error details attached to an event, in the shape the AgentCat API groups.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ErrorData {
    /// The error message.
    pub message: String,
    /// A short type name without a module path, such as `ParseIntError`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    /// The raw backtrace text, when one was captured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    /// Parsed frames, innermost first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<StackFrame>>,
    /// The error's `source()` chain, outermost first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chained_errors: Option<Vec<ChainedErrorData>>,
    /// Always `"rust"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

/// One frame of a captured backtrace.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StackFrame {
    /// Normalized, relative file path.
    pub filename: String,
    /// Function name without the module path or hash suffix.
    pub function: String,
    /// Module path, such as `my_crate::tools`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[allow(missing_docs)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineno: Option<u32>,
    #[allow(missing_docs)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colno: Option<u32>,
    /// True for the customer's own code.
    pub in_app: bool,
    /// The original path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abs_path: Option<String>,
}

/// One error in a `source()` chain.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChainedErrorData {
    /// The error message.
    pub message: String,
    /// A short type name, when known.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
}

impl ErrorData {
    /// Error data with a message and no type.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            platform: Some(crate::constants::ERROR_PLATFORM.to_owned()),
            ..Self::default()
        }
    }

    /// Sets the short type name.
    pub fn with_type(mut self, error_type: impl Into<String>) -> Self {
        self.error_type = Some(error_type.into());
        self
    }

    /// Error data from any `std::error::Error`, with its `source()` chain
    /// (at most 10 levels). Use [`ErrorData::from_error_typed`] to record the
    /// concrete type name.
    pub fn from_error(error: &(dyn std::error::Error + 'static)) -> Self {
        let mut data = Self::new(error.to_string());
        data.chained_errors = chain_of(error);
        data
    }

    /// Like [`ErrorData::from_error`], and records `E`'s short type name.
    pub fn from_error_typed<E: std::error::Error + 'static>(error: &E) -> Self {
        Self::from_error(error).with_type(short_type_name::<E>())
    }

    pub(crate) fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

const MAX_EXCEPTION_CHAIN_DEPTH: usize = 10;

fn chain_of(error: &(dyn std::error::Error + 'static)) -> Option<Vec<ChainedErrorData>> {
    let mut out = Vec::new();
    let mut current = error.source();
    while let Some(err) = current {
        if out.len() >= MAX_EXCEPTION_CHAIN_DEPTH {
            break;
        }
        out.push(ChainedErrorData {
            message: err.to_string(),
            error_type: None,
        });
        current = err.source();
    }
    (!out.is_empty()).then_some(out)
}

/// The last path segment of a type name, generics included:
/// `std::num::ParseIntError` becomes `ParseIntError`.
pub(crate) fn short_type_name<T: ?Sized>() -> String {
    let full = std::any::type_name::<T>();
    let base = full.split('<').next().unwrap_or(full);
    let short = base.rsplit("::").next().unwrap_or(base);
    short.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Outer(Inner);
    #[derive(Debug)]
    struct Inner;
    impl std::fmt::Display for Outer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "outer failed")
        }
    }
    impl std::fmt::Display for Inner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "inner cause")
        }
    }
    impl std::error::Error for Outer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    impl std::error::Error for Inner {}

    #[test]
    fn error_data_from_error_chain() {
        let data = ErrorData::from_error_typed(&Outer(Inner));
        assert_eq!(data.message, "outer failed");
        assert_eq!(data.error_type.as_deref(), Some("Outer"));
        assert_eq!(data.platform.as_deref(), Some("rust"));
        let chain = data.chained_errors.unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].message, "inner cause");
    }

    #[test]
    fn short_type_names() {
        assert_eq!(short_type_name::<std::num::ParseIntError>(), "ParseIntError");
        assert_eq!(short_type_name::<Vec<String>>(), "Vec");
    }

    #[test]
    fn serializes_wire_keys() {
        let v = serde_json::to_value(ErrorData::new("m").with_type("T")).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"message": "m", "type": "T", "platform": "rust"})
        );
    }
}
