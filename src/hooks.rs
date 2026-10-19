//! Type-erased customer hooks.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::event::Event;

pub(crate) type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// The string-level redaction hook.
#[derive(Clone)]
pub(crate) enum StringRedactor {
    Sync(Arc<dyn Fn(&str) -> String + Send + Sync>),
    Async(Arc<dyn Fn(String) -> BoxFuture<String> + Send + Sync>),
}

/// The event-level redaction hook.
#[derive(Clone)]
pub(crate) enum EventRedactor {
    Sync(Arc<dyn Fn(Event) -> Option<Event> + Send + Sync>),
    Async(Arc<dyn Fn(Event) -> BoxFuture<Option<Event>> + Send + Sync>),
}

/// The actor behind a tool call, returned by the `identify` hook.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct UserIdentity {
    /// Your stable ID for the actor.
    pub user_id: String,
    /// A display name.
    pub user_name: Option<String>,
    /// Extra JSON data about the actor (plan, company, …).
    pub user_data: Option<serde_json::Map<String, serde_json::Value>>,
}

impl UserIdentity {
    /// An identity with the given user ID.
    pub fn new(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            ..Self::default()
        }
    }

    /// Sets the display name.
    pub fn name(mut self, user_name: impl Into<String>) -> Self {
        self.user_name = Some(user_name.into());
        self
    }

    /// Sets extra data. Values that are not JSON objects are ignored.
    pub fn data(mut self, user_data: serde_json::Value) -> Self {
        if let serde_json::Value::Object(map) = user_data {
            self.user_data = Some(map);
        }
        self
    }
}
