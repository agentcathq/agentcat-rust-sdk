//! `AgentCat` captures usage analytics for MCP servers built on `rmcp`.
//!
//! Wrap a server handler with [`track`], or with [`AgentCat::track`] where one handle serves many
//! handler instances (streamable HTTP), and serve the [`Tracked`] wrapper in its place. Settings
//! are in [`Options`]. The wrapper forwards every `ServerHandler` method to the handler.
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the pure core is wired into the public API from plan 3 on"
    )
)]

mod client;
mod config;
mod constants;
mod handles;
mod js;
mod pipeline;
mod sink;
#[cfg(test)]
mod test_support;
mod tokens;
mod tracked;

pub use client::{AgentCat, track};
pub use config::Options;
pub use tracked::Tracked;
