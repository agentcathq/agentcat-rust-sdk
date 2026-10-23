# Plan 4: Delivery and lifecycle (event pipeline, worker thread, custom events, `Fake`, event comparison) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn each tool-call record and each custom event into the event TS `9b56e9e` publishes (hook results, redaction, sanitization, truncation, `evt_` ID), deliver it from an SDK-owned thread with TS's queue and retry rules, give the customer `flush`, `shutdown`, a bounded drain on the last drop, the log file, the environment switches, custom events and a test `Fake`, and prove the published events equal TS's in the differential harness.

**Architecture:** The call path hands its `CallRecord` to the `Sink` as before; the new `Delivery` sink queues it without blocking (10 000 pending, drop oldest) and wakes one lazily started thread, `agentcat-delivery`, that runs a current-thread tokio runtime with at most 5 events in flight. For each event the worker assembles the TS in-memory event (`TsEvent`, TS key order), settles the hooks (30 s each), runs the customer's redaction hooks on the customer's runtime, then the plan 2 stages (string redaction, sanitize, truncate), assigns the `evt_` ID and POSTs through `agentcat_api::apis::events_api::publish_event` with 3 retries at 1, 2 and 4 s. The worker also writes the SDK's log lines, off the request path. TS vectors pin the event key order and the whole queue composition; the harness compares the tracked events of TS and Rust after normalization.

**Tech Stack:** Rust 1.88+ (edition 2024); rmcp 3.x (floor 3.0.0); `agentcat-api` `=0.0.3` (R11; `reqwest` 0.13 inside); `reqwest` 0.13 (client builder only); `chrono` 0.4 (`std`); `thiserror` 2; `tracing` 0.1; tokio (`rt`, `time`, `sync`); `serde_json` (`preserve_order`). Dev: `wiremock` 0.6. Node 22: the oracle (TS at the pinned SHA) and the harness.

**Spec:** `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (sections 5, 6.1, 6.6, 6.7, 7, 9, 11, 12 milestone 5, 13). Behavior contract: `docs/research/01-ts-sdk-behavior.md` B-003, B-006, B-009, B-026, B-077 to B-107, B-114 to B-119, K-03, K-04, K-06, K-09, K-11, K-12, K-13. API client facts: `docs/research/05-api-client-and-rmcp-usage.md` Part 1. Decisions: `docs/DECISIONS.md` (C07, C09 to C12, C14, C16, C26, C29, C32, D03, D19, D32, D36, D38, D40, D41, D43, D44, R03, R06, R10, R11, R27). Deferred items: `docs/superpowers/plans/carry-forward.md`, section "Plan 4" (every item maps to a task below; the map is at the end of this plan).

## Global Constraints

- Branch `rewrite`; conventional-commit subjects; no AI attribution lines; push only to `origin rewrite`.
- `edition = "2024"`, `rust-version = "1.88"`. Library code: no `unwrap`, `expect`, `panic!`, `todo!`, printing; every lint exception is `#[expect(<lint>, reason = "...")]` (the rmcp-minor exception of plan 3 still applies). The `testing` module's assertion helpers are the one place that may panic, each under `#[expect(clippy::panic, reason = "assertion helper of the test fake")]`.
- `#![forbid(unsafe_code)]` stays. rmcp's `#[doc(hidden)]` items may be used where they make the code cleaner (D21); private fields may not.
- Public API: only items an Interfaces block marks `pub` become public, each re-exported from the curated `pub use` block in `lib.rs`, each with rustdoc and a `Debug` impl. `reqwest`, `tokio`, `chrono` and `agentcat-api` types never appear in a public signature (C08). Every other item is `pub(crate)` at most. A crate-private item that only a later task of this plan reads carries `#[cfg_attr(not(test), expect(dead_code, reason = "read by <what> (plan 4 task N)"))]`, removed by that task.
- Dependencies: library and binary dependencies go into `[workspace.dependencies]` with `default-features = false`; each floor is the lowest version that passes `cargo +nightly minimal-versions check --direct --workspace`. `agentcat-api = { version = "=0.0.3", default-features = false }` (R11). Test-only dependencies go into the crate's `[dev-dependencies]`.
- `serde_json` keeps `preserve_order` (R07); nothing turns on `float_roundtrip` (R22) or `arbitrary_precision`.
- Expected values come from TS only: a vector file written by `node oracle/src/vectors/run.mjs`, a live oracle call, or a harness run of a TS reference server. A Rust-only test may assert a shape, a Rust-only path (the worker, the thread, the drain, the runtime of a hook) or a deviation, and names the deviation or decision ID. Vector files are never edited by hand.
- Read TS code only in `oracle/.cache/ts-9b56e9e920f35f7c37fc2a63ce6520a9ec97edea`. Never read `~/agentcat-typescript-sdk` or attempt-1 files and tags. Never modify `~/agentcat-server`.
- No test and no harness run reaches a production AgentCat endpoint: Rust tests use `wiremock` or a recording publisher on loopback; the harness keeps its egress guard and the local-URL check. Never print or copy tokens or credentials.
- Never panic, never block the host runtime on the call path, never print (design §7.3). The SDK's own failures become log lines with the TS message text. Lengths are UTF-16 units (`js::utf16_len`, `js::cut_utf16`); JSON objects iterate in JavaScript order where TS iterates them (`js::own_entries`).
- Time-dependent tests never wait for the real budgets (30 s hooks, 1/2/4 s retries, 5 s drain): they shrink them through `DeliveryConfig` (Task 6). A test that needs real time keeps its wall-clock wait under 3 s.
- Parity rule, for each TS test file a task owns: a pure input-to-output test becomes a vector case (`"ts_test"`; manifest `n/a`, `covered-by-vector:<file>.json`). A behavior pinned by a named Rust test is `ported` with that name. A behavior pinned by a harness scenario (MCP-wire delta or event comparison) is `ported` with `rust = ["harness:<scenario>"]`. A TS mechanism Rust does not have is `n/a` with its RD (RD-09 signal handlers, RD-16 exporters and diagnostics as `second-pass`, RD-19 Workers and detection). Entries stay in `parity/ts-tests.json` order. Run `cargo run -q -p xtask -- parity` (exit 0) after each mapping.
- Before each commit, run the full local gate: `scripts/check.sh` must exit 0. CI mirrors it (`.github/workflows/ci.yml`); a change to `scripts/check.sh` gets the same change in the matching CI job.
- Model per task (owner rule: Opus 5.5 for the hardest half, Sonnet 5.5 for the rest; reviewers use the implementer's model): stated at the top of each task.

## Decisions this plan makes

Task 14 records each one in `docs/DECISIONS.md` (builder decisions table) and each RD row in the design's section 9. A reviewer who disagrees raises it; the controller rules.

- **C36 Lifecycle timing lives on the worker.** `flush` and `shutdown` send the deadline to the worker, which answers when the queue and the in-flight events are empty or the deadline passes; the async methods await that answer without a customer timer, the blocking twins wait on a `std::sync::mpsc` channel with `recv_timeout(timeout + 100 ms)` and return `false` when the worker is gone. On the last drop of a handle whose worker runs, `Drop` runs `shutdown_blocking(5 s)`; a drop on the worker thread itself does not wait. `shutdown` is terminal (C12): after its deadline the worker stops and abandons what is left, after TS's `Shutting down with ${n} events still in queue` when the pending queue is not empty.
- **C37 Hook budget.** A call hook that has not settled after 30 s is aborted (`JoinHandle::abort`) after TS's timeout log line (RD-35). A hook that panics or is cancelled gives `Warning: ${label} hook failed in background - ${message}` and no value; the process panic hook prints the panic as for any task. A hook's own error gives TS's per-hook text (`resolveSessionId hook error: `, `Error: identify callback threw - `, `eventTags callback error: `, `eventProperties callback error: ` followed by the error's `Display`).
- **C38 Redaction hooks run on the customer's runtime** (C14): the runtime the call ran on (`Handle::try_current()` at call start), or, for a custom event, the runtime of the caller of `publish_custom_event`. The string hook runs once per string, in walk order, inside one spawned task. Without a runtime, or when that task panics or is cancelled, the event is dropped with TS's stage log line (`Failed to redact event (event-level hook): ...` or `Failed to redact event: ...`, RD-36).
- **C39 The `Event` of `redact_event` is typed.** Public fields, `#[non_exhaustive]`, converted from and back into the `TsEvent` in TS key order. It cannot carry keys TS events do not have (no `__proto__`, no extra keys), and its `tags` are strings (`BTreeMap<String, String>`). RD-24's restore sentence about a top-level `__proto__` becomes: "the hook's result is a typed `Event`, so it has no such key".
- **C40 The wire body.** Each JSON value of `PublishEventRequest` (`parameters`, `response`, `error`, `identify_data`, `properties`) is `js::to_js_value` of the event value: the `js::stringify` text read back, so integers, `-0`, integers above 2^53 and index-like key order equal TS's. The size guard measures the `js::stringify` bytes of the `TsEvent`, as TS measures its in-memory event; the wire text is not the measured text in TS either. Floats keep serde_json's text (RD-32). A `userIntent` that is not a string is sent as its `js::stringify` text (RD-33). `actor_id` is `identify_actor_given_id` (TS's `actorId || identifyActorGivenId`); `ip_address` and `event_id` are never set.
- **C41 A cancelled call.** A call future dropped after its record exists (a customer timeout, a `select!`) publishes the record with no response, `is_error: true` and `error: {"message": "tool call cancelled before it completed", "type": "cancelled", "platform": "rust"}` (RD-34).
- **C42 Logging.** Log lines go through the handle's worker, which writes them; nothing writes the file on the request path. The destination is process-global and the first handle built wins (D38): `LogDestination::File` (default, `<home>/agentcat.log`, appended, created, one open per line, errors ignored, silent without a home directory) or `LogDestination::Tracing` (`tracing` events with target `agentcat`, level ERROR when the line matches `/fail|error/i`, WARN when it contains `Warning:`, else INFO, B-117's rule). Line format: `[<toISOString>] [sdk=<crate version> rust=<rustc version> rmcp=<rmcp version>] <message>`; the rustc version comes from a build script (`unknown` when it cannot run `$RUSTC --version`), the rmcp version from `rmcp::model::Implementation::default().version`. A handle whose worker never starts writes nothing.
- **C43 The `Fake`.** `testing::Fake` (feature `test-util`) builds handles whose events never leave the process. It records every event that finishes the pipeline, telemetry-only events included. Its methods are async and flush the handles first.
- **C44 The HTTP client.** `reqwest` client with a 5 s connect timeout, a 10 s per-attempt timeout and User-Agent `agentcat-rust/<crate version>`, set on the client and as `Configuration::user_agent`; no API key. When the client cannot be built, one log line says so and each event that would be POSTed is dropped with TS's final failure line. A build without a TLS feature logs one warning when the base URL is `https:`.
- **C45 `AGENTCAT_ENABLED` false** (D44, trimmed, case-insensitive `false|0|no|off`): the handle starts no thread and its `Tracked<H>` forwards every method untouched, except that `call_tool` still inserts a `CallContext` without session or agent (C29). Custom events return `Ok(())` and publish nothing; `flush` and `shutdown` return `true`.
- **C46 Harness event comparison.** The matrix compares the normalized tracked events of the two servers per combination, with event rulings in `harness/rulings-events.json` (`rulings-events-ts.json` when both servers are TS). New normalizer rules: `extra-projection` (replaces `extra-headers-only`), `number-source`, `timestamp-class` with RFC 3339 whole seconds (RD-04), derived and minted `ses_` placeholders, sorting on the normalized body. A tracked Rust server must print `{"drained":true}` on stderr before it exits, and any server that needs SIGKILL fails the run.
- **C47 Custom events.** `publish_custom_event` only queues, so it is synchronous and returns `Result<(), Error>`; `Error::ProjectIdRequired` (`projectId is required for publishCustomEvent`) is its only failure. The server form on `CallContext` uses that call's client and server identity; on `Tracked<H>`, the identity of the instance's `initialize`. Neither takes a default session from the call (TS has none): pass `call.session_id()` in the `CustomEvent`.

