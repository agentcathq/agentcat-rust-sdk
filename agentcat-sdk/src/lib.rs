//! `AgentCat` captures usage analytics for MCP servers built on `rmcp`.
//!
//! Wrap a server handler with [`track`], or with [`AgentCat::track`] where one handle serves many
//! handler instances (streamable HTTP), and serve the [`Tracked`] wrapper in its place. Settings
//! are in [`Options`]. The wrapper forwards every `ServerHandler` method to the handler. Inside a
//! tool, [`CallContext`] tells the tool about its call, and [`capture_error`] records the type and
//! the cause chain of an error the tool is about to return.
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the pure core is wired into the public API from plan 3 on"
    )
)]

mod call;
mod capture;
mod client;
mod config;
mod constants;
mod context;
mod event;
mod handles;
mod inject;
mod js;
mod pipeline;
mod registry;
mod sink;
#[cfg(test)]
mod test_support;
mod tokens;
mod tracked;

pub use capture::capture_error;
pub use client::{AgentCat, track};
pub use config::Options;
pub use context::CallContext;
pub use tracked::Tracked;
