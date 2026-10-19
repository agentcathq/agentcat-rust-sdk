//! Error capture for tool calls: JSON-RPC errors, `is_error` results, and
//! panics (with a parsed backtrace).
//!
//! Frames follow the rules the AgentCat API groups by for non-JavaScript
//! platforms: every frame has a string `filename` and `function`, a `module`
//! when the symbol has one, and `in_app: false` for the standard library,
//! Cargo registry crates and the async runtime.

use std::cell::{Cell, RefCell};
use std::sync::Once;

use serde_json::Value;

use crate::event::{ErrorData, StackFrame};

const MAX_STACK_FRAMES: usize = 50;

/// Error data for a JSON-RPC error a handler returned. The message follows
/// the MCP TypeScript SDK's `McpError` (`MCP error <code>: <message>`).
pub(crate) fn capture_protocol_error(code: i32, message: &str) -> ErrorData {
    ErrorData::new(format!("MCP error {code}: {message}")).with_type("McpError")
}

/// Error data for a tool result with `isError: true`: its text blocks joined
/// with a space, or `Unknown error`. No type and no frames: the original
/// error is gone by the time the result exists.
pub(crate) fn capture_tool_result_error(result: &Value) -> ErrorData {
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .map(|b| match b.get("text") {
                    Some(Value::String(s)) => s.clone(),
                    None => "undefined".to_owned(),
                    Some(Value::Null) => String::new(),
                    Some(other) => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let trimmed = crate::js::trim(&text);
    ErrorData::new(if trimmed.is_empty() {
        "Unknown error"
    } else {
        trimmed
    })
}

// ── Panics ──────────────────────────────────────────────────────────────────

/// What the panic hook saw for a panic inside a tracked tool call.
pub(crate) struct PanicCapture {
    pub(crate) location: Option<(String, u32, u32)>,
    pub(crate) backtrace: Option<String>,
}

thread_local! {
    static IN_TRACKED_CALL: Cell<bool> = const { Cell::new(false) };
    static LAST_PANIC: RefCell<Option<PanicCapture>> = const { RefCell::new(None) };
}

static INSTALL_HOOK: Once = Once::new();

/// Installs a panic hook that records the location and backtrace of panics
/// raised inside tracked tool calls, then runs the previous hook unchanged.
/// Panics elsewhere pass straight through.
pub(crate) fn install_panic_hook() {
    INSTALL_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if IN_TRACKED_CALL.with(Cell::get) {
                let location = info
                    .location()
                    .map(|l| (l.file().to_owned(), l.line(), l.column()));
                let backtrace = std::backtrace::Backtrace::force_capture().to_string();
                LAST_PANIC.with(|slot| {
                    *slot.borrow_mut() = Some(PanicCapture {
                        location,
                        backtrace: Some(backtrace),
                    });
                });
            }
            previous(info);
        }));
    });
}

/// Marks the current thread as running a tracked tool call while `f` runs.
pub(crate) fn in_tracked_call<T>(f: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            IN_TRACKED_CALL.with(|c| c.set(self.0));
        }
    }
    let _reset = Reset(IN_TRACKED_CALL.with(|c| c.replace(true)));
    // A capture left over from a panic the handler caught itself must not be
    // attached to a later one.
    LAST_PANIC.with(|slot| slot.borrow_mut().take());
    f()
}

/// Takes what the panic hook recorded on this thread.
pub(crate) fn take_panic_capture() -> Option<PanicCapture> {
    LAST_PANIC.with(|slot| slot.borrow_mut().take())
}

/// Error data for a panic payload, with the hook's capture when available.
pub(crate) fn capture_panic(
    payload: &(dyn std::any::Any + Send),
    capture: Option<PanicCapture>,
) -> ErrorData {
    let mut data = ErrorData::new(crate::util::panic_message(payload)).with_type("panic");
    if let Some(capture) = capture {
        let mut frames = capture
            .backtrace
            .as_deref()
            .map(parse_backtrace)
            .unwrap_or_default();
        if frames.is_empty()
            && let Some((file, line, col)) = &capture.location
        {
            frames.push(frame_for(None, Some(file), Some(*line), Some(*col)));
        }
        if !frames.is_empty() {
            data.frames = Some(frames);
        }
        data.stack = capture.backtrace;
    }
    data
}