New deviation rows (Task 14 adds them to section 9; each task that creates the behavior names its row in rustdoc and tests):

| ID | TS behavior | Rust behavior | Why |
|---|---|---|---|
| RD-32 | The body is `JSON.stringify` text: `1e+21`, `0.000001`. | Values pass through `js::to_js_value`, so integers, `-0` and key order equal TS's; a float keeps serde_json's text (`1e21`, `1e-6`), the same number. | `agentcat-api` serializes with serde_json. |
| RD-33 | An array `userIntent` goes on the wire as a JSON array (K-11). | Its `js::stringify` text, a string. | `agentcat-api` types `user_intent` as a string. |
| RD-34 | A JS call cannot be cancelled. | A dropped call future publishes `type: "cancelled"` (C41). | Rust futures can be dropped. |
| RD-35 | A hook past its 30 s budget keeps running, ignored. | It is aborted (C37). | A detached task would leak. |
| RD-36 | Redaction hooks always have the event loop. | Without a tokio runtime the event is dropped (C38). | Hooks run on the customer's runtime (C14). |
| RD-37 | A 2xx answer with any JSON body is a success. | `agentcat-api` also needs `Content-Type: application/json`; otherwise the attempt fails and is retried. | `agentcat-api` model. |

## Review Focus

1. **A hook that hangs, panics or outlives its runtime, and a current-thread customer runtime.** A person expects the event to publish without that hook's data after the budget, `flush` and `shutdown` to return by their deadlines, and the last-drop drain to return within 5 s even when it blocks the only thread the hook could run on (design §13). Pinned by Task 4 tests `hanging_hook_is_aborted_after_budget` and `panicking_hook_logs_and_publishes_without_it`, and Task 6 tests `last_drop_on_current_thread_runtime_returns_within_budget` and `flush_returns_false_at_deadline`.
2. **The API is down, slow or answers oddly** (connection refused, 500, a 10 s hang, a 202 with a text body). A person expects 4 attempts at 1, 2 and 4 s with TS's log lines, tool calls that never wait, and bounded `flush`/`shutdown`. Pinned by Task 6 test `retries_three_times_then_logs_failure` and Task 8 tests `hung_attempt_times_out`, `refused_connection_is_retried` and `text_body_is_a_failed_attempt` (RD-37).
3. **A burst above 10 000 pending events.** A person expects the oldest pending event dropped with `Event queue full, dropping oldest event`, the call path never blocked, and in-flight events untouched. Pinned by Task 6 tests `overflow_drops_oldest_pending` and `enqueue_never_blocks`.
4. **Customer redaction that fails, drops the event or edits system fields.** A person expects TS's drop and restore results and never an unredacted body on the wire. Pinned by the Task 5 `queue.json` cases (`redact-event:*`, `redact-strings:*`) and tests `redaction_without_runtime_drops_event` and `string_redactor_panic_drops_event`.
5. **Numbers and keys that serde_json writes differently from JavaScript** (`1.0`, `-0.0`, 2^53 + 1, `{"10": …, "2": …}` in `parameters`, `response`, `properties`). A person expects the backend to receive TS's values. Pinned by Task 5 test `wire_values_follow_js_text` and the Task 12 normalizer rule `number-source`.

---

### Task 1: Oracle: TS event key order and queue vectors

Model: Opus 5.5.

**Files:**
- Create: `oracle/src/vectors/gen/event_order.mjs`, `oracle/src/vectors/gen/queue.mjs`, `agentcat-sdk/tests/vectors/event_order.json`, `agentcat-sdk/tests/vectors/queue.json` (generated)
- Modify: `oracle/src/bundle.mjs` (core modules the generators need, for example `eventQueue`, `pendingEvent`, `redaction`, `sanitization`, `internal`), `oracle/src/vectors/run.mjs`, `agentcat-sdk/tests/vectors/MANIFEST.sha256` (generated), `oracle/test/*` (a test per generator, in the style of the existing ones)

**Interfaces:**
- Produces `event_order.json`, written by running the real TS SDK (`track` on a v1 `McpServer` over an in-memory transport, a client that sends `_meta` and `context`; a `redactEvent` hook that records `Object.keys` and returns the event unchanged):
  - `tool_call.event`: the keys of the in-memory tool-call event, in order, including keys whose value is `undefined` (mark each with `"undefined": true`).
  - `tool_call.request`, `tool_call.params`, `tool_call.extra`: `Object.keys` of `parameters.request`, of `parameters.request.params` and of `parameters.extra`, for a call with `_meta`.
  - `custom_string.event`: the key order of a session-string custom event when it reaches the queue (its `id` is added by the queue, last); `custom_server.event`: the same for the server form.
- Produces `queue.json`: cases run through TS's `EventQueue` (its `processEvent`, with the API client replaced by a recorder and the delay replaced by a no-op that records the requested milliseconds). Each case:
  - `name`, `ts_test` where a TS test covers it.
  - `event`: the in-memory event as `publishEvent` or `publishCustomEvent` builds it (camelCase keys, top-level `timestamp` an ISO string, no functions), in TS key order.
  - `hooks` (optional): raw hook outcomes, each `{"returns": <value>}`, `{"throws": "<message>"}`, or `{"hangs": true}` (the 30 s budget, driven with `node:test` mock timers): `resolveSessionId` (hook mode only, with `"hook_mode": true` on the case), `identify`, `eventTags`, `eventProperties`. The generator runs TS's own resolvers (`resolveIdentity`, `resolveEventTags`, `resolveEventProperties`, `invokeSessionHook`) on them, so the logs and the tag validation are TS's.
  - `redact_event` (optional): `{"returns": null}`, `{"throws": "<message>"}`, or `{"merge": {<camelCase field>: <value>}}` (returns `{...event, ...merge}`).
  - `redact_strings` (optional): `{"replace": "<secret>"}` (each occurrence becomes `[REDACTED]`) or `{"throw_at": n}` (the nth call throws `Error("redactor failed")`).
  - `fail_attempts` (optional): how many publish attempts reject with `Error("mock failure")`.
  - Outputs: `requests`: every `PublishEventRequest` object the recorder received (the camelCase model before the generated serializer), with each `evt_` ID as `"<evt>"`; `delays`: the backoff milliseconds requested; `logs`: TS's log lines (heads stripped, `captureLogs`), `evt_` IDs as `"<evt>"`.
- Case families (names are prefixes; at least the listed ones): `call:plain`, `call:telemetry-only` (`projectId: ""`, no request), `call:no-session` (`sessionId: ""`, wire `null`), `hooks:identify`, `hooks:identify-throws`, `hooks:identify-hangs`, `hooks:tags-invalid` (bad key, long key, non-string value, newline, 51 valid), `hooks:tags-collide` (customer tag named like an SDK tag), `hooks:properties-empty`, `hooks:properties-throws`, `hooks:session-hook` (hook mode, returns `"customer-abc"`), `hooks:session-hook-null`, `redact-event:drop`, `redact-event:throws`, `redact-event:merge-restored` (merge changes `id`, `sessionId`, `projectId`, `eventType`, `timestamp` and `resourceName`), `redact-strings:replace` (secret in `parameters`, `response`, `error.message`, `identifyActorData`, and in protected `tags` and `resourceName`), `redact-strings:throws`, `sanitize:image-block`, `truncate:big-response` (a 120 000-character text block), `truncate:non-string-context` (K-11, dropped), `retry:2-failures`, `retry:4-failures`, `custom:string-form`, `custom:string-form-invalid-tags`, `custom:server-form`, `custom:server-form-redact-drop`.

- [ ] **Step 1: Write the generator tests** in `oracle/test/` that run each generator and assert: `event_order.json` lists `id`, `sessionId`, `projectId`, `eventType`, `timestamp` first for a tool call; every `queue.json` case has `requests`, `delays` and `logs`; `retry:4-failures` has `delays` `[1000, 2000, 4000]` and ends with the log line that starts `Failed to deliver AgentCat event after retries:`.
- [ ] **Step 2: Run them to see them fail.** Run: `corepack pnpm --filter oracle test`. Expected: FAIL (generators missing).
- [ ] **Step 3: Implement the generators and register them in `run.mjs`.** Load TS modules through `oracle/src/bundle.mjs`; keep the egress guard; isolate the TS log file with `logs.mjs`.
- [ ] **Step 4: Generate and check.** Run: `node oracle/src/vectors/run.mjs && node oracle/src/vectors/run.mjs --check && corepack pnpm --filter oracle test`. Expected: exit 0, two new vector files, manifest updated.
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add oracle agentcat-sdk/tests/vectors
git commit -m "test(oracle): TS event key order and queue composition vectors"
```

---

### Task 2: Public types and configuration

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/event/public.rs`, `agentcat-sdk/src/custom.rs`, `agentcat-sdk/src/custom/tests.rs`, `agentcat-sdk/src/error.rs`
- Modify: `Cargo.toml`, `agentcat-sdk/Cargo.toml`, `Cargo.lock`, `agentcat-sdk/src/config.rs`, `agentcat-sdk/src/config/tests.rs`, `agentcat-sdk/src/event.rs`, `agentcat-sdk/src/event/tests.rs`, `agentcat-sdk/src/capture.rs`, `agentcat-sdk/src/capture/tests.rs`, `agentcat-sdk/src/js.rs`, `agentcat-sdk/src/js/tests.rs`, `agentcat-sdk/src/lib.rs`

