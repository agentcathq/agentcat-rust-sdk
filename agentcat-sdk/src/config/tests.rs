//! `Options`: each setter changes its own getter only, and an empty custom description means the
//! default one. The default values are pinned by the harness profile `{}`, not here.
use super::env::{
    AGENTCAT_API_URL, AGENTCAT_ENABLED, DEFAULT_API_BASE_URL, Env, MCPCAT_API_URL, MapEnv,
    ProcessEnv, resolve_api_base_url, sdk_enabled,
};
use super::{LogDestination, Options};
use crate::constants::DEFAULT_CONTEXT_PARAMETER_DESCRIPTION;
use crate::event::Event;

/// What the crate getters of an [`Options`] return.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one field for each getter of Options"
)]
struct Seen {
    report_missing: bool,
    tracing: bool,
    tool_call_context: bool,
    agent_tracking: bool,
    context_description: String,
    hook_mode: bool,
}

impl Seen {
    fn of(options: &Options) -> Self {
        Self {
            report_missing: options.report_missing(),
            tracing: options.tracing(),
            tool_call_context: options.tool_call_context(),
            agent_tracking: options.agent_tracking(),
            context_description: options.context_description().to_owned(),
            hook_mode: options.hook_mode(),
        }
    }
}

#[test]
fn setters_chain() {
    let default = Seen::of(&Options::default());
    let cases = [
        (
            Options::default().enable_report_missing(!default.report_missing),
            Seen {
                report_missing: !default.report_missing,
                ..default.clone()
            },
        ),
        (
            Options::default().enable_tracing(!default.tracing),
            Seen {
                tracing: !default.tracing,
                ..default.clone()
            },
        ),
        (
            Options::default().enable_tool_call_context(!default.tool_call_context),
            Seen {
                tool_call_context: !default.tool_call_context,
                ..default.clone()
            },
        ),
        (
            Options::default().enable_agent_tracking(!default.agent_tracking),
            Seen {
                agent_tracking: !default.agent_tracking,
                ..default.clone()
            },
        ),
        (
            Options::default().custom_context_description("Why this call."),
            Seen {
                context_description: "Why this call.".to_owned(),
                ..default.clone()
            },
        ),
    ];
    for (options, expected) in cases {
        assert_eq!(Seen::of(&options), expected);
    }

    let chained = Options::default()
        .enable_report_missing(!default.report_missing)
        .enable_tracing(!default.tracing)
        .enable_tool_call_context(!default.tool_call_context)
        .enable_agent_tracking(!default.agent_tracking)
        .custom_context_description(String::from("Why this call."));
    let expected = Seen {
        report_missing: !default.report_missing,
        tracing: !default.tracing,
        tool_call_context: !default.tool_call_context,
        agent_tracking: !default.agent_tracking,
        context_description: "Why this call.".to_owned(),
        hook_mode: default.hook_mode,
    };
    assert_eq!(Seen::of(&chained), expected);
}

#[test]
fn empty_custom_description_uses_the_default() {
    let empty = Options::default().custom_context_description("");
    assert_eq!(
        empty.context_description(),
        DEFAULT_CONTEXT_PARAMETER_DESCRIPTION
    );
    let replaced = Options::default()
        .custom_context_description("Why this call.")
        .custom_context_description("");
    assert_eq!(
        replaced.context_description(),
        DEFAULT_CONTEXT_PARAMETER_DESCRIPTION
    );
}

/// A set hook shows as `"<hook>"`, never as its closure; the options stay `Clone`.
#[test]
fn debug_hides_hooks() {
    let none = format!("{:?}", Options::default());
    for name in [
        "resolve_session_id",
        "identify",
        "event_tags",
        "event_properties",
    ] {
        assert!(none.contains(&format!("{name}: None")), "{none}");
    }
    assert!(!none.contains("<hook>"));

    let options = Options::default()
        .resolve_session_id(|_call| async { Ok(None) })
        .identify(|_call| async { Ok(None) })
        .event_tags(|_call| async { Ok(None) })
        .event_properties(|_call| async { Ok(None) });
    let set = format!("{options:?}");
    for name in [
        "resolve_session_id",
        "identify",
        "event_tags",
        "event_properties",
    ] {
        assert!(set.contains(&format!("{name}: Some(\"<hook>\")")), "{set}");
    }
    assert_eq!(format!("{:?}", options.clone()), set);
    assert!(options.hook_mode());
    assert!(
        !Options::default()
            .identify(|_call| async { Ok(None) })
            .hook_mode()
    );
}

/// The custom `context` description reaches the listing of the tools that get the
/// parameter, and `get_more_tools` keeps its own.
#[tokio::test]
async fn custom_context_description_lands_in_the_listing() {
    use std::sync::Arc;

    use crate::client::AgentCat;
    use crate::constants::GET_MORE_TOOLS_NAME;
    use crate::test_support::rmcp::{RecordingSink, Sample, connect};

    let options = Options::default().custom_context_description("Explain your reasoning");
    let agentcat = AgentCat::with_sink("", options, Arc::new(RecordingSink::default()));
    let client = connect(agentcat.track(Sample), "custom").await;
    let tools = client.list_all_tools().await.unwrap();
    let description_of = |name: &str| {
        let tool = tools.iter().find(|tool| tool.name == name).unwrap();
        tool.input_schema["properties"]["context"]["description"].clone()
    };
    assert_eq!(description_of("echo"), "Explain your reasoning");
    assert_ne!(
        description_of(GET_MORE_TOOLS_NAME),
        "Explain your reasoning"
    );
    client.cancel().await.unwrap();
}

