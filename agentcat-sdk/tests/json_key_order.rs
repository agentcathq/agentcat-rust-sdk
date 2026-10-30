//! `mcp_session` must stay the first key, so `serde_json/preserve_order` must be on in the SDK's own dependency.
#[test]
fn serde_json_keeps_insertion_order() {
    let mut map = serde_json::Map::new();
    map.insert("mcp_session".into(), serde_json::json!({}));
    map.insert("echoed".into(), serde_json::json!("x"));
    map.insert("a".into(), serde_json::json!(1));
    let text = serde_json::to_string(&map).unwrap();
    assert_eq!(text, r#"{"mcp_session":{},"echoed":"x","a":1}"#);
}