**Interfaces:**
- Dependencies: `thiserror` 2, `chrono` 0.4 (`std` only).
- Produces (`js`): `pub(crate) fn iso_millis(time: SystemTime) -> String`: JavaScript's `Date.prototype.toISOString` (`YYYY-MM-DDTHH:MM:SS.mmmZ`, always three millisecond digits, milliseconds truncated); `pub(crate) fn parse_iso_millis(text: &str) -> Option<SystemTime>` (RFC 3339 with or without a fraction).
- Produces (`config`):
  - `pub fn api_base_url(self, url: impl Into<String>) -> Self`.
  - `pub fn redact_sensitive_information<F, Fut>(self, hook: F) -> Self` where `F: Fn(String) -> Fut + Send + Sync + 'static`, `Fut: Future<Output = Result<String, HookError>> + Send + 'static`.
  - `pub fn redact_event<F, Fut>(self, hook: F) -> Self` where `F: Fn(Event) -> Fut + Send + Sync + 'static`, `Fut: Future<Output = Result<Option<Event>, HookError>> + Send + 'static` (`Ok(None)` drops the event).
  - `pub fn log_destination(self, destination: LogDestination) -> Self`; `#[non_exhaustive] pub enum LogDestination { File, Tracing }` (`Clone, Copy, Debug, Default = File, PartialEq, Eq`).
  - Crate getters: `api_base_url_option(&self) -> Option<&str>` (non-empty only), `string_redactor(&self) -> Option<&StringRedactor>`, `event_redactor(&self) -> Option<&EventRedactor>`, `log_to(&self) -> LogDestination`. `pub(crate) type RedactFuture<T> = Pin<Box<dyn Future<Output = Result<T, HookError>> + Send>>`; `StringRedactor::call(&self, text: String) -> RedactFuture<String>`; `EventRedactor::call(&self, event: Event) -> RedactFuture<Option<Event>>`. Both `Clone` (an `Arc` inside) and `Debug` (prints only that a hook is set).
  - `pub const AGENTCAT_PROJECT_ID: &str = "AGENTCAT_PROJECT_ID";` and likewise `AGENTCAT_API_URL`, `MCPCAT_API_URL`, `AGENTCAT_ENABLED`; `pub(crate) const DEFAULT_API_BASE_URL: &str = "https://api.agentcat.com";`.
  - `pub(crate) trait Env: Send + Sync + fmt::Debug { fn var(&self, name: &str) -> Option<String>; fn home_dir(&self) -> Option<PathBuf>; }`; `pub(crate) struct ProcessEnv;` (the only reader of `std::env`, under `#[expect(clippy::disallowed_methods, reason = "...")]`); `#[cfg(test)] pub(crate) struct MapEnv` with `MapEnv::new(vars: &[(&str, &str)])` and `fn with_home(self, home: PathBuf) -> Self`.
  - `pub(crate) fn resolve_api_base_url(options: &Options, env: &dyn Env) -> String`: the first non-empty of the option, `AGENTCAT_API_URL`, `MCPCAT_API_URL`, else `DEFAULT_API_BASE_URL` (B-006, per handle C10); the value is used as given (no trailing-slash trim, as TS).
  - `pub(crate) fn sdk_enabled(env: &dyn Env) -> bool`: `false` when `AGENTCAT_ENABLED`, trimmed and lower-cased, is `false`, `0`, `no` or `off`; `true` otherwise, unset included (D44, R10).
- Produces (`event::public`): `pub struct Event` (`#[non_exhaustive]`, `Clone, Debug, PartialEq`) with public fields `id: String`, `session_id: Option<String>`, `project_id: String`, `event_type: String`, `timestamp: SystemTime`, `duration: Option<Duration>`, `input_tokens: Option<u32>`, `output_tokens: Option<u32>`, `sdk_language`, `agentcat_version`, `server_name`, `server_version`, `client_name`, `client_version`, `identify_actor_given_id`, `identify_actor_name`, `resource_name` (each `Option<String>`), `identify_actor_data: Option<Map<String, Value>>`, `parameters: Option<Value>`, `response: Option<Value>`, `user_intent: Option<Value>`, `is_error: Option<bool>`, `error: Option<Value>`, `tags: Option<BTreeMap<String, String>>`, `properties: Option<Map<String, Value>>`. Crate: `pub(crate) fn from_ts(event: &TsEvent) -> Event` and `pub(crate) fn into_ts(self) -> TsEvent` (keys in the `event_order.json` order, `None` fields absent; an empty `session_id` is `None` and goes back as `""`; `timestamp` through `js::parse_iso_millis` and back through `js::iso_millis`, `UNIX_EPOCH` when it does not parse, which the restore of `timestamp` makes harmless).
- Produces (`custom`): `pub struct CustomEvent` (`Clone, Debug, Default`) with `pub fn new() -> Self` (= `default()`) and consuming setters `session_id(impl Into<String>)`, `resource_name(impl Into<String>)`, `parameters(Value)`, `response(Value)`, `message(impl Into<String>)`, `duration(Duration)`, `is_error(bool)`, `error(Value)`, `tags(Map<String, Value>)`, `properties(Map<String, Value>)` (TS `CustomEventData`, B-009), and `pub(crate)` getters for Task 7.
- Produces (`error`): `#[derive(Debug, thiserror::Error)] #[non_exhaustive] pub enum Error { #[error("projectId is required for publishCustomEvent")] ProjectIdRequired }`, with a compile-time `Send + Sync + 'static` assertion.
- Changes: `pub fn capture_error<E: Error + ?Sized + 'static>(error: &E)` (R27); its rustdoc says a `dyn Error` records the `dyn` type's name (for example `dyn core::error::Error`).
- `lib.rs` re-exports `Event`, `CustomEvent`, `Error`, `LogDestination` and the four environment constants.

