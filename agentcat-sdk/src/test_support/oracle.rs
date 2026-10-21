//! A long-running Node process that runs the real TS SDK functions, spoken to in JSON lines.
//!
//! Request: `{"id": n, "fn": "<name>", "args": [...]}`. Reply: `{"id": n, "ok": v}`,
//! `{"id": n, "undefined": true}` or `{"id": n, "err": message, "stack": stack}`, where `v`,
//! `message` and `stack` are in the strict JSON encoding of `oracle/src/strict-json.mjs` (see
//! [`super::markers`]) and `stack` may be `null`. The server exits when its stdin closes, which
//! happens when the test process exits.
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use serde_json::{Map, Value, json};

use super::markers;

/// A connection to the oracle server process.
#[derive(Debug)]
pub(crate) struct Oracle {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

/// A failed oracle call: the TS function threw (`message` is `error.message`, `stack` the
/// error's stack when it has one), or the client got no valid reply (`stack` is `None`). The
/// server sends both in the strict encoding; a `$utf16` marker is read with [`markers::d09`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OracleError {
    pub(crate) message: String,
    pub(crate) stack: Option<String>,
}

impl OracleError {
    /// An error of the client itself, with no stack.
    fn client(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            stack: None,
        }
    }
}

/// The message, then the stack on the next line when there is one.
impl fmt::Display for OracleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(stack) = &self.stack {
            write!(f, "\n{stack}")?;
        }
        Ok(())
    }
}

/// Calls `func` on the oracle of this test process (see [`Oracle::call`]). The lock is released
/// before this returns, so a caller that panics on the result does not poison it; a poisoned
/// lock is still used, because every reply is matched to its request by id.
pub(crate) fn call(func: &str, args: &[Value]) -> Result<Option<Value>, OracleError> {
    lock().call(func, args)
}

/// Holds the oracle of this test process, for a test that needs several calls in a row with no
/// other test in between. A lock poisoned by a test that panicked while it held the oracle is
/// still used (see [`call`]).
pub(crate) fn lock() -> MutexGuard<'static, Oracle> {
    oracle().lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether a test panicked while it held the oracle. [`call`] and [`lock`] work either way.
pub(crate) fn is_poisoned() -> bool {
    oracle().is_poisoned()
}

/// The oracle of this test process, started on first use. Private: tests reach it through
/// [`call`] and [`lock`], which recover a poisoned lock (a plain `lock().unwrap()` fails every
/// later test once one test panics while it holds the oracle).
fn oracle() -> &'static Mutex<Oracle> {
    static ORACLE: OnceLock<Mutex<Oracle>> = OnceLock::new();
    ORACLE.get_or_init(|| {
        Mutex::new(Oracle::spawn().unwrap_or_else(|e| panic!("cannot start the JS oracle: {e}")))
    })
}

impl Oracle {
    /// Starts the server and checks that it runs the TS commit `oracle/oracle.lock` pins: an
    /// `ORACLE_SHA` override would compare Rust with another TS commit than the vector files.
    fn spawn() -> Result<Self, OracleError> {
        let server = Path::new(env!("CARGO_MANIFEST_DIR")).join("../oracle/src/server.mjs");
        let mut child = Command::new("node")
            .arg(&server)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                OracleError::client(format!(
                    "cannot run `node` (tests that compare with the TS SDK need Node.js 22 or later \
                     on PATH, and `corepack pnpm install` run once): {e}"
                ))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| OracleError::client("oracle stdin is not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OracleError::client("oracle stdout is not piped"))?;
        let mut oracle = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };
        // On an error the oracle drops here, which closes its stdin, and the server exits.
        check_sha(oracle.call("oracle.sha", &[]), &pinned_sha()?)?;
        Ok(oracle)
    }

