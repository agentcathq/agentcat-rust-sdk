# AgentCat Rust SDK: design

- Date: 2026-10-20
- Status: draft for owner approval
- Repository: `agentcathq/agentcat-rust-sdk` (private), branch `rewrite`, one PR into `main` at the end
- Crate: `agentcat-sdk` (reserved on crates.io), version `1.0.0`
- Decision log: [`../../DECISIONS.md`](../../DECISIONS.md) (R = owner, C = designer, D = PHP log)
- Research: [`../../research/`](../../research/README.md)

## 1. Intent

AgentCat tells MCP server maintainers what agents use their servers for and where the agents get stuck. A customer
adds an AgentCat SDK to an MCP server; the SDK records each tool call with the agent's stated intent, a session that
the agent carries from call to call, the actor, errors and token estimates, and sends these events to the AgentCat
API. AgentCat has SDKs for TypeScript, Python, Go, PHP and C#. This SDK covers MCP servers written in Rust with
`rmcp`, the official Rust MCP SDK.

The SDK is open source. Rust developers will read it before they adopt it, so it must look like the best Rust
telemetry SDKs (Sentry, OpenTelemetry, PostHog, Datadog): small, typed, never panicking, never blocking the host,
well documented.

A first attempt was scrapped because it had too little verification and its behavior drifted from the TypeScript
SDK. This design therefore puts verification first: the TypeScript SDK itself is the test oracle, and drift fails CI.

## 2. Goals, non-goals, anti-goals

Goals (first pass):

1. Behavior parity with the TypeScript SDK at `9b56e9e` for everything in scope: the agent-facing wire (tool lists,
   injected parameters, mint-back text, `mcp_session`), the published events, and the delivery policy. The contract
   is `research/01-ts-sdk-behavior.md` (B-001 to B-119).
2. One-line integration for every common rmcp server shape: stdio, streamable HTTP (stateful and stateless), macro
   servers (`#[tool_router]` + `#[tool_handler]`) and hand-written `ServerHandler`s.
3. Proof of parity a reviewer can check: TS-derived vectors, a differential harness against a real TS server, a
   generated TS test-by-test map, and a dashboard demo on the prod API.
4. Code and packaging that follow the Rust style guide in section 10.

Non-goals (first pass): the exporters (OTLP, Datadog, Sentry, PostHog) and remote SDK diagnostics (R04, C05); a
crates.io release and its workflow; rmcp 1.x or 2.x (R05); events for anything other than `tools/call` and custom
events (TS publishes only those two, B-017, B-078).

Anti-goals (a failure even if everything compiles):

- The SDK changes how the customer's server behaves, beyond the TS-defined injections. Examples: a panic turned into
  an error result, a slower tool call, a blocked runtime, a line on stdout.
- Agent-facing output differs from TS: other wording, other key order, `mcp_session` not first.
- A published event differs from what TS publishes for the same call, outside the deviation register (section 9).
- A test passes because the expected value was copied from the Rust output instead of generated from TS.
- `reqwest`, `tokio`, `chrono` or `agentcat-api` types leak into the public API.

## 3. Constraints

- rmcp 3.x, floor 3.0.0, tested up to the latest 3.x (3.6.0 today). Depend on
  `rmcp = { version = "3", default-features = false, features = ["server"] }`. The wrapper must compile on every
  3.x: use `InitializeResult` (not the 3.4+ alias `ServerConfig`) and do not override `negotiate_initialize` (3.3+).
