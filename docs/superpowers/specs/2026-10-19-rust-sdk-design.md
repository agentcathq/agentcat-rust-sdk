# AgentCat Rust SDK (`agentcat-sdk` on crates.io): design

Status: approved in conversation on 2026-10-19 (sections 1–5); written for owner review.
Decision log: [`../../DECISIONS.md`](../../DECISIONS.md). Where this spec and the log disagree, the log wins.

## 1. Intent

The Rust SDK instruments MCP servers built with `rmcp` 3.x. It must behave like the TypeScript SDK and
the PHP and C# ports: the same injected tool parameters, the same agent-facing text, the same events on
the wire, and the same owner decisions. It must also meet the C# port's engineering bar: a decision log,
milestone plans, a parity suite that ports every TS behavior test by title, strict CI gates, and a
release through trusted publishing.

A first version of the crate already exists (`main` at `6719225`): an rmcp wrapper, the event pipeline,
exporters, golden vectors from the TS code, and 97 tests. It was built before the owner adopted the
PHP/C# decision log for Rust, so it fixes about 29 TS defects that the log says to copy (D15), and it
lacks the features the log added (test fake, kill switch, `AGENTCAT_PROJECT_ID`, the D34 `extra`
shape). This design reworks that code in place, test first.

## 2. Sources

| Source | Use |
|---|---|
| TS SDK `agentcat-typescript-sdk` at `9b56e9e` (2.1.1-beta.1) | Behavioral source of truth |
| `docs/ts-sdk-audit/01`–`03` | TS architecture, pipeline, exporters, rmcp internals |
| `docs/research/04-rust-mcp-usage.md` | Which rmcp versions, features and server shapes exist in the wild |
| `docs/research/05-rust-sdk-best-practices.md` | How Sentry, OpenTelemetry, Datadog, PostHog and others build Rust SDKs |
| `docs/research/06a`–`06c-ts-test-index.md` | Every TS test case (1,024), marked port, adapt or N/A |
| `agentcat-php-sdk/docs/audit/DECISIONS.md` | The shared decision log (D01–D62) |
| `agentcat-php-sdk/docs/audit/fixtures/` | Cross-SDK golden fixtures (handles, listings, exporters, ID vectors) |
| `agentcat-csharp-sdk/docs/superpowers/specs/2026-10-19-csharp-sdk-design.md` | How C# applies the log; engineering bar |
| `tests/fixtures/ts-vectors.json` | Rust-side golden vectors generated from the TS functions |

## 3. Goals, non-goals, anti-goals

### 3.1 Goals

1. One line of customer code tracks an rmcp server on stdio, streamable HTTP (sessions and stateless),
   and any other rmcp transport: `agentcat.track(handler)`.
2. Agent-facing strings, ID algorithms, schema injection, mint-back, tags and event fields are
   byte-identical to TS, proven by golden fixtures shared with PHP and C#.
3. Behavior follows the shared decision log; every Rust deviation is recorded in `DECISIONS.md` with
   its reason.
4. Every TS behavior test has a Rust test with the same title.
5. AgentCat never changes a tool's result or error, never writes to stdout or stderr, and never blocks
   the server's async runtime.
6. Releases go out through crates.io trusted publishing from CI.

### 3.2 Non-goals

- rmcp 0.x, 1.x and 2.x (owner decision R4).
- Other Rust MCP libraries (`rust-mcp-sdk`, `pmcp`, `turbomcp`, `tower-mcp`).
- Automatic detection of test runs (R3).
- A models-only API client (R1).
- Framework auto-integration (PHP D27 and D51–D62 are PHP-specific).

### 3.3 Anti-goals (a technically working SDK that does any of these has failed)

- A tool result, error or panic that differs because AgentCat is installed (apart from the documented
  mint-back block and `mcp_session` mirror).
- Any byte from the SDK on stdout or stderr.
- A blocked or panicked customer runtime caused by AgentCat code or timers.
- A behavior difference from TS that `DECISIONS.md` does not record.

## 4. Decisions

The full log is [`DECISIONS.md`](../../DECISIONS.md). In short:

- **Carried over from the PHP/C# log, unchanged:** D05, D06, D07, D08, D09, D15, D16, D18, D32, D33,
  D34, D35, D36, D40, D41, D43, D44, D45, D50, D60, D61, B09.
