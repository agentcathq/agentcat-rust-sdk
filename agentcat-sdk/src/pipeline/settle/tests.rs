//! The settling of the call hooks and the application of their results (C37, TS's
//! `settleWithTimeout` and `applyPendingEventFields`, B-084, B-093, B-095, B-096): each `hooks:*`
//! case of `tests/vectors/queue.json` against TS, and the Rust-only paths (the abort after the
//! budget, RD-35; a panicking hook). Each hook is a task on the test runtime that has finished
//! (`returns`, `throws`) or never finishes (`hangs`), and every wait uses a 50 ms budget; the
//! timeout text keeps TS's `30000` whatever the budget is.
use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::{
    EVENT_PROPERTIES, EVENT_TAGS, HookResults, IDENTIFY, RESOLVE_SESSION_ID, apply, settle,
};
use crate::config::{HookError, UserIdentity};
use crate::handles::{derive_session_id, is_valid_session_id};
use crate::hooks::PendingHooks;
use crate::js;
use crate::pipeline::TsEvent;
use crate::test_support::{assert_js_eq, vectors};

/// The hook budget of every test.
const BUDGET: Duration = Duration::from_millis(50);

/// The task of one hook.
type Task<T> = JoinHandle<Result<Option<T>, HookError>>;

/// A hook task that gives `value`.
fn returns<T: Send + 'static>(value: Option<T>) -> Task<T> {
    tokio::spawn(async move { Ok(value) })
}

/// A hook task that fails with the error `message`.
fn throws<T: Send + 'static>(message: &str) -> Task<T> {
    let message = message.to_owned();
    tokio::spawn(async move { Err(message.into()) })
}

/// A hook task that never finishes.
fn hangs<T: Send + 'static>() -> Task<T> {
    tokio::spawn(pending())
}

/// Sets its flag when it is dropped.
struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// A hook task that never finishes and sets `dropped` when its future is dropped.
fn hangs_watched<T: Send + 'static>(dropped: &Arc<AtomicBool>) -> Task<T> {
    let flag = DropFlag(Arc::clone(dropped));
    tokio::spawn(async move {
        let _flag = flag;
        pending::<Result<Option<T>, HookError>>().await
    })
}

/// The JSON object `value`.
fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        other => panic!("{other} is not an object"),
    }
}

/// A tool-call event as `from_call` leaves it, in TS key order, with the session `session` (`""`
/// in hook mode) and the SDK tags `tags`.
fn call_event(session: &str, tags: Value) -> TsEvent {
    let mut event = object(json!({
        "id": "",
        "sessionId": session,
        "projectId": "proj_1",
        "eventType": "mcp:tools/call",
        "timestamp": "2026-10-23T11:59:59.250Z",
        "duration": 750,
        "sdkLanguage": "Rust",
        "serverName": "server",
        "clientName": "client",
        "identifyActorData": {},
        "resourceName": "echo",
        "parameters": {"request": {}, "extra": {}},
        "response": {"content": []},
    }));
    event.insert("tags".to_owned(), tags);
    event
}

/// The keys of `event`, in order.
fn keys(event: &TsEvent) -> Vec<&str> {
    event.keys().map(String::as_str).collect()
}

