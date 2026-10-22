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