- [ ] **Step 1: Write the failing tests.** `js/tests.rs`: `iso_millis_matches_to_iso_string` (the oracle's `Date` output for epoch 0, a time with 7 ms, and one with 999 ms, through a live oracle call or a `js.json` vector case added in this step), `parse_iso_millis_reads_both_forms`. `config/tests.rs`: `api_base_url_precedence` (option over `AGENTCAT_API_URL` over `MCPCAT_API_URL` over the default; empty values skipped), `sdk_enabled_values` (`"false"`, `" OFF "`, `"No"`, `"0"` disable; `"true"`, `""`, `"maybe"`, unset enable), `redactors_are_set_and_debug_hides_them`. `event/tests.rs`: `event_round_trips_in_ts_key_order` (a `TsEvent` with every field, keys in `event_order.json` `tool_call.event` order, through `from_ts` and `into_ts`, equals itself), `event_has_no_unknown_keys` (an extra key and a `__proto__` key in the `TsEvent` are absent after the round trip; C39, RD-24). `custom/tests.rs`: `custom_event_setters_store_fields`. `capture/tests.rs`: `capture_error_accepts_dyn_error` (`let error: Box<dyn Error> = ...; capture_error(&*error)` records `message`).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p agentcat-sdk js:: config:: event:: custom:: capture::`. Expected: compile errors for the missing items.
- [ ] **Step 3: Implement the items above.**
- [ ] **Step 4: Run the tests.** Same command. Expected: PASS.
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add Cargo.toml Cargo.lock agentcat-sdk oracle
git commit -m "feat(sdk): Event, CustomEvent, Error, redaction and log options, environment lookup"
```

---

### Task 3: Logging

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/build.rs`, `agentcat-sdk/src/logging.rs`, `agentcat-sdk/src/logging/tests.rs`
- Modify: `Cargo.toml`, `agentcat-sdk/Cargo.toml` (`build = "build.rs"`, `include` gains `build.rs`), `Cargo.lock`, `agentcat-sdk/src/lib.rs`, `parity/manifest.toml`

**Interfaces:**
- Consumes: `js::iso_millis`, `LogDestination` (Task 2).
- Dependencies: `tracing` 0.1 (`std`).
- Produces (`logging`): `pub(crate) struct Logger` with `pub(crate) fn new(destination: LogDestination, home: Option<PathBuf>) -> Self` and `pub(crate) fn write(&self, message: &str)` (C42 format and destinations); `pub(crate) fn configure(destination: LogDestination, home: Option<PathBuf>)` (first call wins, `OnceLock`); `pub(crate) fn write(message: &str)` (the global logger; nothing before `configure`); `pub(crate) fn prefix() -> &'static str` (`[sdk=… rust=… rmcp=…]`); `pub(crate) fn level(message: &str) -> tracing::Level` (B-117's rule).
- `build.rs`: runs `$RUSTC --version`, takes the second word, and emits `cargo:rustc-env=AGENTCAT_RUSTC_VERSION=<it>` (`unknown` on any failure); never fails the build; `cargo:rerun-if-env-changed=RUSTC`.

- [ ] **Step 1: Write the failing tests** in `logging/tests.rs`: `file_logger_appends_prefixed_lines` (a temp home; two writes give two lines matching `^\[\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z\] \[sdk=1\.0\.0 rust=[^ ]+ rmcp=3\.[^\]]+\] message$`), `no_home_is_silent` (no file, no panic), `unwritable_home_is_ignored`, `level_follows_ts_severity` (`"Failed to send"` and `"an ERROR"` give ERROR, `"Warning: x"` WARN, `"Successfully sent"` INFO), `tracing_destination_emits_events` (a `tracing` subscriber scoped with `tracing::subscriber::with_default` captures target `agentcat` and the message).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p agentcat-sdk logging::`. Expected: compile errors.
- [ ] **Step 3: Implement `build.rs` and `logging`.** Nothing calls `logging::write` yet; Task 6 does (mark the items per the Global Constraints).
- [ ] **Step 4: Run the tests.** Same command. Expected: PASS.
- [ ] **Step 5: Map the parity entries.** Owned files: `logging-target.test.ts`, `logging-version-prefix.test.ts` (ported, or `ruled-divergence:RD-15` for Node-only targets), every `diagnostics-*.test.ts` (`second-pass`, RD-16). Run `cargo run -q -p xtask -- parity` (exit 0).
- [ ] **Step 6: Run `scripts/check.sh`, then commit.**

```bash
git add Cargo.toml Cargo.lock agentcat-sdk parity
git commit -m "feat(sdk): log file and tracing destination with the TS line format"
```

---

### Task 4: Event assembly and hook settling

Model: Opus 5.5.

**Files:**
- Create: `agentcat-sdk/src/event/assemble.rs`, `agentcat-sdk/src/event/assemble/tests.rs`, `agentcat-sdk/src/pipeline/settle.rs`, `agentcat-sdk/src/pipeline/settle/tests.rs`
- Modify: `agentcat-sdk/src/event.rs` (`CallRecord` fields; `request_value` and `extra_value` key order), `agentcat-sdk/src/event/tests.rs`, `agentcat-sdk/src/capture.rs` (keep the `Backtrace`), `agentcat-sdk/src/capture/tests.rs`, `agentcat-sdk/src/call/run.rs`, `agentcat-sdk/src/pipeline/mod.rs`, `agentcat-sdk/src/handles/mod.rs` (drop the `expect(dead_code)` on `session_from_hook_value`), `agentcat-sdk/src/lib.rs`, `parity/manifest.toml`

**Interfaces:**
- Consumes: `event_order.json` (Task 1); `Event`, `UserIdentity`, `js::iso_millis` (Tasks 2, 3).
- Changes `CallRecord`: adds `runtime: Option<tokio::runtime::Handle>` (the call's runtime, `Handle::try_current()` at call start) and `backtrace: Option<Backtrace>` (from `capture_error`, rendered by the worker). `capture_error` no longer formats the backtrace: `CaptureSlot` keeps `(Map<String, Value>, Option<Backtrace>)`, and `run.rs` moves both into the record.
- Changes `request_value` and `extra_value`: keys in the `event_order.json` orders `tool_call.request`, `tool_call.params` (including where `_meta` goes) and `tool_call.extra`.
- Produces (`event::assemble`):
  - `pub(crate) struct EventJob { pub(crate) event: TsEvent, pub(crate) hooks: PendingHooks, pub(crate) hook_mode: bool, pub(crate) runtime: Option<Handle>, pub(crate) string_redaction: bool, pub(crate) event_redaction: bool, pub(crate) backtrace: Option<Backtrace> }`.
  - `pub(crate) fn from_call(record: CallRecord, project_id: &str, hook_mode: bool) -> EventJob`: the TS in-memory tool-call event (B-077) in `tool_call.event` order: `id: ""`, `sessionId` (`""` without a session), `projectId`, `eventType: "mcp:tools/call"`, `timestamp` (`iso_millis(started_at)`), `duration`, `inputTokens`, `outputTokens`, `sdkLanguage: "Rust"`, `agentcatVersion` (crate version), server and client fields, `identifyActorData: {}`, `resourceName`, `parameters: {request, extra}`, `response` (without rmcp's `resultType` key), `userIntent`, `isError`, `error`, `tags`; absent keys for `None`. `string_redaction` and `event_redaction` are `true`.
  - `pub(crate) fn render_backtrace(event: &mut TsEvent, backtrace: &Backtrace)`: sets `error.stack` to the backtrace text, after `platform` and before `chained_errors`, when it was captured.
- Produces (`pipeline::settle`):
  - `pub(crate) const HOOK_BUDGET: Duration = Duration::from_secs(30);`
  - `pub(crate) struct HookResults { pub(crate) session_value: Option<String>, pub(crate) identity: Option<UserIdentity>, pub(crate) tags: Option<Map<String, Value>>, pub(crate) properties: Option<Map<String, Value>> }` (tags already through `validate_tags`).
  - `pub(crate) async fn settle(hooks: PendingHooks, budget: Duration, log: &mut Vec<String>) -> HookResults`: awaits the four handles concurrently, each under `tokio::time::timeout(budget)`, with C37's texts and aborts.
  - `#[derive(Debug)] pub(crate) struct SessionMintFailed;` (`Display`: `could not generate a session ID`).
  - `pub(crate) fn apply(event: &mut TsEvent, results: HookResults, hook_mode: bool) -> Result<(), SessionMintFailed>`: TS's `applyPendingEventFields`: in hook mode, `session_from_hook_value(value, projectId or none)` sets `sessionId` and the `agentcat_session_id_source` tag (`Err` when minting fails, before any later field is applied, as a TS throw at that point would); an identity sets `identifyActorGivenId`, `identifyActorName`, `identifyActorData` (`{}` without data); customer tags merge under the SDK tags (`{...customer, ...sdk}`); properties replace (an empty object is kept, B-095).
- Every test that waits on the budget passes a small `budget` (for example 50 ms).

- [ ] **Step 1: Write the failing tests.** `event/tests.rs`: `request_and_extra_follow_ts_key_order` (an rmcp call with `_meta` over HTTP; key orders equal `event_order.json`). `assemble/tests.rs`: `tool_call_event_follows_ts_key_order`, `event_timestamp_is_iso_millis` (`started_at` at a whole second gives `.000Z`), `response_has_no_result_type`, `sessionless_call_has_empty_session`, `backtrace_renders_after_platform` (`RUST_LIB_BACKTRACE` not needed: build a `Backtrace::force_capture()`). `settle/tests.rs`: `settled_hooks_apply_in_ts_order`, `hanging_hook_is_aborted_after_budget` (the hook's task is aborted, its drop guard observes it, and the log line is `Warning: identify hook did not settle within 30000ms; publishing event without it`; the text keeps TS's `30000` whatever the test budget is), `panicking_hook_logs_and_publishes_without_it`, `hook_errors_use_ts_texts` (each of the four texts), `invalid_tags_are_dropped_with_ts_logs`, `sdk_tags_win_collisions`, `hook_mode_without_value_mints`, and one test per `queue.json` `hooks:*` case that compares `apply`'s event with the case (the queue run itself is Task 5's).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p agentcat-sdk event:: pipeline::settle capture::`. Expected: compile errors.
- [ ] **Step 3: Implement the changes and the two modules.** Keep `call/run.rs` changes to filling the new `CallRecord` fields.
- [ ] **Step 4: Run the tests.** Same command, then `cargo nextest run -p agentcat-sdk`. Expected: PASS.
- [ ] **Step 5: Map the parity entries.** Owned files: the `deferred customer hooks` tests of `engine-callwrap.test.ts`; `invokeSessionHook` of `handles.test.ts`; `identify.test.ts`; `event-tags-properties.test.ts`; `resolveIdentity` and `buildSessionInfo` of `client-identity.test.ts`; `token-estimates.test.ts` (events); `v2/hooks.test.ts`; `v2/session-modes.test.ts`. Tests that need a published body and a harness scenario stay `todo` for Task 13. Run `cargo run -q -p xtask -- parity` (exit 0).
- [ ] **Step 6: Run `scripts/check.sh`, then commit.**

```bash
git add agentcat-sdk parity
git commit -m "feat(sdk): assemble the TS in-memory event and settle the call hooks"
```

---

### Task 5: The event pipeline and the wire request

Model: Opus 5.5.

**Files:**
- Create: `agentcat-sdk/src/pipeline/process.rs`, `agentcat-sdk/src/pipeline/process/tests.rs`, `agentcat-sdk/src/delivery.rs` (module root with `mod wire;` only, so later tasks add to it), `agentcat-sdk/src/delivery/wire.rs`, `agentcat-sdk/src/delivery/wire/tests.rs`
- Modify: `Cargo.toml`, `agentcat-sdk/Cargo.toml` (`agentcat-api`), `Cargo.lock`, `deny.toml`, `agentcat-sdk/src/js.rs`, `agentcat-sdk/src/js/tests.rs`, `agentcat-sdk/src/pipeline/mod.rs`, `agentcat-sdk/src/pipeline/redact.rs` (module docs: the RD-24 restore sentence per C39), `agentcat-sdk/src/lib.rs` (remove the `expect(dead_code)` on `pipeline`), `agentcat-sdk/src/test_support/mod.rs` (a `queue.json` loader), `parity/manifest.toml`

**Interfaces:**
- Consumes: `EventJob`, `settle`, `apply`, `render_backtrace` (Task 4); `Event::from_ts`/`into_ts`, `StringRedactor`, `EventRedactor` (Task 2); `redact_strings`, `apply_event_redaction`, `sanitize_event`, `truncate_event`, `new_event_id` (plan 2).
- Dependencies: `agentcat-api = { version = "=0.0.3", default-features = false }` (no TLS feature yet; Task 8 adds the features).
- Produces (`js`): `pub(crate) fn to_js_value(value: &Value) -> Value`: `stringify(value)` read back with `serde_json::from_str` (the input clone if that fails, which cannot happen for `stringify` output).
- Produces (`pipeline::process`):
  - `pub(crate) struct Redactors { pub(crate) string: Option<StringRedactor>, pub(crate) event: Option<EventRedactor> }`.
  - `pub(crate) async fn process(job: EventJob, redactors: &Redactors, hook_budget: Duration, log: &mut Vec<String>) -> Option<TsEvent>`: B-087 in order: render the backtrace; settle and apply the hooks (an `apply` error logs `Failed to resolve pending hook results: could not generate a session ID` and the event continues, as TS's `catch` around `applyPendingEventFields` does); when `job.event_redaction` and a hook is set, the event-level hook on `job.runtime` (C38) and `apply_event_redaction` (`Event dropped by redactEvent hook`, `Failed to redact event (event-level hook): ${error}`); when `job.string_redaction` and a hook is set, the two-pass string redaction through `redact_strings` (collect, await the hook per string in one task on `job.runtime`, stop at the first error, replay; `Failed to redact event: ${error}`), merged with `assign`; `sanitize_event` merged with `assign` (`Failed to sanitize event: ...`); `truncate_event` merged with `assign` (`Failed to truncate event: ${error}`); then `id` = `new_event_id()` when `id` is empty (`None` drops the event with `Failed to deliver AgentCat event after retries: could not generate an event ID`). Returns the final event, or `None` when dropped.
  - `pub(crate) fn assign(event: &mut TsEvent, stage: TsEvent)`: JavaScript's `Object.assign(event, stage)`: each key of `stage` replaces the value in place or is appended; no key of `event` is removed.
- Produces (`delivery::wire`): `pub(crate) fn publish_request(event: &TsEvent) -> PublishEventRequest` (C40): `id`, `project_id`, `session_id` (`Some(None)` for `""`), `timestamp` (the ISO string parsed to `DateTime<FixedOffset>`, milliseconds kept; chrono drops a zero fraction, RD-04), `duration` (saturated to `i32`), tokens, `event_type` (`mcp:tools/call` and `agentcat:custom` map to their variants; any other text gives `None`), `resource_name`, `parameters`, `response`, `user_intent` (a string as is; another value as `js::stringify`, RD-33), `is_error`, `error`, `identify_actor_given_id`, `identify_actor_name`, `identify_data` (from `identifyActorData`), `sdk_language`, `agentcat_version`, server and client fields, `actor_id` (= `identify_actor_given_id`), `tags` (string values), `properties`; every JSON value through `js::to_js_value`.
- Test seam: `#[cfg(test)]` `queue.json` runner in `process/tests.rs` that builds an `EventJob` from each case (the case's `event` as the `TsEvent`; hook outcomes as already-finished tasks on a test runtime: `returns` resolves, `throws` returns `Err`, `hangs` never resolves under a 50 ms budget), runs `process`, and maps the `PublishEventRequest` to TS's camelCase model with a test helper `to_ts_model(&PublishEventRequest) -> Value` (timestamps compared as milliseconds, `id` as `<evt>`, tags as a map). Retry cases (`retry:*`, `delays`) wait for Task 6. Log lines compare after one documented normalization: TS's `Error: <message>` from a thrown `Error` matches Rust's `<message>`.

- [ ] **Step 1: Write the failing tests.** `process/tests.rs`: `queue_vectors_match_ts` (every non-`retry` case: requests and logs), `assign_keeps_keys_the_stage_dropped` (carry-forward: a stage output without `error` leaves `error` in place), `redaction_without_runtime_drops_event` (RD-36), `string_redactor_panic_drops_event`, `string_redactor_runs_on_customer_runtime` (the hook records `Handle::current().id()`, equal to the job's runtime), `event_hook_cannot_change_restored_fields`, `custom_events_skip_string_redaction` (K-03). `wire/tests.rs`: `wire_values_follow_js_text` (`1.0`, `-0.0`, 9007199254740993, keys `"10"` and `"2"` in `parameters`, `response`, `properties`: the serde_json text of the request equals the `js::stringify` text of each value; RD-32's float case `1e21` is the one listed exception), `sessionless_event_sends_null_session`, `actor_id_follows_identity`, `non_string_user_intent_is_text` (RD-33), `timestamp_keeps_milliseconds`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p agentcat-sdk pipeline::process delivery::wire js::`. Expected: compile errors.
- [ ] **Step 3: Implement.** Add the new licenses `agentcat-api` brings to `deny.toml` only as `cargo deny check` asks.
- [ ] **Step 4: Run the tests.** Same command, then `cargo nextest run -p agentcat-sdk`. Expected: PASS.
- [ ] **Step 5: Map the parity entries.** Owned files: the integration groups of `redaction.test.ts`; `e2e-truncation.test.ts`; `e2e-sanitization.test.ts`; `v2/pipeline.test.ts` (`redactSensitiveInformation on captured events`); the integration tests of `error-capture.test.ts` (RD-06 where V8-specific). Run `cargo run -q -p xtask -- parity` (exit 0).
- [ ] **Step 6: Run `scripts/check.sh`, then commit.**

```bash
git add Cargo.toml Cargo.lock deny.toml agentcat-sdk parity
git commit -m "feat(sdk): the TS event pipeline and the PublishEventRequest it sends"
```

---

### Task 6: Delivery: queue, worker thread, retries, flush, shutdown, last-drop drain

Model: Opus 5.5.

**Files:**
- Create: `agentcat-sdk/src/delivery/queue.rs`, `agentcat-sdk/src/delivery/worker.rs`, `agentcat-sdk/src/delivery/tests.rs`
- Modify: `agentcat-sdk/Cargo.toml` (tokio `sync`), `Cargo.lock`, `agentcat-sdk/src/delivery.rs`, `agentcat-sdk/src/sink.rs`, `agentcat-sdk/src/client.rs`, `agentcat-sdk/src/client/tests.rs`, `agentcat-sdk/src/logging.rs` (drop its `expect(dead_code)`), `agentcat-sdk/src/test_support/rmcp.rs` (`RecordingSink` implements the new `Sink` methods), `agentcat-sdk/src/pipeline/process/tests.rs` (the `retry:*` cases), `parity/manifest.toml`

**Interfaces:**
- Consumes: `process`, `Redactors`, `publish_request` (Task 5); `from_call` (Task 4); `logging::write` (Task 3).
- Changes `Sink`:

```rust
pub(crate) trait Sink: Send + Sync + fmt::Debug {
    fn log(&self, message: String);
    fn record(&self, record: CallRecord);
    fn event(&self, job: EventJob);
    fn flush(&self, timeout: Duration) -> BoxFuture<'static, bool> { Box::pin(std::future::ready(true)) }
    fn shutdown(&self, timeout: Duration) -> BoxFuture<'static, bool> { Box::pin(std::future::ready(true)) }
    fn flush_blocking(&self, _timeout: Duration) -> bool { true }
    fn shutdown_blocking(&self, _timeout: Duration) -> bool { true }
}
```

- Produces (`sink`): `pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;` (no `futures` dependency).
- Produces (`delivery`):
  - `pub(crate) trait Publisher: Send + Sync + fmt::Debug + 'static { fn publish(&self, request: PublishEventRequest) -> BoxFuture<'static, Result<(), String>>; }` (one attempt; `Err` holds the text for the log lines). `pub(crate) trait Observer: Send + Sync + fmt::Debug + 'static { fn finished(&self, event: &TsEvent); }` (Task 10's `Fake`).
  - `pub(crate) struct DeliveryConfig { pub(crate) max_pending: usize, pub(crate) concurrency: usize, pub(crate) retry_delays: [Duration; 3], pub(crate) hook_budget: Duration, pub(crate) drop_drain: Duration }`, `Default` = 10 000, 5, 1/2/4 s, 30 s, 5 s.
  - `pub(crate) struct Delivery`, `pub(crate) fn new(project_id: String, options: &Options, publisher: Arc<dyn Publisher>, observer: Option<Arc<dyn Observer>>, config: DeliveryConfig) -> Self`; implements `Sink`: `record` and `event` push a job (`Job::Call(CallRecord)` assembled on the worker with `from_call`, or `Job::Event(EventJob)`) and never block (a `Mutex<VecDeque<Job>>` held only to push, plus a `tokio::sync::Notify`); a push onto `max_pending` pending jobs drops the oldest pending one with `Event queue full, dropping oldest event`; after `shutdown` a push logs `Queue is shutting down, event dropped`. `log` queues the line for the worker (lines are not jobs and do not count toward `max_pending`). The first push or log line starts the thread `agentcat-delivery` (`std::thread::Builder`); a spawn failure leaves the handle without delivery (nothing written, no panic).
  - The worker: a current-thread runtime with the time driver; runs at most `concurrency` jobs at once, each holding its slot through `process` and every attempt; writes queued log lines and each job's `log` lines through `logging::write`; calls the observer with each finished event; skips the POST when `projectId` is `""` (B-104); otherwise attempts the publish up to 4 times: after each failure `Failed to send event ${id}, retrying... [Error: ${text}]`, then the next delay of `retry_delays`, and after the fourth `Failed to deliver AgentCat event after retries: ${text}`; on success `Successfully sent event ${id} | ${eventType} | session ${sessionId} | ${projectId} | ${duration} ms | ${identifyActorGivenId or "anonymous"}`.
  - Lifecycle per C36; `Drop for Delivery` runs the drain.
