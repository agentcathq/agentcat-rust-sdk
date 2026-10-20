//! Parity: `src/tests/tokenEstimate.test.ts`.

use crate::tokens::{
    TOKEN_ESTIMATE_BYTES_PER_TOKEN, estimate_input_tokens, estimate_output_tokens, estimate_tokens,
};
use serde_json::{Value, json};

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn text(t: &str) -> Value {
    json!({"type": "text", "text": t})
}

mod estimatetokens {
    use super::*;

    #[test]
    fn pins_the_divisor() {
        assert_eq!(TOKEN_ESTIMATE_BYTES_PER_TOKEN, 3.5);
    }

    #[test]
    fn bytes_0_0_tokens() {
        assert_eq!(estimate_tokens(0.0), 0);
    }

    #[test]
    fn bytes_1_1_tokens() {
        assert_eq!(estimate_tokens(1.0), 1);
    }

    #[test]
    fn bytes_7_2_tokens() {
        assert_eq!(estimate_tokens(7.0), 2);
    }

    #[test]
    fn bytes_13_4_tokens() {
        assert_eq!(estimate_tokens(13.0), 4);
    }

    #[test]
    fn bytes_4096_1171_tokens() {
        assert_eq!(estimate_tokens(4096.0), 1171);
    }

    #[test]
    fn bytes_7516192765_2147483647_tokens() {
        assert_eq!(estimate_tokens(7516192765.0), 2147483647);
    }
}

mod utf8bytelength {
    #[test]
    fn t_0_bytes() {
        assert_eq!("".len(), 0);
    }

    #[test]
    fn hello_5_bytes() {
        assert_eq!("hello".len(), 5);
    }

    #[test]
    fn caf_5_bytes() {
        assert_eq!("café".len(), 5);
    }

    #[test]
    fn t_6_bytes() {
        assert_eq!("你好".len(), 6);
    }

    #[test]
    fn t_4_bytes() {
        assert_eq!("😀".len(), 4);
    }
}

mod estimateinputtokens_raw_arguments {
    use super::*;

    #[test]
    fn q_hello_4() {
        let args = parse(r#"{"q":"hello"}"#);
        assert_eq!(estimate_input_tokens(Some(&args)), Some(4));
    }

    #[test]
    fn t_1() {
        let args = parse(r#"{}"#);
        assert_eq!(estimate_input_tokens(Some(&args)), Some(1));
    }

    #[test]
    fn name_caf_5() {
        let args = parse(r#"{"name":"café"}"#);
        assert_eq!(estimate_input_tokens(Some(&args)), Some(5));
    }

    #[test]
    fn html_a_a_6() {
        let args = parse(r#"{"html":"<a>&</a>"}"#);
        assert_eq!(estimate_input_tokens(Some(&args)), Some(6));
    }

    #[test]
    fn t_4() {
        let args = parse(r#"{"t":"你好"}"#);
        assert_eq!(estimate_input_tokens(Some(&args)), Some(4));
    }

    #[test]
    fn ids_1_2_3_opts_deep_true_n_null_13() {
        let args = parse(r#"{"ids":[1,2,3],"opts":{"deep":true,"n":null}}"#);
        assert_eq!(estimate_input_tokens(Some(&args)), Some(13));
    }

    #[test]
    fn omits_the_field_for_absent_arguments() {
        assert_eq!(estimate_input_tokens(None), None);
        assert_eq!(estimate_input_tokens(Some(&Value::Null)), None);
    }
}

mod estimateoutputtokens_content_blocks {
    use super::*;

    #[test]
    fn content_type_text_text_hello_world_4() {
        let response = json!({"content": [text("hello world")]});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(4));
    }

    #[test]
    fn content_type_text_text_abcd_type_text_text_e_2() {
        let response = json!({"content": [text("abcd"), text("e")]});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(2));
    }

    #[test]
    fn content_type_text_text_0() {
        let response = json!({"content": [text("")]});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(0));
    }

    #[test]
    fn content_type_image_data_qujd_mimetype_image_png_0() {
        let response = json!({"content": [{"type": "image", "data": "QUJD", "mimeType": "image/png"}]});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(0));
    }

    #[test]
    fn content_type_resource_resource_uri_file_a_text_resource_body_4() {
        let response = json!({"content": [{"type": "resource", "resource": {"uri": "file:///a", "text": "resource body"}}]});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(4));
    }

    #[test]
    fn content_type_resource_resource_uri_file_a_blob_qujd_0() {
        let response =
            json!({"content": [{"type": "resource", "resource": {"uri": "file:///a", "blob": "QUJD"}}]});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(0));
    }