- Rust 1.88 (rmcp's MSRV), edition 2024, `rust-version = "1.88"`.
- `agentcat-api = { version = "=0.0.3", default-features = false }` (R11). The SDK enables reqwest's TLS through its
  own features (`rustls` default, `native-tls` opt-in, R08).
- `serde_json` with `preserve_order` is always on (R07). Document that the feature applies to the whole customer
  build.
- `http = "1"` (same major as rmcp) to read `http::request::Parts` from request extensions.
- tokio is required (rmcp needs it). The SDK never requires a multi-thread runtime and never blocks a runtime
  thread on the hot path.
- Never write to stdout or stderr: stdout is the stdio JSON-RPC channel.
- The TS repository is public, so CI can fetch the pinned TS commit without a token. `agentcat-server` is private;
  anything that needs it runs locally (R14).

## 4. Sources of truth

| Question | Source |
|---|---|
| What must the SDK do? | `research/01-ts-sdk-behavior.md` (the contract), then the TS source at `9b56e9e`. |
| Where may Rust differ? | The deviation register (section 9) and `DECISIONS.md`. Nothing else. |
| What do PHP and C# do? | `research/02-decision-matrix.md`. |
| How does rmcp work? | `research/03-rmcp.md` and the rmcp 3.6.0 source. |
| How must the code look? | Section 10, from `research/04-rust-sdk-practices.md`. |
| How do we prove parity? | Section 11, from `research/06-verification-strategy.md`. |

When the contract and the TS source disagree, the TS source wins and the contract gets a correction.

## 5. Public API

Names may be refined in the plan; the shape and the capabilities are fixed here. A committed `cargo public-api`
snapshot makes every later change visible.

```rust
// stdio: one line
let server = agentcat_sdk::track(MyServer::new(), "proj_…", Options::default());
let running = server.serve(rmcp::transport::stdio()).await?;
running.waiting().await?;
// When the last handle drops, the SDK drains its queue for at most 5 s (R06).

// streamable HTTP: one shared handle, one wrapper per session or request
let agentcat = AgentCat::new("proj_…", Options::default());
let service = StreamableHttpService::new(
    { let ac = agentcat.clone(); move || Ok(ac.track(MyServer::new())) },
    LocalSessionManager::default().into(),
    StreamableHttpServerConfig::default(),
);
// … at shutdown:
agentcat.shutdown(Duration::from_secs(5)).await;
```

Items:

- `fn track<H: ServerHandler>(handler: H, project_id: impl Into<String>, options: Options) -> Tracked<H>`.
- `AgentCat`: a cheap `Clone` over `Arc<Inner>`.
  - `new(project_id, options)`, `from_env(options)` (reads `AGENTCAT_PROJECT_ID`, D36). `""` means telemetry only:
    events are built but never POSTed (B-003). Building never fails (C09).
  - `track(&self, handler) -> Tracked<H>`: cheap (an `Arc` clone, no background work), because stateless HTTP
    builds one handler per request.
  - `publish_custom_event(..)` in the TS session-string form (B-009, C11): honors its `project_id` argument and adds
    no server or client fields, as TS does.
  - The TS server form lives where a server identity exists (C16): on the per-call context inside a tool (that
    call's client and server identity), and on `Tracked<H>` (the identity that instance captured at `initialize`).
    It uses the handle's project ID, `enable_tracing` gate and `redact_event` hook, as TS does (K-03).
  - Both forms return `Result<(), Error>` whose `Display` texts are the TS error messages (D40).
  - `flush(timeout) -> bool`, `shutdown(timeout) -> bool`, and blocking twins (D43, R06). `shutdown` is terminal for
    the handle (C12).
- `Tracked<H>`: implements `ServerHandler` (C01) and `Clone` when `H: Clone`; gives access to the inner handler and
  the `AgentCat` handle.
- `Options`: private fields, `Default` = TS defaults (B-004), consuming setters: `enable_report_missing`,
  `enable_tracing`, `enable_tool_call_context`, `custom_context_description`, `enable_agent_tracking`,
  `api_base_url`, `resolve_session_id`, `identify`, `event_tags`, `event_properties`,
  `redact_sensitive_information`, `redact_event`, and a log-destination setting (C07).
- Hooks are async closures, `Send + Sync + 'static`, boxed inside. `identify`, `event_tags`, `event_properties` and
  `resolve_session_id` receive an owned `ToolCall` snapshot (D35): tool name, raw arguments (injected keys
  included), `_meta`, headers, client name and version, protocol version, transport session ID, and a clone of
  rmcp's `RequestContext<RoleServer>`.
- `UserIdentity { user_id, user_name, user_data }`, where `user_data` is a JSON object (D19).
- `CustomEvent`: setters for the TS `CustomEventData` fields.
- `Event`: the event that `redact_event` receives and returns (B-088). `#[non_exhaustive]` with public fields.
- `capture_error(&err)` (or an equivalent extension on `Result`): records the error's type path
  (`std::any::type_name`), `Display` text, `source()` chain and any captured backtrace for the current tool call
  (R09).
- Inside a tool, the SDK puts a per-call context into `RequestContext::extensions` so tools can read the resolved
  session ID and reach the `AgentCat` handle (proven in the rmcp spike).
- `Error`: `thiserror`, `#[non_exhaustive]`.
- `testing::Fake` behind the `test-util` feature (D41): instance-scoped, with `events()`, `assert_tracked(..)`,
  `assert_tracked_count(n)`, `assert_nothing_tracked()` and `assert_custom_event_published(..)`.
- Public `const`s for the environment variable names: `AGENTCAT_PROJECT_ID`, `AGENTCAT_API_URL`,
  `MCPCAT_API_URL` (legacy), `AGENTCAT_ENABLED`.

`AGENTCAT_ENABLED=false` (case-insensitive `false|0|no|off`) makes `track` return a pass-through wrapper that
forwards every call untouched and starts no thread (D44). There is no test detection (R10).

## 6. Architecture

### 6.1 Modules

The crate keeps every module private and exposes the API through one curated `pub use` block in `lib.rs` (no
prelude). Suggested modules:

| Module | Holds |
|---|---|
| `config` | `Options`, environment lookup (the only place that reads the environment; injectable for tests). |
| `client` | `AgentCat`, `Inner`, lifecycle (lazy worker start, flush, shutdown, last-drop drain). |
| `tracked` | `Tracked<H>` and its `ServerHandler` impl: forwards every method, intercepts `list_tools`, `call_tool`, `get_tool`, `initialize`. |
| `inject` | The pure tool-list pipeline (B-031 to B-049): handle and context injection, output-schema `mcp_session`, `get_more_tools`, registries. |
| `call` | The tool-call pipeline (B-022): resolution, stripping, mint-back and mirror, error capture, panic capture, MRTR tags. |
| `handles` | Session handles: KSUID, `ses_`/`evt_` IDs, validation, resolution (B-052 to B-069), `derive_session_id` (B-065). |
| `event` | `Event`, building (B-077), `parameters.request` and `parameters.extra` (D34), client info (B-085). |
| `pipeline` | Hook settling (B-026), `redact_event` (B-088), string redaction (B-089), sanitization (B-090), truncation and normalize (B-091, B-092), tag validation (B-093). |
| `delivery` | Queue, worker thread, retries, the `agentcat-api` call, success and failure logs (B-099 to B-105). |
| `js` | JavaScript semantics the TS SDK relies on: `JSON.stringify` output (number format, integer-like key order), UTF-16 lengths and cuts. |
| `tokens` | Token estimates (B-070 to B-076). |
| `logging` | `~/agentcat.log` in the TS format, or `tracing` (C07). |
| `constants` | Every agent-facing string and tag name, byte-identical to TS (Appendix A of the contract). |
| `testing` | `Fake` (feature `test-util`). |

### 6.2 Interception

`Tracked<H>` holds the inner handler and an `AgentCat`. It forwards all `ServerHandler` methods by hand, using
rmcp's own `impl_server_handler_for_wrapper!` list (`handler/server.rs:652-873` in 3.6.0) as the template, with
`#[allow(deprecated)]` for `subscribe`/`unsubscribe`. It intercepts:

- `list_tools`: forward, then run the pure injection pipeline on the result (B-021, B-031 to B-048). Pass
  `next_cursor` and other fields through. Merge registries per tool (D66).
- `call_tool`: the TS call pipeline (B-022), adapted to rmcp's `Result<CallToolResult, ErrorData>`.
- `get_tool`: the injected schema for customer tools, the SDK descriptor for `get_more_tools` (C02).
- `initialize`: forward unchanged; keep the client identity on that `Tracked` instance for the server form of custom
  events (C16). A `SessionRestoreMarker` replay restores the identity and is not a new connection.
- `get_info`, `supported_protocol_versions` and all others: forward unchanged.

A CI canary compares the forwarded method list with rmcp's wrapper list on every rmcp release, because rmcp minors
add defaulted methods (a missed one silently hides the customer's override).

### 6.3 Tool-list pipeline

A pure function of (options, listed tools), so vectors can pin it:

1. Append `get_more_tools` when `enable_report_missing` and the list is non-empty and has no tool of that name
   (B-021, B-028, B-045).
2. Handle injection (`session_id` unless hook mode, `agent_id` when agent tracking is on), then context injection,
   with the TS rules: property order customer → `session_id` → `agent_id` → `context`; `required` appended in the
   same order without duplicates; falsy schemas become empty object schemas; `additionalProperties: false` removed;
   composed schemas skipped with the TS warnings; customer-declared names skipped (`session_id` makes the tool
   "foreign"); truthiness collision checks; per-tool fault isolation (B-031 to B-044).
3. Output schemas get `mcp_session` as their last property when a handle is injected (B-047).
4. Registries: per tool name, merged across pages (D66), keyed by the inner handler type so two server types on one
   handle cannot collide.

### 6.4 Tool-call pipeline

In the TS order (B-022), with these Rust mechanics:

- Original schema of the called tool (to know which arguments are AgentCat's): `inner.get_tool(name)`, else the
  merged registry, else a listing rebuild that follows `next_cursor` within 5 s, else the TS heuristic strip (C03,
  B-027, B-051).
- Hooks: spawned on the customer's runtime at call start (C14), never awaited on the call path (B-026).
- The inner call runs under `catch_unwind(AssertUnwindSafe(..))`. A panic publishes an error event
  (`type = "panic"`, message = the payload) and then resumes unwinding unchanged.
- `Ok(result)`: mint-back text as `content[0]` when minted or invalid (B-056); `mcp_session` mirrored as the first
  key of a plain-object `structured_content` when the tool's output schema was extended (B-057, B-058, B-069); the
  event keeps the customer's original result (B-059); `is_error` results get the TS `error` shape (B-097).
- `Err(ErrorData)`: event with `is_error`, `error` = `{message, type: <JSON-RPC code name>, platform: "rust"}`, no
  `response`, no `output_tokens`; return the error unchanged. No mint-back is possible (D16).
- MRTR tags (`agentcat_mrtr`) as B-025, from rmcp's 2026-07-28 types.
- Event building and the `parameters` shape: section 6.6.

### 6.5 Session handles

Exactly B-052 to B-069: `ses_` + 27-char base62 KSUID (segmentio epoch); resolution `minted` / `supplied` /
`invalid` / `foreign`; hook mode with `derive_session_id(id, project_id)` from sha256; mint-back strings byte-equal to
TS; `agent_id` tags clamped to 200 UTF-16 units with CR/LF replaced; `agentcat_protocol_version` from
`_meta["io.modelcontextprotocol/protocolVersion"]` or the 2026-07-28 envelope; `agentcat_session_id_source` always
present. The transport session (`Mcp-Session-Id`) is never used for correlation (B-063).

### 6.6 Event building

Fields as B-077, with `sdk_language = "Rust"` and `agentcat_version` = the crate version (D01). Key points:

- `parameters.request` reproduces the raw JSON-RPC request TS records (B-082): `method`, `params` (`name`, raw
  `arguments` with injected keys, `_meta` when present), `jsonrpc`, `id`, in TS key order.
- `parameters.extra` (D34): `sessionId` (the `Mcp-Session-Id` header, stateful HTTP only), `requestId` (as sent),
  `_meta` (when sent), `requestInfo: {headers, url}` from `http::request::Parts` (all headers, lower-case, repeated
  values joined with `", "`, D06; absolute URL rebuilt from `Host`, path and query). On stdio there is no
  `requestInfo`.
- Client info ladder (B-085): the 2026-07-28 envelope, then `_meta` client info, then `context.client_info()`. rmcp's
  stateless HTTP mode fabricates rmcp's own name and version (`research/03-rmcp.md` §5.2); the builder decides,
  with the harness, how to treat that identity and records the ruling.
- Server info from `inner.get_info().server_info`.
- Token estimates (B-070 to B-076) over `js::stringify` output (C04).

### 6.7 Event pipeline and delivery

- Each `AgentCat` `Inner` owns, lazily, one named OS thread (`agentcat-delivery`) with a current-thread tokio runtime,
  the SDK's `reqwest::Client` (connect timeout, 10 s per-attempt timeout, User-Agent `agentcat-rust/<version>`), and
  an `agentcat_api::apis::configuration::Configuration` built with that client and no API key.
- The call path hands a pending event to the queue without blocking. Queue rules are TS's (B-100, B-101): at most
  10 000 pending, drop the oldest pending with `Event queue full, dropping oldest event`; 5 in flight, each holding
  its slot through hook settling, redaction and retries.
- Per event, in TS order (B-087): settle hooks (30 s each, concurrently; the TS timeout and error log texts), apply
  hook results, `redact_event`, string redaction, sanitization, truncation, assign `evt_` ID, then POST unless
  telemetry-only.
- POST through `agentcat_api::apis::events_api::publish_event` to `{base}/v1/public/events/`, building
  `PublishEventRequest` with `new()` plus field assignments. Retry any failure 3 times after 1, 2 and 4 s (B-102),
  with the TS log lines. `session_id` is `Some(None)` (explicit `null`) for sessionless events.
- API base URL: `api_base_url` option, else `AGENTCAT_API_URL`, else `MCPCAT_API_URL`, else
  `https://api.agentcat.com` (B-006), per handle (C10).
- Lifecycle (R06): `flush(timeout)` waits until the queue and in-flight work are empty and returns whether that
  happened in time. `shutdown(timeout)` does the same and then rejects later events (C12). When the last `Inner`
  drops, the SDK runs a bounded blocking shutdown (5 s) that waits on the worker's acknowledgement with a timeout,
  never `join().unwrap()`. `std::process::exit` and `panic = "abort"` skip it; the README says so.
- Logging (B-114 to B-119, C07): the TS message texts, written to `~/agentcat.log` with a Rust prefix
  (`[sdk=<version> rust=<version> rmcp=<version>]`), silent when there is no home directory, off the request path.

## 7. Behavior rules

1. Copy TS, known bugs included (D15, C15). Each copied K-bug that is reachable in Rust is pinned by a test named
   after it.
2. Where Rust or `agentcat-api` forces a difference, the deviation register (section 9) records it, and the harness
   rulings file names it. An unrecorded difference is a bug.
3. Never panic, never block the host runtime, never print. The SDK's own failures become log lines (D32).
4. The SDK never changes a tool's result beyond the TS injections, and never changes an error.

## 8. Engineering standards

The style guide is section 20 of `research/04-rust-sdk-practices.md`, with these overrides from parity and the
owner's decisions:

| Practices rule | Override |
|---|---|
| 11 (drop the newest event on overflow) | TS rule: drop the oldest pending event (B-101). |
| 19 (retry only network errors, 408, 429, 5xx; honor `Retry-After`; stop on 401/403) | TS rule: retry every failure 3 times at 1/2/4 s (B-102), plus a per-attempt timeout (deviation RD-02). |
| 21 (public `Transport` trait) | Internal only in the first pass. Tests use wiremock and the `Fake`. |
| 26 (log only through `tracing`) | C07: file by default, `tracing` as an option. |
| 36 (no `serde_json::Value` in public signatures) | C08: `serde_json` and `rmcp` types are allowed. |
| 4 (`build() -> Result`) | C09: building cannot fail. |
| 41 (bump `0.y` on an rmcp major) | R13: the crate starts at 1.0.0; an rmcp major forces a major. |
| 43 (release-plz, trusted publishing) | Second pass. |

Everything else applies as written, notably: one published crate in a workspace; `#![forbid(unsafe_code)]`; clippy
`pedantic` at warn with `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `print_stdout`,
`print_stderr`, `dbg_macro` denied and every exception written as `#[expect(.., reason = "..")]`; warnings denied in
CI only; `missing_docs`; `thiserror` 2 with `#[non_exhaustive]` errors; `Debug` on every public type; compile-time
`Send + Sync + 'static` assertions for `AgentCat`, `Tracked<H>` (when `H` is) and `Error`; additive features
`rustls` (default), `native-tls`, `test-util`; docs.rs with `all-features` and `doc_cfg`; `lib.rs` as the guide and
the README doctested; examples for stdio and streamable HTTP; Keep a Changelog.

## 9. Deviation register

Every known place where the Rust SDK acts differently from TS `9b56e9e`. The harness rulings file and the parity
manifest point to these IDs.

| ID | TS behavior | Rust behavior | Why |
|---|---|---|---|
| RD-01 | The wire body has every field twice, camelCase and snake_case (B-079, K-18). | snake_case only. | `agentcat-api` builds the body; the server reads snake_case (PHP and C# do the same). |
| RD-02 | No HTTP timeout (B-102). | Connect timeout and a 10 s per-attempt timeout; same retry count and backoff. | A hung request would hold one of 5 slots forever (C# made the same choice). |
| RD-03 | Tag keys keep insertion order on the wire. | Random order on the wire (`HashMap` in `agentcat-api`); which tags survive the 50 cap is unchanged. | `agentcat-api` model type. |
| RD-04 | Timestamps always have milliseconds (`….000Z`). | chrono drops the fraction when it is zero (`…:00Z`). | `agentcat-api` model type; still RFC 3339. |
| RD-05 | Thrown errors and unknown tools become `isError` results with mint-back (B-024, B-022 step 13). | rmcp returns JSON-RPC errors for `Err(ErrorData)` and unknown tools; no mint-back, the event has `error` and no `response`. Argument deserialization failures stay `is_error` results (rmcp 3.6 `router/tool.rs:145-155`) and get mint-back like any result. | rmcp semantics (D16; C# and TS v2 behave the same). |
| RD-06 | `error` = `{message, type: constructor name, platform: "javascript", stack, frames, chained_errors}` from V8. | `platform: "rust"`; `type` = JSON-RPC code name for `ErrorData`, the type path from `capture_error`, or `"panic"`; frames only from a captured backtrace; `chained_errors` from `source()`. | Rust has no exception classes (R09). |
| RD-07 | `parameters.extra` is the whole JS handler context (`signal`, function stubs, `authInfo` with token). | The D34 shape from `RequestContext`; no function stubs, no `authInfo`. | No equivalent objects in rmcp. |
| RD-08 | Process-global queue, API URL (last wins) and exporter settings (K-09). | Per `AgentCat` handle (C10); the log destination stays process-global (D38). | Explicit handles. |
| RD-09 | Signal and `beforeExit` handlers; no public flush or shutdown (B-105). | No signal handlers; public `flush`/`shutdown`; automatic bounded drain when the last handle drops (R06). | Rust has no `beforeExit`; a tokio signal listener would change the process's signal behavior. |
| RD-10 | No kill switch; no test fake. | `AGENTCAT_ENABLED`, `testing::Fake` (D44, D41). | PHP decisions. |
| RD-11 | `null` project ID = telemetry only. | `from_env` reads `AGENTCAT_PROJECT_ID`; `""` = telemetry only (D36). | PHP decision. |
| RD-12 | Each listing page replaces the registries; rebuild reads page 1 (K-01, K-02, B-048). | Registries merged per tool; rebuild follows `next_cursor`; `get_tool` first (C03). | PHP D66. |
| RD-13 | Host SDK behavior of the TS MCP SDK (`execution` on tools, omitted `isError`, zod validation). | rmcp's own behavior. | Not AgentCat's; the harness compares the AgentCat delta, not host output (section 11). |
| RD-14 | `sdkLanguage: "TypeScript"`, `agentcatVersion` 2.1.1. | `"Rust"`, crate version. | D01. |
| RD-15 | Log prefix `[sdk=… node=… mcp=…]`. | `[sdk=… rust=… rmcp=…]`; optional `tracing` route. | C07. |
| RD-16 | Exporters and remote diagnostics. | Absent in the first pass. | R04, C05. |
| RD-17 | High-level-server inner tap stashes the thrown `Error` for its stack (B-020). | `capture_error` helper (opt-in). | rmcp turns a tool's own error into text before the wrapper sees it. |
| RD-18 | Hooks receive the raw `(request, extra)`. | Hooks receive a `ToolCall` snapshot (D35). | Rust ownership and `Send` bounds. |
| RD-19 | Detection, compatibility errors, late registration, re-arm, Workers `waitUntil` (B-011, B-014 to B-016, B-019, B-049, B-106). | Not applicable. | rmcp is typed and the wrapper sees every call. |
| RD-20 | Lone surrogates, cycles, shared references, functions, `undefined`, NaN in values (B-092, K-13). | Cannot occur (`serde_json::Value` and Rust strings forbid them); the corresponding normalize branches do not exist. A value that TS shares between two fields is two copies in Rust, so the string-redaction hook runs once for each copy. | Rust types. |
| RD-21 | A tool handler function shared by several v1 servers is wrapped only on the first, so errors on later servers lose `stack` and `frames` (a JS function-identity artifact). | Every server records the error the same way. | Owner ruling R19. |
| RD-22 | A cut at a UTF-16 limit that falls inside a surrogate pair keeps the lone high surrogate, which goes on the wire as `\udXXX` (B-066, B-091, B-092). A cut from the end keeps the lone low surrogate in the same way: truncation spreads the last 25 units of a long `error.frames` string into items, and the first item can be a low surrogate. | The cut drops the whole character (D09). In the spread `error.frames` items, half a character at either cut is an empty item. A size check after such a cut counts up to 6 bytes fewer than TS. | Rust strings cannot hold a lone surrogate. |
| RD-23 | `JSON.parse` reads every decimal to the nearest `f64`. | Without `float_roundtrip`, a decimal with 16 or 17 significant digits can become the neighboring `f64`, so a recorded number or a token estimate can differ. | Owner ruling R22. |
| RD-24 | Sanitize, normalize and redaction drop a `__proto__` key of customer JSON while they copy objects. | The key is kept like any other key. | Owner ruling R23. |
| RD-25 | An array tool `response` is spread into an object with index keys (`{"0": …, "1": …}`) before it is published. | The array stays an array. | Owner ruling R23. |
| RD-26 | Truncation keeps an object whose `length` property is at most the limit under JavaScript's loose comparison (for example `{"length": 10}`) as the value of `userIntent`, `resourceName`, a server or client field, `error.message` (the limit of each) or `error.frames` (50); any other object throws (K-11). | Every object there is the error, with TS's text for an object without `length` (`TypeError: str.slice is not a function`, or the `frames.slice` one), so the event is dropped. | Provisional ruling (pending, milestone 3 batch). |
| RD-27 | `String()` of an object with an own `toString` key, at any depth inside an array, throws `TypeError: Cannot convert object to primitive value` (for example in sanitization's unsupported-type message and in the `userIntent` array join of truncation). | The object gives `[object Object]` like any other object. | Owner ruling R23. |
| RD-28 | Truncation assigns an own `message: undefined` to an `error` that has no `message` (an object, or an array spread into index keys), and normalize counts that property toward the 100-key breadth. So an `error` of exactly 100 keys gets `"...": "[MaxProperties ~]"`, and an existing `"..."` key at the breadth limit gets the marker as its value. | Only keys that JSON holds count, so these `error` values keep their keys and values. | Owner ruling R23. |

The builder adds a row for every new difference the harness finds. A forced difference gets its reason. An unforced
difference gets a provisional ruling (marked `pending` in the register and in `rulings.json`), and work continues;
at the end of each milestone the builder brings that milestone's pending rulings to the owner as one batch of
multiple-choice questions, and changes from the answers land in the next milestone (R17).

## 10. Repository layout (suggested)

```
Cargo.toml              workspace: [workspace.package], [workspace.dependencies], [workspace.lints]
agentcat-sdk/           the published crate: src/, tests/, examples/, README.md, CHANGELOG.md
xtask/                  vector regeneration, parity check, harness runner, rmcp canary
harness/                Node: driver, mock API, normalizer, TS reference server, scenarios, rulings.json
oracle/                 Node: builds the TS SDK at the pinned SHA and runs its modules (vectors, proptest oracle)
parity/                 ts-tests.json (generated), manifest.toml
docs/                   DECISIONS.md, research/, superpowers/
scripts/check.sh        the local gate (same as CI)
```

## 11. Verification

The owner asked for side-by-side diff tests, a dashboard demo and a TS test-by-test map (R16). The layers come from
`research/06-verification-strategy.md` (C13):

| Layer | What | Oracle | Cadence |
|---|---|---|---|
| L1 vectors | Pure functions: constants and strings, KSUID and IDs, handle resolution and derivation, mint-back text, injected schemas (with key order), token estimates, `JSON.stringify` output, UTF-16 cuts, tag validation, normalize, truncation, sanitization, redaction walk. | JSON files written by running the real TS modules at the pinned SHA (`oracle/`), with a sha256 integrity manifest. Never typed by hand. | Every push; regenerated nightly from TS main. |
| L2 property tests | The same functions on random inputs. | A long-running Node process that runs the TS module (JSON lines over stdio). | 256 cases per push, 10 000 nightly; regression files committed. |
| L3 differential harness | The whole system: MCP wire and published bodies. | A TS reference server with the real TS SDK beside a Rust reference server; one scripted MCP client (the TS client) drives both from JSON scenarios; one mock API records both. | Every push at the pinned TS SHA and rmcp; nightly with TS main and the latest rmcp 3.x. |
| L4 contract | Bodies the backend accepts. | Every captured body validated against a pinned copy of the server's OpenAPI schema; locally, a replay through `agentcat-server`'s `process_event` (R14). | Schema every push; replay before the PR. |

Harness rules:

- Scenarios run four ways (TS untracked, TS tracked, Rust untracked, Rust tracked). Assert: tracked events equal
  after normalization; and the MCP-wire delta untracked→tracked is equal for TS and Rust, so host-SDK differences
  cancel out (RD-13).
- Transports: stdio and streamable HTTP (stateful and stateless). Client lanes: a 2025-11-25 client and a 2026-07-28
  client. The 2026-07-28 lane needs a second TS reference server built on `@modelcontextprotocol/server` 2.x,
  because a v1 `McpServer` cannot serve that protocol.
- The tool contract (customer schemas) is generated from the TS reference server and loaded by the Rust reference
  server.
- Hooks are driven by an options profile of canned behaviors (returns X, returns null, fails, sleeps N ms).
- The normalizer's rules are explicit and reported; IDs map to first-appearance placeholders consistent between
  the MCP transcript and the events; hook-mode session IDs compare literally; ordered checks at named paths
  (`mcp_session` first, injected property order). A/A runs must diff to zero, and a canary (a deliberately
  perturbed Rust configuration) must fail.
- `harness/rulings.json` lists each allowed difference with a path pattern, a reason and a deviation ID. A ruling
  that matches nothing fails the run.
- Scenarios start from the TS e2e table and add pagination, identify, tags and properties, custom events,
  redaction, large payloads, errors, panics, concurrency (compared as a multiset) and multiple sessions.

Parity manifest: `parity/ts-tests.json` is generated with `vitest list --json` at the pinned SHA (1,020 test keys
today). `parity/manifest.toml` maps each key to Rust tests or harness scenarios, or to `n/a` with a category and a
reason (`ts-host-only`, `second-pass`, `ruled-divergence:RD-xx`, `covered-by-vector:<file>`). The checker fails on
an unmapped key, a dangling Rust test name or a mapping for a key that no longer exists. While the build is in
progress, entries may be `todo`; the final gate requires zero `todo`.

Dashboard demo (R12): an example server tracked into project "Testing" on the prod API runs a fixed scenario; the
same scenario runs through the TS SDK; the events are read back with the AgentCat MCP server tools
(`search_sessions`, `get_session`, `get_events`) and compared. The demo report goes into the PR.

CI gates (C17). One aggregate `all-green` job collects them; making it a required check needs repository admin
rights, so that setting is the owner's step. Every push to `rewrite` runs the gates below on Linux; the final PR and
the nightly run add macOS and Windows and the full harness matrix. The mutation gate compares against the previous
green commit of `rewrite` (not `main`, which is an empty root commit):

1. `cargo fmt --check`; clippy `--all-targets --all-features -D warnings`; `cargo hack` clippy per feature.
2. Tests (nextest) and doctests on stable and on Rust 1.88 (macOS and Windows on the PR and nightly).
3. `cargo minimal-versions check`; docs with `--cfg docsrs -D warnings`; `cargo deny check`.
4. `cargo public-api` snapshot diff and an external-types allowlist (C08).
5. `cargo package --list` / `cargo publish --dry-run`; all examples build; stdio and HTTP examples run against the
   mock API (events arrive, no non-JSON stdout, clean exit with the queue drained).
6. L1 vectors and integrity manifest; L2 at 256 cases; L3 harness with zero unexpected diffs, A/A and canary; L4
   schema validation.
7. Parity manifest check.
8. Line coverage ≥ 90 % overall and ≥ 95 % for the pure modules (`cargo llvm-cov`).
9. `cargo mutants` on the diff against the previous green commit: zero missed mutants in core modules; any skip
   needs a written reason.
10. rmcp legs: floor 3.0.0 and latest 3.x, each with a test that proves which rmcp version compiled.
11. Workflow hygiene: actions pinned by SHA, least-privilege `permissions:`.

Nightly: full mutation run (≥ 90 % caught); branch coverage report; every 3.x minor (latest patch) and rmcp `main`;
the forwarded-method canary; the TS drift sweep (vectors, test list and harness against TS main), which opens or
updates one issue on any difference; proptest at 10 000 cases; a build against the newest `agentcat-api`
(warning only).

Local only: the ingest replay (R14) and the dashboard demo (R12).

## 12. Suggested milestones

Verification infrastructure comes first, so drift is caught while the code is written. All work lands on
`rewrite`; the owner reviews one PR at the end (R15). Each milestone ends with the batch of pending rulings (R17).

1. Repository and gates: workspace, lints, `scripts/check.sh`, CI skeleton with the gates that can run on an empty
   crate, deny, MSRV leg.
2. Oracle: `oracle/` builds TS at the pinned SHA; vector generators; the Node property-test oracle; `ts-tests.json`
   and the manifest checker (with `todo` allowed); the harness TS half (driver, mock API, normalizer, TS reference
   server, tool contract).
3. Pure core against L1 and L2: constants, `js`, KSUID and handles, tokens, tag validation, normalize, truncation,
   sanitization, redaction.
4. Interception: `Tracked<H>`, tool-list pipeline, tool-call pipeline, registries, `get_more_tools`, client info,
   `parameters`, panics, `capture_error`; the Rust reference server joins the harness.
5. Delivery and lifecycle: event pipeline, worker thread, queue, retries, flush, shutdown, last-drop drain, logging,
   environment, custom events, `Fake`; the full scenario set passes.
6. Finish: README and rustdoc, examples, CHANGELOG, public API snapshot, coverage and mutation floors, zero `todo`
   in the manifest, ingest replay, dashboard demo, the PR.

## 13. Risks

- rmcp churn: new defaulted `ServerHandler` methods (canary), and a 4.0 within months (an rmcp major forces an
  AgentCat major, R13).
- Blocking drain in `Drop` on a current-thread runtime while hooks still need that runtime: hooks cannot progress,
  so the drain waits up to its timeout. The plan must test this case and keep the drain bounded.
- `preserve_order` changes JSON map order for the whole customer build. It is required (R07); the README states it.
- The normalizer could hide drift; A/A, the canary and explicit rules guard it.
- Copying TS bugs means the harness proves "same as TS", not "correct"; the ingest replay adds a backend check.
- The local TS checkout is dirty and its `node_modules` is stale; the oracle always builds from the pinned commit
  with a frozen lockfile.

## 14. Left to the builder

- Final public names, provided the capabilities and rules above hold and the public API snapshot is reviewed.
- Internal data structures (queue, registries, task layout), within the rules of sections 6 and 8.
- The handling of rmcp's fabricated stateless client identity (section 6.6), recorded as a ruling.
- The exact scenario list beyond the minimum in section 11.
- Optional improvements to the `agentcat-api` generator (default TLS, User-Agent, `#[non_exhaustive]` enum,
  `Retry-After`, the `session_id` double option) are follow-ups for `agentcat-server`, not part of this pass.