- Produces (`client`): `pub async fn flush(&self, timeout: Duration) -> bool`, `pub async fn shutdown(&self, timeout: Duration) -> bool`, `pub fn flush_blocking(&self, timeout: Duration) -> bool`, `pub fn shutdown_blocking(&self, timeout: Duration) -> bool` on `AgentCat` (rustdoc: the blocking twins block the calling thread; `std::process::exit` and `panic = "abort"` skip the drain); `Inner` keeps `DiscardSink` in `AgentCat::new` until Task 8; `#[cfg(test)] pub(crate) fn with_delivery(project_id: &str, options: Options, publisher: Arc<dyn Publisher>, config: DeliveryConfig) -> Self`.

- [ ] **Step 1: Write the failing tests** in `delivery/tests.rs` with a recording publisher (scripted failures, an optional per-attempt delay) and shrunk `DeliveryConfig` values: `call_record_is_published`, `telemetry_only_is_not_posted_but_observed`, `retries_three_times_then_logs_failure` (four attempts, the scripted delays, TS's lines), `success_line_matches_ts`, `overflow_drops_oldest_pending` (`max_pending` 3, a publisher that holds its slot; the oldest pending, not in-flight, event goes, with the log line), `enqueue_never_blocks` (10 000 pushes under 1 s while every slot is held), `at_most_five_in_flight`, `flush_waits_for_in_flight`, `flush_returns_false_at_deadline`, `shutdown_rejects_later_events` (C12 text), `shutdown_logs_events_left` (`Shutting down with 2 events still in queue`), `last_drop_drains_queue`, `last_drop_on_current_thread_runtime_returns_within_budget` (design §13: a current-thread customer runtime, an identify hook that needs that runtime, the last handle dropped inside `block_on`; `drop_drain` 300 ms; the drop returns in under 1 s), `drop_on_worker_thread_does_not_wait`, `blocking_twins_work_outside_a_runtime`, `log_lines_are_written_by_the_worker` (the file logger of a temp home receives a call-path warning). `process/tests.rs`: `queue_retry_vectors_match_ts` (the `retry:*` cases' requests, delays and logs through the worker).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p agentcat-sdk delivery:: pipeline::process`. Expected: compile errors.
- [ ] **Step 3: Implement `queue.rs`, `worker.rs`, the `Delivery` sink and the lifecycle methods.**
- [ ] **Step 4: Run the tests.** Same command, then `cargo nextest run -p agentcat-sdk`. Expected: PASS, with no test over 3 s.
- [ ] **Step 5: Map the parity entries.** Owned files: `eventQueue.test.ts`; `signal-shutdown.test.ts` (`ruled-divergence:RD-09`); `workers/event-delivery.test.ts` (`ruled-divergence:RD-19`). Run `cargo run -q -p xtask -- parity` (exit 0).
- [ ] **Step 6: Run `scripts/check.sh`, then commit.**

```bash
git add Cargo.lock agentcat-sdk parity
git commit -m "feat(sdk): delivery thread with TS queue, retries, flush, shutdown and last-drop drain"
```

---

### Task 7: Custom events

Model: Sonnet 5.5.

**Files:**
- Modify: `agentcat-sdk/src/custom.rs`, `agentcat-sdk/src/custom/tests.rs`, `agentcat-sdk/src/client.rs`, `agentcat-sdk/src/context.rs`, `agentcat-sdk/src/tracked.rs`, `agentcat-sdk/src/call/run.rs` (the `CallContext` gets the call's identity), `parity/manifest.toml`

**Interfaces:**
- Consumes: `CustomEvent`, `Error` (Task 2); `EventJob`, `Sink::event` (Tasks 4, 6); `validate_tags`; `event_order.json` `custom_string.event` and `custom_server.event`.
- Produces:
  - `pub fn publish_custom_event(&self, session_id: &str, project_id: &str, event: CustomEvent) -> Result<(), Error>` on `AgentCat` (B-009 string form, C11): `ProjectIdRequired` for an empty `project_id`; session = the event's non-empty `session_id`, else `session_id` verbatim; the event in `custom_string.event` order with `projectId` = the argument, `eventType: "agentcat:custom"`, `timestamp` now, `resourceName`, `parameters`, `response`, `userIntent` (= `message`), `duration` (ms), `isError`, `error`, then `tags` (= `validate_tags` result, the key present with `null` when none survive) and `properties` (only with at least one key); no server, client or SDK fields; `string_redaction` and `event_redaction` `false`; `runtime` `None`. Logs `Published custom event for session ${s} with type 'agentcat:custom'` or `Published custom event without a session with type 'agentcat:custom'` after queuing.
  - `pub fn publish_custom_event(&self, project_id: &str, event: CustomEvent) -> Result<(), Error>` on `CallContext` and on `Tracked<H>` (server form, C16, C47, K-03): `ProjectIdRequired` for an empty `project_id` (then ignored); session = the event's non-empty `session_id`, else `""` with the log line `publishCustomEvent: no sessionId provided; event will be published without a session. Pass eventData.sessionId to attribute it to a task.`; with tracing off, nothing is queued (the `Published custom event` line is still written, as TS); otherwise the event in `custom_server.event` order with the handle's project ID, `sdkLanguage`, `agentcatVersion`, server and client fields (the call's on `CallContext`; on `Tracked<H>`, `initialize`'s client and `inner.get_info()`, a panic there contained per C30), `identifyActorData: {}`, `duration` = the given one, else the milliseconds since the event's timestamp; `event_redaction` `true`, `string_redaction` `false`, `runtime` = `Handle::try_current()`.
  - `CallContext` gains crate fields for the call's client identity and server info, filled in `run.rs` (the untraced paths give none).

- [ ] **Step 1: Write the failing tests** in `custom/tests.rs` (with `AgentCat::with_delivery` and a recording publisher): `string_form_requires_project`, `string_form_honors_project_argument` (K-03 contrast), `string_form_session_precedence`, `string_form_has_no_server_fields`, `server_form_uses_handle_project` (K-03), `server_form_dropped_with_tracing_off`, `server_form_without_session_logs_ts_line`, `server_form_applies_redact_event_only` (K-03), `call_context_form_uses_call_identity`, `tracked_form_uses_initialize_identity`, and `custom_vectors_match_ts` (the `custom:*` cases of `queue.json` through the builders, `process` and `publish_request`).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p agentcat-sdk custom::`. Expected: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests.** Same command. Expected: PASS.
- [ ] **Step 5: Map the parity entries.** Owned files: `publishCustomEvent.test.ts`; `v2/pipeline.test.ts` (`publishCustomEvent`). Tests of the TS-only argument shapes (`Server is not tracked`, `First parameter must be ...`) are `n/a`, `ts-host-only` (Rust types rule them out). Run `cargo run -q -p xtask -- parity` (exit 0).
- [ ] **Step 6: Run `scripts/check.sh`, then commit.**

