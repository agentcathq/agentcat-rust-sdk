//! `handles` against the TS SDK: the vectors of `tests/vectors/handles.json`, property tests that
//! run the same inputs through the JS oracle, and Rust-only tests of the random IDs. A minted
//! session ID is random, so both sides check it is valid and write [`MINTED`] in its place.
use std::collections::BTreeSet;

use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::{
    HandleResolution, SessionSource, build_handle_tags, build_mint_back_text,
    build_structured_mint_back, derive_session_id, extract_handle, is_valid_session_id,
    new_event_id, new_session_id, resolve_handles, session_from_hook_value,
};
use crate::constants::{AGENT_ID_PARAM, SESSION_ID_PARAM, SESSION_START_SENTINEL};
use crate::test_support::strategies::{js_char, js_string};
use crate::test_support::{assert_js_eq, config, oracle, vectors};

/// The case name prefixes of `handles.json`.
const KINDS: [&str; 9] = [
    "derive",
    "valid",
    "extract",
    "resolve",
    "mint-back",
    "structured",
    "tags",
    "hook-value",
    "lowercase",
];

/// What the oracle and the vectors write in place of a minted session ID.
const MINTED: &str = "<minted>";

/// The base62 digits of a KSUID.
const BASE62: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Every session source.
const SOURCES: [SessionSource; 5] = [
    SessionSource::Hook,
    SessionSource::Supplied,
    SessionSource::Minted,
    SessionSource::Invalid,
    SessionSource::Foreign,
];

/// The source TS names `text`.
fn source_from_ts(text: &str) -> SessionSource {
    SOURCES
        .into_iter()
        .find(|source| source.as_str() == text)
        .unwrap_or_else(|| panic!("no session source {text:?}"))
}

/// The resolution `value` holds in TS's camelCase form. An `agentId` comes with
/// `agentSource: "supplied"`, the one agent source TS has.
fn resolution_from_ts(value: &Value) -> HandleResolution {
    let field = |name: &str| {
        value
            .get(name)
            .unwrap_or_else(|| panic!("resolution {value} has no {name}"))
    };
    let agent_id = value.get("agentId").map(|agent| {
        assert_eq!(
            value.get("agentSource"),
            Some(&json!("supplied")),
            "{value}"
        );
        agent.as_str().unwrap().to_owned()
    });
    HandleResolution {
        session_id: field("sessionId").as_str().unwrap().to_owned(),
        session_source: source_from_ts(field("sessionSource").as_str().unwrap()),
        hook_mode: field("hookMode").as_bool().unwrap(),
        agent_id,
    }
}

/// `res` in TS's camelCase form, keys in TS's order.
fn resolution_to_ts(res: &HandleResolution) -> Value {
    let mut ts = Map::new();
    ts.insert("sessionId".into(), json!(res.session_id));
    ts.insert("sessionSource".into(), json!(res.session_source.as_str()));
    ts.insert("hookMode".into(), json!(res.hook_mode));
    if let Some(agent) = &res.agent_id {
        ts.insert("agentId".into(), json!(agent));
        ts.insert("agentSource".into(), json!("supplied"));
    }
    Value::Object(ts)
}

/// `session_id`, or [`MINTED`] for a minted one after checking it is valid.
fn mask(session_id: &str, source: SessionSource) -> Value {
    if source == SessionSource::Minted {
        assert!(is_valid_session_id(session_id), "minted {session_id:?}");
        json!(MINTED)
    } else {
        json!(session_id)
    }
}

/// [`resolution_to_ts`] with a minted session ID masked.
fn masked_resolution(res: &HandleResolution) -> Value {
    let mut ts = resolution_to_ts(res);
    ts["sessionId"] = mask(&res.session_id, res.session_source);
    ts
}

/// The tool arguments of an `{args?, ...}` input: `None` when absent or not an object (rmcp's
/// tool arguments are an object or absent; TS reads nothing off any other value).
fn args_of(input: &Value) -> Option<&Map<String, Value>> {
    input.get("args").and_then(Value::as_object)
}

