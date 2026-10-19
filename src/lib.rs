//! # `AgentCat` for Rust MCP servers
//!
//! `AgentCat` is analytics for MCP servers: which tools agents call, why they
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
//! What [`AgentCat::track`] does, in the same way as `AgentCat`'s TypeScript,
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
//! for `AgentCat`, and `AgentCat` failures never fail a tool call.
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
    /// True for a `ses_` ID this SDK could have issued.
    pub fn is_valid_session_id(value: &str) -> bool {
        crate::ksuid::is_valid_session_id(value)
    }

    /// Runs the TS-compatible injection on one tool's schemas.
    pub fn inject_tool_for_test(
        config: (bool, bool, bool, Option<&str>, bool, bool),
        name: &str,
        input: &mut serde_json::Map<String, serde_json::Value>,
        output: Option<&mut serde_json::Map<String, serde_json::Value>>,
    ) -> (bool, bool, bool, bool, bool) {
        let (tracing, hook_mode, agent_tracking, context_description, tool_call_context, report_missing) =
            config;
        let cfg = crate::inject::InjectConfig::new(
            tracing,
            hook_mode,
            agent_tracking,
            tool_call_context,
            context_description,
            report_missing,
        );
        let memory = crate::inject::InjectMemory::default();
        let r = crate::inject::inject_tool(&cfg, &memory, name, input, output);
        (
            r.strip.session_id,
            r.strip.agent_id,
            r.strip.context,
            r.declares_session_id,
            r.output_injected,
        )
    }

    /// The report-missing tool descriptor.
    pub fn report_missing_descriptor() -> serde_json::Value {
        crate::inject::report_missing_descriptor()
    }

    /// Token estimate helpers.
    pub fn estimate_input_tokens(args: Option<&serde_json::Value>) -> Option<u32> {
        crate::tokens::estimate_input_tokens(args)
    }
    /// Token estimate helpers.
    pub fn estimate_output_tokens(response: Option<&serde_json::Value>) -> Option<u32> {
        crate::tokens::estimate_output_tokens(response)
    }
    /// Token estimate helpers.
    pub fn estimate_tokens(bytes: f64) -> u32 {
        crate::tokens::estimate_tokens(bytes)
    }

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

    /// KSUID helpers.
    pub fn ksuid_from_parts(time_ms: u64, payload: [u8; 16]) -> Option<String> {
        crate::ksuid::Ksuid::from_parts(time_ms, payload).map(|k| k.encode())
    }
    /// KSUID helpers: `(timestamp, payload)`.
    pub fn ksuid_parse(s: &str) -> Option<(u32, Vec<u8>)> {
        crate::ksuid::Ksuid::parse(s).map(|k| (k.timestamp(), k.payload().to_vec()))
    }

    /// Trace-context helpers: `(trace_id, span_id, dd_trace_id, dd_span_id)`.
    pub fn trace_context(id: &str) -> (String, String, String, String) {
        use crate::exporters::trace_context as t;
        (
            t::trace_id(id),
            t::span_id(id),
            t::datadog_trace_id(id),
            t::datadog_span_id(id),
        )
    }

    /// `PostHog` `UUIDv7` derivation.
    #[cfg(feature = "posthog")]
    pub fn to_uuid_v7(id: &str) -> String {
        crate::exporters::posthog_uuid_v7(id)
    }

    /// Handle resolution plus everything derived from it:
    /// `(source, session_id, agent_id, mint_back_text, structured, tags)`.
    #[allow(
        clippy::type_complexity,
        reason = "a test helper returns its parts as one tuple"
    )]
    pub fn resolve_for_test(
        hook_mode: bool,
        agent_tracking: bool,
        args: Option<&serde_json::Map<String, serde_json::Value>>,
        session_param_is_ours: bool,
        protocol_version: Option<&str>,
    ) -> (
        &'static str,
        String,
        Option<String>,
        Option<String>,
        Option<serde_json::Map<String, serde_json::Value>>,
        std::collections::BTreeMap<String, String>,
    ) {
        let res = crate::handles::resolve_handles(hook_mode, agent_tracking, args, session_param_is_ours);
        (
            res.session_source.as_str(),
            res.session_id.clone(),
            res.agent_id.clone(),
            crate::handles::build_mint_back_text(&res),
            crate::handles::build_structured_mint_back(&res),
            crate::handles::build_handle_tags(&res, protocol_version),
        )
    }

    /// Mirrors a structured mint-back into `structured`.
    pub fn mirror_for_test(
        structured: &mut Option<serde_json::Value>,
        mint: serde_json::Map<String, serde_json::Value>,
    ) -> bool {
        crate::handles::mirror_structured_mint_back(structured, mint)
    }

    /// Reads a supplied handle off arguments.
    pub fn extract_handle(
        args: Option<&serde_json::Map<String, serde_json::Value>>,
        name: &str,
    ) -> Option<String> {
        crate::handles::extract_handle(args, name)
    }

    /// Strips with an explicit registry entry, or the heuristic when `None`.
    pub fn strip_for_test(
        args: &mut serde_json::Map<String, serde_json::Value>,
        tool: &str,
        entry: Option<(bool, bool, bool)>,
    ) {
        let strip = match entry {
            Some((session_id, agent_id, context)) => crate::inject::Injected {
                session_id,
                agent_id,
                context,
            },
            None => crate::inject::heuristic_strip(tool),
        };
        crate::inject::strip_arguments(args, strip);
    }

    /// The wire request for an event.
    pub fn publish_request_json(event: &crate::Event) -> serde_json::Value {
        serde_json::to_value(crate::publisher::to_publish_request(event)).unwrap_or_default()
    }
}