// ── Backtrace parsing ───────────────────────────────────────────────────────

/// Parses `std::backtrace::Backtrace`'s text form into frames, innermost
/// first, starting after the panic machinery.
pub(crate) fn parse_backtrace(text: &str) -> Vec<StackFrame> {
    struct Raw {
        symbol: String,
        location: Option<(String, Option<u32>, Option<u32>)>,
    }
    let mut raw: Vec<Raw> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("at ") {
            if let Some(last) = raw.last_mut()
                && last.location.is_none()
            {
                last.location = Some(parse_location(rest));
            }
            continue;
        }
        if let Some((index, symbol)) = trimmed.split_once(": ")
            && !index.is_empty()
            && index.bytes().all(|b| b.is_ascii_digit())
        {
            raw.push(Raw {
                symbol: symbol.trim().to_owned(),
                location: None,
            });
        }
    }

    // Skip the capture and panic machinery at the top of the stack. Only the
    // leading block: callers further down (tokio's own `catch_unwind`, for
    // example) are real frames.
    let start = raw.iter().take_while(|r| is_panic_machinery(&r.symbol)).count();

    raw.into_iter()
        .skip(start)
        .take(MAX_STACK_FRAMES)
        .map(|r| {
            let (file, line, col) = match r.location {
                Some((f, l, c)) => (Some(f), l, c),
                None => (None, None, None),
            };
            frame_for(Some(&r.symbol), file.as_deref(), line, col)
        })
        .collect()
}

fn is_panic_machinery(symbol: &str) -> bool {
    let s = strip_hash(symbol);
    let s = s.strip_prefix('<').unwrap_or(s);
    s.starts_with("std::backtrace")
        || s.starts_with("std::panicking")
        || s.starts_with("core::panicking")
        || s.starts_with("std::panic::")
        || s.starts_with("core::panic::")
        || s == "rust_begin_unwind"
        || s.starts_with("__rust")
        || s.starts_with("std::sys::backtrace")
        || s.starts_with("agentcat::exceptions")
        || s.starts_with("alloc::boxed::Box<")
}

fn parse_location(rest: &str) -> (String, Option<u32>, Option<u32>) {
    let mut parts = rest.rsplitn(3, ':');
    let col = parts.next();
    let line = parts.next();
    let file = parts.next();
    if let (Some(file), Some(line), Some(col)) = (
        file,
        line.and_then(|l| l.parse().ok()),
        col.and_then(|c| c.parse().ok()),
    ) {
        (file.to_owned(), Some(line), Some(col))
    } else {
        // `file:line` without a column, or a bare path.
        let mut parts = rest.rsplitn(2, ':');
        let line = parts.next().and_then(|l| l.parse().ok());
        match (parts.next(), line) {
            (Some(file), Some(line)) => (file.to_owned(), Some(line), None),
            _ => (rest.to_owned(), None, None),
        }
    }
}

/// Removes a trailing `::h<16 hex>` symbol hash.
fn strip_hash(symbol: &str) -> &str {
    if let Some(idx) = symbol.rfind("::h") {
        let hash = &symbol[idx + 3..];
        if hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return &symbol[..idx];
        }
    }
    symbol
}

/// Splits a symbol on `::` outside angle brackets.
fn split_path(symbol: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let bytes = symbol.as_bytes();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            b':' if depth == 0 && bytes.get(i + 1) == Some(&b':') => {
                parts.push(&symbol[start..i]);
                i += 2;
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&symbol[start..]);
    parts
}

/// `(module, function)` of a demangled symbol, without the hash and the
/// `{{closure}}` markers, so grouping is stable across builds.
fn split_symbol(symbol: &str) -> (Option<String>, String) {
    let clean = strip_hash(symbol);
    // Legacy (`{{closure}}`) and v0 (`{closure#0}`, `{shim:vtable#0}`)
    // markers are dropped.
    let parts: Vec<&str> = split_path(clean)
        .into_iter()
        .filter(|p| !(p.is_empty() || (p.starts_with('{') && p.ends_with('}'))))
        .collect();
    match parts.split_last() {
        Some((function, module)) if !module.is_empty() => (Some(module.join("::")), (*function).to_owned()),
        Some((function, _)) => (None, (*function).to_owned()),
        None => (None, clean.to_owned()),
    }
}

