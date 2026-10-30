//! The rmcp version this test binary compiled against. The CI matrix sets
//! `RMCP_EXPECTED_VERSION` so that each leg proves it built the rmcp minor it names.
use rmcp::model::Implementation;

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the CI matrix passes the expected version through the environment"
)]
fn compiled_rmcp_version() {
    // `Implementation::default()` carries rmcp's own build-time version (03-rmcp.md section 5.2).
    let version = Implementation::default().version;
    match std::env::var("RMCP_EXPECTED_VERSION") {
        Ok(expected) => assert_eq!(version, expected),
        Err(_) => assert!(version.starts_with("3."), "rmcp version {version}"),
    }
}
