//! The sensitive key mask: the value of a key that names a secret becomes `[redacted]`, at any
//! depth. A port of the shared reference of the `AgentCat` SDKs; its cases are in
//! `tests/sensitive_keys/vectors.json`.
use serde_json::{Map, Value};

#[cfg(test)]
mod tests;

const MASK: &str = "[redacted]";

// A key matches a name when its canonical form equals the name or ends with "-" + name.
const EXACT: [&str; 26] = [
    "accesskey",
    "cb-access-sign",
    "cf-access-client-id",
    "dpop",
    "fastly-key",
    "jenkins-crumb",
    "paypal-transmission-sig",
    "plaid-verification",
    "sec-websocket-protocol",
    "x-airtable-content-mac",
    "x-amzn-ava-user-context",
    "x-amzn-oidc-data",
    "x-firebase-appcheck",
    "x-forge-oauth-system",
    "x-forge-oauth-user",
    "x-honeycomb-team",
    "x-insert-key",
    "x-jfrog-art-api",
    "x-nf-sign",
    "x-registry-config",
    "x-requestdigest",
    "x-sfdc-session",
    "x-trello-webhook",
    "x-vault-mfa",
    "x-vercel-protection-bypass",
    "x-wp-nonce",
];
// A key matches when its canonical form contains the word.
const CONTAINS: [&str; 36] = [
    "authorization",
    "secret",
    "password",
    "passwd",
    "passphrase",
    "cookie",
    "signature",
    "credential",
    "jwt",
    "assertion",
    "bearer",
    "csrf",
    "xsrf",
    "hmac",
    "saml",
    "api-key",
    "apikey",
    "access-key",
    "private-key",
    "privatekey",
    "subscription-key",
    "encryption-key",
    "signing-key",
    "shared-key",
    "sharedkey",
    "master-key",
    "account-key",
    "app-key",
    "appkey",
    "application-key",
    "functions-key",
    "sas-key",
    "license-key",
    "consumer-key",
    "client-key",
    "customer-key",
];
// A key matches when one hyphen-separated part of its canonical form equals the word.
const SEGMENT: [&str; 3] = ["auth", "token", "otp"];
// A key matches when its canonical form ends with the word.
const SUFFIX: [&str; 1] = ["token"];

// The pair rule: a map that names a key in one entry and holds its value in another.
const PAIR_NAME_KEYS: [&str; 3] = ["name", "key", "header"];
const PAIR_VALUE_KEYS: [&str; 2] = ["value", "values"];
// The children of a map stored under this exact key are JSON Schema property names.
const SCHEMA_PROPERTIES_KEY: &str = "properties";

fn camel_split(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &ch) in chars.iter().enumerate() {
        if i > 0 && ch.is_ascii_uppercase() {
            let prev = chars[i - 1];
            let next = chars.get(i + 1).copied().unwrap_or_default();
            if prev.is_ascii_lowercase()
                || prev.is_ascii_digit()
                || (prev.is_ascii_uppercase() && next.is_ascii_lowercase())
            {
                out.push('-');
            }
        }
        out.push(ch);
    }
    out
}

fn canonical_parts(text: &str) -> Vec<String> {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_uppercase() {
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let mut parts: Vec<String> = out
        .split('-')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    while parts
        .last()
        .is_some_and(|part| part.bytes().all(|b| b.is_ascii_digit()))
    {
        parts.pop();
    }
    parts
}

fn parts_match(parts: &[String]) -> bool {
    if parts.is_empty() {
        return false;
    }
    let joined = parts.join("-");
    for name in EXACT {
        if joined == name || joined.ends_with(&format!("-{name}")) {
            return true;
        }
    }
    for word in CONTAINS {
        if joined.contains(word) {
            return true;
        }
    }
    for part in parts {
        if SEGMENT.contains(&part.as_str()) {
            return true;
        }
    }
    for word in SUFFIX {
        if joined.ends_with(word) {
            return true;
        }
    }
    false
}

fn is_sensitive_key(key: &str) -> bool {
    parts_match(&canonical_parts(key)) || parts_match(&canonical_parts(&camel_split(key)))
}

fn canonical(key: &str) -> String {
    canonical_parts(key).join("-")
}

fn mask_value(value: &Value, under_properties: bool) -> Value {
    match value {
        Value::Null | Value::Bool(_) => value.clone(),
        Value::Object(_) if under_properties => mask_sensitive_keys(value, false),
        _ => Value::String(MASK.to_owned()),
    }
}

/// A masked copy of `value`. `under_properties` is true when `value` is the map stored under the
/// key "properties".
pub(crate) fn mask_sensitive_keys(value: &Value, under_properties: bool) -> Value {
    let map = match value {
        Value::Array(items) => {
            return Value::Array(
                items
                    .iter()
                    .map(|item| mask_sensitive_keys(item, false))
                    .collect(),
            );
        }
        Value::Object(map) => map,
        _ => return value.clone(),
    };
    let mut out = Map::new();
    for (key, child) in map {
        if is_sensitive_key(key) {
            out.insert(key.clone(), mask_value(child, under_properties));
        } else {
            out.insert(
                key.clone(),
                mask_sensitive_keys(child, key == SCHEMA_PROPERTIES_KEY),
            );
        }
    }
    let mut names_a_sensitive_key = false;
    for (key, child) in map {
        if PAIR_NAME_KEYS.contains(&canonical(key).as_str())
            && let Value::String(name) = child
            && is_sensitive_key(name)
        {
            names_a_sensitive_key = true;
        }
    }
    if names_a_sensitive_key {
        for (key, child) in map {
            if PAIR_VALUE_KEYS.contains(&canonical(key).as_str()) {
                out.insert(key.clone(), mask_value(child, false));
            }
        }
    }
    Value::Object(out)
}
