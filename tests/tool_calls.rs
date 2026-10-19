//! End to end: an rmcp client calls a tracked rmcp server over an in-memory
//! transport, and the mock AgentCat API receives the events.

mod common;

use agentcat::constants::{
    AGENT_ID_PARAM_DESCRIPTION, DEFAULT_CONTEXT_PARAMETER_DESCRIPTION, GET_MORE_TOOLS_RESPONSE,
    MINT_BACK_HEADER_ISSUED, MINT_BACK_HEADER_UNRECOGNIZED, SESSION_ID_PARAM_DESCRIPTION,
    SESSION_ID_PARAM_PATTERN,
};
use common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn tools_list_advertises_injected_parameters_and_get_more_tools() {
    let agentcat = builder("list-basic").build();
    let client = connect(agentcat.track(Todo::new())).await;
    let tools = client.list_all_tools().await.unwrap();

    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names.last(), Some(&"get_more_tools"), "{names:?}");

    let add = tools.iter().find(|t| t.name == "add_todo").unwrap();
    let schema = Value::Object((*add.input_schema).clone());
    let props = schema["properties"].as_object().unwrap();
    // Customer params first, then session_id, then context.
    let keys: Vec<&str> = props.keys().map(String::as_str).collect();
    assert_eq!(keys, ["text", "session_id", "context"]);
    assert_eq!(props["session_id"]["description"], SESSION_ID_PARAM_DESCRIPTION);
    assert_eq!(props["session_id"]["pattern"], SESSION_ID_PARAM_PATTERN);
    assert_eq!(props["context"]["description"], DEFAULT_CONTEXT_PARAMETER_DESCRIPTION);
    assert_eq!(schema["required"], json!(["text", "session_id", "context"]));
    // agent_id is opt-in.
    assert!(props.get("agent_id").is_none());

    // get_more_tools: handles injected, its own context kept.
    let gmt = tools.iter().find(|t| t.name == "get_more_tools").unwrap();
    let gmt_schema = Value::Object((*gmt.input_schema).clone());
    assert_eq!(gmt_schema["required"], json!(["context", "session_id"]));
    assert_eq!(gmt.annotations.as_ref().unwrap().read_only_hint, Some(true));

    // Structured tools declare mcp_session in their output schema.
    let count = tools.iter().find(|t| t.name == "count_todos").unwrap();
    let output = Value::Object((**count.output_schema.as_ref().unwrap()).clone());
    assert!(output["properties"]["mcp_session"].is_object(), "{output}");

    // tools/list publishes nothing.
    api().assert_count_settles("list-basic", 0).await;
}