- **Carried over with a Rust form:** D10 (dotted full path, R2), D31 (no automatic detection, R3),
  D03 (exception, R1).
- **Rust-specific:** R1–R11 in the log.

## 5. Constraints

1. **Host:** `rmcp` 3.x (`>= 3.0, < 4`); the wrapper must compile against 3.0.0 (no
   `negotiate_initialize`, no `ServerConfig` alias) and the newest 3.x. MSRV 1.88, edition 2024.
2. **Transport:** the generated `agentcat-api` crate (`= 0.0.3` line), whose `PublishEventRequest`
   serializes snake_case keys, an explicit `session_id: null`, `tags` as an unordered map, and
   timestamps with `+00:00`.
3. **stdout is the JSON-RPC channel** of stdio servers.
4. **Customer runtimes** may lack timers or be `current_thread`; the SDK times its own work.
5. **The AgentCat events API** answers 202 before validation and drops bad events later
   (`docs/ts-sdk-audit/02` §2.5).
6. **Cargo feature unification:** `serde_json/preserve_order` is enabled by default (R5) and affects the
   whole customer build.

## 6. Crate, features and dependencies

One crate, `agentcat-sdk`, library name `agentcat`.

| Feature | Default | Purpose |
|---|---|---|
| `rmcp` | yes | `Tracked<H>` for rmcp 3.x |
| `rustls` / `native-tls` | `rustls` | TLS for the API, exporters and diagnostics; one is required |
| `preserve-order` | yes | `serde_json/preserve_order` (R5) |
| `exporters` (`otlp`, `datadog`, `sentry`, `posthog`) | yes | Exporters; no extra dependencies |

Runtime dependencies stay as today: `agentcat-api`, `reqwest` 0.13, `tokio` (rt, sync, time, macros),
`serde`, `serde_json`, `chrono`, `futures`, `sha2`, `getrandom`, `http` 1, `tracing`, `rmcp` 3.
`cargo deny` keeps the tree free of duplicate versions where possible.

## 7. Public API

### 7.1 Entry points

```rust
AgentCat::builder(project_id)        // &str, String, or None (None reads AGENTCAT_PROJECT_ID; "" = telemetry only)
    .identify(..) .resolve_session_id(..) .event_tags(..) .event_properties(..)
    .redact_sensitive_information(..) .redact_sensitive_information_async(..)
    .redact_event(..) .redact_event_async(..)
    .enable_tracing(..) .enable_tool_call_context(..) .custom_context_description(..)
    .enable_report_missing(..) .enable_agent_tracking(..)
    .exporter(name, config) .api_base_url(..) .disable_diagnostics(..)
    .log_to_tracing(..) .server_info(name, version) .capture_panics(..)
    .build() -> AgentCat              // never fails; problems are logged
AgentCat::new(project_id)            // builder with defaults
AgentCat::telemetry_only()           // builder("")
agentcat.track(handler) -> Tracked<H>
agentcat::track(handler, project_id) -> Tracked<H>
```

`Tracked<H>` is `Clone` when `H: Clone` (clones share the tool registry), forwards every
`ServerHandler` method, and exposes `inner()` and `into_inner()`.

### 7.2 Hook argument (D35)

`ToolCallRequest`, immutable and cheap to clone, with: `tool_name()`, `arguments()`, `argument(name)`,
`meta()`, `headers()` (lower-case name → value), `header(name)`, `client_name()`, `client_version()`,
`protocol_version()`, `transport_session_id()` (the `Mcp-Session-Id` header), `params()`, `context()`
and `http_request()` (the full `http::request::Parts`, including extensions set by auth middleware).

### 7.3 Custom events (D40, B09)

```rust
agentcat.publish_custom_event(project_id: &str, event: CustomEvent) -> Result<(), agentcat::Error>
agentcat::publish_custom_event(session_id: &str, project_id: &str, event: CustomEvent) -> Result<(), agentcat::Error>
```

An empty `project_id` returns `Error::MissingProjectId` with the TS message
`projectId is required for publishCustomEvent`. As in TS, the tracked form publishes under the
tracker's own project ID, uses `event.session_id` or no session, and applies `redact_event` but not
`redact_sensitive_information`; it publishes nothing when tracing is off and still logs the TS line.
The session-string form uses the string as given (or `event.session_id`) with no redaction.
`CustomEvent` is a builder-style value type (`resource_name`, `parameters`, `response`, `message`,
`duration`, `error`, `tag`, `properties`, `session_id`).

