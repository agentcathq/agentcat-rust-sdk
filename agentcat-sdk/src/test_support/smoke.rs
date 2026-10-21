//! Smoke test of the JS oracle: Rust talks to the real TS functions through a Node process.
use std::path::Path;

use serde_json::{Value, json};

use super::{config, oracle};

#[test]
fn js_drops_the_fraction_of_whole_floats() {
    let out = oracle::call("JSON.stringify", &[json!({"n": 1.0})]).unwrap();
    assert_eq!(out, Some(json!("{\"n\":1}")));
}

#[test]
fn js_undefined_is_none() {
    let out = oracle::call("tokenEstimate.estimateInputTokens", &[Value::Null]).unwrap();
    assert_eq!(out, None);
}

#[test]
fn an_unknown_function_is_an_err_and_the_oracle_keeps_serving() {
    let mut oracle = oracle::lock();
    let err = oracle.call("nope", &[]).unwrap_err();
    assert_eq!(err.message, "unknown function \"nope\"");
    assert_eq!(err.stack, None);
    assert_eq!(
        oracle.call("JSON.stringify", &[json!(1)]).unwrap(),
        Some(json!("1"))
    );
}

#[test]
fn a_panic_while_holding_the_oracle_does_not_break_the_next_call() {
    assert!(oracle::call("nope", &[]).is_err());
    let panicked = std::panic::catch_unwind(|| {
        let _guard = oracle::lock();
        panic!("a test failed while it held the oracle");
    });
    assert!(panicked.is_err());
    assert!(oracle::is_poisoned());
    assert_eq!(
        oracle::call("JSON.stringify", &[json!(7)]).unwrap(),
        Some(json!("7"))
    );
}

#[test]
fn oracle_runs_the_locked_sha() {
    let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("../oracle/oracle.lock");
    let lock: Value = serde_json::from_str(&std::fs::read_to_string(lock).unwrap()).unwrap();
    assert_eq!(
        oracle::call("oracle.sha", &[]).unwrap(),
        Some(lock["sha"].clone())
    );
}

#[test]
fn an_error_reply_carries_message_and_stack() {
    let err = oracle::call("oracle.throw", &[json!("boom")]).unwrap_err();
    assert_eq!(err.message, "boom");
    let stack = err.stack.as_deref().unwrap();
    assert!(stack.contains("TypeError: boom"), "{stack}");
    assert_eq!(err.to_string(), format!("boom\n{stack}"));
}

#[test]
fn an_f64_marker_in_an_ok_value_is_decoded() {
    let out = oracle::call("oracle.echo", &[json!([-0.0, 1.5])])
        .unwrap()
        .unwrap();
    let zero = out[0].as_f64().unwrap();
    assert_eq!(zero.to_bits(), (-0.0f64).to_bits(), "{out}");
    assert_eq!(out[1], json!(1.5));
}

proptest::proptest! {
    #![proptest_config(config())]
    #[test]
    fn js_stringify_of_strings_round_trips(s in "\\PC*") {
        let out = oracle::call("JSON.stringify", &[json!(s)]).unwrap().unwrap();
        let back: String = serde_json::from_str(out.as_str().unwrap()).unwrap();
        proptest::prop_assert_eq!(back, s);
    }
}
