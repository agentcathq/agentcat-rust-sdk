//! The read of the lists against the [`Demo`] server: every page of every list, which lists a
//! server has, and each way a list fails.
use std::sync::atomic::Ordering;
use std::time::Duration;

use rmcp::model::{InitializeResult, Tool};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler};
use tokio::time::Instant;

use super::{LIST_TIMEOUT, Lists, MAX_PAGES, read};
use crate::test_support::rmcp::bare_context;
use crate::test_support::server_info::{Demo, Mode};

/// Reads the lists of `demo` with the capabilities it declares.
async fn read_demo(demo: &Demo) -> Result<Lists, String> {
    let context: RequestContext<RoleServer> = bare_context(None);
    let info: InitializeResult = demo.get_info();
    read(demo, &context, &info.capabilities).await
}

/// The names of the tools of each page.
fn names(pages: &[Vec<Tool>]) -> Vec<Vec<&str>> {
    pages
        .iter()
        .map(|page| page.iter().map(|tool| tool.name.as_ref()).collect())
        .collect()
}

#[test]
fn the_limits_have_their_values() {
    assert_eq!(LIST_TIMEOUT, Duration::from_secs(5));
    assert_eq!(MAX_PAGES, 100);
}

#[tokio::test]
async fn v1_every_list_of_the_fixture_server_is_read() {
    let demo = Demo::new();
    let lists = read_demo(&demo).await.expect("the read works");
    assert_eq!(names(&lists.tool_pages), [["alpha", "beta", "gamma"]]);
    let resources = lists.resources.expect("the server has resources");
    let uris: Vec<&str> = resources.iter().map(|r| r.uri.as_str()).collect();
    assert_eq!(uris, ["file:///one.txt", "file:///two.txt"]);
    let templates = lists.resource_templates.expect("the server has templates");
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].uri_template, "file:///{name}.txt");
    let prompts = lists.prompts.expect("the server has prompts");
    let prompt_names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(prompt_names, ["greet", "write"]);
    assert_eq!(lists.left_out.len(), 0);
    assert_eq!(demo.listed.tools.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn v2_every_page_of_the_tool_list_is_read() {
    let demo = Demo::paged();
    let lists = read_demo(&demo).await.expect("the read works");
    assert_eq!(
        names(&lists.tool_pages),
        [
            vec!["alpha", "beta"],
            vec!["gamma", "delta"],
            vec!["epsilon"]
        ]
    );
    assert_eq!(demo.listed.tools.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn v3_a_list_the_server_does_not_declare_is_not_read() {
    let demo = Demo::tools_only();
    let lists = read_demo(&demo).await.expect("the read works");
    assert_eq!(names(&lists.tool_pages), [["alpha", "beta", "gamma"]]);
    assert_eq!(lists.resources, None);
    assert_eq!(lists.resource_templates, None);
    assert_eq!(lists.prompts, None);
    assert_eq!(lists.left_out.len(), 0);
    assert_eq!(demo.listed.resources.load(Ordering::SeqCst), 0);
    assert_eq!(demo.listed.templates.load(Ordering::SeqCst), 0);
    assert_eq!(demo.listed.prompts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn each_capability_decides_its_own_lists() {
    let resources_only = Demo {
        prompts: None,
        ..Demo::new()
    };
    let lists = read_demo(&resources_only).await.expect("the read works");
    assert!(lists.resources.is_some() && lists.resource_templates.is_some());
    assert_eq!(lists.prompts, None);

    let prompts_only = Demo {
        resources: None,
        ..Demo::new()
    };
    let lists = read_demo(&prompts_only).await.expect("the read works");
    assert_eq!((lists.resources, lists.resource_templates), (None, None));
    assert!(lists.prompts.is_some());
}

#[tokio::test]
async fn an_empty_list_of_a_declared_capability_is_an_empty_list() {
    let demo = Demo {
        tool_pages: Vec::new(),
        resource_items: Vec::new(),
        ..Demo::new()
    };
    let lists = read_demo(&demo).await.expect("the read works");
    assert_eq!(lists.tool_pages, [Vec::new()]);
    assert_eq!(lists.resources, Some(Vec::new()));
}

#[tokio::test]
async fn v4_a_prompt_list_that_fails_is_left_out() {
    let demo = Demo {
        prompts: Some(Mode::Fails),
        ..Demo::new()
    };
    let lists = read_demo(&demo).await.expect("the read works");
    assert_eq!(lists.prompts, None);
    assert_eq!(lists.left_out, ["prompts"]);
    assert_eq!(names(&lists.tool_pages), [["alpha", "beta", "gamma"]]);
    assert_eq!(lists.resources.map(|resources| resources.len()), Some(2));
    assert_eq!(lists.resource_templates.map(|t| t.len()), Some(1));
}

#[tokio::test]
async fn a_resource_list_that_fails_is_left_out_and_the_others_stay() {
    let demo = Demo {
        resources: Some(Mode::Panics),
        ..Demo::new()
    };
    let lists = read_demo(&demo).await.expect("the read works");
    assert_eq!(lists.resources, None);
    assert_eq!(lists.left_out, ["resources"]);
    assert_eq!(lists.resource_templates.map(|t| t.len()), Some(1));
    assert_eq!(lists.prompts.map(|prompts| prompts.len()), Some(2));
}

#[tokio::test]
async fn v5_a_tool_list_that_fails_fails_the_read() {
    let demo = Demo {
        tools: Mode::Fails,
        ..Demo::new()
    };
    assert_eq!(
        read_demo(&demo).await,
        Err("the tools list gave an error: demo list failure".to_owned())
    );
    assert_eq!(demo.listed.resources.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_tool_list_that_panics_fails_the_read() {
    let demo = Demo {
        tools: Mode::Panics,
        ..Demo::new()
    };
    assert_eq!(
        read_demo(&demo).await,
        Err("the tools list panicked: demo list panic".to_owned())
    );
}

#[tokio::test]
async fn v6_a_cursor_that_comes_twice_fails_the_list_at_once() {
    let started = std::time::Instant::now();
    let demo = Demo {
        tools: Mode::SameCursor,
        ..Demo::new()
    };
    assert_eq!(
        read_demo(&demo).await,
        Err("the tools list gave the same cursor twice".to_owned())
    );
    assert_eq!(demo.listed.tools.load(Ordering::SeqCst), 2);

    let demo = Demo {
        prompts: Some(Mode::SameCursor),
        ..Demo::new()
    };
    let lists = read_demo(&demo).await.expect("the read works");
    assert_eq!(lists.left_out, ["prompts"]);
    assert_eq!(demo.listed.prompts.load(Ordering::SeqCst), 2);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_list_of_more_than_100_pages_fails() {
    let demo = Demo {
        tools: Mode::Endless,
        ..Demo::new()
    };
    assert_eq!(
        read_demo(&demo).await,
        Err("the tools list has more than 100 pages".to_owned())
    );
    assert_eq!(demo.listed.tools.load(Ordering::SeqCst), 100);
}

#[tokio::test]
async fn a_list_of_100_pages_is_read() {
    let demo = Demo {
        tool_pages: vec![Vec::new(); MAX_PAGES],
        ..Demo::tools_only()
    };
    let lists = read_demo(&demo).await.expect("the read works");
    assert_eq!(lists.tool_pages.len(), 100);
}

#[tokio::test(start_paused = true)]
async fn a_list_that_never_answers_fails_after_5_seconds() {
    let demo = Demo {
        tools: Mode::Hangs,
        ..Demo::new()
    };
    let started = Instant::now();
    assert_eq!(
        read_demo(&demo).await,
        Err("the tools list did not respond within 5000ms".to_owned())
    );
    assert_eq!(started.elapsed(), LIST_TIMEOUT);

    let demo = Demo {
        resources: Some(Mode::Hangs),
        prompts: Some(Mode::Hangs),
        ..Demo::new()
    };
    let started = Instant::now();
    let lists = read_demo(&demo).await.expect("the read works");
    assert_eq!(lists.left_out, ["resources", "prompts"]);
    assert_eq!(lists.resource_templates.map(|t| t.len()), Some(1));
    assert_eq!(started.elapsed(), LIST_TIMEOUT * 2);
}
