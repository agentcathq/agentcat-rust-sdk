//! `CustomEvent`: each setter stores its own field and no other (B-009).
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::CustomEvent;

fn object(key: &str, value: Value) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(key.to_owned(), value);
    map
}

#[test]
fn custom_event_setters_store_fields() {
    let empty = CustomEvent::new();
    assert!(empty.session_id.is_none() && empty.resource_name.is_none());
    assert!(empty.parameters.is_none() && empty.response.is_none());
    assert!(empty.message.is_none() && empty.duration.is_none());
    assert!(empty.is_error.is_none() && empty.error.is_none());
    assert!(empty.tags.is_none() && empty.properties.is_none());

    let event = CustomEvent::new()
        .session_id("ses_1")
        .resource_name(String::from("cache"))
        .parameters(json!({"key": 1}))
        .response(json!([true]))
        .message("warmed")
        .duration(Duration::from_millis(40))
        .is_error(true)
        .error(json!({"message": "boom"}))
        .tags(object("team", json!("a")))
        .properties(object("n", json!(2)));
    assert_eq!(event.session_id.as_deref(), Some("ses_1"));
    assert_eq!(event.resource_name.as_deref(), Some("cache"));
    assert_eq!(event.parameters, Some(json!({"key": 1})));
    assert_eq!(event.response, Some(json!([true])));
    assert_eq!(event.message.as_deref(), Some("warmed"));
    assert_eq!(event.duration, Some(Duration::from_millis(40)));
    assert_eq!(event.is_error, Some(true));
    assert_eq!(event.error, Some(json!({"message": "boom"})));
    assert_eq!(event.tags, Some(object("team", json!("a"))));
    assert_eq!(event.properties, Some(object("n", json!(2))));
    assert_eq!(format!("{:?}", event.clone()), format!("{event:?}"));
}
