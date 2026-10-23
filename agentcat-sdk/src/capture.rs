//! The errors of the current tool call that the result cannot show: a panic of the tool, and an
//! error the tool records with [`capture_error`] (R09, RD-06, RD-17). The call path runs the
//! inner handler inside [`scope`] with a fresh [`CaptureSlot`], so `capture_error` reaches the
//! record of that call only. The `error` they give follows C23: a panic gives
//! [`panic_error`]; otherwise a captured error replaces the error that the result or the
//! JSON-RPC error would give.
use std::any::{Any, type_name};
use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Map, Value};

use crate::call::PLATFORM;

#[cfg(test)]
mod tests;

tokio::task_local! {
    /// The slot of the tool call that the current task runs, while [`scope`] polls it.
    static SLOT: CaptureSlot;
}

/// The `type` of the `error` of a panic (C23).
const PANIC_TYPE: &str = "panic";

/// The message of a panic whose payload is not text: the name Rust's panic hook gives it.
const OPAQUE_PAYLOAD: &str = "Box<dyn Any>";

/// The most `chained_errors` entries an `error` holds (B-097).
const MAX_CHAINED_ERRORS: usize = 10;

/// Records `error` as the error of the tool call that is running, for `AgentCat` to report with
/// the call: its `Display` text, its type path (`std::any::type_name`), the text of each error
/// of its `source()` chain (at most 10), and a backtrace of this call site when the environment
/// turns backtraces on (`RUST_BACKTRACE` or `RUST_LIB_BACKTRACE`, see
/// [`Backtrace::capture`]).
///
/// rmcp turns the error a tool returns into the text of an `is_error` result before `AgentCat`
/// sees it, so the type and the chain are lost. Call this before the tool returns the error:
/// when the call then fails (an `is_error` result or an `Err(ErrorData)`), the recorded error
/// takes the place of the one `AgentCat` reads from the result. When the call succeeds, nothing
/// is reported. When the tool panics, the panic is reported instead. When a tool calls this
/// function more than once, the last error counts. The tool's result and error reach the client
/// unchanged.
///
/// `error` may be a `dyn Error`, for example `&*boxed`. Its recorded type is then the name of the
/// `dyn` type (`dyn core::error::Error`), not that of the value behind it.
///
/// The call is found through a tokio task-local value that `AgentCat` sets while it runs the
/// tool's future. Outside a tool call that `AgentCat` traces, this function does nothing. A task
/// the tool spawns (`tokio::spawn`) or a thread it starts does not belong to the call, so
/// `capture_error` does nothing there either: call it from the tool's own future.
///
/// ```
/// use agentcat_sdk::capture_error;
///
/// fn read_config(path: &str) -> Result<String, String> {
///     std::fs::read_to_string(path).map_err(|error| {
///         capture_error(&error);
///         error.to_string()
///     })
/// }
/// # let _ = read_config;
/// ```
pub fn capture_error<E: Error + ?Sized + 'static>(error: &E) {
    // Outside a call the task-local value is not set: nothing to record.
    let _outside = SLOT.try_with(|slot| slot.put(error_shape(error, &Backtrace::capture())));
}

/// Where [`capture_error`] leaves the error of one tool call. Clones share it: the call path
/// keeps one and hands another to [`scope`].
#[derive(Debug, Clone, Default)]
pub(crate) struct CaptureSlot(Arc<Mutex<Option<Map<String, Value>>>>);

impl CaptureSlot {
    /// The captured error, which leaves the slot empty.
    pub(crate) fn take(&self) -> Option<Map<String, Value>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// Puts `error` in the slot, in place of any earlier one.
    fn put(&self, error: Map<String, Value>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
    }
}

/// Runs `future` with `slot` as the slot of the current tool call, so that [`capture_error`]
/// called while `future` is polled records into `slot`. The value is task-local: a task that
/// `future` spawns does not see it.
pub(crate) async fn scope<F: Future>(slot: CaptureSlot, future: F) -> F::Output {
    SLOT.scope(slot, future).await
}

/// The `error` of a panic with `payload` (C23): `{message, type: "panic", platform: "rust"}`.
/// Pass the payload itself (`&*payload`), not the `Box` that holds it.
pub(crate) fn panic_error(payload: &(dyn Any + Send)) -> Map<String, Value> {
    let mut error = Map::new();
    error.insert("message".to_owned(), Value::String(panic_message(payload)));
    error.insert("type".to_owned(), Value::String(PANIC_TYPE.to_owned()));
    error.insert("platform".to_owned(), Value::String(PLATFORM.to_owned()));
    error
}

/// The message of a panic with `payload`: the text of a `&str` or `String` payload (what
/// `panic!` gives), else `Box<dyn Any>`.
pub(crate) fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        OPAQUE_PAYLOAD.to_owned()
    }
}

/// The `error` that [`capture_error`] records for `error` (R09, RD-06): `{message, type,
/// platform: "rust", stack?, chained_errors?}`. `stack` is the text of `backtrace` when it was
/// captured; `chained_errors` has one `{message}` entry per error of the `source()` chain, at
/// most [`MAX_CHAINED_ERRORS`], and is absent for an error without a source.
pub(crate) fn error_shape<E: Error + ?Sized + 'static>(
    error: &E,
    backtrace: &Backtrace,
) -> Map<String, Value> {
    let mut shape = Map::new();
    shape.insert("message".to_owned(), Value::String(error.to_string()));
    shape.insert(
        "type".to_owned(),
        Value::String(type_name::<E>().to_owned()),
    );
    shape.insert("platform".to_owned(), Value::String(PLATFORM.to_owned()));
    if backtrace.status() == BacktraceStatus::Captured {
        shape.insert("stack".to_owned(), Value::String(backtrace.to_string()));
    }
    let chain: Vec<Value> = std::iter::successors(error.source(), |&cause| cause.source())
        .take(MAX_CHAINED_ERRORS)
        .map(|cause| {
            let mut entry = Map::new();
            entry.insert("message".to_owned(), Value::String(cause.to_string()));
            Value::Object(entry)
        })
        .collect();
    if !chain.is_empty() {
        shape.insert("chained_errors".to_owned(), Value::Array(chain));
    }
    shape
}