/// TS's `applyPendingEventFields` order (B-084, B-096, B-095): the hook-mode session first, then
/// the identity in its TS places, then the customer tags under the SDK tags, then the properties.
#[tokio::test]
async fn settled_hooks_apply_in_ts_order() {
    let user = UserIdentity::new("user-42")
        .user_name("Ada")
        .user_data(object(json!({"plan": "pro"})));
    let hooks = PendingHooks {
        resolve_session_id: Some(returns(Some("  customer-abc ".to_owned()))),
        identify: Some(returns(Some(user))),
        event_tags: Some(returns(Some(object(
            json!({"env": "test", "agentcat_session_id_source": "customer"}),
        )))),
        event_properties: Some(returns(Some(object(json!({"device": "mobile"}))))),
    };
    let mut log = Vec::new();
    let results = settle(hooks, BUDGET, &mut log).await;
    assert_eq!(log, Vec::<String>::new());

    let mut event = call_event("", json!({"agentcat_session_id_source": "hook"}));
    apply(&mut event, results, true).expect("a derived session needs no minting");
    let expected = json!({
        "id": "",
        "sessionId": derive_session_id("customer-abc", Some("proj_1")),
        "projectId": "proj_1",
        "eventType": "mcp:tools/call",
        "timestamp": "2026-10-23T11:59:59.250Z",
        "duration": 750,
        "sdkLanguage": "Rust",
        "serverName": "server",
        "clientName": "client",
        "identifyActorGivenId": "user-42",
        "identifyActorName": "Ada",
        "identifyActorData": {"plan": "pro"},
        "resourceName": "echo",
        "parameters": {"request": {}, "extra": {}},
        "response": {"content": []},
        "tags": {"env": "test", "agentcat_session_id_source": "hook"},
        "properties": {"device": "mobile"},
    });
    assert_eq!(
        js::stringify(&Value::Object(event)),
        js::stringify(&expected)
    );
}

/// C37, RD-35: a hook that has not settled within the budget is aborted after TS's timeout line,
/// which names TS's 30 000 ms whatever the budget is, and the event goes on without it.
#[tokio::test]
async fn hanging_hook_is_aborted_after_budget() {
    let dropped = Arc::new(AtomicBool::new(false));
    let hooks = PendingHooks {
        identify: Some(hangs_watched(&dropped)),
        event_tags: Some(returns(Some(object(json!({"env": "test"}))))),
        ..PendingHooks::default()
    };
    let mut log = Vec::new();
    let started = Instant::now();
    let results = settle(hooks, BUDGET, &mut log).await;
    let waited = started.elapsed();
    assert!(waited >= BUDGET, "settled after {waited:?}");
    assert!(waited < Duration::from_secs(2), "settled after {waited:?}");
    assert_eq!(
        log,
        ["Warning: identify hook did not settle within 30000ms; publishing event without it"]
    );
    assert_eq!(results.identity, None);
    assert_eq!(results.tags, Some(object(json!({"env": "test"}))));

    let observed = tokio::time::timeout(Duration::from_secs(1), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        observed.is_ok(),
        "the hook's future was not dropped: no abort"
    );
}

/// C37: a hook that panics gives TS's background-failure line with the panic's message and no
/// value; the other hooks still apply. A panicking `resolve_session_id` (what a synchronous throw
/// of TS's hook becomes, since a hook's code runs when its task is first polled) leaves hook mode
/// to mint a session, as TS's contained throw does. The panic also prints through the process
/// panic hook.
#[tokio::test]
async fn panicking_hook_logs_and_publishes_without_it() {
    let session: Task<String> = tokio::spawn(async { panic!("sync hook boom") });
    let identify: Task<UserIdentity> = tokio::spawn(async { panic!("identify exploded") });
    let hooks = PendingHooks {
        resolve_session_id: Some(session),
        identify: Some(identify),
        event_properties: Some(returns(Some(object(json!({"device": "mobile"}))))),
        ..PendingHooks::default()
    };
    let mut log = Vec::new();
    let results = settle(hooks, BUDGET, &mut log).await;
    assert_eq!(
        log,
        [
            "Warning: resolveSessionId hook failed in background - sync hook boom",
            "Warning: identify hook failed in background - identify exploded",
        ]
    );
    assert_eq!(results.session_value, None);
    assert_eq!(results.identity, None);

    let mut event = call_event("", json!({"agentcat_session_id_source": "hook"}));
    apply(&mut event, results, true).expect("minting works");
    let session = event["sessionId"].as_str().expect("a session");
    assert!(is_valid_session_id(session), "{session}");
    assert_eq!(
        event["tags"],
        json!({"agentcat_session_id_source": "minted"})
    );
    assert_eq!(event.get("identifyActorGivenId"), None);
    assert_eq!(event["identifyActorData"], json!({}));
    assert_eq!(event["properties"], json!({"device": "mobile"}));
}

