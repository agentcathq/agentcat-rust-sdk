# Porting notes: Rust SDK versus the TypeScript SDK

The Rust SDK ports the TypeScript SDK at `agentcat-typescript-sdk` commit `9b56e9e`
(package 2.1.1). The rule for the port:

- Everything agents or the AgentCat dashboard can see is identical: injected parameter names,
  schemas and descriptions, the mint-back text, `mcp_session`, tag names, event fields,
  intent capture, `get_more_tools`, session derivation, token estimates, sanitization and
  truncation markers. `tests/ts_parity.rs` checks these against golden vectors that
  `scripts/ts-fixtures/generate.sh` computes by running the TypeScript code.
- Internal defects of the TypeScript SDK are fixed. Each fix is listed here with its test.

## Shape of the port

| TypeScript | Rust |
| --- | --- |
| `track(server, projectId, options)` | `AgentCat::builder(project_id)…build().track(handler)` → `Tracked<H>`, an rmcp `ServerHandler` |
| Monkey-patched `tools/list` and `tools/call` handlers, inner tap, registration patch | `Tracked<H>` overrides `list_tools`, `call_tool` and `get_tool` and forwards the other `ServerHandler` methods. `tests/trait_drift.rs` fails when rmcp adds a method the wrapper does not forward. |
| Per-server `WeakMap` registries, rebuilt from `tools/list` on demand | Per-instance registry in the wrapper. A call on a fresh instance (stateless HTTP) uses the inner `get_tool`, then the inner `list_tools` (5 s budget, timed on the publisher thread) |
| Hooks get `(request, extra)` | Hooks get a `ToolCallRequest`: the raw `CallToolRequestParams` and the rmcp `RequestContext` (`http_request()`, `header()`, `meta()`) |
| `parameters.request` (the raw JSON-RPC message) | `{ method, params: { name, arguments, _meta?, inputResponses?, requestState? }, jsonrpc, id }` |
| `parameters.extra` (v2 `ServerContext`) | `{ sessionId?, mcpReq: { id, method }, http?: { req: { method, url, headers } } }`; `url` is the URI the handler sees |
| `response` (the handler's result) | rmcp's `CallToolResult` as JSON, without the protocol-level `resultType` (rmcp sets `isError: false` on success results) |
| `platform: "javascript"`, V8 stack parsing | `platform: "rust"`; panics are captured with a parsed `std::backtrace` (module, function, file, line, `in_app`); a JSON-RPC error becomes `type: "McpError"`, `message: "MCP error <code>: <message>"`, like the TS SDK's `McpError` |
| `sdk_language: "TypeScript"` | `sdk_language: "Rust"` |
| Event queue on the event loop; signal handlers and `beforeExit` flush | A dedicated thread with its own tokio runtime. Call `agentcat::shutdown(timeout)` (or `shutdown_blocking`) before exit; the SDK installs no signal handlers |
| `~/agentcat.log` | `~/agentcat.log` (or `$AGENTCAT_LOG_FILE`) and `tracing` events with target `agentcat` |

## Fixed TypeScript defects

| # | TypeScript behavior | Rust behavior | Test |
| --- | --- | --- | --- |
| 1 | Retries every failure, 400/401/415 included | Retries only network errors, 408, 429, 500, 502, 503, 504; 1 s/2 s/4 s with ±20 % jitter, same event ID | `delivery::does_not_retry_client_errors`, `delivery::retries_server_errors_with_the_same_event_id` |
| 2 | A 2xx response with a non-JSON body counts as a failure and is retried | Any 2xx is success | publisher `send_once` |
| 3 | No HTTP timeouts | 5 s connect, 10 s per request | publisher `build_client` |
| 4 | Exporters run again on every AgentCat retry | Each event is exported once | `exporters::every_exporter_receives_each_event_once` |
| 5 | Wire body repeats every field in camelCase | snake_case only | `tool_calls::first_call_issues_a_session_and_publishes_one_event` |
| 6 | Names truncated to 256 + `...` (259 chars), versions too; the API columns are 255 and 50, so such events fail to store | Names cap at 255 characters, versions at 50 | `ts_parity::truncation_matches` |
| 7 | Tag keys may contain a space or start with `agentcat:`; the API drops them later | Validated with the API's rules up front, with a log line | `validation` unit tests, `hooks::tags_are_validated_and_sdk_tags_win` |
| 8 | `identifyActorData` is string-redacted (the protected list names `identifyData`) | Actor data is protected, like the other identity fields | `hooks::string_redaction_applies_to_payloads_but_not_protected_fields` |
| 9 | The string redactor sees content-block `type` discriminators and SDK metadata | Skipped | same test |
| 10 | Custom events get no string redaction | The tracked configuration's `redact_sensitive_information` applies | `AgentCat::publish_custom_event` |
| 11 | Custom events are dropped when `enableTracing` is false | Custom events are explicit calls and always publish | `AgentCat::publish_custom_event` |
| 12 | `publishCustomEvent(server, projectId, …)` ignores its `projectId` | `agentcat.publish_custom_event(event)` takes no project ID; the free function takes one and uses it | `delivery::*custom*` |
| 13 | Custom-event session IDs over 31 characters make the API drop the event | The event is kept and sent without a session, with a log line | `delivery::overlong_custom_session_ids_are_dropped_not_the_event` |
| 14 | `duration` includes the registry rebuild; a 0 ms duration is replaced by a recomputed value | Timed around the handler only; 0 is kept | `tracked.rs` |
| 15 | API base URL: the last `track()` wins | First wins, like exporters and diagnostics | `AgentCatBuilder::build` |
| 16 | With tracing off, `context` is still injected but not stripped | Stripped | `tool_calls::tracing_disabled_publishes_nothing_and_still_strips_context` |
| 17 | Paginated listings append `get_more_tools` to every page; the rebuild reads page 1 only | Appended on the last page; the rebuild reads every page | `tracked.rs` `inject_list`, `rebuild_registry` |
| 18 | Composed-schema tools (`oneOf`/`anyOf`/`allOf`) never advertise the handles, but echoed handles still reach the handler | Echoed `session_id`/`agent_id` are stripped for them, unless any branch or definition of the schema declares that name (then it is the customer's argument) | `composed::a_branch_declared_session_id_reaches_the_handler`, `inject` unit tests |
| 19 | A `{}` input schema gains `properties` but no `type` | Also gets `"type": "object"` (first key), which MCP requires | `ts_parity::schema_injection_matches` |
| 20 | Sentry transactions start one duration too early; error events name their own span as parent | Start at the request start; no `parent_span_id` | `exporters::sentry` unit tests |
| 21 | `destroy()` silently disables the queue forever after `beforeExit` | Explicit `shutdown()`; `flush()` never closes the queue | `shutdown::shutdown_drains_queued_events_then_drops_new_ones` |
| 22 | The queue drops the oldest event when full, with no memory bound | Drops the newest when 10,000 events wait or the untruncated events held add up to 32 MiB (an event counts until it is truncated; one large event is still accepted and truncated), with a counted log line. Exports are capped at 64 in flight; extra exports are dropped and counted | publisher `enqueue`, `export`; `tool_calls::large_calls_still_publish_a_truncated_event` |
| 23 | An identity without a user ID still stamps empty actor fields | An identity needs a user ID, like the Go SDK | publisher `apply_pending` |
| 24 | A second `track()` on a tracked server is a no-op (TS) | Same: `track(track(h))` forwards without injecting or publishing twice | `tool_calls::tracking_twice_does_not_inject_or_publish_twice` |
| 25 | A non-string `context` argument (number, object) makes truncation throw, and the event is dropped | Recorded as its JSON text | `tracked.rs` `intent_of` |
| 26 | The API keeps the first 50 tags; TS sends customer tags first, so SDK tags are the ones dropped | The request map has no order, so customer tags are capped at 50 minus the SDK tags before the merge | publisher `apply_pending` |
| 27 | `agentcat_protocol_version` copies the client's value unclamped; the API drops values over 200 characters or with newlines | Clamped like the agent tag | `handles::build_handle_tags` |
| 28 | `timestamp` and `duration` include the registry rebuild | `timestamp` is the request's arrival, at millisecond precision; `duration` times the handler only | `tracked.rs` `call_tool` |
| 29 | Token estimates count `JSON.stringify` bytes | Same bytes: numbers are counted as JavaScript writes them (`10`, not `10.0`) | `tokens` unit tests |

Custom events published through `AgentCat::publish_custom_event` carry `duration: 0` when none
is given (as TS), the server name from `server_info()` or from the last tool call, and no client
name (TS read the connection's client; a Rust `AgentCat` is not tied to one connection).

## Kept on purpose (visible behavior)

- A customer-declared `context` argument is recorded as intent, and a customer-declared
  `agent_id` becomes the agent tag when agent tracking is on (TS quirks 01-§15 #6 and #7).
- A server with no tools advertises no `get_more_tools`.
- The derived-session timestamp offset (at most about 49.7 days after 2024-01-01).
- Diagnostics severity: only `Warning:` lines are WARN.
- Sanitization replaces any string of 10,240+ characters that matches the base64 alphabet,
  even plain letters.
- Inbound HTTP headers are published verbatim, including `authorization` and `cookie`
  (product decision, 2026-10-19). Use `redact_event` to remove them.

## Not ported

- TS MCP SDK v1/v2 shape detection, `setRequestHandler` patching, the registry proxy: not
  needed with a typed wrapper.
- Cloudflare Workers `waitUntil` and Node signal handling: Rust servers call `shutdown`.
- rmcp 0.x, 1.x and 2.x: their `ServerHandler` traits differ (see
  `docs/research/04-rust-mcp-usage.md`).