```bash
git add agentcat-sdk parity
git commit -m "feat(sdk): custom events in the session-string and server forms"
```

---

### Task 8: HTTP delivery, TLS features, and the Rust reference server publishes

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/delivery/http.rs`, `agentcat-sdk/src/delivery/http/tests.rs`
- Modify: `Cargo.toml`, `agentcat-sdk/Cargo.toml` (features; `reqwest`; dev `wiremock`), `Cargo.lock`, `deny.toml`, `agentcat-sdk/src/delivery.rs`, `agentcat-sdk/src/client.rs`, `agentcat-sdk/src/client/tests.rs`, `scripts/check.sh`, `.github/workflows/ci.yml`, `harness/rust-server/src/main.rs`, `harness/rust-server/src/profile.rs`, `harness/rust-server/src/tools.rs`, `harness/src/matrix.mjs`, `harness/README.md`, `parity/manifest.toml`

**Interfaces:**
- Consumes: `Publisher`, `Delivery` (Task 6); `resolve_api_base_url`, `sdk_enabled`, `ProcessEnv` (Task 2); `logging::configure` (Task 3); custom events (Task 7).
- Dependencies and features: `reqwest = { version = "0.13", default-features = false }`; `[features] default = ["rustls"]`, `rustls = ["agentcat-api/rustls"]`, `native-tls = ["agentcat-api/native-tls"]`, `test-util = []` (filled by Task 10). Dev: `wiremock = "0.6"`.
- Produces (`delivery::http`): `pub(crate) struct ApiPublisher` with `pub(crate) fn new(base_url: String, log: &mut Vec<String>) -> Self` (C44) and `impl Publisher` (one `publish_event` call; any `Err` is a failed attempt whose text is the error's `Display`; a 2xx needs a JSON body under `application/json`, RD-37).
- Changes `AgentCat::new(project_id, options)`: builds `Delivery` with `ApiPublisher::new(resolve_api_base_url(&options, &ProcessEnv), …)` and `DeliveryConfig::default()`; calls `logging::configure(options.log_to(), ProcessEnv.home_dir())`; when the project ID is empty, queues TS's line `Warning: No projectId provided and no exporters configured. Events will not be sent anywhere.` (B-003). Building still cannot fail and starts no thread (the first event or log line starts it). `AGENTCAT_ENABLED` is Task 9's.
- Changes the Rust reference server (carry-forward): `AGENTCAT_API_URL` goes in as `Options::api_base_url` (remove the `expect(dead_code)` on `Startup.api_url`); the profile's `redactSensitiveInformation` goes in as `Options::redact_sensitive_information` (`replace` replaces each occurrence with `[REDACTED]`, `throws` fails, `returns` returns the value; `sleepMs` waits first); `emit_custom` calls `publish_custom_event(session, project_id, CustomEvent::new().resource_name(resource).message(message).parameters({"from": "emit_custom"}).tags({"kind": "custom"}))`, as `common.mjs` does; on stdin EOF (stdio) and on SIGTERM (every mode) it runs `agentcat.shutdown(Duration::from_secs(5)).await`, prints `{"drained":<bool>}` on stderr, then exits (an untracked server prints nothing).
- Changes `harness/src/matrix.mjs`: tracked Rust runs no longer get `--expect-publishes 0`; the usage text and `harness/README.md` say so.
- Changes `scripts/check.sh` and the CI deny job: `cargo deny check --all-features`; `deny.toml` loses every license that is still unused after `reqwest`, `rustls` and `native-tls` join (carry-forward) and gains only those the check asks for.

- [ ] **Step 1: Write the failing tests.** `http/tests.rs` (`wiremock` on loopback, through `AgentCat::with_delivery` with an `ApiPublisher` and shrunk delays): `posts_to_public_events_path` (POST `/v1/public/events/`, `content-type: application/json`, User-Agent `agentcat-rust/1.0.0`, no `authorization` header, a body without camelCase keys, RD-01), `base_url_option_wins_over_env`, `refused_connection_is_retried`, `server_error_is_retried`, `hung_attempt_times_out` (a 10 s mock delay against a test attempt timeout of 200 ms), `text_body_is_a_failed_attempt` (RD-37), `accepted_json_body_is_success`. `client/tests.rs`: `empty_project_logs_ts_warning`, `new_starts_no_thread` (no thread named `agentcat-delivery` until the first event).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p agentcat-sdk delivery::http client::`. Expected: compile errors.
- [ ] **Step 3: Implement `ApiPublisher`, the features and the new `AgentCat::new`.** Give `ApiPublisher::new` a crate-private attempt-timeout parameter for the test (default 10 s).
- [ ] **Step 4: Run the tests.** Same command, then `cargo hack check -p agentcat-sdk --each-feature --no-dev-deps`. Expected: PASS.
- [ ] **Step 5: Update the reference server and the matrix, then run the harness.** Run: `cargo build -p harness-ref-server && node harness/src/matrix.mjs`. Expected: exit 0: every tracked Rust run publishes exactly the scenario's `expectPublishes` (TS's count) and every body passes the L4 contract. A count that differs is a bug to fix in this task.
- [ ] **Step 6: Map the parity entries.** Owned file: `api-base-url.test.ts` (the process-global last-wins cases `ruled-divergence:RD-08`). Run `cargo run -q -p xtask -- parity` (exit 0).
- [ ] **Step 7: Run `scripts/check.sh`, then commit.**

```bash
git add Cargo.toml Cargo.lock deny.toml agentcat-sdk scripts .github harness parity
git commit -m "feat(sdk): publish through agentcat-api with TLS features; the Rust reference server publishes and drains"
```

---

### Task 9: Call path: tracing-off strip, lazy snapshot, cancelled calls, `AGENTCAT_ENABLED`, `from_env`

Model: Opus 5.5.

**Files:**
- Modify: `agentcat-sdk/src/call/run.rs`, `agentcat-sdk/src/call/run/tests.rs`, `agentcat-sdk/src/call/tests.rs`, `agentcat-sdk/src/client.rs`, `agentcat-sdk/src/client/tests.rs`, `agentcat-sdk/src/tracked.rs`, `agentcat-sdk/src/tracked/tests.rs`, `agentcat-sdk/src/hooks.rs`, `agentcat-sdk/src/hooks/tests.rs`, `harness/rulings.json` (RD-29 reasons), `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (RD-29 row: present tense; RD-06 row: the cancelled type, C41), `parity/manifest.toml`

**Interfaces:**
- Consumes: `learn_schema`, `strip_arguments`, `CallContext` (plan 3); `sdk_enabled`, `ProcessEnv`, `MapEnv`, `AGENTCAT_PROJECT_ID` (Task 2); `Delivery` (Task 6).
- Changes (R27, RD-29): with `enable_tracing(false)`, a call other than an intercepted `get_more_tools` learns the tool's schema as the traced path does (`get_tool`, registry, rebuild, heuristic; C03) and strips the injected `session_id`, `agent_id` and `context` before the handler runs, so a tool's own `context` stays; nothing is recorded. The degraded path (minting failed) keeps the raw arguments (TS, K-05).
- Changes (carry-forward): `start_hooks` builds the `ToolCall` snapshot only when at least one hook will start (in hook mode the session hook counts).
- Changes (C41, RD-34): the record lives in a guard from its creation until `finish` sends it; dropping the call future in between sends the record with no response, `is_error: true`, the cancelled `error` and the duration so far.
- Produces: `pub fn from_env(options: Options) -> Self` on `AgentCat` (D36: the project ID from `AGENTCAT_PROJECT_ID`, `""` when unset or empty); `AgentCat::new` and `from_env` read `AGENTCAT_ENABLED` through `sdk_enabled` (C45); `Inner` gains `pub(crate) enabled: bool`; `#[cfg(test)] pub(crate) fn with_env(project_id: &str, options: Options, env: &dyn Env) -> Self`.
- Changes `Tracked<H>`: with `enabled` false, `list_tools`, `get_tool`, `initialize` and `call_tool` forward untouched (`call_tool` inserts a `CallContext` without session or agent, C29), the free `track` included.