/// The option beats `AGENTCAT_API_URL`, which beats `MCPCAT_API_URL`, which beats the
/// default; an empty value counts as unset; the value is not trimmed.
#[test]
fn api_base_url_precedence() {
    let both = MapEnv::new(&[
        (AGENTCAT_API_URL, "https://a.example/"),
        (MCPCAT_API_URL, "https://m.example"),
    ]);
    let option = Options::default().api_base_url("https://o.example");
    assert_eq!(resolve_api_base_url(&option, &both), "https://o.example");
    assert_eq!(
        resolve_api_base_url(&Options::default(), &both),
        "https://a.example/"
    );
    let legacy = MapEnv::new(&[
        (AGENTCAT_API_URL, ""),
        (MCPCAT_API_URL, "https://m.example"),
    ]);
    assert_eq!(
        resolve_api_base_url(&Options::default().api_base_url(""), &legacy),
        "https://m.example"
    );
    let empty = MapEnv::new(&[(AGENTCAT_API_URL, ""), (MCPCAT_API_URL, "")]);
    assert_eq!(
        resolve_api_base_url(&Options::default(), &empty),
        DEFAULT_API_BASE_URL
    );
    assert_eq!(DEFAULT_API_BASE_URL, "https://api.agentcat.com");
    assert_eq!(Options::default().api_base_url_option(), None);
    assert_eq!(option.api_base_url_option(), Some("https://o.example"));
}

/// `false`, `0`, `no` and `off` turn the SDK off, trimmed and in any case; every other
/// value, an empty one and an unset variable leave it on.
#[test]
fn sdk_enabled_values() {
    let enabled = |value: &str| sdk_enabled(&MapEnv::new(&[(AGENTCAT_ENABLED, value)]));
    for off in ["false", " OFF ", "No", "0", "FALSE", "\toff\n"] {
        assert!(!enabled(off), "{off:?}");
    }
    for on in ["true", "", "maybe", "1", "yes", "on"] {
        assert!(enabled(on), "{on:?}");
    }
    assert!(sdk_enabled(&MapEnv::new(&[])));
}

/// The redaction hooks and the log destination are set by their own setters; `Debug` shows that
/// a hook is set, never the closure; clones share the hooks.
#[tokio::test]
async fn redactors_are_set_and_debug_hides_them() {
    let none = Options::default();
    assert!(none.string_redactor().is_none() && none.event_redactor().is_none());
    assert_eq!(none.log_to(), LogDestination::File);
    assert!(format!("{none:?}").contains("string_redactor: None"));

    let options = Options::default()
        .redact_sensitive_information(|text: String| async move { Ok(text.to_uppercase()) })
        .redact_event(|event: Event| async move { Ok(Some(event)) })
        .log_destination(LogDestination::Tracing);
    assert_eq!(options.log_to(), LogDestination::Tracing);
    let shown = format!("{options:?}");
    assert!(
        shown.contains("string_redactor: Some(\"<hook>\")"),
        "{shown}"
    );
    assert!(
        shown.contains("event_redactor: Some(\"<hook>\")"),
        "{shown}"
    );

    let cloned = options.clone();
    let string = cloned.string_redactor().unwrap();
    assert_eq!(string.call("abc".to_owned()).await.unwrap(), "ABC");
    let event = Event::from_object(&serde_json::Map::new());
    let kept = cloned
        .event_redactor()
        .unwrap()
        .call(event.clone())
        .await
        .unwrap();
    assert_eq!(kept, Some(event));
}

/// `MapEnv` gives the home directory it was built with, and none by default.
#[test]
fn map_env_has_a_home_only_when_given() {
    assert_eq!(MapEnv::new(&[]).home_dir(), None);
    let home = std::path::PathBuf::from("/home/test");
    assert_eq!(
        MapEnv::new(&[]).with_home(home.clone()).home_dir(),
        Some(home)
    );
}

/// `ProcessEnv`, where `AGENTCAT_PROJECT_ID`, `AGENTCAT_ENABLED` and the home
/// directory of `<home>/agentcat.log` come from, reads the real environment: a variable that nobody
/// sets is `None`, one that the test runner sets for every test process (`CARGO_PKG_NAME`, as cargo
/// and nextest do) has its value, and the home directory is the user's, an absolute path.
#[test]
fn process_env_reads_the_process() {
    assert_eq!(ProcessEnv.var("AGENTCAT_SDK_TEST_NEVER_SET"), None);
    assert_eq!(
        ProcessEnv.var("CARGO_PKG_NAME").as_deref(),
        Some(env!("CARGO_PKG_NAME"))
    );
    let home = ProcessEnv
        .home_dir()
        .expect("the test process has a home directory");
    assert!(home.is_absolute(), "{}", home.display());
}