/// A hook that is not set and a hook that gives nothing (TS's `null`) give no result and no log
/// line, and the event keeps what the call gave it: no identity, the SDK tags only, no
/// properties.
#[tokio::test]
async fn unset_and_null_hooks_change_nothing() {
    let mut log = Vec::new();
    let results = settle(PendingHooks::default(), BUDGET, &mut log).await;
    assert!(results.identity.is_none() && results.tags.is_none());
    assert!(results.session_value.is_none() && results.properties.is_none());

    let hooks = PendingHooks {
        identify: Some(returns(None)),
        event_tags: Some(returns(None)),
        event_properties: Some(returns(None)),
        ..PendingHooks::default()
    };
    let results = settle(hooks, BUDGET, &mut log).await;
    assert_eq!(log, Vec::<String>::new());
    let mut event = call_event("ses_1", json!({"agentcat_session_id_source": "supplied"}));
    let before = event.clone();
    apply(&mut event, results, false).expect("no session step");
    assert_eq!(
        js::stringify(&Value::Object(event)),
        js::stringify(&Value::Object(before))
    );
}

/// A hook's own error gives TS's per-hook text followed by the error's `Display` (C37), in the
/// order TS logs hooks that settle together.
#[tokio::test]
async fn hook_errors_use_ts_texts() {
    let hooks = PendingHooks {
        resolve_session_id: Some(throws("no session")),
        identify: Some(throws("no identity")),
        event_tags: Some(throws("no tags")),
        event_properties: Some(throws("no properties")),
    };
    let mut log = Vec::new();
    let results = settle(hooks, BUDGET, &mut log).await;
    assert_eq!(
        log,
        [
            "resolveSessionId hook error: no session",
            "Error: identify callback threw - no identity",
            "eventTags callback error: no tags",
            "eventProperties callback error: no properties",
        ]
    );
    assert_eq!(results.session_value, None);
    assert_eq!(results.identity, None);
    assert_eq!(results.tags, None);
    assert_eq!(results.properties, None);
}

/// B-093: the `eventTags` result goes through `validate_tags` when it settles; each dropped tag
/// gives TS's line, and only the valid tags remain.
#[tokio::test]
async fn invalid_tags_are_dropped_with_ts_logs() {
    let raw = object(json!({"bad/key": "x", "count": 5, "env": "prod"}));
    let hooks = PendingHooks {
        event_tags: Some(returns(Some(raw))),
        ..PendingHooks::default()
    };
    let mut log = Vec::new();
    let results = settle(hooks, BUDGET, &mut log).await;
    assert_eq!(
        log,
        [
            "Dropping invalid tag: \"bad/key\" \u{2014} key contains invalid characters or is empty",
            "Dropping invalid tag: \"count\" \u{2014} non-string value (got number)",
        ]
    );
    assert_eq!(results.tags, Some(object(json!({"env": "prod"}))));
}

/// B-096: `{...customerTags, ...sdkTags}`: an SDK tag keeps its value on a name collision, at the
/// customer's place in the key order.
#[test]
fn sdk_tags_win_collisions() {
    let mut event = call_event("ses_1", json!({"agentcat_session_id_source": "supplied"}));
    let results = HookResults {
        tags: Some(object(
            json!({"agentcat_session_id_source": "customer", "env": "test"}),
        )),
        ..HookResults::default()
    };
    apply(&mut event, results, false).expect("no session step");
    assert_eq!(
        js::stringify(&event["tags"]),
        r#"{"agentcat_session_id_source":"supplied","env":"test"}"#
    );
}