### 7.4 Exporters (D45)

Public config types `OtlpExporter`, `DatadogExporter`, `SentryExporter`, `PostHogExporter`
(fields named after the TS keys in snake_case), and
`ExporterConfig::from_json(json!({"type": "otlp", "endpoint": ...}))`, which accepts the TS key names.

### 7.5 Lifecycle and testing (D05, D41, D43, D44)

- `agentcat::flush(timeout) -> bool`, `flush_blocking`, `shutdown(timeout) -> bool`,
  `shutdown_blocking`. `true` means every queued event was sent.
- `AGENTCAT_ENABLED=false`: `track()` returns a pass-through wrapper, custom events do nothing, no
  publisher or diagnostics thread starts, and one log line says so. `true` or unset: on.
- `agentcat::testing::fake() -> AgentCatFake`: while the guard lives, events run the whole pipeline and
  are recorded in memory instead of sent. Methods: `events()`, `assert_tracked(tool)`,
  `assert_tracked_count(tool, n)`, `assert_nothing_tracked()`,
  `assert_custom_event_published(resource_name)`. Dropping the guard restores delivery. The fake is
  process-wide; the docs tell users to run tests that use it serially.

### 7.6 Stability

Public structs are `#[non_exhaustive]`; CI compares the public API with a committed
`cargo public-api` baseline and runs `cargo semver-checks` against the last published version.

## 8. Runtime services

### 8.1 Event pipeline

A process-wide publisher: a dedicated thread with its own current-thread tokio runtime, fed by a
bounded channel. Stages and semantics follow TS (`eventQueue.ts`, `pendingEvent.ts`), D15 included:

- Queue capacity 10,000; when full, the **oldest** waiting event is dropped with the TS log line.
- Five events in flight; stage 0 awaits the hook results (30 s each, in parallel), then `redactEvent`,
  string redaction (D08 scope), sanitization, truncation, `evt_` ID.
- Delivery: up to four attempts with 1 s, 2 s, 4 s backoff; every failure is retried (4xx included); a
  2xx whose body is not JSON counts as a failure; exporters run before each attempt (TS behavior).
- Each HTTP attempt has a 10 s timeout (R8).
- The API base URL: option, then `AGENTCAT_API_URL`, then `MCPCAT_API_URL`, then
  `https://api.agentcat.com`; the last configuration that sets one wins (TS).
- Timers for the request path (the 5 s registry rebuild) run on a separate small timer thread.

### 8.2 Logging (D60, D61)

Every SDK line is `[<ISO time>] [sdk=<ver> lang=rust mcp=<rmcp ver>] <message>`, with the TS message
texts. Default destination: `~/agentcat.log` (or `$AGENTCAT_LOG_FILE`); `.log_to_tracing(true)` sends
the lines to `tracing` (target `agentcat`) instead. Never stdout or stderr.

### 8.3 Diagnostics (D07, D33, D50)

Every log line is batched (1,000 records, 2 s) and sent as OTLP logs to the AgentCat collector.
Environment names: `AGENTCAT_DISABLE_DIAGNOSTICS`, `AGENTCAT_DIAGNOSTICS_ENDPOINT`,
`AGENTCAT_DIAGNOSTICS_TOKEN`; the unprefixed `DISABLE_DIAGNOSTICS` only turns diagnostics off. With
no test detection (R3), diagnostics stay on in customer test runs unless turned off. The README
describes exactly what is sent (D33).

### 8.4 Lifecycle