/// The string `field` of `input`: `None` when absent or `null`.
fn optional_str<'a>(input: &'a Value, field: &str) -> Option<&'a str> {
    match input.get(field) {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .unwrap_or_else(|| panic!("{field} of {input} is not a string")),
        ),
    }
}

fn flag(input: &Value, field: &str) -> bool {
    input[field]
        .as_bool()
        .unwrap_or_else(|| panic!("{field} of {input} is not a boolean"))
}

/// Checks one `handles.json` case against its TS output; returns its kind (the name prefix).
fn check_case(case: &Map<String, Value>) -> &'static str {
    let name = vectors::name(case);
    let kind = KINDS
        .into_iter()
        .find(|kind| {
            name.strip_prefix(kind)
                .is_some_and(|rest| rest.starts_with(':'))
        })
        .unwrap_or_else(|| panic!("{name}: unknown kind"));
    // The `lowercase` case has no input.
    let input = case.get("input").unwrap_or(&Value::Null);
    let output = &case["output"];
    match kind {
        "derive" => {
            let id = input["id"].as_str().unwrap();
            let actual = derive_session_id(id, optional_str(input, "projectId"));
            assert_js_eq(&json!(actual), output, name);
        }
        "valid" => {
            let value = input.as_str().unwrap();
            assert_js_eq(&json!(is_valid_session_id(value)), output, name);
        }
        "extract" => {
            let param = input["name"].as_str().unwrap();
            let actual = extract_handle(args_of(input), param);
            assert_js_eq(&json!(actual), output, name);
        }
        "resolve" => {
            let resolve = || {
                resolve_handles(
                    args_of(input),
                    flag(input, "hookMode"),
                    flag(input, "agentTracking"),
                    flag(input, "sessionParamIsOurs"),
                )
                .unwrap_or_else(|| panic!("{name}: minting failed"))
            };
            let res = resolve();
            assert_js_eq(&masked_resolution(&res), output, name);
            if res.session_source == SessionSource::Minted {
                let again = resolve();
                assert_ne!(res.session_id, again.session_id, "{name}: one ID per mint");
            }
        }
        "mint-back" => {
            let actual = build_mint_back_text(&resolution_from_ts(input));
            assert_js_eq(&json!(actual), output, name);
        }
        "structured" => {
            let actual = build_structured_mint_back(&resolution_from_ts(input));
            assert_js_eq(&json!(actual), output, name);
        }
        "tags" => {
            let res = resolution_from_ts(&input["res"]);
            let actual = build_handle_tags(&res, optional_str(input, "protocolVersion"));
            assert_js_eq(&Value::Object(actual), output, name);
        }
        "hook-value" => {
            let session = || {
                session_from_hook_value(
                    optional_str(input, "value"),
                    optional_str(input, "projectId"),
                )
                .unwrap_or_else(|| panic!("{name}: minting failed"))
            };
            let (session_id, source) = session();
            let actual = json!({
                "sessionId": mask(&session_id, source),
                "sessionSource": source.as_str(),
            });
            assert_js_eq(&actual, output, name);
            if source == SessionSource::Minted {
                assert_ne!(session_id, session().0, "{name}: one ID per mint");
            }
        }
        "lowercase" => check_lowercase(case),
        _ => unreachable!("{name}: kind {kind:?} has no check"),
    }
    kind
}

