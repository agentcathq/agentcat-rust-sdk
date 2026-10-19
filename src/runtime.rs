//! Facts about the running process that the SDK reports.

/// The version of the MCP library the SDK instruments, when it is compiled in.
#[allow(
    clippy::unnecessary_wraps,
    reason = "builds without the rmcp feature return None"
)]
pub(crate) fn mcp_library_version() -> Option<&'static str> {
    #[cfg(feature = "rmcp")]
    {
        use std::sync::OnceLock;
        static VERSION: OnceLock<String> = OnceLock::new();
        // `Implementation::default()` expands `env!("CARGO_PKG_VERSION")`
        // inside rmcp, so it carries the resolved rmcp version.
        Some(VERSION.get_or_init(|| rmcp::model::Implementation::default().version))
    }
    #[cfg(not(feature = "rmcp"))]
    {
        None
    }
}
