//! The rmcp canary: every method of rmcp's `ServerHandler` trait must be forwarded by
//! `Tracked<H>` (or be allowlisted), or a customer handler's override would be lost.
//!
//! `cargo run -q -p xtask -- rmcp-canary [--rmcp-src <dir>] [--tracked <file>]` reads
//! `src/handler/server.rs` of the resolved rmcp package (found with `cargo metadata`) and
//! `agentcat-sdk/src/tracked.rs`, and prints one line per method that `Tracked<H>` does not
//! forward. It exits 1 if there is any.
use std::path::{Path, PathBuf};
use std::process::Command;

/// Methods `Tracked<H>` deliberately does not forward, with the reason.
pub const ALLOWLIST: &[(&str, &str)] = &[(
    "negotiate_initialize",
    "rmcp added it in 3.3.0, so forwarding it would break the 3.0 floor",
)];

const TRAIT_HEADER: &str = "pub trait ServerHandler";
const IMPL_HEADER: &str = "impl<H: ServerHandler> ServerHandler for Tracked<H>";

/// Parses the arguments and runs the canary; returns the exit code and the lines to print.
#[must_use]
pub fn run(args: &[String], root: &Path) -> (u8, Vec<String>) {
    let mut rmcp_src: Option<PathBuf> = None;
    let mut tracked = root.join("agentcat-sdk/src/tracked.rs");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let slot = match arg.as_str() {
            "--rmcp-src" => &mut rmcp_src,
            "--tracked" => {
                let Some(value) = iter.next() else {
                    return usage();
                };
                tracked = PathBuf::from(value);
                continue;
            }
            _ => return usage(),
        };
        let Some(value) = iter.next() else {
            return usage();
        };
        *slot = Some(PathBuf::from(value));
    }
    let src = match rmcp_src {
        Some(dir) => Ok(dir),
        None => rmcp_source_dir(root),
    };
    match src.and_then(|dir| check(&dir, &tracked)) {
        Ok(problems) if problems.is_empty() => (0, Vec::new()),
        Ok(problems) => (1, problems),
        Err(message) => (1, vec![message]),
    }
}

fn usage() -> (u8, Vec<String>) {
    (
        2,
        vec![
            "usage: cargo run -q -p xtask -- rmcp-canary [--rmcp-src <dir>] [--tracked <file>]"
                .into(),
        ],
    )
}

/// The directory of the resolved `rmcp` package, from `cargo metadata`.
fn rmcp_source_dir(root: &Path) -> Result<PathBuf, String> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("rmcp-canary: cannot run `cargo metadata`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "rmcp-canary: `cargo metadata` failed ({}):\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    manifest_dir_of_rmcp(&output.stdout)
}

/// The directory of the `rmcp` package's `manifest_path` in `cargo metadata` output.
fn manifest_dir_of_rmcp(metadata: &[u8]) -> Result<PathBuf, String> {
    let value: serde_json::Value = serde_json::from_slice(metadata)
        .map_err(|e| format!("rmcp-canary: cannot parse `cargo metadata`: {e}"))?;
    let manifest = value
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|p| p.get("name").and_then(serde_json::Value::as_str) == Some("rmcp"))
        .and_then(|p| p.get("manifest_path"))
        .and_then(serde_json::Value::as_str)
        .ok_or("rmcp-canary: `cargo metadata` lists no `rmcp` package")?;
    Path::new(manifest)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("rmcp-canary: odd manifest path {manifest}"))
}

