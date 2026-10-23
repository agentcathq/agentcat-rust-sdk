//! `AgentCat` captures usage analytics for MCP servers built on `rmcp`.
//!
//! Wrap a server handler with [`track`], or with [`AgentCat::track`] where one handle serves many
//! handler instances (streamable HTTP), and serve the [`Tracked`] wrapper in its place. Settings
//! are in [`Options`]. The wrapper forwards every `ServerHandler` method to the handler. Inside a
//! tool, [`CallContext`] tells the tool about its call, and [`capture_error`] records the type and
//! the cause chain of an error the tool is about to return.
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod call;
mod capture;
mod client;
mod config;
mod constants;
mod context;
mod custom;
mod delivery;
mod error;
mod event;
mod handles;
mod hooks;
mod inject;
mod js;
mod logging;
mod pipeline;
mod registry;
mod sink;
#[cfg(test)]
mod test_support;
#[cfg(feature = "test-util")]
#[cfg_attr(docsrs, doc(cfg(feature = "test-util")))]
pub mod testing;
mod tokens;
mod tracked;

pub use capture::capture_error;
pub use client::{AgentCat, track};
pub use config::{
    AGENTCAT_API_URL, AGENTCAT_ENABLED, AGENTCAT_PROJECT_ID, HookError, LogDestination,
    MCPCAT_API_URL, Options, UserIdentity,
};
pub use context::CallContext;
pub use custom::CustomEvent;
pub use error::Error;
pub use event::Event;
pub use hooks::ToolCall;
pub use tracked::Tracked;