/// The sentinel test of `resolve_handles` is `eq_ignore_ascii_case`, TS's
/// `toLowerCase() === "start"`. The two agree when no character but the letter in its two ASCII
/// cases has that letter as its lowercase, and no character lowercases to a part of the sentinel.
/// The case lists them for every character of the BMP, as this ICU version of TS gives them.
fn check_lowercase(case: &Map<String, Value>) {
    let name = vectors::name(case);
    let letters = case["output"]["letters"]
        .as_object()
        .unwrap_or_else(|| panic!("{name}: output.letters is not an object"));
    let sentinel: BTreeSet<char> = SESSION_START_SENTINEL.chars().collect();
    assert_eq!(
        letters.keys().cloned().collect::<BTreeSet<_>>(),
        sentinel.iter().map(char::to_string).collect(),
        "{name}: one entry per letter of the sentinel"
    );
    for (letter, chars) in letters {
        let upper = letter.to_ascii_uppercase();
        let listed: BTreeSet<String> = chars
            .as_array()
            .unwrap_or_else(|| panic!("{name}: letters.{letter} is not an array"))
            .iter()
            .map(|c| {
                c.as_str()
                    .unwrap_or_else(|| panic!("{name}: {c} is not a string"))
                    .to_owned()
            })
            .collect();
        assert_eq!(
            listed,
            BTreeSet::from([letter.clone(), upper]),
            "{name}: only the ASCII cases of {letter} lowercase to it"
        );
        for unit in (0_u32..=0xFFFF).filter_map(char::from_u32) {
            let text = unit.to_string();
            assert_eq!(
                text.eq_ignore_ascii_case(letter),
                listed.contains(&text),
                "{name}: U+{:04X} against {letter}",
                u32::from(unit)
            );
        }
    }
    for (character, lower) in case["output"]["expanding"]
        .as_object()
        .unwrap_or_else(|| panic!("{name}: output.expanding is not an object"))
    {
        let lower = lower
            .as_str()
            .unwrap_or_else(|| panic!("{name}: expanding.{character} is not a string"));
        assert!(
            !SESSION_START_SENTINEL.contains(lower),
            "{name}: {character:?} lowercases to {lower:?}, a part of the sentinel"
        );
    }
}

#[test]
fn vectors_match_ts() {
    let seen: BTreeSet<&str> = vectors::cases("handles").iter().map(check_case).collect();
    assert_eq!(
        seen,
        BTreeSet::from(KINDS),
        "handles.json must have cases of every kind"
    );
}

/// The value-contract test of `constants.test.ts`: `SESSION_ID_PARAM_PATTERN` (whose text
/// `constants::tests::every_ported_constant_equals_ts` checks) accepts exactly the
/// `is_valid_session_id` shape and the start sentinel. Each `valid:*` case carries the verdict of
/// `new RegExp(SESSION_ID_PARAM_PATTERN)` in TS.
#[test]
fn pattern_is_the_valid_shape_plus_the_sentinel() {
    let cases = vectors::cases("handles");
    let valid: Vec<_> = cases
        .iter()
        .filter(|case| vectors::name(case).starts_with("valid:"))
        .collect();
    for case in &valid {
        let name = vectors::name(case);
        let input = case["input"].as_str().unwrap();
        let expected = is_valid_session_id(input) || input == SESSION_START_SENTINEL;
        assert_eq!(case["pattern"].as_bool(), Some(expected), "{name}");
    }
    let contract = valid
        .iter()
        .filter(|case| vectors::name(case).starts_with("valid:contract:"))
        .count();
    assert_eq!(contract, 5, "the five inputs of the value-contract test");
}

/// A session ID's shape, checked without `is_valid_session_id`: `ses_` and 27 base62 digits.
fn has_session_shape(id: &str) -> bool {
    id.strip_prefix("ses_")
        .is_some_and(|body| body.chars().count() == 27 && body.chars().all(|c| BASE62.contains(c)))
}

#[test]
fn new_ids_have_the_shape() {
    let first = new_session_id().unwrap();
    let second = new_session_id().unwrap();
    for id in [&first, &second] {
        assert!(has_session_shape(id), "{id}");
        assert!(is_valid_session_id(id), "{id}");
    }
    assert_ne!(first, second);
    let derived = derive_session_id("anything", Some("proj_1"));
    assert!(has_session_shape(&derived), "{derived}");
    assert!(is_valid_session_id(&derived), "{derived}");
    let event = new_event_id().unwrap();
    let body = event.strip_prefix("evt_").unwrap();
    assert_eq!(body.chars().count(), 27, "{event}");
    assert!(body.chars().all(|c| BASE62.contains(c)), "{event}");
    assert_ne!(event, new_event_id().unwrap());
}

