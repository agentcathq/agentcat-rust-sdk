//! A long-running Node process that runs the real TS SDK functions, spoken to in JSON lines.
//!
//! Request: `{"id": n, "fn": "<name>", "args": [...]}`. Reply: `{"id": n, "ok": v}`,
//! `{"id": n, "undefined": true}` or `{"id": n, "err": "<message>"}`. The server exits when its
//! stdin closes, which happens when the test process exits.
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};

/// A connection to the oracle server process.
#[derive(Debug)]
pub(crate) struct Oracle {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

/// The oracle of this test process, started on first use.
#[expect(clippy::expect_used, reason = "a test cannot run without the oracle")]
pub(crate) fn oracle() -> &'static Mutex<Oracle> {
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
                    "cannot run `node` (tests that compare with the TS SDK need Node.js 20 or later \
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
        let reply: Value =
            serde_json::from_str(&line).map_err(|e| format!("bad oracle reply {line:?}: {e}"))?;
        if let Some(err) = reply.get("err") {
            return Err(err.as_str().map_or_else(|| err.to_string(), str::to_owned));
        }
        if reply.get("id") != Some(&json!(id)) {
            return Err(format!(
                "oracle reply {line:?} does not answer request {id}"
            ));
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

    /// Text about the exit of the server, if it has exited.
    fn exit_note(&mut self) -> String {
        match self.child.try_wait() {
            Ok(Some(status)) => format!(" (the server exited: {status})"),
            _ => String::new(),
        }
    }
}
