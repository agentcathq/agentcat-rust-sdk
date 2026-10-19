//! Customer hooks: identify, resolve_session_id, event tags and properties,
//! and both redaction hooks.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use agentcat::UserIdentity;
use common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn identify_stamps_the_actor_on_every_call() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let agentcat = builder("identify")
        .identify(move |req| {
            seen.fetch_add(1, Ordering::SeqCst);
            let tool = req.tool_name().to_owned();
            async move {
                Some(
                    UserIdentity::new("user-42")
                        .name("Ada")
                        .data(json!({"plan": "pro", "tool": tool})),
                )
            }
        })
        .build();
    let client = connect(agentcat.track(Todo::new())).await;
    call(&client, "add_todo", json!({"text": "a"})).await.unwrap();
    call(&client, "count_todos", json!({})).await.unwrap();

    let events = api().events_for("identify", 2).await;
    for e in &events {
        assert_eq!(e["identify_actor_given_id"], "user-42");
        assert_eq!(e["identify_actor_name"], "Ada");
        assert_eq!(e["actor_id"], "user-42");
        assert_eq!(e["identify_data"]["plan"], "pro");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2, "identify runs on every call, no cache");
}

#[tokio::test]
async fn a_failing_or_slow_hook_never_affects_the_tool_call() {
    let agentcat = builder("bad-hooks")
        .identify(|_req| async move { panic!("identify bug") })
        .event_tags(|_req| async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Some(vec![("slow", "yes")])
        })
        .event_properties(|_req| async move { Some(json!("not an object")) })
        .build();
    let client = connect(agentcat.track(Todo::new())).await;
    let started = std::time::Instant::now();
    let result = call(&client, "add_todo", json!({"text": "a"})).await.unwrap();
    assert!(started.elapsed() < Duration::from_millis(150), "hooks must not delay the call");
    assert_eq!(texts(&result)[1], "Added: a");

    let events = api().events_for("bad-hooks", 1).await;
    let e = &events[0];
    assert!(e.get("identify_actor_given_id").is_none());
    assert_eq!(e["tags"]["slow"], "yes");
    assert!(e.get("properties").is_none());
}

#[tokio::test]
async fn tags_are_validated_and_sdk_tags_win() {
    let agentcat = builder("tags")
        .event_tags(|_req| async move {
            Some(vec![
                ("env".to_owned(), "prod".to_owned()),
                ("bad key".to_owned(), "x".to_owned()),
                ("agentcat:reserved".to_owned(), "x".to_owned()),
                ("multi".to_owned(), "line\nbreak".to_owned()),
                ("agentcat_session_id_source".to_owned(), "spoofed".to_owned()),
            ])
        })
        .event_properties(|req| {
            let tool = req.tool_name().to_owned();
            async move { Some(json!({"tool": tool, "nested": {"flags": ["a", "b"]}})) }
        })
        .build();
    let client = connect(agentcat.track(Todo::new())).await;
    call(&client, "add_todo", json!({"text": "a"})).await.unwrap();

    let e = &api().events_for("tags", 1).await[0];
    let tags = e["tags"].as_object().unwrap();
    assert_eq!(tags["env"], "prod");
    assert!(!tags.contains_key("bad key") && !tags.contains_key("agentcat:reserved") && !tags.contains_key("multi"));
    assert_eq!(tags["agentcat_session_id_source"], "minted");
    assert_eq!(e["properties"], json!({"tool": "add_todo", "nested": {"flags": ["a", "b"]}}));
}

#[tokio::test]
async fn hook_mode_derives_the_session_and_injects_no_session_id() {
    let agentcat = builder("hook-mode")
        .resolve_session_id(|req| {
            let user = req.argument("text").and_then(Value::as_str).map(|t| format!("conv-{t}"));
            async move { user }
        })
        .build();
    let client = connect(agentcat.track(Todo::new())).await;
    let tools = client.list_all_tools().await.unwrap();
    let add = tools.iter().find(|t| t.name == "add_todo").unwrap();
    assert!(!add.input_schema["properties"].as_object().unwrap().contains_key("session_id"));

    let result = call(&client, "add_todo", json!({"text": "42"})).await.unwrap();
    // No issuance text in hook mode.
    assert_eq!(texts(&result), ["Added: 42"]);

    let e = &api().events_for("hook-mode", 1).await[0];
    assert_eq!(e["session_id"], agentcat::derive_session_id("conv-42", Some(PROJECT_ID)));
    assert_eq!(e["tags"]["agentcat_session_id_source"], "hook");
}