/// A string around the `ses_` shape: a right or wrong prefix, 25 to 29 base62 digits, sometimes
/// with one of them replaced or one character inserted, and sometimes a trailing character.
fn session_shaped() -> impl Strategy<Value = String> + Clone {
    let prefix = prop::sample::select(vec![
        "ses_", "ses_", "ses_", "SES_", "Ses_", "ses", "ses-", "evt_", "", " ses_", "sess_",
    ]);
    let digit = prop::sample::select(BASE62.chars().collect::<Vec<_>>());
    let body = prop::collection::vec(digit, 25..=29).prop_map(String::from_iter);
    let edit = proptest::option::of((js_char(), 0..30_usize, any::<bool>()));
    let tail = prop::sample::select(vec![
        "", "", "", "\n", "\r", " ", "\u{85}", "\u{FEFF}", "\u{2028}",
    ]);
    (prefix, body, edit, tail).prop_map(|(prefix, mut body, edit, tail)| {
        // The body is ASCII, so every index up to its length is a character boundary.
        if let Some((c, at, replace)) = edit {
            let at = at.min(body.len());
            if replace && at < body.len() {
                body.replace_range(at..=at, c.encode_utf8(&mut [0; 4]));
            } else {
                body.insert(at, c);
            }
        }
        format!("{prefix}{body}{tail}")
    })
}

/// JavaScript whitespace and its neighbors that `trim` keeps, 0 to 2 of them.
fn padding() -> impl Strategy<Value = String> + Clone {
    let c = prop::sample::select(vec![
        ' ', '\t', '\n', '\r', '\u{A0}', '\u{3000}', '\u{FEFF}', '\u{85}', '\u{180E}', '\u{200B}',
    ]);
    prop::collection::vec(c, 0..=2).prop_map(String::from_iter)
}

/// Spellings of the start sentinel: each letter in either ASCII case or a non-ASCII look-alike
/// (`ſ`, fullwidth letters, small capital S), padded, sometimes inside a longer word.
fn start_spelling() -> impl Strategy<Value = String> + Clone {
    let letter = |options: &[char]| prop::sample::select(options.to_vec());
    let word = (
        letter(&['s', 'S', 's', 'S', 'ſ', '\u{FF33}', '\u{A731}']),
        letter(&['t', 'T', 't', 'T', '\u{FF54}']),
        letter(&['a', 'A', 'a', 'A', '\u{FF21}']),
        letter(&['r', 'R']),
        letter(&['t', 'T']),
    )
        .prop_map(|(s, t, a, r, t2)| String::from_iter([s, t, a, r, t2]));
    let affix = prop::sample::select(vec!["", "", "", "", "re", "s", " now"]);
    (padding(), affix.clone(), word, affix, padding())
        .prop_map(|(head, before, word, after, tail)| format!("{head}{before}{word}{after}{tail}"))
}

/// A valid session ID, sometimes padded.
fn padded_valid_id() -> impl Strategy<Value = String> + Clone {
    let digit = prop::sample::select(BASE62.chars().collect::<Vec<_>>());
    let body = prop::collection::vec(digit, 27).prop_map(String::from_iter);
    (padding(), body, padding()).prop_map(|(head, body, tail)| format!("{head}ses_{body}{tail}"))
}

/// A `session_id` or `agent_id` argument (`None`: the key is absent).
fn handle_value() -> impl Strategy<Value = Option<Value>> + Clone {
    let other = prop::sample::select(vec![
        json!(null),
        json!(42),
        json!(true),
        json!(["start"]),
        json!({"a": "start"}),
    ]);
    prop_oneof![
        1 => Just(None),
        3 => js_string(8).prop_map(|s| Some(json!(s))),
        3 => start_spelling().prop_map(|s| Some(json!(s))),
        2 => padded_valid_id().prop_map(|s| Some(json!(s))),
        1 => other.prop_map(Some),
    ]
}