    /// Calls the TS function `func` with `args`. `Ok(None)` means the function returned JS
    /// `undefined`; an `Ok` value has its `$f64` markers decoded and keeps its `$utf16` markers.
    /// `Err` means the function threw, or the server died or sent a bad reply.
    pub(crate) fn call(
        &mut self,
        func: &str,
        args: &[Value],
    ) -> Result<Option<Value>, OracleError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"id": id, "fn": func, "args": args});
        writeln!(self.stdin, "{request}")
            .and_then(|()| self.stdin.flush())
            .map_err(|e| {
                OracleError::client(format!(
                    "oracle is gone (cannot send {func}): {e}{}",
                    self.exit_note()
                ))
            })?;
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).map_err(|e| {
            OracleError::client(format!("cannot read the oracle reply to {func}: {e}"))
        })?;
        if read == 0 {
            return Err(OracleError::client(format!(
                "oracle closed its output before replying to {func}{}",
                self.exit_note()
            )));
        }
        match_reply(&line, id, func)
    }

    /// Text about the exit of the server, if it has exited.
    fn exit_note(&mut self) -> String {
        match self.child.try_wait() {
            Ok(Some(status)) => format!(" (the server exited: {status})"),
            _ => String::new(),
        }
    }
}

/// The `sha` of `oracle/oracle.lock`.
fn pinned_sha() -> Result<String, OracleError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../oracle/oracle.lock");
    let fail = |why: String| OracleError::client(format!("{}: {why}", path.display()));
    let text = std::fs::read_to_string(&path).map_err(|e| fail(format!("cannot read: {e}")))?;
    let lock: Value = serde_json::from_str(&text).map_err(|e| fail(e.to_string()))?;
    lock.get("sha")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| fail("no \"sha\" string".to_owned()))
}

/// `Ok` when `reply`, the result of `oracle.sha`, is `pinned`.
fn check_sha(reply: Result<Option<Value>, OracleError>, pinned: &str) -> Result<(), OracleError> {
    let runs = match reply? {
        Some(Value::String(sha)) => sha,
        Some(other) => other.to_string(),
        None => "undefined".to_owned(),
    };
    if runs == pinned {
        return Ok(());
    }
    Err(OracleError::client(format!(
        "the oracle runs TS {runs} but oracle/oracle.lock pins {pinned}; unset ORACLE_SHA"
    )))
}

/// Turns one reply line into the result of request `id`. The id is checked first, for `err`
/// replies too: a reply for another request (or with a null id) means the stream is out of step.
fn match_reply(line: &str, id: u64, func: &str) -> Result<Option<Value>, OracleError> {
    let bad =
        |why: &str| OracleError::client(format!("bad oracle reply {line:?} to {func}: {why}"));
    let mut reply: Map<String, Value> =
        serde_json::from_str(line).map_err(|e| bad(&e.to_string()))?;
    if reply.get("id") != Some(&json!(id)) {
        return Err(OracleError::client(format!(
            "protocol error: oracle reply {line:?} does not answer request {id} ({func})"
        )));
    }
    if let Some(err) = reply.remove("err") {
        let stack = reply.remove("stack").filter(|stack| !stack.is_null());
        return Err(OracleError {
            message: text(err).map_err(|e| bad(&e))?,
            stack: stack.map(text).transpose().map_err(|e| bad(&e))?,
        });
    }
    if reply.get("undefined") == Some(&Value::Bool(true)) {
        return Ok(None);
    }
    let ok = reply
        .remove("ok")
        .ok_or_else(|| bad("no ok, undefined or err"))?;
    markers::try_decode_f64(ok).map(Some).map_err(|e| bad(&e))
}

/// The text of an `err` or `stack` field: a string, or a `$utf16` marker without its unpaired
/// surrogates (RD-22).
fn text(value: Value) -> Result<String, String> {
    match markers::try_d09(value)? {
        Value::String(text) => Ok(text),
        other => Err(format!("not a string: {other}")),
    }
}

mod tests {
    use serde_json::json;

    use super::{OracleError, check_sha, match_reply};

