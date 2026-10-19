//! Tripwire: `Tracked<H>` must forward every `ServerHandler` method of the
//! resolved rmcp version. A method rmcp adds in a minor release would
//! otherwise fall back to rmcp's default for tracked servers, silently.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Methods deliberately not forwarded, with the reason.
const NOT_FORWARDED: &[(&str, &str)] = &[
    // Added in rmcp 3.3. Only rmcp's default `initialize` calls it, and the
    // wrapper forwards `initialize` to the inner handler, whose own
    // `negotiate_initialize` then runs. Forwarding it would break 3.0-3.2.
    (
        "negotiate_initialize",
        "only called by the inner handler's initialize",
    ),
];

fn fn_names(body: &str) -> BTreeSet<String> {
    body.lines()
        .filter_map(|line| {
            let t = line.trim_start();
            let t = t.strip_prefix("async ").unwrap_or(t);
            let rest = t.strip_prefix("fn ")?;
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

fn rmcp_server_rs() -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata runs");
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).expect("metadata JSON");
    let manifest = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "rmcp")
        .expect("rmcp is a dependency")["manifest_path"]
        .as_str()
        .unwrap()
        .to_owned();
    PathBuf::from(manifest)
        .parent()
        .unwrap()
        .join("src/handler/server.rs")
}

#[test]
fn tracked_forwards_every_server_handler_method() {
    let rmcp_src = std::fs::read_to_string(rmcp_server_rs()).expect("rmcp source");
    let start = rmcp_src
        .find("macro_rules! server_handler_methods")
        .expect("trait macro");
    let end = rmcp_src[start..]
        .find("\n}\n")
        .map(|i| start + i)
        .expect("macro end");
    let rmcp_methods = fn_names(&rmcp_src[start..end]);

    let ours = include_str!("../src/tracked.rs");
    let impl_start = ours
        .find("impl<H: ServerHandler> ServerHandler for Tracked<H>")
        .expect("impl");
    let our_methods = fn_names(&ours[impl_start..]);

    let skipped: BTreeSet<String> = NOT_FORWARDED.iter().map(|(n, _)| (*n).to_owned()).collect();
    let missing: Vec<&String> = rmcp_methods
        .iter()
        .filter(|m| !our_methods.contains(*m) && !skipped.contains(*m))
        .collect();
    assert!(
        missing.is_empty(),
        "rmcp's ServerHandler has methods Tracked<H> does not forward: {missing:?}"
    );
    assert!(rmcp_methods.len() >= 28, "parsed {rmcp_methods:?}");
}
