//! A long-running Node process that runs the real TS SDK functions, spoken to in JSON lines.
//!
//! Request: `{"id": n, "fn": "<name>", "args": [...]}`. Reply: `{"id": n, "ok": v}`,
//! `{"id": n, "undefined": true}` or `{"id": n, "err": "<message>"}`. The server exits when its
//! stdin closes, which happens when the test process exits.
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use serde_json::{Value, json};

/// A connection to the oracle server process.
#[derive(Debug)]
pub(crate) struct Oracle {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

/// Calls `func` on the oracle of this test process. The lock is released before this returns,
/// so a caller that panics on the result does not poison it; a poisoned lock is still used,
/// because every reply is matched to its request by id.
pub(crate) fn call(func: &str, args: &[Value]) -> Result<Option<Value>, String> {
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
#[expect(clippy::expect_used, reason = "a test cannot run without the oracle")]
fn oracle() -> &'static Mutex<Oracle> {
    static ORACLE: OnceLock<Mutex<Oracle>> = OnceLock::new();
    ORACLE.get_or_init(|| Mutex::new(Oracle::spawn().expect("cannot start the JS oracle")))
}

impl Oracle {
    fn spawn() -> Result<Self, String> {
        let server = Path::new(env!("CARGO_MANIFEST_DIR")).join("../oracle/src/server.mjs");
        let mut child = Command::new("node")
            .arg(&server)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                format!(
                    "cannot run `node` (tests that compare with the TS SDK need Node.js 22 or later \
                     on PATH, and `corepack pnpm install` run once): {e}"
                )
            })?;
        let stdin = child.stdin.take().ok_or("oracle stdin is not piped")?;
        let stdout = child.stdout.take().ok_or("oracle stdout is not piped")?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    /// Calls the TS function `func` with `args`. `Ok(None)` means the function returned JS
    /// `undefined`. `Err` means the function threw, or the server died.
    pub(crate) fn call(&mut self, func: &str, args: &[Value]) -> Result<Option<Value>, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"id": id, "fn": func, "args": args});
        writeln!(self.stdin, "{request}")
            .and_then(|()| self.stdin.flush())
            .map_err(|e| {
                format!(
                    "oracle is gone (cannot send {func}): {e}{}",
                    self.exit_note()
                )
            })?;
        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .map_err(|e| format!("cannot read the oracle reply to {func}: {e}"))?;
        if read == 0 {
            return Err(format!(
                "oracle closed its output before replying to {func}{}",
                self.exit_note()
            ));
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

/// Turns one reply line into the result of request `id`. The id is checked first, for `err`
/// replies too: a reply for another request (or with a null id) means the stream is out of step.
fn match_reply(line: &str, id: u64, func: &str) -> Result<Option<Value>, String> {
    let reply: Value =
        serde_json::from_str(line).map_err(|e| format!("bad oracle reply {line:?}: {e}"))?;
    if reply.get("id") != Some(&json!(id)) {
        return Err(format!(
            "protocol error: oracle reply {line:?} does not answer request {id} ({func})"
        ));
    }
    if let Some(err) = reply.get("err") {
        return Err(err.as_str().map_or_else(|| err.to_string(), str::to_owned));
    }
    if reply.get("undefined") == Some(&Value::Bool(true)) {
        return Ok(None);
    }
    reply
        .get("ok")
        .cloned()
        .map(Some)
        .ok_or_else(|| format!("oracle reply {line:?} has no ok, undefined or err"))
}

#[cfg(test)]
mod tests {
    use super::match_reply;

    #[test]
    fn an_err_reply_with_the_right_id_is_the_error() {
        assert_eq!(
            match_reply(r#"{"id":3,"err":"boom"}"#, 3, "f"),
            Err("boom".into())
        );
    }

    #[test]
    fn an_err_reply_with_a_wrong_or_null_id_is_a_protocol_error() {
        for line in [
            r#"{"id":2,"err":"boom"}"#,
            r#"{"id":null,"err":"boom"}"#,
            r#"{"err":"boom"}"#,
        ] {
            let err = match_reply(line, 3, "f").unwrap_err();
            assert!(err.starts_with("protocol error"), "{line}: {err}");
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
}