/// The problem lines for the rmcp package in `rmcp_dir` and the `Tracked<H>` source in `tracked`.
///
/// # Errors
/// A file cannot be read, or a source holds no `ServerHandler` methods.
pub fn check(rmcp_dir: &Path, tracked: &Path) -> Result<Vec<String>, String> {
    let server = rmcp_dir.join("src/handler/server.rs");
    let server_src = read(&server)?;
    let trait_methods = trait_methods(&server_src);
    if trait_methods.is_empty() {
        return Err(format!(
            "rmcp-canary: found no ServerHandler methods in {}",
            server.display()
        ));
    }
    let tracked_src = read(tracked)?;
    let forwarded = impl_methods(&tracked_src);
    if forwarded.is_empty() {
        return Err(format!(
            "rmcp-canary: found no ServerHandler methods in {}",
            tracked.display()
        ));
    }
    Ok(trait_methods
        .iter()
        .filter(|m| !forwarded.contains(m) && !ALLOWLIST.iter().any(|(name, _)| name == m))
        .map(|m| format!("rmcp-canary: ServerHandler::{m} is not forwarded by Tracked<H>"))
        .collect())
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|e| format!("rmcp-canary: cannot read {}: {e}", path.display()))
}

/// The `fn` names of the first `pub trait ServerHandler`, following the macro invocations in its
/// body (rmcp keeps the items in `macro_rules! server_handler_methods`).
#[must_use]
pub fn trait_methods(src: &str) -> Vec<String> {
    let masked = mask(src);
    let Some(body) = block_after(&masked, TRAIT_HEADER) else {
        return Vec::new();
    };
    let mut names = fn_names(body);
    for macro_name in invoked_macros(body) {
        if let Some(def) = block_after(&masked, &format!("macro_rules! {macro_name}"))
            && let Some(arm) = arm_body(def)
        {
            names.extend(fn_names(arm));
        }
    }
    names
}

/// The `fn` names of `impl<H: ServerHandler> ServerHandler for Tracked<H>`.
#[must_use]
pub fn impl_methods(src: &str) -> Vec<String> {
    let masked = mask(src);
    block_after(&masked, IMPL_HEADER)
        .map(fn_names)
        .unwrap_or_default()
}