- [ ] **Step 1: Write the failing tests.** `run/tests.rs`: `tracing_off_strips_injected_arguments` (a `deny_unknown_fields` tool after a listing receives no `context`), `tracing_off_keeps_own_context` (`own_context` declares `context`; before any listing too, through `get_tool`), `tracing_off_records_nothing`, `degraded_path_keeps_raw_arguments`, `snapshot_built_only_with_hooks` (a counter in a test-only hook constructor, or a `ToolCall` construction counter under `#[cfg(test)]`), `cancelled_call_publishes_record` (the tracked call under `tokio::time::timeout` shorter than a slow tool; the recording sink gets `type: "cancelled"`), `completed_call_is_recorded_once`. `client/tests.rs`: `from_env_reads_project_id`, `from_env_without_project_is_telemetry_only`, `disabled_handle_starts_no_thread`. `tracked/tests.rs`: `disabled_handle_forwards_untouched` (listing without `get_more_tools` and injections, raw arguments to the handler, `Extension<CallContext>` present, nothing recorded).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p agentcat-sdk call:: client:: tracked:: hooks::`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests and the harness.** Run: `cargo nextest run -p agentcat-sdk && cargo build -p harness-ref-server && node harness/src/matrix.mjs`. Expected: PASS; the RD-29 rulings still match on their transports and lanes. Update their `reason` texts to say Rust strips with the known schema (R27).
- [ ] **Step 5: Map the parity entries.** Owned files: `tracing-initialization.test.ts`; the remaining `todo` keys of `context-parameters.test.ts`, `custom-context-description.test.ts`, `structured-mintback.test.ts`, `handle-injection.test.ts` (`registry storage + request cloning`) and `report-missing.test.ts` that a Rust test of this task or an earlier one pins. Run `cargo run -q -p xtask -- parity` (exit 0).
- [ ] **Step 6: Run `scripts/check.sh`, then commit.**

```bash
git add agentcat-sdk harness/rulings.json docs parity
git commit -m "feat(sdk): strip with tracing off, cancelled calls, AGENTCAT_ENABLED and from_env"
```

---

### Task 10: `testing::Fake`

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/testing.rs`, `agentcat-sdk/tests/fake.rs`
- Modify: `agentcat-sdk/Cargo.toml` (`test-util`; a `[[test]] required-features = ["test-util"]` entry for `fake`), `agentcat-sdk/src/lib.rs`, `agentcat-sdk/src/client.rs` (a crate constructor with a publisher and an observer), `scripts/check.sh` and `.github/workflows/ci.yml` only if a test command needs the feature named

**Interfaces:**
- Consumes: `Observer`, `Publisher`, `Delivery`, `DeliveryConfig` (Task 6); `Event::from_ts` (Task 2).
- Produces (`#[cfg(feature = "test-util")] pub mod testing`, `#[cfg_attr(docsrs, doc(cfg(feature = "test-util")))]`): `pub struct Fake` (`Clone`, `Debug`, `Default`; instance-scoped, D41) with `pub fn new() -> Self`; `pub fn agentcat(&self, project_id: impl Into<String>, options: Options) -> AgentCat` (a handle whose publisher never sends and whose observer records into this `Fake`; C43); `pub async fn events(&self) -> Vec<Event>` (flushes every handle it built for up to 5 s, then the recorded events in the order they finished); `pub async fn assert_tracked(&self, tool_name: &str)`; `pub async fn assert_tracked_count(&self, count: usize)` (tool-call events); `pub async fn assert_nothing_tracked(&self)`; `pub async fn assert_custom_event_published(&self, resource_name: &str)`. Each assertion panics with a message that lists the recorded events' types and resource names.

- [ ] **Step 1: Write the failing tests** in `agentcat-sdk/tests/fake.rs` (`#[tokio::test]`, an rmcp in-memory client): `fake_records_tool_calls`, `fake_records_telemetry_only_events`, `fake_is_instance_scoped` (two fakes in one test see only their own events), `assert_tracked_count_panics_with_listing` (`#[should_panic(expected = "...")]`), `fake_records_custom_events`, `fake_runs_hooks_and_redaction` (an `identify` hook and a `redact_event` hook both show in the recorded event).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p agentcat-sdk --features test-util --test fake`. Expected: compile errors.
- [ ] **Step 3: Implement `testing`.**
- [ ] **Step 4: Run the tests.** Same command, then `cargo hack check -p agentcat-sdk --each-feature --no-dev-deps`. Expected: PASS.
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add agentcat-sdk scripts .github
git commit -m "feat(sdk): testing::Fake behind the test-util feature"
```

---

### Task 11: Pipeline performance and the feature-unification self-check

Model: Sonnet 5.5.

**Files:**
- Modify: `agentcat-sdk/src/pipeline/truncate.rs`, `agentcat-sdk/src/pipeline/truncate/tests.rs`, `agentcat-sdk/src/pipeline/normalize.rs`, `agentcat-sdk/src/pipeline/process.rs`, `agentcat-sdk/src/js.rs`, `agentcat-sdk/src/js/tests.rs`, `agentcat-sdk/src/delivery/worker.rs` (the self-check line), `agentcat-sdk/src/delivery/tests.rs`

**Interfaces:**
- Changes (carry-forward, behavior unchanged; the plan 2 vectors and proptests are the proof): `truncate_event(event: TsEvent) -> Result<TsEvent, TruncateError>` takes the event by value (callers move it); `limit_fields` works in place; the size guard counts bytes with a counting `fmt::Write` sink fed by a `js::write_stringify(value: &Value, out: &mut impl fmt::Write) -> fmt::Result` (which `js::stringify` also uses), so no text is built to be measured; `js::own_entries` gets a non-allocating path for objects without index-like keys (the common case), keeping its order contract.
- Produces: the worker's first start logs one line per serde_json feature that changes the SDK's JSON when a customer build unifies it on: `Warning: serde_json feature "arbitrary_precision" is enabled in this build; AgentCat token estimates and size checks may differ from the TypeScript SDK` (and the same with `"float_roundtrip"`), detected at run time by parsing a probe value (`arbitrary_precision`: a `Number` parsed from `1e400` succeeds; `float_roundtrip`: `"0.30000000000000004"` and its neighbors parse to the exact `f64`) through crate-private `fn json_features() -> Vec<&'static str>`.

- [ ] **Step 1: Write the failing tests.** `js/tests.rs`: `write_stringify_equals_stringify` (the `js.json` stringify cases through both), `own_entries_fast_path_keeps_order`. `delivery/tests.rs`: `json_features_empty_in_this_build` (the workspace turns neither feature on). `truncate/tests.rs`: `truncate_event_by_value_matches_vectors` (the existing vector test, moved to the new signature).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p agentcat-sdk js:: pipeline:: delivery::`. Expected: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every pipeline vector and proptest.** Run: `cargo nextest run -p agentcat-sdk`, then `PROPTEST_CASES=2000 cargo nextest run -p agentcat-sdk pipeline:: js::`. Expected: PASS.
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add agentcat-sdk
git commit -m "perf(sdk): truncate by value, count bytes without building text; log unified serde_json features"
```

---

### Task 12: Harness: event comparison, normalizer rules, drain protocol

Model: Opus 5.5.

**Files:**
- Create: `harness/src/events.mjs` (the structural diff, shared by `diff.mjs` and `matrix.mjs`), `harness/rulings-events.json`, `harness/rulings-events-ts.json` (`[]`), tests under `harness/test/` for each change
- Modify: `harness/src/mock-api.mjs`, `harness/src/normalize.mjs`, `harness/src/diff.mjs`, `harness/src/matrix.mjs`, `harness/src/rulings.mjs`, `harness/src/driver.mjs`, `harness/src/l3-expect.mjs`, `harness/README.md`, `scripts/check.sh`, `.github/workflows/ci.yml` (job `harness-l3`)

**Interfaces:**
- `mock-api.mjs` keeps each request's raw body text as `rawBody` beside the parsed `body`.
- `normalize.mjs` rules (each listed in `RULES` and in the README rule table):
  - `extra-projection` (replaces `extra-headers-only`): `parameters.extra` becomes `{"<keys>": <sorted top-level keys>, "<headers>": <every header found in a "headers" object anywhere under extra, by lower-case name>, "<url>": <the request URL at requestInfo.url or http.req.url>}`, with the values of `host` and `content-length` as `<host>` and `<content-length>`, `mcp-session-id` as `<mcp-session-id>`, and the URL's port as `<port>`; an absent part is left out (stdio has none).
  - `number-source`: a publish body is read from `rawBody` with source-text access (`JSON.parse` reviver `context.source`, Node 22); an integer number below 1e21 in magnitude whose source text differs from `String(Number(source))` becomes `"<non-js-number:<source>>"` (it catches `1.0`, `-0`, `1e2`, an exact integer above 2^53); other numbers compare by value (RD-32).
  - `timestamp-class`: ISO 8601 with milliseconds or whole seconds and `Z` becomes `<iso8601-Z>` (RD-04); anything else stays `<bad-timestamp:…>`.
  - `id-placeholders`: a `ses_` ID whose KSUID timestamp is before 2024-03-01 is derived (B-065) and becomes `<ses-derived#n>`; other IDs keep `<ses#n>`, `<evt#n>`, `<agt#n>`; `literal-derived-ids` still keeps the canned hook value's ID.
  - Event order: sort on (raw `timestamp`, untimed after timed, `resource_name`, the normalized body with IDs masked), so the raw duration no longer decides ties; a scenario with `"eventOrder": "multiset"` sorts on the normalized masked body alone. The README's scenario-authoring rules gain the line: two events of one session whose relative order matters must not start in the same millisecond, unless the scenario compares events as a multiset.
- `driver.mjs`: `--expect-drained`: the driver reads the server's stderr (stdio: the client transport's `stderr: "pipe"`; HTTP: the child's stderr) and fails a run whose server does not print `{"drained":true}` before it exits. Any HTTP server that needs SIGKILL after SIGTERM fails the run (exit 1), with or without the flag.
- `matrix.mjs`: per combination it also compares the normalized `events` and `nonPublishRequests` of the two tracked runs through `events.mjs` with the event rulings (`--event-rulings`, default `harness/rulings-events-ts.json` when both servers are TS, else `harness/rulings-events.json`; scoped by `scenario`, `transports` and `lanes` as delta rulings, C35); passes `--expect-drained` to tracked Rust runs; prints `events <total>/<ruled>/<unexpected>` on each line and stores `events` and `eventDifferences` in `matrix.json`; exits 1 on an unruled event difference or, for the whole matrix, an unused event ruling. `rulings.json` keeps the delta rulings only (carry-forward: split file).
- `l3-expect.mjs`: `none` also needs `events.total` 0; `caught` also needs no event difference.
- `rulings-events.json`: one ruling per known difference the first full run shows, each with its RD: at least RD-07 (`$.events[*].body.parameters.extra.<keys>`) and RD-06 (`error.type`, `error.stack`, `error.frames`, `error.chained_errors` where Rust has none). Any other difference is a bug: fix it in the SDK in this task when the fix is local, else add a ruling with `"pending": true` and a new RD row marked pending in the design (R17).