/// In hook mode a missing hook value mints a session with the `minted` source (TS's
/// `sessionFromHookValue`); outside hook mode the session stays as the call resolved it.
#[test]
fn hook_mode_without_value_mints() {
    let mut event = call_event("", json!({"agentcat_session_id_source": "hook"}));
    apply(&mut event, HookResults::default(), true).expect("minting works");
    let session = event["sessionId"].as_str().expect("a session");
    assert!(is_valid_session_id(session), "{session}");
    assert_eq!(
        event["tags"],
        json!({"agentcat_session_id_source": "minted"})
    );
    assert_eq!(
        keys(&event),
        keys(&call_event("", json!({}))),
        "no key moves"
    );

    let mut untouched = call_event("", json!({"agentcat_session_id_source": "invalid"}));
    apply(&mut untouched, HookResults::default(), false).expect("no session step");
    assert_eq!(untouched["sessionId"], json!(""));
    assert_eq!(
        untouched["tags"],
        json!({"agentcat_session_id_source": "invalid"})
    );
}

// ------------------------------------------------------------------- queue.json hooks:* cases

/// The task of a raw hook outcome of `queue.json`: `{"returns": v}` (`null` gives nothing),
/// `{"throws": message}` or `{"hangs": true}`; `value` reads a returned value.
fn outcome_task<T: Send + 'static>(outcome: &Value, value: fn(&Value) -> T) -> Task<T> {
    if outcome.get("hangs").is_some() {
        return hangs();
    }
    if let Some(message) = outcome.get("throws") {
        return throws(message.as_str().expect("a message"));
    }
    let returned = &outcome["returns"];
    returns((!returned.is_null()).then(|| value(returned)))
}

/// The `UserIdentity` of an identify result `{userId, userName?, userData?}`.
fn identity_of(value: &Value) -> UserIdentity {
    let mut identity = UserIdentity::new(value["userId"].as_str().expect("a userId"));
    if let Some(name) = value.get("userName").and_then(Value::as_str) {
        identity = identity.user_name(name);
    }
    if let Some(Value::Object(data)) = value.get("userData") {
        identity = identity.user_data(data.clone());
    }
    identity
}

/// The hook tasks of a case's `hooks`.
fn pending_of(hooks: &Map<String, Value>) -> PendingHooks {
    PendingHooks {
        resolve_session_id: hooks.get("resolveSessionId").map(|outcome| {
            outcome_task(outcome, |value| {
                value.as_str().expect("a string").to_owned()
            })
        }),
        identify: hooks
            .get("identify")
            .map(|outcome| outcome_task(outcome, identity_of)),
        event_tags: hooks
            .get("eventTags")
            .map(|outcome| outcome_task(outcome, |value| object(value.clone()))),
        event_properties: hooks
            .get("eventProperties")
            .map(|outcome| outcome_task(outcome, |value| object(value.clone()))),
    }
}

/// A TS log line as Rust writes it (ruling P7): TS renders a thrown `Error` as `Error: M` after
/// its per-hook text, Rust writes the error's `Display`, `M`.
fn ts_line_as_rust(line: &str) -> String {
    let prefixes = [
        RESOLVE_SESSION_ID.error_prefix,
        IDENTIFY.error_prefix,
        EVENT_TAGS.error_prefix,
        EVENT_PROPERTIES.error_prefix,
    ];
    for prefix in prefixes {
        let message = line
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix("Error: "));
        if let Some(message) = message {
            return format!("{prefix}{message}");
        }
    }
    line.to_owned()
}

/// The request fields `apply` decides (ruling P3), each with the event key it comes from.
const APPLIED: [(&str, &str); 6] = [
    ("sessionId", "sessionId"),
    ("identifyActorGivenId", "identifyActorGivenId"),
    ("identifyActorName", "identifyActorName"),
    ("identifyData", "identifyActorData"),
    ("tags", "tags"),
    ("properties", "properties"),
];

