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

e2e_case!(basic_stdio_2025_11_25, basic, Stdio, V2025_11_25);
e2e_case!(basic_http_2025_11_25, basic, Http, V2025_11_25);
e2e_case!(basic_http_sse_2025_11_25, basic, HttpSse, V2025_11_25);
e2e_case!(
    basic_http_stateless_2025_11_25,
    basic,
    HttpStateless,
    V2025_11_25
);
e2e_case!(basic_stdio_2026_07_28, basic, Stdio, V2026_07_28);
e2e_case!(basic_http_2026_07_28, basic, Http, V2026_07_28);
e2e_case!(basic_http_sse_2026_07_28, basic, HttpSse, V2026_07_28);
e2e_case!(
    basic_http_stateless_2026_07_28,
    basic,
    HttpStateless,
    V2026_07_28
);

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