- [ ] **Step 1: Write the failing tests** in `harness/test/`: `normalize` (each rule on a crafted run: header projection, `1.0` caught, `1e21` and `0.5` not, a whole-second timestamp, a derived and a minted `ses_`, the multiset order), `mock-api` (`rawBody` kept), `driver` (a fake stdio server that exits without the drained line fails with `--expect-drained`; a fake HTTP server that ignores SIGTERM fails), `matrix` (an event difference without a ruling exits 1; an event ruling scoped to another scenario is not counted unused).
- [ ] **Step 2: Run them to see them fail.** Run: `corepack pnpm --filter harness test`. Expected: FAIL.
- [ ] **Step 3: Implement the changes.**
- [ ] **Step 4: Run the full matrix and the A/A run.** Run: `cargo build -p harness-ref-server && node harness/src/matrix.mjs && node harness/src/matrix.mjs --servers rust,rust --scenarios basic && node harness/src/l3-expect.mjs none`. Expected: exit 0; every event difference ruled; A/A total 0 for delta and events.
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add harness scripts .github agentcat-sdk docs
git commit -m "test(harness): compare published events, new normalizer rules, drain protocol"
```

---

### Task 13: The scenario set and the call and event canaries

Model: Opus 5.5.

**Files:**
- Create: `harness/scenarios/redaction.json`, `harness/scenarios/redact-drop.json`, `harness/scenarios/hooks-fail.json`, `harness/scenarios/concurrency.json`, `harness/scenarios/pagination.json`, `harness/scenarios/custom-events.json`
- Modify: `harness/src/servers/common.mjs`, `harness/src/servers/ts-v1.mjs`, `harness/src/servers/ts-v2.mjs`, `harness/contract/tool-contract.json` (generated by `node harness/src/dump-contract.mjs`), `harness/rust-server/src/{main.rs,profile.rs,tools.rs}`, `harness/src/driver.mjs`, `harness/src/normalize.mjs` (only if `eventOrder` needs it), `harness/src/l3-expect.mjs`, `harness/rulings.json`, `harness/rulings-events.json`, `harness/README.md`, `scripts/check.sh`, `.github/workflows/ci.yml`, `parity/manifest.toml`

**Interfaces:**
- Options profile, both servers: `redactEvent` with `{"returns": null}` (drop), `{"throws": "<message>"}` or `{"merge": {<field>: <value>}}`, where a field is one of `resourceName`, `sessionId`, `eventType`, `userIntent`, `properties` (another key exits 2). TS returns `{...event, ...merge}`; Rust sets the matching `Event` fields. README "Options profile" documents it.
- Tool `emit_custom_server` (in `TOOLS` of `common.mjs`, C33; arguments `resource`, `message`, optional `session`): TS calls `publishCustomEvent(<the tracked server>, projectId, {resourceName, message, sessionId: session, parameters: {from: "emit_custom_server"}})`; Rust calls `CallContext::publish_custom_event` with the same fields. Untracked servers answer `published` without publishing, as `emit_custom` does.
- Pagination: a scenario top-level key `"pageSize": n` makes the driver start the server with `HARNESS_PAGE_SIZE=n`; both servers then page `tools/list` (cursor = the decimal start index, `nextCursor` while tools remain). TS: before `track()`, replace the `tools/list` entry of the low-level server's request-handler map with a wrapper that pages the full result, so AgentCat wraps the paging handler (test-server only). Rust: `list_tools` pages the contract's tools. The `listTools` step gains `"saveCursorAs": "<NAME>"` (the result's `nextCursor`) and `"cursor": "${NAME}"`.
- Parallel calls: a step `{"op": "parallel", "steps": [<callTool steps>]}` runs its calls at once (`Promise.all`) and records their transcript entries in step order.
- Canaries: `HARNESS_CANARY=call` makes the Rust server wrap its `Tracked` handler in an outer `ServerHandler` that changes `issued` to `lssued` in a result's first text block when it starts with `[session_id issued`; `HARNESS_CANARY=event` sets a `redact_event` hook that sets `resource_name` to `canary` (replacing a profile hook). `l3-expect.mjs` gains modes `caught-call` (every unruled difference under `$.delta.transcript[*].result.content[0].text`) and `caught-event` (every unruled difference under `$.events[*].body.resource_name`, no delta difference). `scripts/check.sh` and CI run both after the existing canary, each on `--servers rust,rust --scenarios basic`, expecting matrix exit 1 and then `l3-expect` exit 0.
- Scenarios (counts and orders come from TS runs; a scenario's `expectPublishes` may be per transport or lane):
  - `redaction`: `redactSensitiveInformation` replace and `redactEvent` merge (`resourceName` and `sessionId`): renamed resources, restored sessions, `emit_custom` untouched by both hooks, `emit_custom_server` renamed but not string-redacted (K-03).
  - `redact-drop`: `redactEvent` returns `null`: only the session-string custom events publish.
  - `hooks-fail`: `identify` throws, `eventTags` returns invalid entries (bad key, 33-character key, number value, newline in a value) and one valid tag, `eventProperties` throws, `identify` with `sleepMs` 200 in a second session.
  - `concurrency`: `"eventOrder": "multiset"`; one session with a `parallel` step of 8 `echo` calls carrying the same handle, plus a second session in parallel with the first's calls.
  - `pagination`: `pageSize` 3; page 1, a call to a page-2 tool before page 2 is listed (K-01 against RD-12), page 2, the call again, `own_context` on the later page.
  - `custom-events`: session-string and server forms, with and without a session, invalid custom tags.
  - Panics have no scenario: TS has no panic (RD-06); Rust tests (Tasks 6 and 9) cover their events. README says so.
- Every new difference gets a ruling with its RD (RD-12 for pagination, RD-07, RD-06), or is a bug to fix, or a pending RD (R17).

- [ ] **Step 1: Write the failing tests** in `harness/test/`: the driver's `parallel` and `saveCursorAs` steps against a fake server, profile validation of `redactEvent` in both servers (`node --test` for TS; `profile.rs` unit tests for Rust: `redact_event_profile_parses`, `unknown_merge_field_is_refused`), the pagination of both servers (a direct `tools/list` with `cursor`), `l3-expect` `caught-call` and `caught-event` on crafted `matrix.json` files.
- [ ] **Step 2: Run them to see them fail.** Run: `corepack pnpm --filter harness test && cargo nextest run -p harness-ref-server`. Expected: FAIL.
- [ ] **Step 3: Implement the servers, the driver steps, the canaries, then write the scenarios with counts from TS runs.** Regenerate the contract: `node harness/src/dump-contract.mjs`.
- [ ] **Step 4: Run the gate parts.** Run: `cargo build -p harness-ref-server && node harness/src/matrix.mjs`, then the A/A run and the three canaries as `scripts/check.sh` runs them. Expected: matrix exit 0; A/A `none` ok; each canary exits 1 and its `l3-expect` mode exits 0.
- [ ] **Step 5: Map the parity entries.** Owned files: `e2e-http/v1-matrix.test.ts`, `e2e-http/v2-matrix.test.ts`, `v2/era-2026.test.ts`, `v2/get-more-tools.test.ts`, `v2/agent-tracking.test.ts`, `v2/lowlevel-handles.test.ts`, the `published events` groups of `handles-integration.test.ts`, and every key left `todo` by Task 4 for a harness scenario (`harness:<scenario>`); `posthog-*.test.ts` are `second-pass` (RD-16). Run `cargo run -q -p xtask -- parity` (exit 0).
- [ ] **Step 6: Run `scripts/check.sh`, then commit.**

```bash
git add harness scripts .github parity docs
git commit -m "test(harness): redaction, hook failures, concurrency, pagination and custom-event scenarios; call and event canaries"
```

---

### Task 14: Milestone close

Model: Opus 5.5 (the controller).

**Files:**
- Modify: `docs/DECISIONS.md` (C36 to C47; the R28 owner batch once answered), `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (section 9: RD-32 to RD-37, the pending rows of Tasks 12 and 13; RD-24 per C39), `docs/superpowers/plans/carry-forward.md` (close the plan 4 section; the plan 4 final review's deferred items go to plan 5), `docs/superpowers/plans/README.md` (plan 4 row), `harness/README.md` (scenario table)

- [ ] **Step 1: Run the final gate.** Run: `scripts/check.sh`. Expected: exit 0, with the full matrix, A/A, three canaries, parity and every test green. Record the parity counts (`ported`, `n/a`, `todo`).
- [ ] **Step 2: Record the decisions and deviation rows** listed in this plan, with the wording the code ended up with.
- [ ] **Step 3: Close the carry-forward section** (each item: the commit that closed it) and move the final review's deferred items to plan 5.
- [ ] **Step 4: Bring the pending rulings to the owner** as one batch of multiple-choice questions (R17), recommended option first; record the answers as R28 and queue their changes for plan 5.
- [ ] **Step 5: Update the plan index** (plan 4 done: local gate commit, CI status: owed while GitHub Actions billing blocks runs), commit, and push to `origin rewrite`.

```bash
git add docs harness/README.md
git commit -m "docs: plan 4 decisions C36-C47, RD-32 to RD-37, carry-forward, plan index"
git push origin rewrite
```

---

## Carry-forward map

| Carry-forward item (plan 4 section) | Task |
|---|---|
| Mock API keeps raw body text | 12 (`rawBody`, `number-source`) |
| stdio close path and the 2 s SIGTERM | 8 (server drains on EOF and SIGTERM), 12 (drained line) |
| HTTP server that ignores SIGTERM | 12 (SIGKILL fails the run) |
| `TsEvent` key order from a TS probe | 1 (`event_order.json`), 4 |
| Async string redaction in two passes through `redact_strings` | 5 |
| `new_event_id()` `None` drops with the delivery-failure line | 5 |
| `cargo deny check --all-features` | 8 |
| Wire body with `js::stringify` rules, or an RD | 5 (C40, RD-32) |
| Size guard measures the sent text | 5 (C40: the guard measures the `TsEvent`, as TS) |
| `Object.assign` merge pinned by a test | 5 (`assign_keeps_keys_the_stage_dropped`) |
| Timestamp in `toISOString` form | 3 (`iso_millis`), 4 |
| Performance of truncation, `byte_size`, `own_entries` | 11 |
| RD-24 and the typed event | 2, 5 (C39) |
| `extra-headers-only` hides D06 and D34; `extra` and `_meta` order | 12 (`extra-projection`), 1 and 4 (orders) |
| RD-04 whole-second timestamps | 12 (`timestamp-class`) |
| Derived and minted placeholders, sort tie-breaker, same-millisecond line | 12 |
| `parameters.request` key order from a TS probe | 1, 4 |
| Drop `resultType` from the response | 4 |
| `CallContext` on the `AGENTCAT_ENABLED=false` pass-through | 9 |
| Hook settling reads `pending`, bounds or aborts, hook panics documented | 4 (C37) |
| Render the backtrace in the pipeline | 4 |
| Reference server: `emit_custom`, redaction option, API URL, HTTP drain | 8 |
| `matrix.mjs` stops `--expect-publishes 0` for Rust | 8 |
| Split `rulings.json` for event rulings | 12 |
| Feature-unification self-check | 11 |
| Unused `cargo deny` licenses | 8 |
| Build `ToolCall` only when a hook is set | 9 |
| Drop guard for a dropped call future | 9 (C41) |
| A call-path canary | 13 (`HARNESS_CANARY=call`, plus `event`) |
| R27: strip with tracing off | 9 |
| R27: `capture_error` `?Sized` | 2 |
