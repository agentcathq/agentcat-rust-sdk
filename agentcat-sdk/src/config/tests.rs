//! `Options`: each setter changes its own getter only, and an empty custom description means the
//! default one (B-004). B-004's default values are pinned by the harness profile `{}`, not here.
use super::Options;
use crate::constants::DEFAULT_CONTEXT_PARAMETER_DESCRIPTION;

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

/// A set hook shows as `"<hook>"`, never as its closure; the options stay `Clone` (D19, C14).
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

/// B-004, B-031: the custom `context` description reaches the listing of the tools that get the
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
