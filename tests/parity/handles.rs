//! Parity: `src/tests/handles.test.ts`.

use super::support::{obj, sid};
use crate::agentcat::Options;
use crate::constants::{AGENT_ID_PARAM, MCP_SESSION_KEY, SESSION_ID_PARAM};
use crate::handles::{
    HandleResolution, SessionSource, build_handle_tags, build_mint_back_text, build_structured_mint_back,
    extract_handle, mirror_structured_mint_back, resolve_handles, session_from_hook_value,
};
use crate::ksuid::{derive_session_id, is_valid_session_id, new_session_id};
use crate::util::utf16_len;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

const T: &str = "ses_2xF9kQm3rTvB8nLpYw7ZcHd4Ke1";
const A: &str = "opus-4.80-1m|claude-code|k3n9x";

/// The TS test helper `MINTED_BLOCK(id)`.
fn minted_block(id: &str) -> String {
    format!(
        "[session_id issued — see this tool's session_id parameter description]\n\
         session_id: {id}\n\
         This is the first-call issuance described in this tool's session_id parameter description."
    )
}

/// A `HandleResolution` literal of the TS tests; `agentSource` is
/// `"supplied"` exactly when `agent_id` is set.
fn res(session_id: &str, source: SessionSource, hook_mode: bool, agent_id: Option<&str>) -> HandleResolution {
    HandleResolution {
        session_id: session_id.to_owned(),
        session_source: source,
        hook_mode,
        agent_id: agent_id.map(str::to_owned),
    }
}

/// TS `resolveHandles(options, "proj_1", req(args), undefined, ours)`:
/// `hook_mode` is `options.resolveSessionId` being set, `agent_tracking` is
/// `options.enableAgentTracking === true`.
fn resolve(hook_mode: bool, agent_tracking: bool, args: Value, ours: bool) -> HandleResolution {
    resolve_handles(hook_mode, agent_tracking, Some(&obj(args)), ours)
}

fn keys(map: &Map<String, Value>) -> Vec<&str> {
    map.keys().map(String::as_str).collect()
}

