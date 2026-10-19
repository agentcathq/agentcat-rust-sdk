//! Delivery: retries, custom events, flushing.

mod common;

use std::time::Duration;

use agentcat::{CustomEvent, ErrorData};
use common::*;
use serde_json::{Value, json};

#[tokio::test(start_paused = false)]
async fn retries_server_errors_with_the_same_event_id() {
    let agentcat = builder("retry-503").build();
    api().fail_next("retry-503-event", 2, 503);
    agentcat.publish_custom_event(CustomEvent::new().resource_name("retry-503-event"));
    let requests = api()
        .wait_for(3, |v| v["resource_name"] == "retry-503-event")
        .await;
    let ids: Vec<&Value> = requests.iter().map(|r| &r["id"]).collect();
    assert!(
        ids.windows(2).all(|w| w[0] == w[1]),
        "retries reuse the id: {ids:?}"
    );
    assert!(agentcat::flush(Duration::from_secs(10)).await);
}

#[tokio::test]
async fn does_not_retry_client_errors() {
    let agentcat = builder("no-retry-400").build();
    api().fail_next("no-retry-400-event", 1, 400);
    agentcat.publish_custom_event(CustomEvent::new().resource_name("no-retry-400-event"));
    api()
        .wait_for(1, |v| v["resource_name"] == "no-retry-400-event")
        .await;
    assert!(agentcat::flush(Duration::from_secs(5)).await);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let all = api().requests_where(|v| v["resource_name"] == "no-retry-400-event");
    assert_eq!(all.len(), 1, "a 400 is not retried");
}

#[tokio::test]
async fn custom_events_carry_their_data() {
    let agentcat = builder("custom").build();
    let error = ErrorData::new("quota exceeded").with_type("QuotaError");
    agentcat.publish_custom_event(
        CustomEvent::new()
            .session_id("ses_2cOHEO0LYGADMzRvWTXXVbbgxgm")
            .resource_name("feedback")
            .parameters(json!({"rating": 5}))
            .response(json!({"ok": true}))
            .message("User gave feedback")
            .duration(12)
            .error(error)
            .tag("source", "widget")
            .tag("bad!key", "dropped")
            .properties(json!({"plan": "pro"})),
    );
    let e = &api().wait_for(1, |v| v["resource_name"] == "feedback").await[0];
    assert_eq!(e["event_type"], "agentcat:custom");
    assert_eq!(e["session_id"], "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm");
    assert_eq!(e["parameters"], json!({"rating": 5}));
    assert_eq!(e["response"], json!({"ok": true}));
    assert_eq!(e["user_intent"], "User gave feedback");
    assert_eq!(e["duration"], 12);
    assert_eq!(e["is_error"], true);
    assert_eq!(
        e["error"],
        json!({"message": "quota exceeded", "type": "QuotaError", "platform": "rust"})
    );
    assert_eq!(e["tags"], json!({"source": "widget"}));
    assert_eq!(e["properties"], json!({"plan": "pro"}));
    assert_eq!(e["sdk_language"], "Rust");
    assert_eq!(e["server_name"], "custom");
}

#[tokio::test]
async fn custom_events_without_a_session_send_null() {
    let agentcat = builder("custom-null").build();
    agentcat.publish_custom_event(CustomEvent::new().resource_name("custom-null-event"));
    let e = &api()
        .wait_for(1, |v| v["resource_name"] == "custom-null-event")
        .await[0];
    assert!(e["session_id"].is_null());
}

#[tokio::test]
async fn session_string_custom_events_use_the_string_verbatim() {
    let _ = builder("custom-free").build(); // configures the mock API URL
    agentcat::publish_custom_event(
        "my-own-session",
        PROJECT_ID,
        CustomEvent::new().resource_name("custom-free-event"),
    )
    .unwrap();
    let e = &api()
        .wait_for(1, |v| v["resource_name"] == "custom-free-event")
        .await[0];
    assert_eq!(e["session_id"], "my-own-session");
    assert_eq!(e["project_id"], PROJECT_ID);
    // No tracked configuration: no SDK metadata.
    assert!(e.get("sdk_language").is_none());

    assert_eq!(
        agentcat::publish_custom_event("s", "", CustomEvent::new()),
        Err(agentcat::Error::MissingProjectId)
    );
}

#[tokio::test]
async fn overlong_custom_session_ids_are_dropped_not_the_event() {
    let agentcat = builder("custom-long").build();
    agentcat.publish_custom_event(
        CustomEvent::new()
            .session_id("x".repeat(40))
            .resource_name("custom-long-event"),
    );
    let e = &api()
        .wait_for(1, |v| v["resource_name"] == "custom-long-event")
        .await[0];
    assert!(e["session_id"].is_null());
}

#[tokio::test]
async fn telemetry_only_mode_sends_nothing_to_agentcat() {
    let agentcat = agentcat::AgentCat::telemetry_only()
        .disable_diagnostics(true)
        .log_file(false)
        .server_info("telemetry-only", "1")
        .build();
    let client = connect(agentcat.track(Todo::new())).await;
    call(&client, "add_todo", json!({"text": "a"})).await.unwrap();
    assert!(agentcat::flush(Duration::from_secs(5)).await);
    api().assert_count_settles("telemetry-only", 0).await;
}

#[test]
fn blocking_flush_works_without_a_runtime() {
    let agentcat = builder("blocking-flush").build();
    agentcat.publish_custom_event(CustomEvent::new().resource_name("blocking-flush-event"));
    assert!(agentcat::flush_blocking(Duration::from_secs(10)));
    assert_eq!(
        api()
            .requests_where(|v| v["resource_name"] == "blocking-flush-event")
            .len(),
        1
    );
}
