use std::ffi::OsString;

use axum::http::{Method, StatusCode};

use super::client::{EventStream, json_rpc_message};
use super::judge::{event_type, issued_session_id, resource_name};
use super::mock::{PUBLISH_PATH, Recorded};
use super::sandbox::is_loopback_url;
use super::*;

const STDIO: Example = EXAMPLES[0];
const HOOKS: Example = EXAMPLES[2];

/// A recorded request with `body` (JSON text, or any text), received at `received`.
fn request(method: Method, path: &str, body: &str, received: Instant) -> Recorded {
    Recorded {
        method,
        path: path.to_owned(),
        received,
        body: serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_owned())),
        raw: body.to_owned(),
    }
}

/// A publish of a tool call of `tool`, for the project `project`.
fn tool_publish(tool: &str, project: &str, received: Instant) -> Recorded {
    let body = json!({
        "project_id": project,
        "event_type": "mcp:tools/call",
        "resource_name": tool,
    });
    request(Method::POST, PUBLISH_PATH, &body.to_string(), received)
}

/// The tool-call publishes of the plain calls (`add`, `echo`, `add`), at `received`.
fn plain_publishes(received: Instant) -> Vec<Recorded> {
    ["add", "echo", "add"]
        .into_iter()
        .map(|tool| tool_publish(tool, PROJECT_ID, received))
        .collect()
}

/// A `CallToolResult` with the text blocks `texts` and `isError`.
fn result(texts: &[&str], is_error: bool) -> Value {
    let content: Vec<Value> = texts
        .iter()
        .map(|text| json!({"type": "text", "text": text}))
        .collect();
    json!({"content": content, "isError": is_error})
}

const ISSUED: &str = "[session_id issued: ses_2abcDEF_9 — pass it back]\nsession_id: ses_2abcDEF_9";

#[test]
fn the_first_call_issues_a_session_and_keeps_the_result() {
    let calls = STDIO.calls();
    let mut session = None;
    let failures = judge_call(&calls[0], &result(&[ISSUED, "5"], false), &mut session);
    assert_eq!(failures, Vec::<String>::new());
    assert_eq!(session.as_deref(), Some("ses_2abcDEF_9"));
    let failures = judge_call(&calls[1], &result(&["hello"], false), &mut session);
    assert_eq!(failures, Vec::<String>::new());
}

