//! End-to-end scenarios: a tracked rmcp server, a scripted client and a mock `AgentCat` API, over
//! every transport and protocol lane. Each run's transcript and published events, normalized, must
//! equal the golden file of the run (see `e2e/mod.rs`).
//!
//! `UPDATE_GOLDEN=1 cargo nextest run -p agentcat-sdk --test e2e` rewrites the golden files.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers: a failed step fails the test"
)]

#[path = "e2e/mod.rs"]
mod suite;

use suite::e2e_case;

e2e_case! {
    agent_tracking_stdio_2025_11_25, agent_tracking, Stdio, V2025_11_25;
    agent_tracking_http_2025_11_25, agent_tracking, Http, V2025_11_25;
    agent_tracking_http_sse_2025_11_25, agent_tracking, HttpSse, V2025_11_25;
    agent_tracking_http_stateless_2025_11_25, agent_tracking, HttpStateless, V2025_11_25;
    agent_tracking_stdio_2026_07_28, agent_tracking, Stdio, V2026_07_28;
    agent_tracking_http_2026_07_28, agent_tracking, Http, V2026_07_28;
    agent_tracking_http_sse_2026_07_28, agent_tracking, HttpSse, V2026_07_28;
    agent_tracking_http_stateless_2026_07_28, agent_tracking, HttpStateless, V2026_07_28;
}

e2e_case! {
    basic_stdio_2025_11_25, basic, Stdio, V2025_11_25;
    basic_http_2025_11_25, basic, Http, V2025_11_25;
    basic_http_sse_2025_11_25, basic, HttpSse, V2025_11_25;
    basic_http_stateless_2025_11_25, basic, HttpStateless, V2025_11_25;
    basic_stdio_2026_07_28, basic, Stdio, V2026_07_28;
    basic_http_2026_07_28, basic, Http, V2026_07_28;
    basic_http_sse_2026_07_28, basic, HttpSse, V2026_07_28;
    basic_http_stateless_2026_07_28, basic, HttpStateless, V2026_07_28;
}

e2e_case! {
    concurrency_stdio_2025_11_25, concurrency, Stdio, V2025_11_25;
    concurrency_http_2025_11_25, concurrency, Http, V2025_11_25;
    concurrency_http_sse_2025_11_25, concurrency, HttpSse, V2025_11_25;
    concurrency_http_stateless_2025_11_25, concurrency, HttpStateless, V2025_11_25;
    concurrency_stdio_2026_07_28, concurrency, Stdio, V2026_07_28;
    concurrency_http_2026_07_28, concurrency, Http, V2026_07_28;
    concurrency_http_sse_2026_07_28, concurrency, HttpSse, V2026_07_28;
    concurrency_http_stateless_2026_07_28, concurrency, HttpStateless, V2026_07_28;
}

e2e_case! {
    custom_events_stdio_2025_11_25, custom_events, Stdio, V2025_11_25;
    custom_events_http_2025_11_25, custom_events, Http, V2025_11_25;
    custom_events_http_sse_2025_11_25, custom_events, HttpSse, V2025_11_25;
    custom_events_http_stateless_2025_11_25, custom_events, HttpStateless, V2025_11_25;
    custom_events_stdio_2026_07_28, custom_events, Stdio, V2026_07_28;
    custom_events_http_2026_07_28, custom_events, Http, V2026_07_28;
    custom_events_http_sse_2026_07_28, custom_events, HttpSse, V2026_07_28;
    custom_events_http_stateless_2026_07_28, custom_events, HttpStateless, V2026_07_28;
}

e2e_case! {
    features_off_stdio_2025_11_25, features_off, Stdio, V2025_11_25;
    features_off_http_2025_11_25, features_off, Http, V2025_11_25;
    features_off_http_sse_2025_11_25, features_off, HttpSse, V2025_11_25;
    features_off_http_stateless_2025_11_25, features_off, HttpStateless, V2025_11_25;
    features_off_stdio_2026_07_28, features_off, Stdio, V2026_07_28;
    features_off_http_2026_07_28, features_off, Http, V2026_07_28;
    features_off_http_sse_2026_07_28, features_off, HttpSse, V2026_07_28;
    features_off_http_stateless_2026_07_28, features_off, HttpStateless, V2026_07_28;
}

