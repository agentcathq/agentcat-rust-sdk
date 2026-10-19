//! JavaScript semantics the TypeScript SDK relies on: `String.prototype.trim`,
//! truthiness, and `Number.prototype.toString`.

use serde_json::Value;

/// JavaScript's `String.prototype.trim` whitespace set: Rust's `char::is_whitespace`
/// without U+0085 (NEL), plus U+FEFF (BOM).
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}')
}

/// JavaScript truthiness of a JSON value.
pub(crate) fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// JavaScript's `typeof` for a JSON value.
pub(crate) fn type_of(v: &Value) -> &'static str {
    match v {
        Value::Null | Value::Array(_) | Value::Object(_) => "object",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
    }
}

/// JavaScript's `Number.prototype.toString` (ECMA-262 `Number::toString`) for a
/// finite double.
pub(crate) fn number_to_string(f: f64) -> String {
    if f == 0.0 {
        return "0".into();
    }
    if !f.is_finite() {
        return "null".into(); // JSON.stringify writes non-finite numbers as null
    }
    // Rust's `{:e}` gives the shortest round-trip digits, as JS requires.
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exp: i32 = exp.parse().unwrap_or(0);
    let k = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    let n = exp + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat(usize::try_from(n - k).unwrap_or(0)))
    } else if 0 < n && n <= 21 {
        let n = usize::try_from(n).unwrap_or(0);
        format!("{}.{}", &digits[..n], &digits[n..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat(usize::try_from(-n).unwrap_or(0)))
    } else {
        let sign = if n > 0 { "+" } else { "-" };
        let e = (n - 1).abs();
        if k == 1 {
            format!("{digits}e{sign}{e}")
        } else {
            format!("{}.{}e{sign}{e}", &digits[..1], &digits[1..])
        }
    };
    if f < 0.0 { format!("-{body}") } else { body }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn trim_uses_the_ecmascript_whitespace_set() {
        assert_eq!(trim("\u{feff} x \u{a0}\n"), "x");
        assert_eq!(trim("\u{2028}x\u{3000}"), "x");
        assert_eq!(trim("\u{85}x"), "\u{85}x");
        assert_eq!(trim("\u{180e}x"), "\u{180e}x");
    }

    #[test]
    fn type_of_matches_js() {
        for (v, ty) in [
            (json!(null), "object"),
            (json!([1]), "object"),
            (json!({}), "object"),
            (json!(true), "boolean"),
            (json!(1.5), "number"),
            (json!("a"), "string"),
        ] {
            assert_eq!(type_of(&v), ty, "{v}");
        }
    }

    #[test]
    fn truthy_follows_javascript() {
        for v in [json!(null), json!(false), json!(0), json!(-0.0), json!("")] {
            assert!(!truthy(Some(&v)), "{v}");
        }
        assert!(!truthy(None));
        for v in [json!("0"), json!([]), json!({}), json!(1), json!(true)] {
            assert!(truthy(Some(&v)), "{v}");
        }
    }

    #[test]
    fn numbers_format_like_javascript() {
        for (f, js) in [
            (10.0, "10"),
            (1.5, "1.5"),
            (-0.25, "-0.25"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1.5e21, "1.5e+21"),
            (0.000001, "0.000001"),
            (1e-7, "1e-7"),
            (1.2345e-7, "1.2345e-7"),
            (123.456, "123.456"),
            (0.1, "0.1"),
            (0.1 + 0.2, "0.30000000000000004"),
            (-0.0, "0"),
            (5e-324, "5e-324"),
        ] {
            assert_eq!(number_to_string(f), js, "{f}");
        }
    }
}