const LIBRARY_MODULE_PREFIXES: &[&str] = &[
    "std",
    "core",
    "alloc",
    "tokio",
    "futures",
    "futures_util",
    "futures_core",
    "rmcp",
    "agentcat",
    "tower",
    "hyper",
    "axum",
    "tracing",
    "<std",
    "<core",
    "<alloc",
    "<tokio",
    "<futures",
    "<futures_util",
    "<rmcp",
    "<agentcat",
    "<tower",
    "<hyper",
    "<axum",
];

fn module_is_library(module: Option<&str>) -> bool {
    let Some(module) = module else {
        return false;
    };
    let head = module.split("::").next().unwrap_or(module);
    LIBRARY_MODULE_PREFIXES.contains(&head)
}

/// Normalizes a source path and decides whether it is the customer's code.
fn normalize_path(path: &str) -> (String, bool) {
    let unix = path.replace('\\', "/");
    if let Some(idx) = unix.find("/.cargo/registry/src/") {
        // …/registry/src/<index>/<crate>-<version>/src/x.rs → <crate>-<version>/src/x.rs
        let rest = &unix[idx + "/.cargo/registry/src/".len()..];
        let rest = rest.split_once('/').map_or(rest, |(_, r)| r);
        return (rest.to_owned(), false);
    }
    if let Some(idx) = unix.find("/.cargo/git/checkouts/") {
        let rest = &unix[idx + "/.cargo/git/checkouts/".len()..];
        return (rest.to_owned(), false);
    }
    if unix.starts_with("/rustc/") {
        let rest = unix.splitn(4, '/').nth(3).unwrap_or(&unix);
        return (rest.to_owned(), false);
    }
    if let Some(rest) = unix.strip_prefix("./") {
        return (rest.to_owned(), true);
    }
    if !unix.starts_with('/') && unix.get(1..3).is_none_or(|s| s != ":/") {
        return (unix, true);
    }
    if let Ok(cwd) = std::env::current_dir() {
        let cwd = cwd.to_string_lossy().replace('\\', "/");
        if let Some(rest) = unix.strip_prefix(&format!("{}/", cwd.trim_end_matches('/'))) {
            return (rest.to_owned(), true);
        }
    }
    // Hide the user's home directory.
    for prefix in ["/home/", "/Users/"] {
        if let Some(rest) = unix.strip_prefix(prefix)
            && let Some((_, after)) = rest.split_once('/')
        {
            return (format!("~/{after}"), true);
        }
    }
    (unix, true)
}