e2e_case! {
    hook_mode_stdio_2025_11_25, hook_mode, Stdio, V2025_11_25;
    hook_mode_http_2025_11_25, hook_mode, Http, V2025_11_25;
    hook_mode_http_sse_2025_11_25, hook_mode, HttpSse, V2025_11_25;
    hook_mode_http_stateless_2025_11_25, hook_mode, HttpStateless, V2025_11_25;
    hook_mode_stdio_2026_07_28, hook_mode, Stdio, V2026_07_28;
    hook_mode_http_2026_07_28, hook_mode, Http, V2026_07_28;
    hook_mode_http_sse_2026_07_28, hook_mode, HttpSse, V2026_07_28;
    hook_mode_http_stateless_2026_07_28, hook_mode, HttpStateless, V2026_07_28;
}

e2e_case! {
    hooks_fail_stdio_2025_11_25, hooks_fail, Stdio, V2025_11_25;
    hooks_fail_http_2025_11_25, hooks_fail, Http, V2025_11_25;
    hooks_fail_http_sse_2025_11_25, hooks_fail, HttpSse, V2025_11_25;
    hooks_fail_http_stateless_2025_11_25, hooks_fail, HttpStateless, V2025_11_25;
    hooks_fail_stdio_2026_07_28, hooks_fail, Stdio, V2026_07_28;
    hooks_fail_http_2026_07_28, hooks_fail, Http, V2026_07_28;
    hooks_fail_http_sse_2026_07_28, hooks_fail, HttpSse, V2026_07_28;
    hooks_fail_http_stateless_2026_07_28, hooks_fail, HttpStateless, V2026_07_28;
}

e2e_case! {
    hooks_fail_resolve_stdio_2025_11_25, hooks_fail_resolve, Stdio, V2025_11_25;
    hooks_fail_resolve_http_2025_11_25, hooks_fail_resolve, Http, V2025_11_25;
    hooks_fail_resolve_http_sse_2025_11_25, hooks_fail_resolve, HttpSse, V2025_11_25;
    hooks_fail_resolve_http_stateless_2025_11_25, hooks_fail_resolve, HttpStateless, V2025_11_25;
    hooks_fail_resolve_stdio_2026_07_28, hooks_fail_resolve, Stdio, V2026_07_28;
    hooks_fail_resolve_http_2026_07_28, hooks_fail_resolve, Http, V2026_07_28;
    hooks_fail_resolve_http_sse_2026_07_28, hooks_fail_resolve, HttpSse, V2026_07_28;
    hooks_fail_resolve_http_stateless_2026_07_28, hooks_fail_resolve, HttpStateless, V2026_07_28;
}

e2e_case! {
    pagination_stdio_2025_11_25, pagination, Stdio, V2025_11_25;
    pagination_http_2025_11_25, pagination, Http, V2025_11_25;
    pagination_http_sse_2025_11_25, pagination, HttpSse, V2025_11_25;
    pagination_http_stateless_2025_11_25, pagination, HttpStateless, V2025_11_25;
    pagination_stdio_2026_07_28, pagination, Stdio, V2026_07_28;
    pagination_http_2026_07_28, pagination, Http, V2026_07_28;
    pagination_http_sse_2026_07_28, pagination, HttpSse, V2026_07_28;
    pagination_http_stateless_2026_07_28, pagination, HttpStateless, V2026_07_28;
}

