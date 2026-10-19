//! A call larger than the queue's 32 MiB memory budget. Until the publisher
//! truncates its event, the queue drops every new event, so this test has its
//! own process.

mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn large_calls_still_publish_a_truncated_event() {
    let agentcat = builder("large-call").build();
    let client = connect(agentcat.track(Todo::new())).await;
    let big = "x".repeat(40 * 1024 * 1024);
    call(&client, "echo", json!({"blob": big})).await.unwrap();
    let e = &api().events_for("large-call", 1).await[0];
    let size = serde_json::to_vec(e).unwrap().len();
    assert!(size < 200_000, "truncated to about 100 KB, got {size}");
}