fn frame_for(symbol: Option<&str>, file: Option<&str>, line: Option<u32>, col: Option<u32>) -> StackFrame {
    let (module, function) = match symbol {
        Some(s) => split_symbol(s),
        None => (None, "<unknown>".to_owned()),
    };
    let (filename, path_in_app) = match file {
        Some(f) => normalize_path(f),
        None => ("<unknown>".to_owned(), false),
    };
    let in_app = path_in_app && !module_is_library(module.as_deref()) && file.is_some();
    StackFrame {
        filename,
        function,
        module,
        lineno: line,
        colno: col,
        in_app,
        abs_path: file.map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "   0: std::backtrace_rs::backtrace::libunwind::trace
             at /rustc/abc123/library/std/src/../../backtrace/src/backtrace/libunwind.rs:117:9
   1: std::backtrace::Backtrace::create
             at /rustc/abc123/library/std/src/backtrace.rs:331:13
   2: agentcat::exceptions::install_panic_hook::{{closure}}::{{closure}}
             at ./src/exceptions.rs:80:33
   3: std::panicking::rust_panic_with_hook
             at /rustc/abc123/library/std/src/panicking.rs:841:13
   4: core::panicking::panic_fmt
             at /rustc/abc123/library/core/src/panicking.rs:75:14
   5: my_server::tools::Counter::explode::{{closure}}
             at ./src/tools.rs:42:9
   6: <my_server::Counter as rmcp::handler::server::ServerHandler>::call_tool::{{closure}}
             at /home/alice/code/my_server/src/main.rs:10:5
   7: tokio::runtime::task::core::Core<T,S>::poll
             at /home/alice/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tokio-1.47.1/src/runtime/task/core.rs:375:24
   8: main
";

    #[test]
    fn parses_std_backtrace() {
        let frames = parse_backtrace(SAMPLE);
        assert_eq!(frames.len(), 4, "{frames:#?}");
        assert_eq!(frames[0].function, "explode");
        assert_eq!(frames[0].module.as_deref(), Some("my_server::tools::Counter"));
        assert_eq!(frames[0].filename, "src/tools.rs");
        assert_eq!(frames[0].lineno, Some(42));
        assert_eq!(frames[0].colno, Some(9));
        assert!(frames[0].in_app);

        assert_eq!(frames[1].function, "call_tool");
        assert_eq!(
            frames[1].module.as_deref(),
            Some("<my_server::Counter as rmcp::handler::server::ServerHandler>")
        );
        assert!(frames[1].in_app);

        assert_eq!(frames[2].filename, "tokio-1.47.1/src/runtime/task/core.rs");
        assert!(!frames[2].in_app);

        assert_eq!(frames[3].function, "main");
        assert_eq!(frames[3].filename, "<unknown>");
        assert!(!frames[3].in_app);
    }

    #[test]
    fn skips_only_the_leading_panic_machinery() {
        let text = "   0: std::backtrace::Backtrace::force_capture
   1: <alloc::boxed::Box<dyn for<'a, 'b> core::ops::function::Fn<(&'a std::panic::PanicHookInfo<'b>,), Output = ()> + core::marker::Sync + core::marker::Send> as core::ops::function::Fn<(&std::panic::PanicHookInfo,)>>::call
   2: std::panicking::panic_with_hook
   3: std::panicking::begin_panic::<&str>::{closure#0}
   4: std::sys::backtrace::__rust_end_short_backtrace::<std::panicking::begin_panic<&str>::{closure#0}, !>
   5: std::panicking::begin_panic::<&str>
   6: <my_server::Todo>::explode::{closure#0}
             at ./tests/common/mod.rs:210:9
   7: std::panicking::catch_unwind::<(), tokio::runtime::task::harness::poll_future::{closure#0}>
   8: tokio::runtime::task::harness::poll_future
";
        let frames = parse_backtrace(text);
        assert_eq!(frames[0].function, "explode");
        assert_eq!(frames[0].module.as_deref(), Some("<my_server::Todo>"));
        assert_eq!(frames[0].filename, "tests/common/mod.rs");
        assert!(frames[0].in_app);
        assert_eq!(frames.len(), 3);
    }

    #[test]
    fn strips_hashes() {
        assert_eq!(strip_hash("foo::bar::h0123456789abcdef"), "foo::bar");
        assert_eq!(strip_hash("foo::hello"), "foo::hello");
        assert_eq!(
            split_symbol("a::b::c::h0123456789abcdef"),
            (Some("a::b".into()), "c".into())
        );
    }

    #[test]
    fn protocol_error_message_matches_ts() {
        let e = capture_protocol_error(-32602, "tool not found");
        assert_eq!(e.message, "MCP error -32602: tool not found");
        assert_eq!(e.error_type.as_deref(), Some("McpError"));
    }

    #[test]
    fn panic_capture_inside_tracked_call() {
        install_panic_hook();
        let payload = std::panic::catch_unwind(|| in_tracked_call(|| panic!("boom"))).unwrap_err();
        let capture = take_panic_capture();
        assert!(capture.is_some());
        let data = capture_panic(&*payload, capture);
        assert_eq!(data.message, "boom");
        assert_eq!(data.error_type.as_deref(), Some("panic"));
        assert!(data.stack.is_some());
        // Panics outside tracked calls are not recorded.
        let _ = std::panic::catch_unwind(|| panic!("outside"));
        assert!(take_panic_capture().is_none());
    }
}