e2e_case! {
    redact_drop_stdio_2025_11_25, redact_drop, Stdio, V2025_11_25;
    redact_drop_http_2025_11_25, redact_drop, Http, V2025_11_25;
    redact_drop_http_sse_2025_11_25, redact_drop, HttpSse, V2025_11_25;
    redact_drop_http_stateless_2025_11_25, redact_drop, HttpStateless, V2025_11_25;
    redact_drop_stdio_2026_07_28, redact_drop, Stdio, V2026_07_28;
    redact_drop_http_2026_07_28, redact_drop, Http, V2026_07_28;
    redact_drop_http_sse_2026_07_28, redact_drop, HttpSse, V2026_07_28;
    redact_drop_http_stateless_2026_07_28, redact_drop, HttpStateless, V2026_07_28;
}

e2e_case! {
    redaction_stdio_2025_11_25, redaction, Stdio, V2025_11_25;
    redaction_http_2025_11_25, redaction, Http, V2025_11_25;
    redaction_http_sse_2025_11_25, redaction, HttpSse, V2025_11_25;
    redaction_http_stateless_2025_11_25, redaction, HttpStateless, V2025_11_25;
    redaction_stdio_2026_07_28, redaction, Stdio, V2026_07_28;
    redaction_http_2026_07_28, redaction, Http, V2026_07_28;
    redaction_http_sse_2026_07_28, redaction, HttpSse, V2026_07_28;
    redaction_http_stateless_2026_07_28, redaction, HttpStateless, V2026_07_28;
}

e2e_case! {
    schemas_stdio_2025_11_25, schemas, Stdio, V2025_11_25;
    schemas_http_2025_11_25, schemas, Http, V2025_11_25;
    schemas_http_sse_2025_11_25, schemas, HttpSse, V2025_11_25;
    schemas_http_stateless_2025_11_25, schemas, HttpStateless, V2025_11_25;
    schemas_stdio_2026_07_28, schemas, Stdio, V2026_07_28;
    schemas_http_2026_07_28, schemas, Http, V2026_07_28;
    schemas_http_sse_2026_07_28, schemas, HttpSse, V2026_07_28;
    schemas_http_stateless_2026_07_28, schemas, HttpStateless, V2026_07_28;
}

e2e_case! {
    tracing_off_stdio_2025_11_25, tracing_off, Stdio, V2025_11_25;
    tracing_off_http_2025_11_25, tracing_off, Http, V2025_11_25;
    tracing_off_http_sse_2025_11_25, tracing_off, HttpSse, V2025_11_25;
    tracing_off_http_stateless_2025_11_25, tracing_off, HttpStateless, V2025_11_25;
    tracing_off_stdio_2026_07_28, tracing_off, Stdio, V2026_07_28;
    tracing_off_http_2026_07_28, tracing_off, Http, V2026_07_28;
    tracing_off_http_sse_2026_07_28, tracing_off, HttpSse, V2026_07_28;
    tracing_off_http_stateless_2026_07_28, tracing_off, HttpStateless, V2026_07_28;
}

#[test]
fn golden_compare_ignores_crlf() {
    let actual = serde_json::json!({"events": [{"body": {"id": "<evt#1>"}}], "transcript": []});
    let golden = "{\r\n  \"events\": [\r\n    {\r\n      \"body\": {\r\n        \"id\": \"<evt#1>\"\r\n      }\r\n    }\r\n  ],\r\n  \"transcript\": []\r\n}\r\n";
    suite::assert_matches(golden, &actual, "golden");
}

#[test]
#[should_panic(
    expected = "golden differs at $.events[0].body[\"io.modelcontextprotocol/x\"]: expected \"a\", got \"b\""
)]
fn golden_compare_reports_the_first_difference() {
    let golden = r#"{"events": [{"body": {"io.modelcontextprotocol/x": "a", "duration": 1}}]}"#;
    let actual = serde_json::json!({"events": [{"body": {"io.modelcontextprotocol/x": "b", "duration": 2}}]});
    suite::assert_matches(golden, &actual, "golden");
}

#[tokio::test]
#[should_panic(expected = "the test of basic over Http on V2026_07_28")]
async fn a_case_is_named_after_its_scenario_transport_and_lane() {
    suite::run_case(
        "basic_http_sse_2026_07_28",
        "basic",
        suite::Transport::Http,
        suite::Lane::V2026_07_28,
    )
    .await;
}
