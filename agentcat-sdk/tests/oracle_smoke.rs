//! Smoke test of the JS oracle: Rust talks to the real TS functions through a Node process.
mod support;

#[test]
fn js_drops_the_fraction_of_whole_floats() {
    let out = support::oracle::oracle()
        .lock()
        .unwrap()
        .call("JSON.stringify", &[serde_json::json!({"n": 1.0})])
        .unwrap();
    assert_eq!(out, Some(serde_json::json!("{\"n\":1}")));
}

#[test]
fn js_undefined_is_none() {
    let out = support::oracle::oracle()
        .lock()
        .unwrap()
        .call(
            "tokenEstimate.estimateInputTokens",
            &[serde_json::Value::Null],
        )
        .unwrap();
    assert_eq!(out, None);
}

#[test]
fn an_unknown_function_is_an_err_and_the_oracle_keeps_serving() {
    let mut oracle = support::oracle::oracle().lock().unwrap();
    assert!(oracle.call("nope", &[]).is_err());
    assert_eq!(
        oracle
            .call("JSON.stringify", &[serde_json::json!(1)])
            .unwrap(),
        Some(serde_json::json!("1"))
    );
}

proptest::proptest! {
    #![proptest_config(support::proptest_config())]
    #[test]
    fn js_stringify_of_strings_round_trips(s in "\\PC*") {
        let out = support::oracle::oracle().lock().unwrap()
            .call("JSON.stringify", &[serde_json::json!(s)]).unwrap().unwrap();
        let back: String = serde_json::from_str(out.as_str().unwrap()).unwrap();
        proptest::prop_assert_eq!(back, s);
    }
}