/// Copy of `src` with comments and the insides of string and char literals blanked out, so that
/// braces in them cannot confuse the brace matching. Byte offsets stay the same.
fn mask(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let mut depth = 0;
                while i < b.len() {
                    if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        depth += 1;
                        blank(&mut out, i, 2);
                        i += 2;
                    } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        blank(&mut out, i, 2);
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        blank(&mut out, i, 1);
                        i += 1;
                    }
                }
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    let step = if b[i] == b'\\' { 2 } else { 1 };
                    blank(&mut out, i, step);
                    i += step;
                }
                i += 1;
            }
            // A char literal such as '{' or '\n'; a lifetime has no closing quote after one char.
            b'\'' => {
                let close = if b.get(i + 1) == Some(&b'\\') {
                    i + 3
                } else {
                    i + 2
                };
                if b.get(close) == Some(&b'\'') {
                    blank(&mut out, i + 1, close - i - 1);
                    i = close + 1;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    // Only ASCII bytes were replaced by spaces, and multi-byte sequences were blanked whole or
    // not at all, so this stays valid UTF-8; the lossy form is a safe fallback.
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn blank(out: &mut [u8], from: usize, len: usize) {
    for byte in out.iter_mut().skip(from).take(len) {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

/// The text between the braces of the first block that follows `header`.
fn block_after<'a>(masked: &'a str, header: &str) -> Option<&'a str> {
    let start = masked.find(header)? + header.len();
    let open = start + masked.get(start..)?.find('{')?;
    let mut depth = 0usize;
    for (offset, ch) in masked.get(open..)?.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return masked.get(open + 1..open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// The body of the first `=> { ... }` arm of a `macro_rules!` body.
fn arm_body(def: &str) -> Option<&str> {
    let arrow = def.find("=>")? + 2;
    let rest = def.get(arrow..)?;
    block_after(rest, "")
}

/// Names `x` of the `x!(...)` invocations at depth 0 of a trait body.
fn invoked_macros(body: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (depth, ident) in tokens_at_depth(body) {
        if depth == 0
            && let Some(name) = ident.strip_suffix('!')
        {
            names.push(name.to_owned());
        }
    }
    names
}

/// The `fn <name>` items at brace depth 0 of `body`.
fn fn_names(body: &str) -> Vec<String> {
    let tokens = tokens_at_depth(body);
    tokens
        .windows(2)
        .filter(|w| w[0].0 == 0 && w[0].1 == "fn" && w[1].0 == 0)
        .map(|w| w[1].1.clone())
        .collect()
}

/// Identifiers (a trailing `!` kept) with the brace depth at which they stand.
fn tokens_at_depth(body: &str) -> Vec<(usize, String)> {
    let mut tokens = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    let flush = |current: &mut String, depth: usize, tokens: &mut Vec<(usize, String)>| {
        if !current.is_empty() {
            tokens.push((depth, std::mem::take(current)));
        }
    };
    for ch in body.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            current.push(ch);
            continue;
        }
        if ch == '!' && !current.is_empty() {
            current.push(ch);
        }
        flush(&mut current, depth, &mut tokens);
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    flush(&mut current, depth, &mut tokens);
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/testdata/canary")
            .join(rel)
    }

    fn lines(rmcp: &str, tracked: &str) -> Result<Vec<String>, String> {
        check(&data(rmcp), &data(tracked))
    }

    #[test]
    fn reports_a_missing_method() {
        let problems = lines("plain", "tracked_ab.rs").expect("fixtures read");
        assert_eq!(
            problems,
            ["rmcp-canary: ServerHandler::c is not forwarded by Tracked<H>"]
        );
    }

    #[test]
    fn passes_when_every_method_is_forwarded() {
        assert_eq!(lines("plain", "tracked_abc.rs"), Ok(Vec::new()));
    }

    #[test]
    fn allowlisted_method_passes() {
        // `negotiate_initialize` is in the trait fixture and not in the impl fixture.
        assert!(
            trait_methods(
                &std::fs::read_to_string(data("macro_shape/src/handler/server.rs"))
                    .expect("fixture")
            )
            .contains(&"negotiate_initialize".to_owned())
        );
        assert_eq!(lines("macro_shape", "tracked_ping.rs"), Ok(Vec::new()));
    }

    #[test]
    fn nested_braces_do_not_end_the_trait() {
        let src = std::fs::read_to_string(data("plain/src/handler/server.rs")).expect("fixture");
        assert_eq!(trait_methods(&src), ["a", "b", "c"]);
    }

    #[test]
    fn macro_shape_collects_the_macro_items_only() {
        let src =
            std::fs::read_to_string(data("macro_shape/src/handler/server.rs")).expect("fixture");
        assert_eq!(
            trait_methods(&src),
            ["ping", "initialize", "negotiate_initialize"]
        );
    }

    #[test]
    fn no_methods_found_is_an_error_naming_the_file() {
        let (code, out) = run(
            &[
                "--rmcp-src".into(),
                data("empty").display().to_string(),
                "--tracked".into(),
                data("tracked_ab.rs").display().to_string(),
            ],
            Path::new("."),
        );
        assert_eq!(code, 1);
        assert_eq!(
            out,
            [format!(
                "rmcp-canary: found no ServerHandler methods in {}",
                data("empty").join("src/handler/server.rs").display()
            )]
        );
    }

    #[test]
    fn run_exits_one_with_one_line_per_missing_method() {
        let (code, out) = run(
            &[
                "--rmcp-src".into(),
                data("plain").display().to_string(),
                "--tracked".into(),
                data("tracked_ab.rs").display().to_string(),
            ],
            Path::new("."),
        );
        assert_eq!((code, out.len()), (1, 1));
    }

    #[test]
    fn finds_the_rmcp_manifest_in_metadata() {
        let json = br#"{"packages":[{"name":"x","manifest_path":"/a/x/Cargo.toml"},{"name":"rmcp","manifest_path":"/r/rmcp-3.0.0/Cargo.toml"}]}"#;
        assert_eq!(
            manifest_dir_of_rmcp(json),
            Ok(PathBuf::from("/r/rmcp-3.0.0"))
        );
    }
}
