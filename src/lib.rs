//! # AgentCat for Rust MCP servers
//!
//! AgentCat is analytics for MCP servers: which tools agents call, why they
//! call them (intent), which tasks and agents the calls belong to, who the
//! actor is, and how the calls fail. This crate instruments servers built
//! with [`rmcp`](https://crates.io/crates/rmcp) 3.x.
//!
//! ```no_run
//! use std::time::Duration;
//! use rmcp::{ServiceExt, transport::stdio};
//! # #[derive(Clone)] struct Counter;
//! # impl rmcp::ServerHandler for Counter {}
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let agentcat = agentcat::AgentCat::new("proj_0000000000000000000000000000");
//!     let service = agentcat.track(Counter).serve(stdio()).await?;
//!     service.waiting().await?;
//!     // Deliver queued events before the process exits.
//!     agentcat::shutdown(Duration::from_secs(5)).await;
//!     Ok(())
//! }
//! ```
//!
//! What [`AgentCat::track`] does, in the same way as AgentCat's TypeScript,
//! Python and Go SDKs:
//!
//! - **`tools/list`:** adds a required `context` parameter (the agent's
//!   intent), a `session_id` handle (task continuity across stateless
//!   requests) and optionally an `agent_id` handle to every tool, declares
//!   `mcp_session` in output schemas, and adds a `get_more_tools` tool that
//!   lets agents report missing functionality.
//! - **`tools/call`:** strips those parameters before your handler runs,
//!   issues session IDs, returns them in a leading text block and in
//!   `structuredContent.mcp_session`, and publishes one `mcp:tools/call`
//!   event with the request, the result, timing, token estimates and errors.
//!
//! Events are published from a background thread; your tool calls never wait
//! for AgentCat, and AgentCat failures never fail a tool call.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(not(test), warn(clippy::pedantic))]
// AgentCat must never take the customer's server down.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::allow_attributes_without_reason
    )
)]
// Pedantic lints that do not fit this crate: options mirror the TS flags
// (many bools), public names repeat their module (`exporters::ExporterConfig`),
// and `#[must_use]` goes on public values that matter, not on every function.
#![cfg_attr(
    not(test),
    allow(
        clippy::must_use_candidate,
        clippy::module_name_repetitions,
        clippy::struct_excessive_bools,
        clippy::fn_params_excessive_bools,
        reason = "pedantic lints that do not fit this crate"
    )
)]
#![cfg_attr(
    not(all(
        feature = "rmcp",
        feature = "otlp",
        feature = "datadog",
        feature = "sentry",
        feature = "posthog"
    )),
    allow(
        dead_code,
        unreachable_code,
        unused_imports,
        unused_variables,
        reason = "builds without the rmcp adapter or the exporters leave parts unused"
    )
)]

mod agentcat;
pub mod constants;
mod diagnostics;
mod event;
mod exceptions;
pub mod exporters;
mod handles;
mod hooks;
mod inject;
mod js;
mod ksuid;
mod logging;
mod publisher;
mod redaction;
mod runtime;
mod sanitization;
mod timer;
mod tokens;
#[cfg(feature = "rmcp")]
mod tool_call;
#[cfg(feature = "rmcp")]
mod tracked;
mod truncation;
mod util;
mod validation;

#[cfg(test)]
#[cfg(all(
    feature = "rmcp",
    feature = "preserve-order",
    feature = "otlp",
    feature = "datadog",
    feature = "sentry",
    feature = "posthog"
))]
#[path = "../tests/parity/mod.rs"]
mod parity;

pub use agentcat::{
    AgentCat, AgentCatBuilder, CustomEvent, Error, flush, flush_blocking, publish_custom_event, shutdown,
    shutdown_blocking,
};
pub use event::{ChainedErrorData, ErrorData, Event, StackFrame};
pub use exporters::ExporterConfig;
pub use hooks::UserIdentity;
pub use ksuid::derive_session_id;
#[cfg(feature = "rmcp")]
pub use tool_call::ToolCallRequest;
#[cfg(feature = "rmcp")]
pub use tracked::{Tracked, track};

#[cfg(not(any(feature = "rustls", feature = "native-tls")))]
compile_error!("agentcat-sdk needs a TLS backend: enable the `rustls` (default) or `native-tls` feature.");

#[doc(hidden)]
/// Internals exposed for the crate's integration tests. Not part of the
/// public API; may change in any release.
pub mod __private {
    /// Runs sanitization on an event's response and parameters.
    pub fn sanitize(
        response: Option<serde_json::Value>,
        parameters: Option<serde_json::Value>,
    ) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
        let mut e = crate::Event {
            parameters,
            response,
            ..Default::default()
        };
        crate::sanitization::sanitize_event(&mut e);
        (e.response, e.parameters)
    }

    /// Runs truncation on an event.
    pub fn truncate(event: crate::Event) -> crate::Event {
        crate::truncation::truncate_event(event)
    }

    /// Recursive normalization.
    pub fn normalize(value: &serde_json::Value, depth: usize, max_string: usize) -> serde_json::Value {
        crate::truncation::normalize(value, depth, 100, max_string)
    }

    /// Error capture for an `isError` tool result.
    pub fn capture_tool_result_error(result: &serde_json::Value) -> crate::ErrorData {
        crate::exceptions::capture_tool_result_error(result)
    }

    /// The wire request for an event.
    pub fn publish_request_json(event: &crate::Event) -> serde_json::Value {
        serde_json::to_value(crate::publisher::to_publish_request(event)).unwrap_or_default()
    }
}