#[tokio::test]
async fn first_call_issues_a_session_and_publishes_one_event() {
    let agentcat = builder("first-call").build();
    let client = connect(agentcat.track(Todo::new())).await;

    let result = call(&client, "add_todo", json!({"text": "milk", "context": "Adding milk to the shared grocery list."}))
        .await
        .unwrap();
    let texts = texts(&result);
    assert!(texts[0].starts_with(MINT_BACK_HEADER_ISSUED), "{texts:?}");
    assert_eq!(texts[1], "Added: milk");
    let session = minted_session(&result);
    assert!(agentcat::__private::is_valid_session_id(&session));

    let events = api().events_for("first-call", 1).await;
    let e = &events[0];
    assert_eq!(e["project_id"], PROJECT_ID);
    assert_eq!(e["event_type"], "mcp:tools/call");
    assert_eq!(e["session_id"], session.as_str());
    assert_eq!(e["resource_name"], "add_todo");
    assert_eq!(e["user_intent"], "Adding milk to the shared grocery list.");
    assert_eq!(e["sdk_language"], "Rust");
    assert_eq!(e["agentcat_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(e["server_version"], "1.0.0");
    assert_eq!(e["client_name"], "test-client");
    assert_eq!(e["client_version"], "9.9.9");
    assert_eq!(e["tags"]["agentcat_session_id_source"], "minted");
    assert!(e["id"].as_str().unwrap().starts_with("evt_"));
    assert!(e["timestamp"].is_string());
    assert!(e["duration"].is_number());
    assert_eq!(e["identify_data"], json!({}));
    // The event records the raw request (injected params included) and the
    // customer's original result (no mint-back block).
    assert_eq!(e["parameters"]["request"]["method"], "tools/call");
    assert_eq!(e["parameters"]["request"]["params"]["name"], "add_todo");
    assert_eq!(
        e["parameters"]["request"]["params"]["arguments"],
        json!({"text": "milk", "context": "Adding milk to the shared grocery list."})
    );
    assert_eq!(e["parameters"]["extra"]["mcpReq"]["method"], "tools/call");
    assert_eq!(e["response"]["content"], json!([{"type": "text", "text": "Added: milk"}]));
    // Token estimates: raw args JSON and the content text.
    let args_len = json!({"text": "milk", "context": "Adding milk to the shared grocery list."}).to_string().len();
    assert_eq!(e["input_tokens"], ((args_len as f64) / 3.5).ceil() as u64);
    assert_eq!(e["output_tokens"], ((("Added: milk".len()) as f64) / 3.5).ceil() as u64);
    // Only snake_case keys go on the wire.
    assert!(e.get("projectId").is_none() && e.get("sessionId").is_none());
}

#[tokio::test]
async fn supplied_session_is_echoed_and_stripped_from_the_handler() {
    let agentcat = builder("supplied").build();
    let client = connect(agentcat.track(Todo::new())).await;

    let first = call(&client, "add_todo", json!({"text": "a", "session_id": "start"})).await.unwrap();
    let session = minted_session(&first);

    // A strict (deny_unknown_fields) handler accepts the call: AgentCat's
    // parameters never reach it.
    let second = call(&client, "add_todo", json!({"text": "b", "session_id": session, "context": "Second item."}))
        .await
        .unwrap();
    assert_eq!(texts(&second), ["Added: b"]);
    assert_ne!(second.is_error, Some(true));

    let events = api().events_for("supplied", 2).await;
    let second_event = events.iter().find(|e| e["parameters"]["request"]["params"]["arguments"]["text"] == "b").unwrap();
    assert_eq!(second_event["session_id"], session.as_str());
    assert_eq!(second_event["tags"]["agentcat_session_id_source"], "supplied");
}

#[tokio::test]
async fn echo_handler_sees_stripped_arguments() {
    let agentcat = builder("echo-strip").enable_agent_tracking(true).build();
    let client = connect(agentcat.track(Todo::new())).await;
    let _ = client.list_all_tools().await.unwrap();
    let result = call(
        &client,
        "echo",
        json!({"x": 1, "session_id": "start", "agent_id": "opus|cc|k3n9x", "context": "Testing."}),
    )
    .await
    .unwrap();
    let echoed: Value = serde_json::from_str(texts(&result).last().unwrap()).unwrap();
    assert_eq!(echoed, json!({"x": 1}));

    let events = api().events_for("echo-strip", 1).await;
    assert_eq!(events[0]["tags"]["agentcat_agent_id"], "opus|cc|k3n9x");
    assert_eq!(events[0]["tags"]["agentcat_agent_id_source"], "supplied");
}

#[tokio::test]
async fn unrecognized_session_is_reported_and_published_without_a_session() {
    let agentcat = builder("unrecognized").build();
    let client = connect(agentcat.track(Todo::new())).await;
    let result = call(&client, "add_todo", json!({"text": "x", "session_id": "ses_made_up"})).await.unwrap();
    assert!(texts(&result)[0].starts_with(MINT_BACK_HEADER_UNRECOGNIZED));

    let events = api().events_for("unrecognized", 1).await;
    // An explicit null tells the API "no session" (stateless mode).
    assert!(events[0].get("session_id").is_some_and(Value::is_null), "{}", events[0]);
    assert_eq!(events[0]["tags"]["agentcat_session_id_source"], "invalid");
}

#[tokio::test]
async fn structured_results_mirror_mcp_session_as_the_first_key() {
    let agentcat = builder("structured").enable_agent_tracking(true).build();
    let client = connect(agentcat.track(Todo::new())).await;
    let _ = client.list_all_tools().await.unwrap();
    let result = call(&client, "count_todos", json!({"agent_id": "a|b|c"})).await.unwrap();
    let sc = result.structured_content.clone().unwrap();
    let text = serde_json::to_string(&sc).unwrap();
    assert!(text.starts_with(r#"{"mcp_session":{"session_id":"ses_"#), "{text}");
    assert_eq!(sc["mcp_session"]["status"], "issued");
    assert_eq!(sc["mcp_session"]["agent_id"], "a|b|c");
    assert_eq!(sc["count"], 0);

    // The event keeps the original structured content.
    let events = api().events_for("structured", 1).await;
    assert!(events[0]["response"]["structuredContent"].get("mcp_session").is_none());
}

#[tokio::test]
async fn get_more_tools_answers_and_records_intent() {
    let agentcat = builder("report-missing").build();
    let client = connect(agentcat.track(Todo::new())).await;
    let result = call(&client, "get_more_tools", json!({"context": "Needs a calendar tool."})).await.unwrap();
    let texts = texts(&result);
    assert!(texts[0].starts_with(MINT_BACK_HEADER_ISSUED));
    assert_eq!(texts[1], GET_MORE_TOOLS_RESPONSE);

    let events = api().events_for("report-missing", 1).await;
    assert_eq!(events[0]["resource_name"], "get_more_tools");
    assert_eq!(events[0]["user_intent"], "Needs a calendar tool.");
}

#[tokio::test]
async fn tool_error_results_are_captured() {
    let agentcat = builder("tool-error").build();
    let client = connect(agentcat.track(Todo::new())).await;
    let result = call(&client, "fail", json!({})).await.unwrap();
    assert_eq!(result.is_error, Some(true));
    // The retry after an error must carry the same task: mint-back applies.
    assert!(texts(&result)[0].starts_with(MINT_BACK_HEADER_ISSUED));

    let events = api().events_for("tool-error", 1).await;
    assert_eq!(events[0]["is_error"], true);
    assert_eq!(events[0]["error"], json!({"message": "disk full", "platform": "rust"}));
}

#[tokio::test]
async fn protocol_errors_reach_the_client_unchanged_and_are_captured() {
    let agentcat = builder("protocol-error").build();
    let client = connect(agentcat.track(Todo::new())).await;
    let err = call(&client, "reject", json!({})).await.unwrap_err();
    assert!(err.to_string().contains("bad input"), "{err}");

    let events = api().events_for("protocol-error", 1).await;
    assert_eq!(events[0]["is_error"], true);
    assert_eq!(events[0]["error"]["type"], "McpError");
    assert_eq!(events[0]["error"]["message"], "MCP error -32602: bad input");
    assert!(events[0].get("response").is_none());
}

#[tokio::test]
async fn unknown_tools_are_captured() {
    let agentcat = builder("unknown-tool").build();
    let client = connect(agentcat.track(Todo::new())).await;
    let err = call(&client, "nope", json!({})).await;
    assert!(err.is_err());
    let events = api().events_for("unknown-tool", 1).await;
    assert_eq!(events[0]["resource_name"], "nope");
    assert_eq!(events[0]["is_error"], true);
    assert_eq!(events[0]["error"]["type"], "McpError");
}

#[tokio::test]
async fn panics_are_captured_and_still_propagate() {
    let agentcat = builder("panic").build();
    let client = connect(agentcat.track(Todo::new())).await;
    // rmcp drops the request when a handler panics; the client times out.
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), call(&client, "explode", json!({}))).await;
    assert!(outcome.is_err() || outcome.unwrap().is_err());

    let events = api().events_for("panic", 1).await;
    let e = &events[0];
    assert_eq!(e["is_error"], true);
    assert_eq!(e["error"]["type"], "panic");
    assert_eq!(e["error"]["message"], "kaboom");
    let frames = e["error"]["frames"].as_array().expect("frames");
    assert!(frames.iter().any(|f| f["in_app"] == true && f["filename"].as_str().unwrap().contains("common")), "{frames:#?}");
}

#[tokio::test]
async fn tracing_disabled_publishes_nothing_and_still_strips_context() {
    let agentcat = builder("tracing-off").enable_tracing(false).build();
    let client = connect(agentcat.track(Todo::new())).await;
    let tools = client.list_all_tools().await.unwrap();
    let add = tools.iter().find(|t| t.name == "add_todo").unwrap();
    let props = add.input_schema["properties"].as_object().unwrap();
    // With tracing off there are no handles, but intent capture still injects context.
    assert!(props.contains_key("context") && !props.contains_key("session_id"));

    let result = call(&client, "add_todo", json!({"text": "y", "context": "Some intent."})).await.unwrap();
    assert_eq!(texts(&result), ["Added: y"]);
    let gmt = call(&client, "get_more_tools", json!({"context": "x"})).await.unwrap();
    assert_eq!(texts(&gmt), [GET_MORE_TOOLS_RESPONSE]);
    api().assert_count_settles("tracing-off", 0).await;
}

#[tokio::test]
async fn options_turn_off_context_and_report_missing() {
    let agentcat = builder("opts-off")
        .enable_tool_call_context(false)
        .enable_report_missing(false)
        .custom_context_description("unused")
        .build();
    let client = connect(agentcat.track(Todo::new())).await;
    let tools = client.list_all_tools().await.unwrap();
    assert!(tools.iter().all(|t| t.name != "get_more_tools"));
    let add = tools.iter().find(|t| t.name == "add_todo").unwrap();
    let keys: Vec<&String> = add.input_schema["properties"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["text", "session_id"]);
}

#[tokio::test]
async fn agent_tracking_injects_agent_id() {
    let agentcat = builder("agent-tracking").enable_agent_tracking(true).build();
    let client = connect(agentcat.track(Todo::new())).await;
    let tools = client.list_all_tools().await.unwrap();
    let add = tools.iter().find(|t| t.name == "add_todo").unwrap();
    let props = add.input_schema["properties"].as_object().unwrap();
    let keys: Vec<&String> = props.keys().collect();
    assert_eq!(keys, ["text", "session_id", "agent_id", "context"]);
    assert_eq!(props["agent_id"]["description"], AGENT_ID_PARAM_DESCRIPTION);
}

#[tokio::test]
async fn custom_context_description_is_advertised() {
    let agentcat = builder("custom-context").custom_context_description("Why are you calling this?").build();
    let client = connect(agentcat.track(Todo::new())).await;
    let tools = client.list_all_tools().await.unwrap();
    let add = tools.iter().find(|t| t.name == "add_todo").unwrap();
    assert_eq!(add.input_schema["properties"]["context"]["description"], "Why are you calling this?");
}

#[tokio::test]
async fn a_call_before_any_listing_still_strips() {
    // A fresh instance that never served tools/list (per-request factories)
    // recomputes the injection from get_tool.
    let agentcat = builder("no-listing").build();
    let client = connect(agentcat.track(Todo::new())).await;
    let result = call(&client, "add_todo", json!({"text": "z", "session_id": "start", "context": "Pre-listing call."}))
        .await
        .unwrap();
    assert_eq!(texts(&result)[1], "Added: z");
    let events = api().events_for("no-listing", 1).await;
    assert_eq!(events[0]["user_intent"], "Pre-listing call.");
}

#[tokio::test]
async fn concurrent_calls_never_cross_attribute() {
    let agentcat = builder("concurrent").build();
    let client = connect(agentcat.track(Todo::new())).await;
    let mut sessions = Vec::new();
    for i in 0..5 {
        let r = call(&client, "add_todo", json!({"text": format!("seed{i}")})).await.unwrap();
        sessions.push(minted_session(&r));
    }
    let calls = sessions.iter().enumerate().map(|(i, s)| {
        let client = &client;
        async move { call(client, "add_todo", json!({"text": format!("task{i}"), "session_id": s})).await.unwrap() }
    });
    futures::future::join_all(calls).await;
    let events = api().events_for("concurrent", 10).await;
    for (i, s) in sessions.iter().enumerate() {
        let e = events
            .iter()
            .find(|e| e["parameters"]["request"]["params"]["arguments"]["text"] == format!("task{i}"))
            .unwrap();
        assert_eq!(e["session_id"], s.as_str());
    }
}