No signal handlers (the customer's runtime owns them, as in C#). Rust has no reliable exit hook, so
the README and examples call `shutdown()` before the process exits; `shutdown_blocking()` serves code
without a runtime.

### 8.5 HTTP

One SDK-owned `reqwest::Client` (10 s per request, `User-Agent: agentcat-rust/<version> rmcp/<version>`)
shared by the API call, exporters and diagnostics. No `Authorization` header. Wire body: the generated
`PublishEventRequest` (snake_case only, `session_id: null` when sessionless, `identify_data: {}` on
anonymous tool-call events, `sdk_language: "Rust"`, `agentcat_version` from the crate version).

## 9. The engine

### 9.1 Seam

`Tracked<H: ServerHandler>` overrides `list_tools`, `get_tool` and `call_tool`, and forwards the other
methods (`negotiate_initialize` excepted, recorded in `tests/trait_drift.rs`). A second `track()` on a
`Tracked` value is a pass-through.

### 9.2 Per-instance state

A registry per wrapper instance (shared by its clones) records, per tool, which parameters were
injected, whether the customer declares `session_id`, and whether the output schema got
`mcp_session`. Grow-only sets (declared `session_id`, reported conflicts) live per `AgentCat`.

### 9.3 `tools/list`

The TS pipeline (`buildInjectedList`), byte-identical: `get_more_tools` appended to every page unless
the customer lists one; handles injected first, then `context`; composed schemas skipped; empty
schemas get `properties` but no `type`; `additionalProperties: false` removed; `required` extended;
`mcp_session` declared in plain output schemas. An empty list is returned unchanged.

### 9.4 `tools/call`

TS `callWrap` order: rebuild the registry when this instance never listed (customer `get_tool`, then
`list_tools` page 1 with a 5 s budget; the event's `duration` includes it, TS behavior); resolve
handles; start hooks; build the event; answer `get_more_tools`; strip injected arguments (registry, or
the TS heuristic for unseen tools; composed tools are not stripped); run the handler; decorate
(mint-back block first, `mcp_session` first key); publish. With tracing off, `context` is still
stripped (R8). MRTR rounds are tagged; tasks are recorded undecorated.

### 9.5 Events

`parameters`: `{ request: <raw JSON-RPC message: method, params, jsonrpc, id>, extra: { sessionId?,
requestId, _meta?, requestInfo?: { headers, url? } } }` (D34). `sessionId` is the `Mcp-Session-Id`
header; `requestInfo` exists on HTTP; `url` is present only when the request URI is absolute (axum's
`nest_service` hands the handler a relative path). `response`: the customer's result without rmcp's
`resultType`. Client identity per request: `_meta` clientInfo, then the handshake, never rmcp's
placeholder `Implementation::default()`. Tags: TS names; customer tags capped at 50 minus the SDK
tags (R8).

### 9.6 Errors (D10)

- A JSON-RPC `Err`: `type` `McpError`, message `MCP error <code>: <message>`, rethrown unchanged.
- An `is_error` result: the TS `captureCallToolResultError` shape (text blocks joined, no type).
- A panic: caught, recorded with a parsed backtrace (dotted module paths, `in_app` rules), then resumed.
- `ErrorData::from_error_typed::<E>()`: `type` is the dotted full path (`std.num.ParseIntError`).

## 10. Deliberate differences from TS

| # | Difference | Reason |
|---|---|---|
| 1 | Hooks take `ToolCallRequest`, not `(request, extra)` | D35 |
| 2 | `parameters.extra` uses the D34 shape; `url` only when absolute | D34; axum strips the mount path |
| 3 | `error.type` is a dotted Rust path; `platform` is `rust` | D10, R2 |
| 4 | No test-run detection; `AGENTCAT_ENABLED`, `testing::fake()` | D31 (R3), D41, D44 |
| 5 | `AgentCat::builder(None)` reads `AGENTCAT_PROJECT_ID` | D36 |
| 6 | 10 s HTTP timeout; `context` stripped with tracing off; customer tag cap | R8 |
| 7 | Timestamps serialize `+00:00` | Generated client (R1) |
| 8 | Explicit `shutdown()`, no signal handlers | Rust has no exit hook |
| 9 | Unknown tools produce a JSON-RPC error without a text block | rmcp behavior (as TS v2 and C#) |

Everything else follows TS, D15 included. `docs/PORTING.md` is retired by plan 1.

## 11. Engineering standards

### 11.1 Build and lints

`forbid(unsafe_code)`, `deny(missing_docs)`; clippy `-D warnings` plus `clippy::pedantic` with a
documented allow-list; `unwrap_used`, `expect_used`, `panic` and `indexing_slicing` denied outside
tests; `clippy.toml` disallows `println!`, `eprintln!`, `print!`, `eprint!`, `dbg!`,
`std::io::stdout`, `std::io::stderr`; `cargo fmt --check`; `RUSTDOCFLAGS=-D warnings`.

### 11.2 Tests

- **Parity suite:** every `port` and `adapt` row of `docs/research/06a`–`06c` (873 cases) has a test in
  `tests/parity/<ts_file>.rs` named after the TS title in snake_case. `N/A` rows (151) are listed in
  `tests/parity/NOT_PORTED.md` with reasons. A completeness test fails when a row lacks its test.
  The vacuous TS tests get strict versions.
- **Harness:** `testing::fake()` capture, `sid(label)`, the TS todo fixture texts, low-level echo
  servers, a log-capture sink, test-only queue settings, a fixed-ID generator, and mock HTTP servers
  for the API and each exporter.
- **End to end:** stdio subprocess, in-memory duplex, streamable HTTP with sessions, stateless HTTP with
  a legacy client and with a 2026-07-28 client.
- **Golden fixtures:** TS-generated vectors plus the PHP audit fixtures; 100 % match required.
- **Host safety:** stdout carries only JSON-RPC; fault injection (panicking hooks, redactors, exporters;
  unreachable API; hanging `list_tools`) never changes a tool result or error.
- **Gates:** `cargo llvm-cov` ≥ 95 % lines; `cargo mutants` ≥ 90 % core and ≥ 80 % rmcp adapter
  (nightly, whole crate) and ≥ 95 % on changed code (`--in-diff`, pull requests).

### 11.3 CI and release

- Jobs: lint, test (Ubuntu, macOS, Windows), rmcp matrix (3.0.0, 3.3.0, newest 3.x), MSRV 1.88,
  minimal versions, `cargo hack --feature-powerset check`, docs, `cargo deny`, public API and
  semver checks, coverage, mutants on changed code, packaging probe (a fresh rmcp server built
  against the packaged crate makes one tracked call), and one aggregate "all green" job as the only
  required check with branch protection on `main`.
- Nightly: newest rmcp release and rmcp `main` with the trait-drift tripwire; full mutation run.
- Dependabot for security updates only.
- `scripts/check.sh` runs the pull-request gates locally.
- Release: on a `v*` tag, check tag = crate version and a `CHANGELOG.md` entry, then `cargo publish`
  through `rust-lang/crates-io-auth-action` in a protected `release` environment, with SHA-pinned
  actions. Owner action: add the trusted publisher for `agentcat-sdk` on crates.io.

## 12. Milestones and plans

Each plan is written when the previous one is done and runs through
`superpowers:subagent-driven-development`.

| Plan | Milestones | Delivers | Exit check |
|---|---|---|---|
| 1 Foundation | M1–M3 | `DECISIONS.md` in force; section 11 gates in CI; parity harness and completeness test; PHP fixtures; parity tests for constants, IDs, tags, tokens, handles, injection, stripping; D15 reverts there | CI all green; fixtures 100 %; completeness passes for these files |
| 2 Event hygiene | M4 | Event model, redaction (D08), sanitization, truncation (TS cuts), error capture (D10, D09) | Parity tests for redaction, sanitization, truncation, exceptions |
| 3 Delivery | M5 | TS queue semantics, retries, exports per attempt, base URL last-wins, 10 s timeout, flush/shutdown | `eventQueue` parity; mock-server tests for 202/400/401/503/timeout; wire snapshots |
| 4 Engine and public API | M6–M7 | `Tracked` parity, D34, D35, D36, D40, hooks | Engine, handles, hooks, identify, tags, custom-event parity; e2e lanes |
| 5 Operations | M8 | Logging (D60/D61), diagnostics (D50), kill switch (D44), fake (D41) | Logging and diagnostics parity; kill switch and fake tests |
| 6 Exporters | M9 | D45 types and `from_json`, D18, TS Sentry behavior | Golden exporter payloads; PostHog parity |
| 7 Release | M10 | Packaging probe, nightly jobs, release workflow, README, CHANGELOG, `0.1.0` | All jobs green; `0.1.0` published |

## 13. Owner answers (2026-10-19)

| Question | Answer |
|---|---|
| Baseline for Rust | Adopt the PHP/C# decision log, including every language-agnostic PHP decision |
| `error.type` form | Dotted full path (R2) |
| Test-run detection | None; explicit opt-out only (R3) |
| D03 transport | Keep the full generated client (R1) |
| Approach | Rework in place, test first |
| Rust keeps (10 s timeout, `context` strip with tracing off, tag cap) | Approved |
| Public API, gates, tests, milestones | Approved as in sections 7, 11, 12 |

## 14. Left to the builder

Internal module layout; the queue mechanics as long as the TS semantics hold; the log writer; how
backtraces are parsed; the internals of `testing::fake()` as long as it is process-wide and restores
state on drop; benchmark harness (recommended, not required).
