//! `AgentCat` captures usage analytics for MCP servers built on `rmcp`.
//!
//! This crate is a work in progress and exposes no public items yet.
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the pure core is wired into the public API from plan 3 on"
    )
)]

mod constants;
mod handles;
mod js;
#[cfg(test)]
mod test_support;
mod tokens;
