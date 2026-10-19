//! Parity: `src/tests/constants.test.ts`.

use crate::constants::*;
use regex::Regex;

mod brand_constants_wire_literals {
    use super::*;

    #[test]
    fn emits_the_agentcat_custom_event_type_on_the_wire() {
        assert_eq!(AGENTCAT_CUSTOM_EVENT_TYPE, "agentcat:custom");
    }

    #[test]
    fn stamps_exporter_source_as_agentcat() {
        assert_eq!(AGENTCAT_SOURCE, "agentcat");
    }

    #[test]
    fn uses_the_agentcat_diagnostics_scope_name() {
        assert_eq!(DIAGNOSTICS_SCOPE_NAME, "agentcat-diagnostics");
    }

    #[test]
    fn mirrors_under_the_mcp_session_key() {
        assert_eq!(MCP_SESSION_KEY, "mcp_session");
    }
}

mod handle_param_descriptions_name_the_delivery_channel {
    use super::*;

    #[test]
    fn session_id_pre_announces_both_delivery_channels_and_both_text_headers() {
        assert!(SESSION_ID_PARAM_DESCRIPTION.contains("mcp_session"));
        assert!(
            SESSION_ID_PARAM_DESCRIPTION
                .contains("a text block at the start of the result beginning [session_id issued")
        );
        assert!(SESSION_ID_PARAM_DESCRIPTION.contains("a text block beginning [session_id unrecognized"));
        assert!(
            Regex::new(r"^\[session_id issued")
                .unwrap()
                .is_match(MINT_BACK_HEADER_ISSUED)
        );
        assert!(
            Regex::new(r"^\[session_id unrecognized")
                .unwrap()
                .is_match(MINT_BACK_HEADER_UNRECOGNIZED)
        );
    }

    #[test]
    fn agent_id_describes_self_chosen_generation_not_server_delivery() {
        assert!(AGENT_ID_PARAM_DESCRIPTION.contains("opus-4.80-1m|claude-code|k3n9x"));
        assert!(AGENT_ID_PARAM_DESCRIPTION.contains("never inherited"));
        assert!(AGENT_ID_PARAM_DESCRIPTION.contains("generates a new one"));
        assert!(!AGENT_ID_PARAM_DESCRIPTION.contains("the server will issue"));
    }
}

mod v4_copy_pins_byte_exact {
    use super::*;

    #[test]
    fn pins_session_id_param_description() {
        assert_eq!(
            SESSION_ID_PARAM_DESCRIPTION,
            "Session continuity handle, one of two values: the ses_ ID issued for the task underway, or start. This server cannot link your calls between requests on its own, so session continuity travels in this parameter instead. If you were handed a session_id for this task \u{2014} for example by the agent that spawned you \u{2014} send that exact value from your first call. Otherwise send start on your first call; the server will issue an opaque correlation ID in the mcp_session field of the result, or in a text block at the start of the result beginning [session_id issued. Then send that exact ses_ value on every later call and hand it to any subagents working the same task. start always begins a new, unrelated task \u{2014} never send it mid-task. If you send a value this server does not recognize, the result reports it: mcp_session.status of unrecognized, or a text block beginning [session_id unrecognized; re-send the ID issued for this task, or start if none was issued yet. Never invent a ses_ value."
        );
    }

    #[test]
    fn pins_mcp_session_status_description() {
        assert_eq!(
            MCP_SESSION_STATUS_DESCRIPTION,
            "issued: first call of a task; the session_id above was just created. active: the session_id you sent was accepted; keep sending it. unrecognized: the value sent was not issued by this server \u{2014} re-send the one issued earlier for this task; if none was issued yet, send start to be issued a new one."
        );
    }

    #[test]
    fn pins_mint_back_issued_body() {
        assert_eq!(
            MINT_BACK_ISSUED_BODY,
            "This is the first-call issuance described in this tool's session_id parameter description."
        );
    }

    #[test]
    fn pins_mint_back_unrecognized_body() {
        assert_eq!(
            MINT_BACK_UNRECOGNIZED_BODY,
            "The value sent was not issued by this server. Re-send the session_id issued earlier for this task; if none was issued yet, send start and one will be issued."
        );
    }

    #[test]
    fn pins_the_session_id_value_contract_constants() {
        assert_eq!(SESSION_ID_PARAM_PATTERN, "^(start|ses_[0-9A-Za-z]{27})$");
        assert_eq!(SESSION_START_SENTINEL, "start");
        let pattern = Regex::new(SESSION_ID_PARAM_PATTERN).unwrap();
        assert!(pattern.is_match("start"));
        assert!(pattern.is_match(&format!("ses_{}", "a".repeat(27))));
        assert!(!pattern.is_match(&format!("ses_{}", "a".repeat(25))));
        assert!(!pattern.is_match("restart"));
        assert!(!pattern.is_match("START"));
    }
}
