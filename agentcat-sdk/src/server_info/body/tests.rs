//! The body of a heartbeat: its six keys, the tool list as `tools/list` shows it, the lists a
//! read left out, and the smaller body that follows a `413`.
use rmcp::model::{Implementation, Tool};
use serde_json::{Value, json};

use super::{body, tool_count, without_schemas};
use crate::config::Options;
use crate::constants::{GET_MORE_TOOLS_NAME, get_more_tools_descriptor};
use crate::inject::{InjectConfig, InjectState, build_injected_list};
use crate::server_info::lists::Lists;
use crate::test_support::server_info::{Demo, LONG, PROJECT, beta, tool};

/// The server of the bodies.
fn server() -> Implementation {
    Implementation::new("demo-server", "1.2.3").with_title("Demo Title")
}

/// The body of `lists` for [`server`] with the instructions `instructions`, under `options`, as
/// JSON.
fn body_json(lists: Lists, instructions: Option<&str>, options: &Options) -> Value {
    let inject = InjectConfig::from_options(options);
    let request = body(
        PROJECT,
        server(),
        instructions.map(str::to_owned),
        lists,
        &inject,
        &mut InjectState::default(),
    )
    .expect("the body is built");
    serde_json::to_value(request).expect("the body is JSON")
}

/// What `tools/list` shows for the one page `tools` under `options`, as JSON.
fn listed(tools: Vec<Tool>, options: &Options) -> Value {
    let inject = InjectConfig::from_options(options);
    let list = build_injected_list(&inject, tools, &mut InjectState::default());
    serde_json::to_value(list.tools).expect("the tools are JSON")
}

/// The names of the tools of `body`.
fn tool_names(body: &Value) -> Vec<&str> {
    body["features"]["tools"]
        .as_array()
        .expect("the body has tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("a tool has a name"))
        .collect()
}

/// The lists of the fixture server.
fn demo_lists() -> Lists {
    let demo = Demo::new();
    Lists {
        tool_pages: demo.tool_pages.clone(),
        resources: Some(demo.resource_items.clone()),
        resource_templates: Some(Vec::new()),
        prompts: Some(Vec::new()),
        left_out: Vec::new(),
    }
}

#[test]
fn v1_the_body_holds_the_six_keys_and_the_listed_tools() {
    let options = Options::default();
    let body = body_json(demo_lists(), Some("Use the demo tools."), &options);
    let keys: Vec<&str> = body
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "project_id",
            "server_name",
            "server_version",
            "server_title",
            "instructions",
            "features"
        ]
    );
    assert_eq!(body["project_id"], PROJECT);
    assert_eq!(body["server_name"], "demo-server");
    assert_eq!(body["server_version"], "1.2.3");
    assert_eq!(body["server_title"], "Demo Title");
    assert_eq!(body["instructions"], "Use the demo tools.");
    assert_eq!(
        body["features"]["tools"],
        listed(Demo::new().tool_pages.remove(0), &options)
    );
    assert_eq!(
        tool_names(&body),
        ["alpha", "beta", "gamma", GET_MORE_TOOLS_NAME]
    );
    let beta = &body["features"]["tools"][1];
    assert_eq!(beta["description"], "b".repeat(LONG));
    assert_eq!(beta["title"], "Beta Tool");
    assert_eq!(beta["annotations"]["readOnlyHint"], true);
    assert!(beta["outputSchema"].is_object());
    for injected in ["session_id", "context"] {
        assert!(
            beta["inputSchema"]["properties"][injected].is_object(),
            "{injected}"
        );
    }
    assert_eq!(
        body["features"]["resources"],
        json!([
            {"uri": "file:///one.txt", "name": "one", "description": "The first file"},
            {"uri": "file:///two.txt", "name": "two"},
        ])
    );
    assert_eq!(body["features"]["resourceTemplates"], json!([]));
    assert_eq!(body["features"]["prompts"], json!([]));
}

#[test]
fn the_tools_follow_the_options_of_the_handle() {
    let options = Options::default()
        .enable_report_missing(false)
        .enable_tool_call_context(false)
        .enable_agent_tracking(true);
    let body = body_json(demo_lists(), None, &options);
    assert_eq!(tool_names(&body), ["alpha", "beta", "gamma"]);
    let properties = &body["features"]["tools"][0]["inputSchema"]["properties"];
    assert!(properties["agent_id"].is_object());
    assert!(properties["context"].is_null());
    assert_eq!(
        body["features"]["tools"],
        listed(Demo::new().tool_pages.remove(0), &options)
    );
}

#[test]
fn v2_get_more_tools_comes_one_time_for_many_pages() {
    let lists = Lists {
        tool_pages: Demo::paged().tool_pages,
        ..Lists::default()
    };
    let body = body_json(lists, None, &Options::default());
    assert_eq!(
        tool_names(&body),
        [
            "alpha",
            "beta",
            GET_MORE_TOOLS_NAME,
            "gamma",
            "delta",
            "epsilon"
        ]
    );
    assert_eq!(tool_count(&serde_json::from_value(body).unwrap()), 6);
}