fn tags(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

mod handle_primitives {
    use super::*;

    #[test]
    fn mints_session_ids_with_the_ses_prefix() {
        let id = new_session_id();
        assert!(Regex::new("^ses_[0-9A-Za-z]{27}$").unwrap().is_match(&id), "{id}");
        assert_ne!(new_session_id(), id);
    }

    #[test]
    fn derives_deterministic_session_ids_from_id_project() {
        let a = derive_session_id("customer-abc", Some("proj_1"));
        let b = derive_session_id("customer-abc", Some("proj_1"));
        assert_eq!(a, b);
        assert!(a.starts_with("ses_"), "{a}");
    }

    #[test]
    fn different_project_yields_a_different_derived_id() {
        assert_ne!(
            derive_session_id("customer-abc", Some("proj_1")),
            derive_session_id("customer-abc", Some("proj_2"))
        );
    }

    #[test]
    fn derives_without_a_project_id() {
        assert_eq!(
            derive_session_id("customer-abc", None),
            derive_session_id("customer-abc", None)
        );
        assert_ne!(
            derive_session_id("customer-abc", None),
            derive_session_id("customer-abc", Some("proj_1"))
        );
    }

    #[test]
    fn pins_derived_session_ids_to_their_golden_values() {
        assert_eq!(
            derive_session_id("customer-abc", Some("proj_1")),
            "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm"
        );
        assert_eq!(
            derive_session_id("customer-abc", None),
            "ses_2cZY3tvyI25O2AmL2CGVo2B1IIj"
        );
    }

    #[test]
    fn does_not_trim_its_input() {
        assert_ne!(
            derive_session_id(" x ", Some("p")),
            derive_session_id("x", Some("p"))
        );
        assert_eq!(
            derive_session_id(" x ", Some("p")),
            "ses_2c3yR5mYKQdLaXsJNgZH6erbfQK"
        );
        assert_eq!(
            derive_session_id("x", Some("p")),
            "ses_2bw285VY9apdgUgTPXKFnT6P4G0"
        );
    }

    #[test]
    fn extracthandle_returns_trimmed_non_empty_strings_only() {
        let extract = |args: Value| extract_handle(Some(&obj(args)), SESSION_ID_PARAM);
        assert_eq!(
            extract(json!({"session_id": " ses_x "})).as_deref(),
            Some("ses_x")
        );
        assert_eq!(extract(json!({"session_id": ""})), None);
        assert_eq!(extract(json!({"session_id": "   "})), None);
        assert_eq!(extract(json!({"session_id": 42})), None);
        // `undefined`, `null` and `"not-an-object"` arguments all reach Rust as
        // no argument object.
        assert_eq!(extract_handle(None, SESSION_ID_PARAM), None);
        assert_eq!(extract_handle(None, AGENT_ID_PARAM), None);
    }

    #[test]
    fn extracthandle_returns_any_non_empty_string_validation_happens_in_resolvehandles() {
        assert_eq!(
            extract_handle(
                Some(&obj(json!({"session_id": "my-own-correlation-id"}))),
                SESSION_ID_PARAM
            )
            .as_deref(),
            Some("my-own-correlation-id")
        );
    }
}

mod buildmintbacktext {
    use super::*;

    #[test]
    fn task_minted_task_only_block_no_agent_mention_even_when_agent_supplied() {
        let r = res(T, SessionSource::Minted, false, Some(A));
        assert_eq!(build_mint_back_text(&r), Some(minted_block(T)));
    }

    #[test]
    fn task_minted_without_agent_tracking_same_task_only_block() {
        let r = res(T, SessionSource::Minted, false, None);
        assert_eq!(build_mint_back_text(&r), Some(minted_block(T)));
    }

    #[test]
    fn task_supplied_null_nothing_to_announce_agent_never_mints() {
        let r = res(T, SessionSource::Supplied, false, Some(A));
        assert_eq!(build_mint_back_text(&r), None);
    }

    #[test]
    fn hook_mode_never_emits_task_instructions_even_when_hook_null_minted() {
        let r = res(T, SessionSource::Minted, true, Some(A));
        assert_eq!(build_mint_back_text(&r), None);
    }
}

mod buildhandletags {
    use super::*;

    #[test]
    fn stamps_sources_agent_id_and_protocol_version() {
        let r = res(T, SessionSource::Supplied, false, Some(A));
        assert_eq!(
            build_handle_tags(&r, Some("2026-07-28")),
            tags(&[
                ("agentcat_session_id_source", "supplied"),
                ("agentcat_agent_id", A),
                ("agentcat_agent_id_source", "supplied"),
                ("agentcat_protocol_version", "2026-07-28"),
            ])
        );
    }

    #[test]
    fn omits_agent_and_protocol_keys_when_absent() {
        let r = res(T, SessionSource::Hook, true, None);
        assert_eq!(
            build_handle_tags(&r, None),
            tags(&[("agentcat_session_id_source", "hook")])
        );
    }

    #[test]
    fn clamps_a_200_char_agent_id_to_exactly_200_chars_in_the_tag_only() {
        let long_id = "a".repeat(500);
        let r = res(T, SessionSource::Supplied, false, Some(&long_id));
        let out = build_handle_tags(&r, None);
        assert_eq!(utf16_len(&out["agentcat_agent_id"]), 200);
        assert_eq!(out["agentcat_agent_id"], "a".repeat(200));
        assert_eq!(r.agent_id, Some(long_id));
    }

    #[test]
    fn strips_newlines_from_the_agent_id_tag() {
        let r = res(T, SessionSource::Supplied, false, Some("line1\nline2\r\nline3"));
        let out = build_handle_tags(&r, None);
        let tag = &out["agentcat_agent_id"];
        assert!(!tag.contains('\n'), "{tag:?}");
        assert!(!tag.contains('\r'), "{tag:?}");
        assert_eq!(tag, "line1 line2  line3");
        assert_eq!(r.agent_id.as_deref(), Some("line1\nline2\r\nline3"));
    }

    #[test]
    fn passes_a_normal_agent_id_through_unchanged() {
        let r = res(T, SessionSource::Supplied, false, Some(A));
        assert_eq!(build_handle_tags(&r, None)["agentcat_agent_id"], A);
    }
}

mod resolvehandles_prompted_mode {
    use super::*;

    fn start_sentinel_mints(value: &str) {
        let r = resolve(false, false, json!({"session_id": value}), true);
        assert_eq!(r.session_source, SessionSource::Minted, "{value:?}");
        assert!(r.session_id.starts_with("ses_"), "{value:?}");
    }

    #[test]
    fn mints_the_task_but_never_the_agent_when_nothing_supplied_agent_tracking_on() {
        let r = resolve(false, true, json!({}), true);
        assert!(!r.hook_mode);
        assert_eq!(r.session_source, SessionSource::Minted);
        assert!(r.session_id.starts_with("ses_"), "{}", r.session_id);
        assert_eq!(r.agent_id, None);
    }

    #[test]
    fn subagent_flow_supplied_task_omitted_agent_leaves_the_agent_unresolved() {
        let r = resolve(false, true, json!({"session_id": sid("parent")}), true);
        assert_eq!(r.session_source, SessionSource::Supplied);
        assert_eq!(r.session_id, sid("parent"));
        assert_eq!(r.agent_id, None);
    }

    #[test]
    fn uses_supplied_handles_verbatim() {
        let r = resolve(
            false,
            true,
            json!({
                "session_id": sid("anythinggoes"),
                "agent_id": " opus-4.80-1m|claude-code|k3n9x ",
            }),
            true,
        );
        assert_eq!(r.session_source, SessionSource::Supplied);
        assert_eq!(r.session_id, sid("anythinggoes"));
        assert_eq!(r.agent_id.as_deref(), Some("opus-4.80-1m|claude-code|k3n9x"));
    }

    #[test]
    fn enableagenttracking_false_resolves_no_agent_id_at_all() {
        let r = resolve(false, false, json!({"agent_id": "agt_ignored"}), true);
        assert_eq!(r.agent_id, None);
    }

    #[test]
    fn agent_tracking_is_off_by_default_omitted_option_resolves_no_agent_id() {
        let options = Options::default();
        assert!(!options.enable_agent_tracking);
        let r = resolve(
            options.resolve_session_id.is_some(),
            options.enable_agent_tracking,
            json!({"agent_id": "agt_ignored"}),
            true,
        );
        assert_eq!(r.session_source, SessionSource::Minted);
        assert_eq!(r.agent_id, None);
    }

    #[test]
    fn rejects_a_malformed_supplied_session_id_sessionless_source_invalid() {
        let r = resolve(false, false, json!({"session_id": "nope"}), true);
        assert_eq!(r.session_source, SessionSource::Invalid);
        assert_eq!(r.session_id, "");
    }

    #[test]
    fn never_stores_the_raw_rejected_value() {
        let r = resolve(false, false, json!({"session_id": "sk_live_secret_token"}), true);
        assert_eq!(r.session_id, "");
        assert!(!format!("{r:?}").contains("sk_live_secret_token"), "{r:?}");
    }

    #[test]
    fn accepts_a_well_formed_supplied_session_id_verbatim() {
        let r = resolve(false, false, json!({"session_id": sid("parent")}), true);
        assert_eq!(r.session_source, SessionSource::Supplied);
        assert_eq!(r.session_id, sid("parent"));
    }

    #[test]
    fn foreign_param_sessionless_regardless_of_what_the_agent_sent() {
        let with_value = resolve(false, false, json!({"session_id": "customer-value"}), false);
        assert_eq!(with_value.session_source, SessionSource::Foreign);
        assert_eq!(with_value.session_id, "");

        let without_value = resolve(false, false, json!({}), false);
        assert_eq!(without_value.session_source, SessionSource::Foreign);
        assert_eq!(without_value.session_id, "");
    }

    #[test]
    fn hook_mode_wins_over_foreign_arguments_are_never_read() {
        let r = resolve(true, false, json!({"session_id": "customer-value"}), false);
        assert!(r.hook_mode);
        assert_eq!(r.session_source, SessionSource::Hook);
        assert_eq!(r.session_id, "");
    }

    #[test]
    fn still_mints_when_nothing_is_supplied_on_our_own_param() {
        let r = resolve(false, false, json!({}), true);
        assert_eq!(r.session_source, SessionSource::Minted);
        assert!(r.session_id.starts_with("ses_"), "{}", r.session_id);
    }

    #[test]
    fn start_sentinel_resolves_exactly_like_omission_fresh_mint_source_minted() {
        let r = resolve(false, false, json!({"session_id": "start"}), true);
        assert_eq!(r.session_source, SessionSource::Minted);
        assert!(r.session_id.starts_with("ses_"), "{}", r.session_id);
        let again = resolve(false, false, json!({"session_id": "start"}), true);
        assert_ne!(again.session_id, r.session_id);
    }

    #[test]
    fn start_sentinel_is_case_insensitive_and_whitespace_tolerant_start_mints() {
        start_sentinel_mints("START");
    }

    #[test]
    fn start_sentinel_is_case_insensitive_and_whitespace_tolerant_start_mints_2() {
        start_sentinel_mints("Start");
    }

    #[test]
    fn start_sentinel_is_case_insensitive_and_whitespace_tolerant_start_mints_3() {
        start_sentinel_mints("sTaRt");
    }

    #[test]
    fn start_sentinel_is_case_insensitive_and_whitespace_tolerant_start_mints_4() {
        start_sentinel_mints("  start  ");
    }

    #[test]
    fn start_sentinel_is_case_insensitive_and_whitespace_tolerant_start_t_mints() {
        start_sentinel_mints(" START\t");
    }

    #[test]
    fn near_sentinel_values_stay_on_the_invalid_path() {
        for value in ["restart", "start now", "starts", "star t"] {
            let r = resolve(false, false, json!({"session_id": value}), true);
            assert_eq!(r.session_source, SessionSource::Invalid, "{value:?}");
            assert_eq!(r.session_id, "", "{value:?}");
        }
    }

    #[test]
    fn foreign_param_start_is_the_customer_s_value_never_a_sentinel() {
        let r = resolve(false, false, json!({"session_id": "start"}), false);
        assert_eq!(r.session_source, SessionSource::Foreign);
        assert_eq!(r.session_id, "");
    }
}

mod resolvehandles_hook_mode {
    use super::*;

    #[test]
    fn returns_a_provisional_resolution_empty_id_source_hook_hookmode_on() {
        let r = resolve(true, false, json!({}), true);
        assert!(r.hook_mode);
        assert_eq!(r.session_source, SessionSource::Hook);
        assert_eq!(r.session_id, "");
    }

    #[test]
    fn ignores_agent_supplied_session_id_the_hook_wins() {
        let r = resolve(true, false, json!({"session_id": "ses_agent_sent"}), true);
        assert_eq!(r.session_source, SessionSource::Hook);
        assert_ne!(r.session_id, "ses_agent_sent");
    }

    #[test]
    fn hook_mode_supplied_agent_id_resolves_omitted_stays_unresolved() {
        let supplied = resolve(
            true,
            true,
            json!({"agent_id": "opus-4.80-1m|claude-code|k3n9x"}),
            true,
        );
        assert_eq!(
            supplied.agent_id.as_deref(),
            Some("opus-4.80-1m|claude-code|k3n9x")
        );

        let omitted = resolve(true, true, json!({}), true);
        assert_eq!(omitted.agent_id, None);
    }
}

mod sessionfromhookvalue {
    use super::*;

    #[test]
    fn derives_deterministically_from_the_hook_value_project() {
        let (a_id, a_source) = session_from_hook_value(Some("customer-42"), Some("proj_1"));
        let (b_id, _) = session_from_hook_value(Some("customer-42"), Some("proj_1"));
        assert_eq!(a_source, SessionSource::Hook);
        assert_eq!(a_id, b_id);
        assert!(a_id.starts_with("ses_"), "{a_id}");
        assert_eq!(a_id, derive_session_id("customer-42", Some("proj_1")));
    }

    #[test]
    fn trims_the_value_before_deriving() {
        assert_eq!(
            session_from_hook_value(Some("  corr-1  "), Some("proj_1")).0,
            derive_session_id("corr-1", Some("proj_1"))
        );
    }

    #[test]
    fn mints_a_fresh_id_per_null_silent_mint_never_supplied() {
        let (a_id, a_source) = session_from_hook_value(None, Some("proj_1"));
        let (b_id, b_source) = session_from_hook_value(None, Some("proj_1"));
        assert_eq!(a_source, SessionSource::Minted);
        assert_eq!(b_source, SessionSource::Minted);
        assert!(a_id.starts_with("ses_"), "{a_id}");
        assert_ne!(a_id, b_id);
    }

    #[test]
    fn treats_a_whitespace_only_value_as_null() {
        assert_eq!(
            session_from_hook_value(Some("   "), Some("proj_1")).1,
            SessionSource::Minted
        );
    }
}

mod buildstructuredmintback {
    use super::*;

    #[test]
    fn task_minted_agent_supplied_both_ids_status_issued() {
        let mint = build_structured_mint_back(&res(T, SessionSource::Minted, false, Some(A))).unwrap();
        assert_eq!(mint["session_id"], T);
        assert_eq!(mint["agent_id"], A);
        assert_eq!(mint["status"], "issued");
        assert_eq!(keys(&mint), ["session_id", "agent_id", "status"]);
    }

    #[test]
    fn steady_state_both_supplied_ids_plus_status_active() {
        let mint = build_structured_mint_back(&res(T, SessionSource::Supplied, false, Some(A))).unwrap();
        assert_eq!(mint["session_id"], T);
        assert_eq!(mint["agent_id"], A);
        assert_eq!(mint["status"], "active");
    }

    #[test]
    fn agent_tracking_off_task_only_status_active() {
        let mint = build_structured_mint_back(&res(T, SessionSource::Supplied, false, None)).unwrap();
        assert_eq!(mint["session_id"], T);
        assert!(!mint.contains_key("agent_id"));
        assert_eq!(mint["status"], "active");
        assert_eq!(keys(&mint), ["session_id", "status"]);
    }

    #[test]
    fn hook_mode_never_exposes_session_id_or_status_supplied_agent_is_echoed() {
        let mint = build_structured_mint_back(&res(T, SessionSource::Minted, true, Some(A))).unwrap();
        assert!(!mint.contains_key("session_id"));
        assert!(!mint.contains_key("status"));
        assert_eq!(mint["agent_id"], A);
        assert_eq!(keys(&mint), ["agent_id"]);
    }

    #[test]
    fn hook_mode_with_agent_tracking_off_nothing_echoable_null() {
        assert_eq!(
            build_structured_mint_back(&res(T, SessionSource::Hook, true, None)),
            None
        );
    }
}

mod mirrorstructuredmintback {
    use super::*;

    fn mint() -> Map<String, Value> {
        obj(json!({"session_id": T, "status": "issued"}))
    }

    #[test]
    fn adds_the_field_as_the_first_key_of_structuredcontent_without_mutating() {
        let original = json!({"a": 1});
        let mut out = Some(original.clone());
        assert!(mirror_structured_mint_back(&mut out, mint()));
        let out = out.unwrap();
        assert_eq!(out[MCP_SESSION_KEY], Value::Object(mint()));
        assert_eq!(out["a"], 1);
        assert_eq!(keys(out.as_object().unwrap()), [MCP_SESSION_KEY, "a"]);
        assert_eq!(original, json!({"a": 1}));
        assert_ne!(out, original);
    }

    #[test]
    fn returns_the_input_untouched_when_structuredcontent_is_absent_or_not_a_69b9a407() {
        for input in [None, Some(json!([1, 2])), Some(json!("nope")), Some(Value::Null)] {
            let mut out = input.clone();
            assert!(!mirror_structured_mint_back(&mut out, mint()), "{input:?}");
            assert_eq!(out, input);
        }
    }

    #[test]
    fn never_clobbers_an_existing_key_customer_data_wins() {
        let input = Some(json!({"mcp_session": "customer-owned"}));
        let mut out = input.clone();
        assert!(!mirror_structured_mint_back(&mut out, mint()));
        assert_eq!(out, input);
    }

    #[test]
    fn applies_to_iserror_results_that_carry_structuredcontent() {
        let mut out = Some(json!({"msg": "boom"}));
        assert!(mirror_structured_mint_back(&mut out, mint()));
        let out = out.unwrap();
        assert_eq!(out[MCP_SESSION_KEY], Value::Object(mint()));
        assert_eq!(keys(out.as_object().unwrap()), [MCP_SESSION_KEY, "msg"]);
    }
}

mod isvalidsessionid {
    use super::*;

    #[test]
    fn accepts_an_id_this_sdk_actually_mints() {
        assert!(is_valid_session_id(&new_session_id()));
        assert!(is_valid_session_id(&derive_session_id(
            "anything",
            Some("proj_1")
        )));
    }

    #[test]
    fn accepts_the_test_helper_s_ids() {
        assert!(is_valid_session_id(&sid("parent")));
    }

    #[test]
    fn rejects_wrong_prefix() {
        assert!(!is_valid_session_id("task_2xF9kQm3rTvB8nLpYw7ZcHd4Ke"));
    }

    #[test]
    fn rejects_no_prefix() {
        assert!(!is_valid_session_id("2xF9kQm3rTvB8nLpYw7ZcHd4Ke1"));
    }

    #[test]
    fn rejects_too_short() {
        assert!(!is_valid_session_id("ses_abc"));
    }

    #[test]
    fn rejects_too_long() {
        assert!(!is_valid_session_id(&format!("ses_{}", "a".repeat(28))));
    }

    #[test]
    fn rejects_empty() {
        assert!(!is_valid_session_id(""));
    }

    #[test]
    fn rejects_prefix_only() {
        assert!(!is_valid_session_id("ses_"));
    }

    #[test]
    fn rejects_customer_value() {
        assert!(!is_valid_session_id("my-app-session-42"));
    }

    #[test]
    fn rejects_non_base62_body() {
        assert!(!is_valid_session_id(&format!("ses_{}", "-".repeat(27))));
    }

    #[test]
    fn rejects_inner_whitespace() {
        assert!(!is_valid_session_id(&format!("ses_ {}", "a".repeat(26))));
    }
}

mod invalid_and_foreign_mint_back {
    use super::*;

    fn invalid_res(agent_id: Option<&str>) -> HandleResolution {
        res("", SessionSource::Invalid, false, agent_id)
    }

    fn foreign_res(agent_id: Option<&str>) -> HandleResolution {
        res("", SessionSource::Foreign, false, agent_id)
    }

    #[test]
    fn invalid_corrects_the_agent_without_issuing_a_replacement() {
        let text = build_mint_back_text(&invalid_res(None)).unwrap();
        assert_eq!(
            text,
            "[session_id unrecognized — see this tool's session_id parameter description]\n\
             The value sent was not issued by this server. Re-send the session_id issued earlier for \
             this task; if none was issued yet, send start and one will be issued."
        );
        assert!(!Regex::new("ses_[0-9A-Za-z]{27}").unwrap().is_match(&text));
    }

    #[test]
    fn invalid_offers_an_escape_to_an_agent_that_was_never_issued_one() {
        let text = build_mint_back_text(&invalid_res(None)).unwrap();
        assert!(text.contains("if none was issued yet, send start and one will be issued."));
    }

    #[test]
    fn invalid_structured_mirror_carries_status_unrecognized_but_no_session_id() {
        let mint = build_structured_mint_back(&invalid_res(None)).unwrap();
        assert_eq!(mint["status"], "unrecognized");
        assert!(!mint.contains_key("session_id"));
        assert_eq!(keys(&mint), ["status"]);
    }

    #[test]
    fn invalid_agent_supplied_agent_id_rides_along_with_the_unrecognized_status() {
        let mint = build_structured_mint_back(&invalid_res(Some("opus|cc|k3n9x"))).unwrap();
        assert_eq!(mint["status"], "unrecognized");
        assert!(!mint.contains_key("session_id"));
        assert_eq!(mint["agent_id"], "opus|cc|k3n9x");
        assert_eq!(keys(&mint), ["agent_id", "status"]);
    }

    #[test]
    fn foreign_says_nothing_when_agent_id_is_not_in_play_either() {
        assert_eq!(build_mint_back_text(&foreign_res(None)), None);
        assert_eq!(build_structured_mint_back(&foreign_res(None)), None);
    }

    #[test]
    fn foreign_never_echoes_session_state_which_is_the_customer_s() {
        let mint = build_structured_mint_back(&foreign_res(Some("opus|cc|k3n9x"))).unwrap();
        assert!(!mint.contains_key("session_id"));
        assert!(!mint.contains_key(SESSION_ID_PARAM));
        assert!(!mint.contains_key("status"));
    }

    #[test]
    fn foreign_still_echoes_agent_id_which_agentcat_did_inject() {
        let mint = build_structured_mint_back(&foreign_res(Some("opus|cc|k3n9x"))).unwrap();
        assert_eq!(mint[AGENT_ID_PARAM], "opus|cc|k3n9x");
        assert_eq!(keys(&mint), ["agent_id"]);
    }
}

mod rust_extras {
    use super::*;

    fn non_strings() -> [Value; 5] {
        [
            json!(123),
            json!({}),
            Value::Null,
            json!(true),
            json!([sid("parent")]),
        ]
    }

    #[test]
    fn non_string_handles_resolve_like_omission() {
        for value in non_strings() {
            let r = resolve(false, true, json!({"session_id": value}), true);
            assert_eq!(r.session_source, SessionSource::Minted, "{value}");
            assert!(is_valid_session_id(&r.session_id), "{value}");
            assert_ne!(r.session_id, sid("parent"), "{value}");
        }
        for value in non_strings() {
            let r = resolve(false, true, json!({"agent_id": value}), true);
            assert_eq!(r.agent_id, None, "{value}");
        }
    }

    #[test]
    fn hook_mode_ignores_a_valid_supplied_session_id() {
        let r = resolve(true, false, json!({"session_id": sid("agent_sent")}), true);
        assert_eq!(r.session_source, SessionSource::Hook);
        assert_eq!(r.session_id, "");
    }

    #[test]
    fn agent_id_tag_cut_counts_utf16_units_and_never_splits_a_pair() {
        let emoji = "😀".repeat(150);
        let out = build_handle_tags(&res(T, SessionSource::Supplied, false, Some(&emoji)), None);
        assert_eq!(out["agentcat_agent_id"], "😀".repeat(100));

        let mixed = format!("a{}", "😀".repeat(100));
        let out = build_handle_tags(&res(T, SessionSource::Supplied, false, Some(&mixed)), None);
        assert_eq!(out["agentcat_agent_id"], format!("a{}", "😀".repeat(99)));
    }

    #[test]
    fn protocol_version_tag_is_sent_verbatim() {
        let r = res(T, SessionSource::Supplied, false, None);
        for version in ["2026-07-28\nX".to_owned(), "\r".to_owned(), "v".repeat(300)] {
            assert_eq!(
                build_handle_tags(&r, Some(&version))["agentcat_protocol_version"],
                version
            );
        }
        assert!(!build_handle_tags(&r, Some("")).contains_key("agentcat_protocol_version"));
    }
}