/// Runs the `queue.json` case `name`: its hook outcomes settle under [`BUDGET`] and apply to its
/// event. The log lines must be TS's (without the delivery lines, Tasks 5 and 6), and each field
/// of [`APPLIED`] must be the one of TS's first request; a minted session (`<minted>`) must be a
/// valid session ID with the `minted` source. A case without a request checks only the logs.
async fn check_hook_case(name: &str) {
    let cases = vectors::cases("queue");
    let case = cases
        .iter()
        .find(|case| vectors::name(case) == name)
        .unwrap_or_else(|| panic!("queue.json has no case {name}"));
    let hooks = case["hooks"].as_object().expect("hooks");
    let hook_mode = case.get("hook_mode").and_then(Value::as_bool) == Some(true);
    let mut event = object(case["event"].clone());

    let mut log = Vec::new();
    let results = settle(pending_of(hooks), BUDGET, &mut log).await;
    let expected_logs: Vec<String> = case["logs"]
        .as_array()
        .expect("logs")
        .iter()
        .map(|line| line.as_str().expect("a line"))
        .filter(|line| {
            !line.starts_with("Successfully sent event ")
                && !line.starts_with("Failed to send event ")
        })
        .map(ts_line_as_rust)
        .collect();
    assert_eq!(log, expected_logs, "{name}: logs");

    apply(&mut event, results, hook_mode).expect("minting works");
    let Some(request) = case["requests"]
        .as_array()
        .and_then(|requests| requests.first())
    else {
        return;
    };
    for (wire, key) in APPLIED {
        let context = format!("{name}: {wire}");
        match (event.get(key), request.get(wire)) {
            (Some(Value::String(session)), Some(Value::String(minted)))
                if key == "sessionId" && minted == "<minted>" =>
            {
                assert!(is_valid_session_id(session), "{context}: {session}");
                assert_eq!(
                    event["tags"]["agentcat_session_id_source"],
                    json!("minted"),
                    "{context}"
                );
            }
            (Some(Value::String(session)), Some(Value::Null)) if key == "sessionId" => {
                assert_eq!(session, "", "{context}");
            }
            (Some(actual), Some(expected)) => assert_js_eq(actual, expected, &context),
            (None, None) => {}
            (actual, expected) => panic!("{context}: {actual:?} where TS has {expected:?}"),
        }
    }
}

/// Generates one test per `queue.json` `hooks:*` case, and [`HOOK_CASES`], their names.
macro_rules! hook_cases {
    ($($test:ident => $case:literal,)*) => {
        $(
            #[tokio::test]
            async fn $test() {
                check_hook_case($case).await;
            }
        )*

        /// The `hooks:*` cases that have a test.
        const HOOK_CASES: &[&str] = &[$($case),*];
    };
}

hook_cases! {
    queue_hooks_identify => "hooks:identify",
    queue_hooks_identify_no_data => "hooks:identify-no-data",
    queue_hooks_identify_null => "hooks:identify-null",
    queue_hooks_identify_throws => "hooks:identify-throws",
    queue_hooks_identify_hangs => "hooks:identify-hangs",
    queue_hooks_tags_invalid_mixed => "hooks:tags-invalid:mixed",
    queue_hooks_tags_invalid_51_valid => "hooks:tags-invalid:51-valid",
    queue_hooks_tags_invalid_none_valid => "hooks:tags-invalid:none-valid",
    queue_hooks_tags_invalid_ts_bang_key => "hooks:tags-invalid:ts-bang-key",
    queue_hooks_tags_collide => "hooks:tags-collide",
    queue_hooks_tags_throws => "hooks:tags-throws",
    queue_hooks_properties => "hooks:properties",
    queue_hooks_properties_empty => "hooks:properties-empty",
    queue_hooks_properties_throws => "hooks:properties-throws",
    queue_hooks_session_hook => "hooks:session-hook",
    queue_hooks_session_hook_telemetry_only => "hooks:session-hook-telemetry-only",
    queue_hooks_session_hook_null => "hooks:session-hook-null",
    queue_hooks_session_hook_throws => "hooks:session-hook-throws",
    queue_hooks_all_hang => "hooks:all-hang",
    queue_hooks_all_settle => "hooks:all-settle",
}

/// Every `hooks:*` case of `queue.json` has its test above.
#[test]
fn every_hook_case_has_a_test() {
    let cases = vectors::cases("queue");
    let names: Vec<&str> = cases
        .iter()
        .map(vectors::name)
        .filter(|name| name.starts_with("hooks:"))
        .collect();
    assert_eq!(names, HOOK_CASES);
}