#[tokio::test]
async fn hook_mode_null_mints_silently() {
    let agentcat = builder("hook-null").resolve_session_id(|_req| async move { None }).build();
    let client = connect(agentcat.track(Todo::new())).await;
    let result = call(&client, "add_todo", json!({"text": "a"})).await.unwrap();
    assert_eq!(texts(&result), ["Added: a"]);
    let e = &api().events_for("hook-null", 1).await[0];
    assert!(agentcat::__private::is_valid_session_id(e["session_id"].as_str().unwrap()));
    assert_eq!(e["tags"]["agentcat_session_id_source"], "minted");
}

#[tokio::test]
async fn string_redaction_applies_to_payloads_but_not_protected_fields() {
    let agentcat = builder("redact-strings")
        .identify(|_req| async move { Some(UserIdentity::new("secret-user").data(json!({"email": "secret@x.com"}))) })
        .redact_sensitive_information(|text| text.replace("secret", "[REDACTED]"))
        .build();
    let client = connect(agentcat.track(Todo::new())).await;
    call(&client, "add_todo", json!({"text": "my secret", "context": "a secret plan"})).await.unwrap();

    let e = &api().events_for("redact-strings", 1).await[0];
    assert_eq!(e["parameters"]["request"]["params"]["arguments"]["text"], "my [REDACTED]");
    assert_eq!(e["user_intent"], "a [REDACTED] plan");
    assert_eq!(e["response"]["content"][0]["text"], "Added: my [REDACTED]");
    // Content-block discriminators and protected fields stay intact.
    assert_eq!(e["response"]["content"][0]["type"], "text");
    assert_eq!(e["identify_actor_given_id"], "secret-user");
    assert_eq!(e["identify_data"]["email"], "secret@x.com");
    assert_eq!(e["resource_name"], "add_todo");
}

#[tokio::test]
async fn async_string_redaction_works() {
    let agentcat = builder("redact-async")
        .redact_sensitive_information_async(|text| async move { text.replace("milk", "[x]") })
        .build();
    let client = connect(agentcat.track(Todo::new())).await;
    call(&client, "add_todo", json!({"text": "milk"})).await.unwrap();
    let e = &api().events_for("redact-async", 1).await[0];
    assert_eq!(e["parameters"]["request"]["params"]["arguments"]["text"], "[x]");
    assert_eq!(e["response"]["content"][0]["text"], "Added: [x]");
}

#[tokio::test]
async fn event_redaction_can_rewrite_or_drop_but_not_change_system_fields() {
    let agentcat = builder("redact-event")
        .redact_event(|mut event| {
            if event.resource_name.as_deref() == Some("echo") {
                return None;
            }
            event.response = None;
            event.session_id = "ses_forged".into();
            event.event_type = "forged".into();
            Some(event)
        })
        .build();
    let client = connect(agentcat.track(Todo::new())).await;
    let first = call(&client, "add_todo", json!({"text": "a"})).await.unwrap();
    let session = minted_session(&first);
    call(&client, "echo", json!({"x": 1})).await.unwrap();

    api().assert_count_settles("redact-event", 1).await;
    let e = &api().events_for("redact-event", 1).await[0];
    assert_eq!(e["resource_name"], "add_todo");
    assert!(e.get("response").is_none());
    assert_eq!(e["session_id"], session.as_str());
    assert_eq!(e["event_type"], "mcp:tools/call");
}

#[tokio::test]
async fn a_panicking_event_hook_drops_the_event() {
    let agentcat = builder("redact-panic").redact_event(|_event| panic!("hook bug")).build();
    let client = connect(agentcat.track(Todo::new())).await;
    let result = call(&client, "add_todo", json!({"text": "a"})).await.unwrap();
    assert_eq!(texts(&result)[1], "Added: a");
    api().assert_count_settles("redact-panic", 0).await;
}
