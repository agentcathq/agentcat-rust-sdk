//! Shutdown closes the process-wide publisher, so it has its own process.

mod common;

use std::time::Duration;

use agentcat::CustomEvent;
use common::*;
use serde_json::json;

#[tokio::test]
async fn shutdown_drains_queued_events_then_drops_new_ones() {
    let agentcat = builder("shutdown").build();
    let client = connect(agentcat.track(Todo::new())).await;
    for i in 0..20 {
        call(&client, "add_todo", json!({"text": format!("item {i}")}))
            .await
            .unwrap();
    }
    assert!(agentcat::shutdown(Duration::from_secs(10)).await);
    assert_eq!(api().requests_where(|v| v["server_name"] == "shutdown").len(), 20);

    // After shutdown, events are dropped and tool calls keep working.
    let result = call(&client, "add_todo", json!({"text": "late"})).await.unwrap();
    assert_eq!(texts(&result)[1], "Added: late");
    agentcat.publish_custom_event(CustomEvent::new().resource_name("late"));
    assert!(agentcat::flush(Duration::from_secs(1)).await);
    api().assert_count_settles("shutdown", 20).await;
}