    #[test]
    fn content_type_text_text_hi_structuredcontent_big_yyyyyyyyyyyyyyyyyyyyyy_50c21595() {
        let response = json!({"content": [text("hi")], "structuredContent": {"big": "y".repeat(1000)}});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(1));
    }

    #[test]
    fn content_type_text_text_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx_b2530d32() {
        let response = json!({"content": [text(&"x".repeat(4096))]});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(1171));
    }

    #[test]
    fn content_type_text_text_hi_iserror_true_1() {
        let response = json!({"content": [text("hi")], "isError": true});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(1));
    }

    #[test]
    fn content_structuredcontent_result_ok_5() {
        let response = json!({"content": [], "structuredContent": {"result": "ok"}});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(5));
    }

    #[test]
    fn content_0() {
        let response = json!({"content": []});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(0));
    }

    #[test]
    fn content_structuredcontent_null_0() {
        let response = json!({"content": [], "structuredContent": null});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(0));
    }

    #[test]
    fn content_type_image_data_qujd_mimetype_image_png_structuredcontent_result_ok_0() {
        let response = json!({"content": [{"type": "image", "data": "QUJD", "mimeType": "image/png"}], "structuredContent": {"result": "ok"}});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(0));
    }

    #[test]
    fn falls_back_to_the_whole_response_when_there_is_no_content_list() {
        let response = json!({"result": "ok"});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(5));
    }

    #[test]
    fn omits_the_field_for_an_absent_response() {
        assert_eq!(estimate_output_tokens(None), None);
        assert_eq!(estimate_output_tokens(Some(&Value::Null)), None);
    }

    #[test]
    fn ignores_blocks_whose_text_is_not_a_string() {
        let response = json!({"content": [{"type": "text", "text": 42}, null, "str"]});
        assert_eq!(estimate_output_tokens(Some(&response)), Some(0));
    }
}

mod rust_extras {
    use super::*;

    #[test]
    fn escapes_count_like_json_stringify() {
        // 4-byte code points are not escaped: {"e":"😀"} is 12 bytes.
        assert_eq!(estimate_input_tokens(Some(&json!({"e": "😀"}))), Some(4));
        // A control character is escaped as \u0001: 14 bytes.
        assert_eq!(estimate_input_tokens(Some(&json!({"c": "\u{1}"}))), Some(4));
        // A quote is escaped as \": 10 bytes.
        assert_eq!(estimate_input_tokens(Some(&json!({"q": "\""}))), Some(3));
    }

    fn nested(depth: usize) -> Value {
        let mut v = Value::Null;
        for _ in 0..depth {
            v = Value::Array(vec![v]);
        }
        v
    }

    #[test]
    fn deeply_nested_values_omit_the_estimate() {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(|| {
                let deep = nested(5_000);
                assert_eq!(estimate_input_tokens(Some(&deep)), None);
                let response = json!({"content": [], "structuredContent": deep});
                assert_eq!(estimate_output_tokens(Some(&response)), None);
                let shallow = nested(999);
                assert!(estimate_input_tokens(Some(&shallow)).is_some());
                let response = json!({"content": [], "structuredContent": shallow});
                assert!(estimate_output_tokens(Some(&response)).is_some());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn object_nesting_counts_toward_the_depth_limit() {
        let nested_objects = |depth: usize| {
            let mut v = Value::Null;
            for _ in 0..depth {
                v = json!({"a": v});
            }
            v
        };
        assert!(estimate_input_tokens(Some(&nested_objects(1_000))).is_some());
        assert_eq!(estimate_input_tokens(Some(&nested_objects(1_001))), None);
    }
}
