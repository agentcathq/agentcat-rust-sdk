//! Parity: `src/tests/handle-conflict-report.test.ts`.
//!
//! TS keys its dedupe set and its declared set per `AgentCatData` object; in
//! Rust both live in the `InjectMemory` of one `AgentCat`, so a second
//! server is a second `InjectMemory`.

use super::support::capture_logs;
use crate::inject::{InjectConfig, InjectMemory, build_injected_list};
use serde_json::{Value, json};

/// The TS test helper `data(overrides)`: report-missing, context and agent
/// tracking off, tracing on.
fn data() -> InjectConfig {
    InjectConfig::new(true, false, false, false, None, false)
}

/// `data({resolveSessionId: …})`.
fn data_hook_mode() -> InjectConfig {
    InjectConfig::new(true, true, false, false, None, false)
}

fn own_session_tool() -> Value {
    session_tool("own_session")
}

fn session_tool(name: &str) -> Value {
    json!({
        "name": name,
        "inputSchema": {"type": "object", "properties": {"session_id": {"type": "string"}}}
    })
}

/// Runs `passes` listings of `tools` on one memory; returns the log lines.
fn list(cfg: &InjectConfig, memory: &InjectMemory, tools: &[Value], passes: usize) -> Vec<String> {
    capture_logs(|| {
        for _ in 0..passes {
            build_injected_list(cfg, memory, tools);
        }
    })
    .1
}

fn lines_naming<'a>(logs: &'a [String], name: &str) -> Vec<&'a String> {
    logs.iter().filter(|l| l.contains(name)).collect()
}

mod session_id_collision_report {
    use super::*;

    #[test]
    fn reports_a_session_id_collision_once_per_tool_as_an_error_with_remediation() {
        let memory = InjectMemory::default();
        let logs = list(&data(), &memory, &[own_session_tool()], 3);
        let collisions = lines_naming(&logs, "own_session");
        assert_eq!(collisions.len(), 1, "{logs:?}");
        assert!(collisions[0].contains("ERROR:"));
        assert!(collisions[0].contains("resolveSessionId"));
        assert!(collisions[0].contains("without a session"));
        assert_eq!(
            collisions[0],
            "ERROR: Tool \"own_session\" already declares a 'session_id' parameter. AgentCat will not inject its own, and calls to this tool are published without a session, so they cannot be correlated. Your parameter is untouched and still reaches your handler. If you already manage sessions, pass a resolveSessionId hook to track() — AgentCat will derive its session from your identifier and stop injecting session_id entirely."
        );
    }

    #[test]
    fn reports_two_different_colliding_tools_each_once() {
        let memory = InjectMemory::default();
        let tools = [session_tool("tool_a"), session_tool("tool_b")];
        let logs = list(&data(), &memory, &tools, 2);
        assert_eq!(lines_naming(&logs, "tool_a").len(), 1, "{logs:?}");
        assert_eq!(lines_naming(&logs, "tool_b").len(), 1, "{logs:?}");
    }

    #[test]
    fn getreportedconflicts_returns_a_stable_set_per_key_and_independent_sets_3803b9c2() {
        let key_a = InjectMemory::default();
        let key_b = InjectMemory::default();
        assert_eq!(key_a.reported_conflicts(), key_a.reported_conflicts());
        assert!(key_b.reported_conflicts().is_empty());

        list(&data(), &key_a, &[session_tool("tool_x")], 1);
        assert_eq!(key_a.reported_conflicts(), ["tool_x"]);
        assert!(key_b.reported_conflicts().is_empty());
        assert_eq!(key_a.reported_conflicts(), ["tool_x"]);
    }

    #[test]
    fn buildinjectedlist_dedupes_per_agentcatdata_object_repeat_tools_list_on_1a81588d() {
        let server_a = InjectMemory::default();
        let server_b = InjectMemory::default();
        let mut logs = list(&data(), &server_a, &[own_session_tool()], 3);
        assert_eq!(lines_naming(&logs, "own_session").len(), 1, "{logs:?}");
        logs.extend(list(&data(), &server_b, &[own_session_tool()], 2));
        assert_eq!(lines_naming(&logs, "own_session").len(), 2, "{logs:?}");
    }
}

mod customer_declared_session_id_tracking {
    use super::*;

    fn composed_tool() -> Value {
        json!({
            "name": "union_tool",
            "inputSchema": {
                "type": "object",
                "oneOf": [
                    {"properties": {"kind": {"const": "a"}}},
                    {"properties": {"kind": {"const": "b"}}}
                ]
            }
        })
    }

    fn plain_tool() -> Value {
        json!({"name": "normal", "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}}})
    }

    #[test]
    fn records_only_the_tools_whose_own_schema_declares_session_id() {
        let server = InjectMemory::default();
        list(
            &data(),
            &server,
            &[own_session_tool(), composed_tool(), plain_tool()],
            1,
        );
        assert_eq!(server.declared_session_params(), ["own_session"]);
        assert!(server.declares_session_id("own_session"));
        assert!(!server.declares_session_id("union_tool"));
        assert!(!server.declares_session_id("normal"));
    }

    #[test]
    fn records_a_composed_schema_that_also_declares_session_id_at_its_root() {
        let server = InjectMemory::default();
        let mut tool = composed_tool();
        tool["name"] = json!("union_with_own_session");
        tool["inputSchema"]["properties"] = json!({"session_id": {"type": "string"}});
        list(&data(), &server, &[tool], 1);
        assert!(server.declares_session_id("union_with_own_session"));
    }

    #[test]
    fn keys_per_agentcatdata_object_like_reportedconflicts() {
        let server_a = InjectMemory::default();
        let server_b = InjectMemory::default();
        list(&data(), &server_a, &[own_session_tool()], 1);
        assert!(server_a.declares_session_id("own_session"));
        assert!(!server_b.declares_session_id("own_session"));
    }

    #[test]
    fn records_nothing_in_hook_mode_where_session_id_is_never_injected() {
        let server = InjectMemory::default();
        list(&data_hook_mode(), &server, &[own_session_tool()], 1);
        assert!(server.declared_session_params().is_empty());
    }
}
