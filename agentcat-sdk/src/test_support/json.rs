//! JSON helpers shared by the unit tests.
use serde_json::{Map, Value};

/// The JSON object `value`.
///
/// # Panics
///
/// When `value` is not an object, with the value in the message.
pub(crate) fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        other => panic!("{other} is not an object"),
    }
}

/// The array index that the object key `key` names: the canonical decimal text of an integer
/// from 0 to 4 294 967 294 (so not `"01"`, `"+1"`, `"-1"` or `"4294967295"`); `None` for any
/// other key.
pub(crate) fn index_key(key: &str) -> Option<u32> {
    key.parse::<u32>()
        .ok()
        .filter(|index| *index < u32::MAX && index.to_string() == key)
}

/// `keys` in the order the event format writes an object's keys: the array indices ascending
/// (see [`index_key`]), then the other keys in their given order.
pub(crate) fn index_keys_first<'a>(keys: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let (mut indices, others): (Vec<&str>, Vec<&str>) =
        keys.into_iter().partition(|key| index_key(key).is_some());
    indices.sort_by_key(|key| index_key(key));
    indices.extend(others);
    indices
}

mod tests {
    use super::{index_key, index_keys_first};

    #[test]
    fn index_keys_are_canonical_integers_below_the_largest_u32() {
        for (key, index) in [
            ("0", Some(0)),
            ("10", Some(10)),
            ("4294967294", Some(4_294_967_294)),
        ] {
            assert_eq!(index_key(key), index, "{key}");
        }
        for key in ["01", "+1", "-1", "1.5", "4294967295", "", "a", " 1"] {
            assert_eq!(index_key(key), None, "{key}");
        }
    }

    #[test]
    fn index_keys_come_first_in_ascending_order() {
        assert_eq!(
            index_keys_first(["b", "10", "a", "2", "01", "4294967294"]),
            ["2", "10", "4294967294", "b", "a", "01"]
        );
    }
}