/// Tool arguments: absent, or an object with `session_id`, `agent_id` and another key, each
/// sometimes absent.
fn handle_args() -> impl Strategy<Value = Option<Map<String, Value>>> + Clone {
    let args = (
        handle_value(),
        handle_value(),
        proptest::option::of(js_string(4)),
    )
        .prop_map(|(session, agent, other)| {
            let mut args = Map::new();
            if let Some(other) = other {
                args.insert("other".into(), json!(other));
            }
            if let Some(agent) = agent {
                args.insert(AGENT_ID_PARAM.into(), agent);
            }
            if let Some(session) = session {
                args.insert(SESSION_ID_PARAM.into(), session);
            }
            args
        });
    prop_oneof![1 => Just(None), 9 => args.prop_map(Some)]
}

/// An agent ID for the tags: any [`js_string`], or one whose 200th UTF-16 unit is near its end.
fn tag_agent_id() -> impl Strategy<Value = String> + Clone {
    prop_oneof![
        js_string(260),
        (197_usize..=201, js_string(3)).prop_map(|(n, tail)| "a".repeat(n) + &tail),
    ]
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn derive_matches_ts(id in js_string(40), project in proptest::option::of(js_string(10))) {
        let mut args = vec![json!(id)];
        if let Some(project) = &project {
            args.push(json!(project));
        }
        let js = oracle::call("handles.deriveSessionId", &args).unwrap();
        prop_assert_eq!(Some(json!(derive_session_id(&id, project.as_deref()))), js);
    }

    #[test]
    fn is_valid_matches_ts(value in prop_oneof![4 => session_shaped(), 1 => js_string(30)]) {
        let js = oracle::call("handles.isValidSessionId", &[json!(value)]).unwrap();
        prop_assert_eq!(Some(json!(is_valid_session_id(&value))), js, "{:?}", value);
    }

    #[test]
    fn extract_and_resolve_match_ts(
        args in handle_args(),
        hook_mode in any::<bool>(),
        agent_tracking in any::<bool>(),
        session_param_is_ours in any::<bool>(),
    ) {
        let js_args = args.clone().map_or(Value::Null, Value::Object);
        for param in [SESSION_ID_PARAM, AGENT_ID_PARAM] {
            let js = oracle::call("handles.extractHandle", &[js_args.clone(), json!(param)]).unwrap();
            prop_assert_eq!(extract_handle(args.as_ref(), param).map(Value::from), js, "{}", param);
        }
        let mut input = json!({
            "hookMode": hook_mode,
            "agentTracking": agent_tracking,
            "sessionParamIsOurs": session_param_is_ours,
        });
        if let Some(args) = &args {
            input["args"] = Value::Object(args.clone());
        }
        let js = oracle::call("oracle.resolveHandles", &[input.clone()]).unwrap().unwrap();
        let res = resolve_handles(args.as_ref(), hook_mode, agent_tracking, session_param_is_ours)
            .unwrap();
        assert_js_eq(&masked_resolution(&res), &js, &input.to_string());
    }

    #[test]
    fn handle_tags_match_ts(
        agent_id in proptest::option::of(tag_agent_id()),
        source in prop::sample::select(SOURCES.to_vec()),
        hook_mode in any::<bool>(),
        protocol_version in proptest::option::of(prop_oneof![
            Just(String::new()),
            Just("2025-11-25".to_owned()),
            js_string(6),
        ]),
    ) {
        let res = HandleResolution {
            session_id: String::new(),
            session_source: source,
            hook_mode,
            agent_id,
        };
        let mut args = vec![resolution_to_ts(&res)];
        if let Some(version) = &protocol_version {
            args.push(json!(version));
        }
        let js = oracle::call("handles.buildHandleTags", &args).unwrap().unwrap();
        let actual = build_handle_tags(&res, protocol_version.as_deref());
        assert_js_eq(&Value::Object(actual), &js, &format!("{args:?}"));
    }
}
