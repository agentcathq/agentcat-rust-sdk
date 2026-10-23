//! [`CustomEvent`]: an event the server publishes by itself, apart from tool calls (B-009).
use std::time::Duration;

use serde_json::{Map, Value};

#[cfg(test)]
mod tests;

/// The data of a custom event, as TypeScript's `CustomEventData`. Every field is optional; each
/// setter consumes the event and returns it changed.
///
/// ```
/// use std::time::Duration;
/// use agentcat_sdk::CustomEvent;
///
/// let event = CustomEvent::new()
///     .session_id("ses_1")
///     .message("cache warmed")
///     .duration(Duration::from_millis(40));
/// # let _ = event;
/// ```
#[derive(Clone, Debug, Default)]
pub struct CustomEvent {
    // Read by publish_custom_event (plan 4 task 7).
    pub(crate) session_id: Option<String>,
    pub(crate) resource_name: Option<String>,
    pub(crate) parameters: Option<Value>,
    pub(crate) response: Option<Value>,
    pub(crate) message: Option<String>,
    pub(crate) duration: Option<Duration>,
    pub(crate) is_error: Option<bool>,
    pub(crate) error: Option<Value>,
    pub(crate) tags: Option<Map<String, Value>>,
    pub(crate) properties: Option<Map<String, Value>>,
}

impl CustomEvent {
    /// An event with no data.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The session the event belongs to.
    #[must_use]
    pub fn session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }

    /// The name of the resource the event is about.
    #[must_use]
    pub fn resource_name(mut self, name: impl Into<String>) -> Self {
        self.resource_name = Some(name.into());
        self
    }

    /// The parameters of what happened.
    #[must_use]
    pub fn parameters(mut self, parameters: Value) -> Self {
        self.parameters = Some(parameters);
        self
    }

    /// The response of what happened.
    #[must_use]
    pub fn response(mut self, response: Value) -> Self {
        self.response = Some(response);
        self
    }

    /// A free text about the event.
    #[must_use]
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// How long it took.
    #[must_use]
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Whether the event is an error.
    #[must_use]
    pub fn is_error(mut self, is_error: bool) -> Self {
        self.is_error = Some(is_error);
        self
    }

    /// The error, when there is one.
    #[must_use]
    pub fn error(mut self, error: Value) -> Self {
        self.error = Some(error);
        self
    }

    /// Tags on the event.
    #[must_use]
    pub fn tags(mut self, tags: Map<String, Value>) -> Self {
        self.tags = Some(tags);
        self
    }

    /// Properties on the event.
    #[must_use]
    pub fn properties(mut self, properties: Map<String, Value>) -> Self {
        self.properties = Some(properties);
        self
    }
}