#[test]
fn an_empty_page_gets_no_tool() {
    let lists = Lists {
        tool_pages: vec![Vec::new(), vec![tool("alpha")], Vec::new()],
        ..Lists::default()
    };
    let body = body_json(lists, None, &Options::default());
    assert_eq!(tool_names(&body), ["alpha", GET_MORE_TOOLS_NAME]);

    let none = Lists {
        tool_pages: vec![Vec::new()],
        ..Lists::default()
    };
    assert_eq!(
        body_json(none, None, &Options::default())["features"],
        json!({"tools": []})
    );
}

#[test]
fn a_server_that_lists_its_own_get_more_tools_keeps_each_one_without_report_missing() {
    let own: Tool = serde_json::from_value(get_more_tools_descriptor()).unwrap();
    let lists = Lists {
        tool_pages: vec![vec![own.clone()], vec![own]],
        ..Lists::default()
    };
    let options = Options::default().enable_report_missing(false);
    let body = body_json(lists.clone(), None, &options);
    assert_eq!(
        tool_names(&body),
        [GET_MORE_TOOLS_NAME, GET_MORE_TOOLS_NAME]
    );
    let body = body_json(lists, None, &Options::default());
    assert_eq!(tool_names(&body), [GET_MORE_TOOLS_NAME]);
}

#[test]
fn v3_a_list_that_was_not_read_has_no_key() {
    let lists = Lists {
        tool_pages: vec![vec![tool("alpha")]],
        ..Lists::default()
    };
    let body = body_json(lists, None, &Options::default());
    let features = body["features"].as_object().expect("an object");
    assert_eq!(features.keys().collect::<Vec<_>>(), ["tools"]);
}

#[test]
fn a_server_without_a_title_or_instructions_gives_empty_strings() {
    let lists = Lists {
        tool_pages: vec![vec![tool("alpha")]],
        ..Lists::default()
    };
    let request = body(
        PROJECT,
        Implementation::new("plain", "0.1.0"),
        None,
        lists,
        &InjectConfig::from_options(&Options::default()),
        &mut InjectState::default(),
    )
    .expect("the body is built");
    assert_eq!(request.server_title.as_deref(), Some(""));
    assert_eq!(request.instructions.as_deref(), Some(""));
    assert_eq!(request.server_name.as_deref(), Some("plain"));
    assert_eq!(request.server_version.as_deref(), Some("0.1.0"));
}

#[test]
fn v13_without_schemas_removes_only_the_schemas_of_the_tools() {
    let options = Options::default();
    let full = body_json(demo_lists(), Some("Use the demo tools."), &options);
    let mut request = serde_json::from_value(full.clone()).expect("the body is a request");
    assert_eq!(tool_count(&request), 4);
    without_schemas(&mut request);
    assert_eq!(tool_count(&request), 4);
    let slim = serde_json::to_value(&request).expect("the body is JSON");

    let mut expected = full.clone();
    for tool in expected["features"]["tools"].as_array_mut().unwrap() {
        let tool = tool.as_object_mut().unwrap();
        tool.retain(|key, _| key != "inputSchema" && key != "outputSchema");
    }
    assert_eq!(slim, expected);
    let beta = &slim["features"]["tools"][1];
    let keys: Vec<&str> = beta
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["name", "title", "description", "annotations"]);
    assert_eq!(
        beta["description"],
        full["features"]["tools"][1]["description"]
    );
    assert_eq!(slim["features"]["resources"], full["features"]["resources"]);
}

#[test]
fn a_body_without_tools_counts_none() {
    let mut request = agentcat_api::models::PublishServerInfoRequest::new(PROJECT.to_owned());
    assert_eq!(tool_count(&request), 0);
    without_schemas(&mut request);
    request.features = Some(json!({"tools": "not a list"}));
    assert_eq!(tool_count(&request), 0);
    without_schemas(&mut request);
    request.features = Some(json!({"tools": ["not an object", {"name": "a", "inputSchema": {}}]}));
    without_schemas(&mut request);
    assert_eq!(
        request.features,
        Some(json!({"tools": ["not an object", {"name": "a"}]}))
    );
    assert_eq!(tool_count(&request), 2);
}

#[test]
fn the_fixture_beta_tool_has_what_the_table_names() {
    let beta = serde_json::to_value(beta()).unwrap();
    for key in [
        "title",
        "description",
        "inputSchema",
        "outputSchema",
        "annotations",
    ] {
        assert!(!beta[key].is_null(), "{key}");
    }
    assert_eq!(beta["description"].as_str().map(str::len), Some(LONG));
}