#[test]
fn a_first_call_whose_session_block_replaces_the_result_fails() {
    let calls = STDIO.calls();
    let mut session = None;
    let failures = judge_call(&calls[0], &result(&[ISSUED], false), &mut session);
    assert_eq!(
        failures,
        [r#"add returned [] (isError false), expected "5" (isError false)"#]
    );
}

#[test]
fn a_first_call_that_issues_no_session_id_fails_once() {
    let calls = STDIO.calls();
    let mut session = None;
    let failures = judge_call(&calls[0], &result(&["5"], false), &mut session);
    assert_eq!(
        failures,
        [r#"the first call (add) did not issue a session ID: "5""#]
    );
    // The later calls send no session ID and are judged on their first block.
    assert_eq!(session.as_deref(), Some(""));
    assert!(!call_arguments(&calls[1], session.as_deref()).contains_key("session_id"));
    let failures = judge_call(&calls[1], &result(&["hello"], false), &mut session);
    assert_eq!(failures, Vec::<String>::new());
}

#[test]
fn a_wrong_text_or_is_error_fails() {
    let calls = HOOKS.calls();
    let mut session = Some("ses_1".to_owned());
    let failures = judge_call(
        &calls[2],
        &result(&["the record does not exist"], false),
        &mut session,
    );
    assert_eq!(
        failures,
        [
            r#"fail returned "the record does not exist" (isError false), expected "the record does not exist" (isError true)"#
        ]
    );
    let failures = judge_call(&calls[0], &result(&["5", "extra"], false), &mut session);
    assert_eq!(
        failures,
        [r#"add returned ["5","extra"] (isError false), expected "5" (isError false)"#]
    );
    let failures = judge_call(
        &calls[2],
        &result(&["the record does not exist"], true),
        &mut session,
    );
    assert_eq!(failures, Vec::<String>::new());
}

#[test]
fn a_session_id_is_the_ses_word_after_the_mark() {
    assert_eq!(issued_session_id(ISSUED).as_deref(), Some("ses_2abcDEF_9"));
    assert_eq!(
        issued_session_id("session_id: ses_ no; session_id: ses_x1").as_deref(),
        Some("ses_x1")
    );
    assert_eq!(issued_session_id("session_id: ses_"), None);
    assert_eq!(issued_session_id("session_id: abc"), None);
    assert_eq!(issued_session_id("5"), None);
}

#[test]
fn every_call_sends_the_context_and_later_ones_the_session() {
    let calls = STDIO.calls();
    let first = call_arguments(&calls[0], None);
    assert_eq!(
        Value::Object(first),
        json!({"a": 2, "b": 3, "context": "Checking the example"})
    );
    let later = call_arguments(&calls[1], Some("ses_1"));
    assert_eq!(
        Value::Object(later),
        json!({"text": "hello", "context": "Checking the example", "session_id": "ses_1"})
    );
}

#[test]
fn tools_list_must_list_every_tool() {
    let tools = json!([{"name": "add"}, {"name": "echo"}]);
    assert_eq!(judge_tools(&tools, STDIO.tools()), Vec::<String>::new());
    assert_eq!(
        judge_tools(&tools, HOOKS.tools()),
        [r#"tools/list does not list fail: ["add","echo"]"#]
    );
}

#[test]
fn only_json_rpc_lines_pass() {
    let line = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
    assert_eq!(
        json_rpc_message(line),
        Some(json!({"jsonrpc": "2.0", "id": 1, "result": {}}))
    );
    for line in [
        "starting up",
        r#"{"jsonrpc":"1.0"}"#,
        r#"[{"jsonrpc":"2.0"}]"#,
        "null",
    ] {
        assert_eq!(json_rpc_message(line), None, "{line}");
    }
}

#[test]
fn an_event_stream_gives_the_data_of_each_whole_event() {
    let mut events = EventStream::default();
    events.push(b"id: 0\r\nretry: 3000\r\ndata:\r\n\r\nevent: message\ndata: {\"id\":1,");
    assert_eq!(events.take_data(), Vec::<String>::new());
    events.push(b"\ndata: \"result\":{}}\n\ndata: x");
    assert_eq!(events.take_data(), ["{\"id\":1,\n\"result\":{}}"]);
    events.push(b"\n\n");
    assert_eq!(events.take_data(), ["x"]);
}

#[test]
fn one_publish_per_call_before_the_exit_passes() {
    let at = Instant::now();
    let exit = at + Duration::from_millis(5);
    let failures = judge_publishes(&plain_publishes(at), &STDIO.calls(), Some(exit));
    assert_eq!(failures, Vec::<String>::new());
}

#[test]
fn a_missing_publish_fails() {
    let at = Instant::now();
    let mut requests = plain_publishes(at);
    requests.remove(1);
    let failures = judge_publishes(&requests, &STDIO.calls(), Some(at));
    assert_eq!(
        failures,
        [r#"publishes for the tool calls: got ["add","add"], expected ["add","echo","add"]"#]
    );
}

#[test]
fn a_request_other_than_a_publish_fails() {
    let at = Instant::now();
    let mut requests = plain_publishes(at);
    requests.push(request(Method::GET, "/v1/other", "", at));
    requests.push(request(Method::GET, PUBLISH_PATH, "", at));
    let failures = judge_publishes(&requests, &STDIO.calls(), Some(at));
    assert_eq!(
        failures,
        ["the mock received 2 request(s) that are not publishes"]
    );
}

#[test]
fn the_last_publish_after_the_exit_fails() {
    let at = Instant::now();
    let mut requests = plain_publishes(at);
    requests[2].received = at + Duration::from_millis(10);
    let failures = judge_publishes(&requests, &STDIO.calls(), Some(at));
    assert_eq!(
        failures,
        ["the publish of the last call arrived after the example exited"]
    );
    // An example that did not exit in time has no exit to compare with.
    assert_eq!(
        judge_publishes(&requests, &STDIO.calls(), None),
        Vec::<String>::new()
    );
}

#[test]
fn a_publish_for_another_project_fails() {
    let at = Instant::now();
    let mut requests = plain_publishes(at);
    requests[0] = tool_publish("add", "proj_other", at);
    let custom = json!({"project_id": PROJECT_ID, "event_type": "agentcat:custom"});
    requests.push(request(Method::POST, PUBLISH_PATH, &custom.to_string(), at));
    let failures = judge_publishes(&requests, &STDIO.calls(), Some(at));
    assert_eq!(
        failures,
        [r#"a publish names the project "proj_other", expected proj_examples"#]
    );
}

/// The publishes of a `hooks` run where every hook ran.
fn hooks_publishes() -> Vec<Recorded> {
    let at = Instant::now();
    let tool = |name: &str, extra: Value| {
        let mut body = json!({
            "project_id": PROJECT_ID,
            "event_type": "mcp:tools/call",
            "resource_name": name,
            "identify_actor_given_id": "demo-user",
            "identify_actor_name": "Demo User",
            "tags": {"example": "hooks"},
            "properties": {"source": "hooks-example", "redacted_by": "example"},
        });
        if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
            body.extend(extra.clone());
        }
        request(Method::POST, PUBLISH_PATH, &body.to_string(), at)
    };
    let custom = json!({
        "project_id": PROJECT_ID,
        "event_type": "agentcat:custom",
        "resource_name": "echo-called",
    });
    vec![
        tool("add", json!({})),
        tool(
            "echo",
            json!({"parameters": {"text": "my [REDACTED] is here"}}),
        ),
        request(Method::POST, PUBLISH_PATH, &custom.to_string(), at),
        tool(
            "fail",
            json!({"is_error": true, "error": {"type": "std::io::Error"}}),
        ),
    ]
}

#[test]
fn every_hook_value_passes() {
    assert_eq!(judge_hooks(&hooks_publishes()), Vec::<String>::new());
}

#[test]
fn a_missing_hook_value_fails() {
    let mut requests = hooks_publishes();
    let body = &mut requests[0].body;
    body["identify_actor_given_id"] = json!("someone");
    body["identify_actor_name"] = Value::Null;
    body["tags"] = json!({});
    body["properties"] = json!({"source": "other"});
    assert_eq!(
        judge_hooks(&requests),
        [
            r#"add: identify user_id is not "demo-user""#,
            r#"add: identify user_name is not "Demo User""#,
            r#"add: the tag example is not "hooks""#,
            r#"add: the property source is not "hooks-example""#,
            "add: the redact_event hook did not run",
        ]
    );
}

#[test]
fn an_unredacted_secret_fails() {
    let mut requests = hooks_publishes();
    requests[1].raw = requests[1].raw.replace("[REDACTED]", "secret-token");
    assert_eq!(
        judge_hooks(&requests),
        [
            "a publish still holds secret-token",
            "no publish holds [REDACTED]"
        ]
    );
}

#[test]
fn a_fail_call_without_its_error_fails() {
    let mut requests = hooks_publishes();
    requests[3].body["error"] = json!({"type": ""});
    assert_eq!(
        judge_hooks(&requests),
        ["the fail call has no error recorded by capture_error"]
    );
    requests[3].body["error"] = json!({"name": "NotFound"});
    requests[3].body["is_error"] = json!(false);
    assert_eq!(
        judge_hooks(&requests),
        ["the fail call has no error recorded by capture_error"]
    );
    requests.remove(3);
    assert_eq!(
        judge_hooks(&requests),
        ["the fail call has no error recorded by capture_error"]
    );
}

#[test]
fn the_custom_event_must_come_once_with_its_name() {
    let mut requests = hooks_publishes();
    requests[2].body["resource_name"] = json!("other");
    assert_eq!(
        judge_hooks(&requests),
        [r#"the custom event has resource "other", expected "echo-called""#]
    );
    requests.remove(2);
    assert_eq!(judge_hooks(&requests), ["expected 1 custom event, got 0"]);
}

#[test]
fn only_the_wire_names_of_the_event_fields_count() {
    let event = json!({
        "eventType": "mcp:tools/call",
        "event_type": "other",
        "resourceName": "camel",
        "resource_name": null,
    });
    assert_eq!(event_type(&event), Some("other"));
    assert_eq!(resource_name(&event), None);
}

#[test]
fn a_tool_event_without_a_resource_name_is_called_absent() {
    let mut requests = hooks_publishes();
    requests[0].body["resource_name"] = Value::Null;
    requests[0].body["identifyActorGivenId"] = json!("demo-user");
    requests[0].body["identify_actor_given_id"] = Value::Null;
    assert_eq!(
        judge_hooks(&requests),
        [r#"<absent>: identify user_id is not "demo-user""#]
    );
}

#[test]
fn the_example_env_removes_every_agentcat_and_mcpcat_variable() {
    let inherited = [
        "PATH",
        "AGENTCAT_PROJECT_ID",
        "AGENTCAT_ENABLED",
        "AGENTCAT_API_URL",
        "MCPCAT_API_URL",
        "MCPCAT_PROJECT_ID",
        "agentcat_lower",
        "MY_AGENTCAT_X",
    ]
    .map(OsString::from);
    let home = Path::new("/tmp/examples-home");
    let env = example_env(inherited, "http://127.0.0.1:4100", home, None).unwrap();
    let removed: Vec<&str> = env.remove.iter().filter_map(|n| n.to_str()).collect();
    assert_eq!(
        removed,
        [
            "AGENTCAT_PROJECT_ID",
            "AGENTCAT_ENABLED",
            "AGENTCAT_API_URL",
            "MCPCAT_API_URL",
            "MCPCAT_PROJECT_ID",
            "agentcat_lower",
            "PORT",
        ]
    );
    let set: Vec<(&str, &str)> = env
        .set
        .iter()
        .map(|(name, value)| (*name, value.to_str().unwrap()))
        .collect();
    assert_eq!(
        set,
        [
            ("HOME", "/tmp/examples-home"),
            ("USERPROFILE", "/tmp/examples-home"),
            ("AGENTCAT_API_URL", "http://127.0.0.1:4100"),
            ("AGENTCAT_PROJECT_ID", "proj_examples"),
        ]
    );
}

#[test]
fn an_http_example_gets_its_port() {
    let env = example_env([], "http://127.0.0.1:4100", Path::new("/h"), Some(8123)).unwrap();
    assert!(env.remove.is_empty(), "{:?}", env.remove);
    assert_eq!(env.set.last(), Some(&("PORT", OsString::from("8123"))));
}

#[test]
fn the_example_env_refuses_an_api_url_that_is_not_loopback() {
    for url in [
        "",
        "https://api.agentcat.com",
        "http://api.agentcat.com",
        "http://10.0.0.1:80",
    ] {
        let error = example_env([], url, Path::new("/h"), None).unwrap_err();
        assert!(
            error.starts_with("refusing to start the example"),
            "{url}: {error}"
        );
    }
}

#[test]
fn a_loopback_url_names_a_loopback_host() {
    for url in [
        "http://127.0.0.1:4100",
        "http://127.0.0.1",
        "http://127.8.9.10:80/v1",
        "http://localhost:3000",
        "http://LOCALHOST",
        "http://[::1]:8080",
        "http://[::1]",
    ] {
        assert!(is_loopback_url(url), "{url}");
    }
    for url in [
        "",
        "127.0.0.1:4100",
        "https://127.0.0.1",
        "http://127.0.0.1.example.com",
        "http://localhost.example.com:80",
        "http://127.0.0.1@example.com",
        "http://user@127.0.0.1",
        "http://[::2]:80",
        "http://[::1]x",
        "http://0.0.0.0:80",
        "http://128.0.0.1",
    ] {
        assert!(!is_loopback_url(url), "{url}");
    }
}

#[test]
fn the_executable_comes_from_the_compiler_artifact_of_the_example() {
    let artifact = |name: &str, kind: &str, executable: Value| {
        json!({
            "reason": "compiler-artifact",
            "target": {"name": name, "kind": [kind]},
            "executable": executable,
        })
    };
    let messages = [
        artifact("agentcat_sdk", "lib", Value::Null),
        artifact("hooks", "example", json!("/t/debug/examples/hooks")),
        artifact("stdio", "bin", json!("/t/debug/stdio")),
        artifact("stdio", "example", json!("/t/debug/examples/stdio")),
        json!({"reason": "build-finished", "success": true}),
    ]
    .map(|message| message.to_string())
    .join("\n");
    assert_eq!(
        example_executable(&messages, "stdio"),
        Some(PathBuf::from("/t/debug/examples/stdio"))
    );
    assert_eq!(example_executable(&messages, "stateless_http"), None);
}

#[test]
fn an_unknown_example_is_a_usage_error() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for args in [
        &[][..],
        &["sse".to_owned()][..],
        &["stdio".to_owned(), "x".to_owned()][..],
        &["--all".to_owned(), "stdio".to_owned()][..],
    ] {
        let (code, lines) = run(args, root);
        assert_eq!(code, 2);
        assert_eq!(
            lines,
            [
                "usage: cargo run -q -p xtask -- examples-check <--all | stdio | streamable_http | hooks | stateless_http>"
            ]
        );
    }
}

#[test]
fn all_selects_every_example_in_order_and_a_name_selects_one() {
    let names = |args: &[&str]| {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        selected(&args).map(|examples| examples.iter().map(|e| e.name).collect::<Vec<_>>())
    };
    assert_eq!(
        names(&["--all"]),
        Some(vec!["stdio", "streamable_http", "hooks", "stateless_http"])
    );
    assert_eq!(names(&["hooks"]), Some(vec!["hooks"]));
    assert_eq!(names(&["sse"]), None);
    assert_eq!(names(&["all"]), None);
    assert_eq!(names(&[]), None);
    assert_eq!(names(&["--all", "--all"]), None);
}

#[test]
fn a_passing_run_prints_ok_and_a_failing_run_prints_each_failure() {
    let passed = Outcome {
        exit_after: Some(Duration::from_millis(42)),
        stderr: "listening".to_owned(),
        ..Outcome::default()
    };
    let stop = if cfg!(unix) { "SIGTERM" } else { "process end" };
    assert_eq!(
        report(EXAMPLES[1], &passed),
        (
            0,
            vec![format!("ok streamable_http (exit 42 ms after the {stop})")]
        )
    );
    let failed = Outcome {
        failures: vec!["one".to_owned(), "two".to_owned()],
        fatal: Some("stopped".to_owned()),
        stderr: "boom\n".to_owned(),
        exit_after: None,
    };
    assert_eq!(
        report(STDIO, &failed),
        (
            1,
            vec![
                "--- stderr of the example ---\nboom".to_owned(),
                "FAIL stdio: one".to_owned(),
                "FAIL stdio: two".to_owned(),
                "examples-check: stopped".to_owned(),
            ]
        )
    );
}

#[test]
fn the_mock_answers_a_publish_with_202_and_records_every_request() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mock = MockApi::start().await.unwrap();
        assert!(is_loopback_url(&mock.url), "{}", mock.url);
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let published = http
            .post(format!("{}{PUBLISH_PATH}", mock.url))
            .body(r#"{"event_type":"mcp:tools/call"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(published.status(), StatusCode::ACCEPTED);
        assert_eq!(published.text().await.unwrap(), r#"{"status":"accepted"}"#);
        let other = http
            .get(format!("{}/v1/other", mock.url))
            .send()
            .await
            .unwrap();
        assert_eq!(other.status(), StatusCode::NOT_FOUND);
        let requests = mock.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].is_publish());
        assert_eq!(requests[0].body, json!({"event_type": "mcp:tools/call"}));
        assert!(!requests[1].is_publish());
        assert_eq!(requests[1].body, Value::Null);
        assert!(requests[0].received <= requests[1].received);
    });
}

#[test]
fn the_temporary_home_is_removed_after_the_run() {
    let home = TempHome::new().unwrap();
    let path = home.0.clone();
    std::fs::write(path.join("agentcat.log"), "line\n").unwrap();
    drop(home);
    assert!(!path.exists());
}