    fn err(message: &str, stack: Option<&str>) -> OracleError {
        OracleError {
            message: message.to_owned(),
            stack: stack.map(str::to_owned),
        }
    }

    #[test]
    fn an_err_reply_with_the_right_id_is_the_error() {
        assert_eq!(
            match_reply(r#"{"id":3,"err":"boom","stack":null}"#, 3, "f"),
            Err(err("boom", None))
        );
        assert_eq!(
            match_reply(
                r#"{"id":3,"err":"boom","stack":"TypeError: boom\n    at f"}"#,
                3,
                "f"
            ),
            Err(err("boom", Some("TypeError: boom\n    at f")))
        );
    }

    #[test]
    fn an_err_reply_with_a_wrong_or_null_id_is_a_protocol_error() {
        for line in [
            r#"{"id":2,"err":"boom","stack":null}"#,
            r#"{"id":null,"err":"boom","stack":null}"#,
            r#"{"err":"boom"}"#,
        ] {
            let e = match_reply(line, 3, "f").unwrap_err();
            assert!(e.message.starts_with("protocol error"), "{line}: {e}");
            assert_eq!(e.stack, None);
        }
    }

    #[test]
    fn ok_and_undefined_replies_with_a_wrong_id_are_protocol_errors() {
        assert!(match_reply(r#"{"id":4,"ok":1}"#, 3, "f").is_err());
        assert!(match_reply(r#"{"id":4,"undefined":true}"#, 3, "f").is_err());
        assert_eq!(
            match_reply(r#"{"id":3,"undefined":true}"#, 3, "f"),
            Ok(None)
        );
    }

    #[test]
    fn utf16_markers_in_err_and_stack_lose_their_unpaired_surrogates() {
        let line =
            r#"{"id":3,"err":{"$utf16":[97,55357]},"stack":{"$utf16":[84,58,32,97,55357,10,120]}}"#;
        assert_eq!(match_reply(line, 3, "f"), Err(err("a", Some("T: a\nx"))));
    }

    #[test]
    fn an_ok_value_has_its_f64_markers_decoded_and_its_utf16_markers_kept() {
        let line = r#"{"id":3,"ok":[{"$f64":"8000000000000000"},{"$utf16":[97,55357]},{"k":{"$f64":"3fd3333333333334"}}]}"#;
        let out = match_reply(line, 3, "f").unwrap().unwrap();
        assert_eq!(out[0].as_f64().unwrap().to_bits(), (-0.0f64).to_bits());
        assert_eq!(out[1], json!({"$utf16": [97, 55357]}));
        assert_eq!(out[2]["k"].as_f64(), Some(0.1 + 0.2));
    }

    #[test]
    fn a_malformed_marker_in_an_ok_value_is_an_error() {
        let e = match_reply(r#"{"id":3,"ok":{"$f64":"zz"}}"#, 3, "f").unwrap_err();
        assert!(e.message.contains("$f64"), "{e}");
    }

    #[test]
    fn display_is_the_message_then_the_stack() {
        assert_eq!(err("boom", None).to_string(), "boom");
        assert_eq!(
            err("boom", Some("TypeError: boom\n    at f")).to_string(),
            "boom\nTypeError: boom\n    at f"
        );
    }

    #[test]
    fn the_sha_check_passes_the_pinned_sha() {
        assert_eq!(check_sha(Ok(Some(json!("abc"))), "abc"), Ok(()));
    }

    #[test]
    fn the_sha_check_names_both_shas_on_a_mismatch() {
        assert_eq!(
            check_sha(Ok(Some(json!("abc"))), "def"),
            Err(err(
                "the oracle runs TS abc but oracle/oracle.lock pins def; unset ORACLE_SHA",
                None
            ))
        );
    }

    #[test]
    fn the_sha_check_returns_the_error_of_a_failed_call() {
        assert_eq!(
            check_sha(Err(err("gone", None)), "def"),
            Err(err("gone", None))
        );
    }
}
