# AgentCat Rust SDK: decision log

This log records every decision that shapes the Rust SDK. It has three parts:

1. **R-decisions**: the owner's answers for Rust (attempt 2, 2026-10-20).
2. **C-decisions**: calls the designer made where no owner answer exists. The owner can overturn any of them.
3. **The PHP decision log mapped to Rust**: every row of `agentcat-php-sdk/docs/audit/DECISIONS.md` with its Rust
   ruling. Details and sources are in [`research/02-decision-matrix.md`](research/02-decision-matrix.md).

Precedence: an R-decision beats a C-decision, and both beat the PHP log. The behavior baseline is the TypeScript SDK
at `9b56e9e` (see [`research/01-ts-sdk-behavior.md`](research/01-ts-sdk-behavior.md)). Every place where the Rust SDK
acts differently from that baseline is listed in the deviation register of the design
([`superpowers/specs/2026-10-20-rust-sdk-design.md`](superpowers/specs/2026-10-20-rust-sdk-design.md), section 9).

## 1. Owner decisions (R)

| ID | Decision | Owner's answer |
|---|---|---|
| R01 | Start point | A clean slate. Attempt 1 is ignored completely and archived under the tags `attempt-1`, `attempt-1-plan-1a`, `attempt-1-plan-1b`. Main was reset to one empty root commit. |
| R02 | Baseline | The latest TypeScript SDK is the behavior baseline. The PHP decision log is the starting point for shaping the SDK. Every Rust-specific deviation is flagged to the owner. |
| R03 | API client | Use the generated Rust client `agentcat-api` (reqwest), not a models-only client as PHP and C# chose. |
| R04 | First-pass scope | Core first: tool calls, sessions, intent, identify, custom events, delivery. The exporters (OTLP, Datadog, Sentry, PostHog) come in a second pass. No crates.io release in the first pass. |
| R05 | rmcp versions | rmcp 3.x only. No 1.x or 2.x release lines. |
| R06 | Shutdown | Delivery runs on one SDK-owned thread with its own runtime. When the last handle drops, the SDK drains the queue for at most 5 s. Public `flush` and `shutdown` also exist. No signal handlers. |
| R07 | JSON key order | `mcp_session` must be the first key of `structuredContent`, because some agent harnesses truncate it out otherwise. The SDK always turns on `serde_json/preserve_order`, and a test guards the order. |
| R08 | TLS | rustls is the default. native-tls is an opt-in feature. |
| R09 | `error.type` | For an `Err(McpError)`: the JSON-RPC error code name. A `capture_error(&err)` helper records the Rust type path, the `Display` text and the `source()` chain when the customer opts in. No global panic hook. |
| R10 | Customer test suites | TS behavior: no automatic test detection. The SDK stays on in tests; `AGENTCAT_ENABLED=false` turns it off. This overrides PHP D31 and P5. |
| R11 | `agentcat-api` version | Exact pin `0.0.3`, bumped on purpose with SDK releases. |
| R12 | Dashboard demo | Events go to the prod API, project "Testing" (`proj_2yAR5QHJ3CE3icTpJlKJqu1ZiXG`), and are read back with the AgentCat MCP server. |
| R13 | First version | `1.0.0` (PHP D63). |
| R14 | Ingest replay | The replay of captured bodies through `agentcat-server`'s ingest code runs locally only. CI checks bodies against a pinned copy of the OpenAPI schema. |
| R15 | Review | One PR at the end, from `rewrite` into `main`. |
| R16 | Parity evidence | Side-by-side diff tests against the TS SDK, a real dashboard demo, and a TS test-by-test map. |
| R18 | Private APIs of integrated libraries | The owner prefers reflection and patching of private members when it makes the code meaningfully cleaner, with rigorous monitoring of new releases. Rust form: see D21. |
| R17 | Rulings during the build | For an unforced TS-vs-Rust difference, the builder records a provisional ruling and keeps building. At the end of each milestone, the builder brings that batch to the owner as multiple choice; changes land in the next milestone. |
| R19 | Shared handler on several servers (milestone 1-2 batch, 2026-10-21) | At `9b56e9e`, a tool handler function shared by two v1 servers is wrapped only on the first server, so errors on later servers lose their stack. Rust does not copy this: every server keeps the stack. Recorded as RD-21. |
| R20 | Harness answer key (milestone 1-2 batch, 2026-10-21) | The TS v2 reference server is the answer key on both protocol lanes, 2025-11-25 and 2026-07-28. The v1 server stays in the harness but is not what the Rust server is diffed against. Plan 3 makes v2 the default reference server and generates the tool contract from it. |
| R21 | Nightly schedule (milestone 1-2 batch, 2026-10-21) | `main` stays untouched until the final PR (R15). The scheduled nightly drift sweep starts when that PR merges; until then it runs on pushes to `rewrite` that change the oracle, the harness or the workflow. |
| R22 | Float parsing (plan 2, 2026-10-21) | serde_json's `float_roundtrip` feature stays off. Without it, a decimal with 16 or 17 significant digits can parse to the neighboring `f64`, so a recorded number or a token estimate can differ from TS by one digit. The owner accepts this for the smaller and faster parser. Recorded as RD-23. |
| R23 | JavaScript object-model artifacts (plan 2, 2026-10-21) | Rust does not copy TS results that come from the JavaScript object model rather than from SDK logic, as in R19. A `__proto__` key in customer JSON is kept (TS drops it while it copies objects), and an array tool `response` stays an array (TS turns it into an object with index keys). Recorded as RD-24 to RD-28 (RD-26 to RD-28 confirmed in R24). |
| R24 | Milestone 3 batch (plan 2, 2026-10-21) | RD-26: Rust treats every object in a truncated text field (`userIntent`, `resourceName`, the server and client fields, `error.message`) and in `error.frames` as an error and drops the event, also when TS lets an object with a small `length` through. The owner files this under R23, because the gap comes from JavaScript duck typing, not from SDK logic. The owner also confirms RD-27 and RD-28 under R23. |
| R25 | rmcp version matrix (2026-10-22) | Like the TS SDK's MCP SDK compatibility workflow, CI runs the test suite against every rmcp 3.x minor release at its latest patch, read from crates.io at run time, on every push and pull request. This replaces "floor and latest only" (design §11 gate 10). Plan 3 adds it, because rmcp joins the crate there. |
| R26 | Docs site (2026-10-22) | The Rust SDK gets pages on docs.agentcat.com: Rust tabs next to TypeScript, Python and Go in the Mintlify site (`agentcathq/agentcat-ui`, `apps/docs`), in the quickstart and in every `sdk/*.mdx` page that shows code. Plan 5 writes them as a pull request to that repository, which the owner approves at that time. |

## 2. Designer calls (C)

| ID | Call | Reason |
|---|---|---|
| C01 | The wrapper `Tracked<H>` implements rmcp's `ServerHandler`, not `Service<RoleServer>`. | `StreamableHttpService::new` accepts only `S: ServerHandler` (rmcp 3.6 `tower.rs:1408-1413`). A CI canary covers the risk of new defaulted trait methods. |
| C02 | `get_tool` returns the schema the client saw in `tools/list` (with the injected parameters) for customer tools, and the SDK descriptor for `get_more_tools`. | rmcp uses `get_tool` to validate SEP-2243 `Mcp-Param-*` headers against what the client was shown. |
| C03 | The call path learns a tool's original schema from `inner.get_tool(name)` first, then from the merged listing registry, then from a listing rebuild (5 s budget, all pages), then from the TS heuristic strip. | The result equals TS's correct behavior, and it removes the K-01/K-02 failure modes, as PHP D66 asks. |
| C04 | Token estimates and the 100 KB size check serialize JSON exactly as JavaScript's `JSON.stringify` does (number format and integer-like key order). | Token counts must match TS exactly; the vectors prove it. |
| C05 | Remote SDK diagnostics move to the second pass with the exporters. | They are not core, and they ship every log line (PHP D07), which needs its own review. |
| C06 | The license is MIT, like the TypeScript SDK. | One license across the AgentCat SDKs. |
| C07 | SDK log lines go to `~/agentcat.log` by default (TS format). An option routes them to `tracing` instead. | PHP D30: many stdio servers install a `tracing` subscriber that writes to stdout, which would corrupt the JSON-RPC stream. |
| C08 | `serde_json` and `rmcp` types may appear in the public API; `reqwest`, `tokio`, `chrono`, `agentcat-api` and `http-body` types may not. | rmcp's own public types expose `serde_json::Map`, so hiding it gains nothing. |
| C09 | Building the client cannot fail. A bad option logs one line and falls back to the default. | TS `track()` never throws (B-002); PHP D32. |
| C10 | Settings belong to one `AgentCat` handle, not to the process (except the log destination, first wins, PHP D38). | Rust has explicit handles; TS's process-global last-wins API URL (K-09) has no reason to exist in Rust. |
| C11 | The session-string custom-event form is a method on the `AgentCat` handle and honors its `project_id` argument. | No global client in Rust (practices rule 7). |
| C12 | After an explicit `shutdown()`, later events on that handle are dropped with the TS log line `Queue is shutting down, event dropped`. `flush()` does not stop the queue. | TS `destroy()` semantics. |
| C13 | The verification layers L1–L4 of `research/06-verification-strategy.md` are adopted, with TS-derived vectors generated from the public TS repository at a pinned SHA. | The owner named too little verification and drift from TS as the reasons attempt 1 was scrapped. |
| C14 | Customer hooks run on the customer's tokio runtime (captured as a `Handle` when the handle is built or the call starts), never on the SDK thread. | Hooks are customer code and may use the customer's runtime resources. |
| C15 | The Rust SDK keeps TS behavior when the K-bugs are reachable (PHP D15), except where this log says otherwise (D66, and the forced differences in the deviation register). | Cross-SDK parity. |
| C16 | The TS server form of custom events lives on the per-call context inside a tool (that call's identity) and on `Tracked<H>` (the identity that instance captured at `initialize`, restored on a `SessionRestoreMarker` replay). The bare handle has only the session-string form. | One handle serves many concurrent HTTP sessions, so a handle-level "last client" would be wrong. |
| C17 | Every push to `rewrite` runs the gates on Linux; the final PR and nightly add macOS, Windows and the full harness matrix. Mutation testing compares against the previous green commit. Making `all-green` a required check is the owner's admin step. | CI minutes on a private repo; `main` is an empty root commit, so a diff against it would mutate the whole crate on every push. |
| C18 | The 2026-07-28 harness lane uses a second TS reference server built on `@modelcontextprotocol/server` 2.x. | A v1 `McpServer` cannot serve that protocol. |
| C19 | One tool registry per `AgentCat` handle and inner handler type (`TypeId`), merged per tool name across pages and instances. A tool found through `inner.get_tool` takes its foreign status from its current schema; the set of tools that declare `session_id` only grows. | Design §6.3 and RD-12; TS grows that set the same way. |
| C20 | `get_more_tools` is appended to every non-empty listing page. | TS does the same (B-048). |
| C21 | The mirrored `structured_content` has JavaScript's key order: array-index keys ascending, then `mcp_session`, then the customer's other keys in their order. R07's "first key" holds for every key that is not an array index, which is what the harness rule `mcp-session-first` checks. | It copies TS's wire bytes. |
| C22 | For an HTTP request without an `mcp-session-id` header, the client identity skips rmcp's initialize-time `peer_info`. | rmcp fabricates `rmcp@<version>` there, and TS's per-request server never saw `initialize` either, so both have no client identity. |
| C23 | A panic gives the error `type: "panic"`. Otherwise an error recorded with `capture_error` during the call replaces both the result-derived error (B-097) and the JSON-RPC code name (R09). The Rust error shape has no `frames` (RD-06). | The captured error is the customer's own, and it is the most specific one. |
| C24 | JSON-RPC code names: -32700 `ParseError`, -32600 `InvalidRequest`, -32601 `MethodNotFound`, -32602 `InvalidParams`, -32603 `InternalError`, -32002 `ResourceNotFound`; any other code `JsonRpcError(<code>)`. | One fixed name per code for the error `type` (RD-05). |
| C25 | `negotiate_initialize` is not forwarded. A customer override of it is kept on the normal path: `Tracked::initialize` forwards to the inner handler's `initialize`, and rmcp's default `initialize` calls `negotiate_initialize` on the inner type (rmcp 3.7.0 `handler/server.rs:323`). Only a direct call of `negotiate_initialize` on the `Tracked` wrapper uses rmcp's default. The rmcp canary allowlists it with this reason, and design §13 lists the risk. | rmcp added it in 3.3.0, so forwarding it breaks the 3.0 floor. |
| C26 | The call path builds the `ToolCall` snapshot and spawns the hooks on the customer's runtime (C14) when the call starts. Plan 4 settles them before the event is sent. | Hooks never delay a tool call. |
| C27 | rmcp's `local` feature is not supported. Plan 5 documents it. | Hooks are spawned as `Send` tasks. |
| C28 | The protocol-version tag is read from the request `_meta` only (B-067), never from `RequestContext::protocol_version()`. | That method falls back to the negotiated version, which TS does not do. |
| C29 | `CallContext` is inserted on every request path that forwards to the inner handler, also with tracing off and on the degraded path (then with no session and no agent). | The SDK must never change a tool's result (design §7.4); an option must not make `Extension<CallContext>` fail with `missing extension`. |
| C30 | The panic boundary also covers the SDK's own calls into customer code on the call path: the rebuild's `list_tools`, the `get_tool` lookup and `get_info`. A panic there is logged (the TS rebuild-failure text for the rebuild) and the call goes on without that data. | A customer panic must not fail a call that the customer's own handler can serve. |
| C31 | A `get_tool` lookup runs the injection on a clone of the registry's state and writes nothing back. | A lookup before the first listing must not use up the one-time conflict and shadow logs, which TS writes on the listing. |
| C32 | A tracked server needs a tokio runtime with the time driver: the 5 s registry rebuild uses `tokio::time::timeout`. Plan 5 documents it. | rmcp's own service uses tokio timers too; a timer-free fallback costs a thread per rebuild. |
| C33 | Harness: the tool definitions live once as `TOOLS` in `harness/src/servers/common.mjs`. `dump-contract.mjs` generates `harness/contract/tool-contract.json` from the `ts-v2` server (R20); the Rust reference server embeds that file; a test keeps `ts-v1`'s zod tools equal to it. | A contract that its generator reads back is no answer key. |
| C34 | Harness: host-library differences stay out of the MCP-wire delta. The Rust server's stateful `http` mode answers 2025-era POSTs with rmcp's SSE stream (rmcp serves JSON only without session mode), and the normalizer rule `server-info-meta-drop` drops the `_meta` server info that the TS v2 host adds to every 2026-07-28 result (RD-13). | The harness measures what AgentCat changes, not the host's framing. |
| C35 | Harness: a delta ruling can name a scenario, and `matrix.mjs` applies it only to that scenario's runs. | A delta path names a transcript index, which is a different call in each scenario. |

## 3. The PHP decision log mapped to Rust

Verdicts: **ADOPT** (as PHP decided), **ADAPT** (same intent, Rust mechanism), **N/A** (PHP framework or runtime item
with nothing to port), **OVERRIDDEN** (an R- or C-decision replaces it).

| PHP ID | Title | Rust ruling |
|---|---|---|
| D01 | `sdk_language` value | ADAPT: `sdk_language = "Rust"`, `error.platform = "rust"`. |
| D02 | `agentcat:custom` in the API enum | ADOPT: present in `agentcat-api` 0.0.3. |
| D03 | Transport and API client | OVERRIDDEN by R03. The SDK still owns the delivery policy: its own `reqwest::Client` with timeouts and a branded User-Agent, no API key, TS retry rules. |
| D04 | When hooks run | ADAPT: TS semantics. Hooks start when the call starts, run concurrently with the tool, and get 30 s each in the pipeline (C14). |
| D05 | Public `flush()` / `shutdown()` | OVERRIDDEN by R06. |
| D06 | HTTP headers in events | ADOPT: every header verbatim, `Authorization` and `Cookie` included, from `http::request::Parts`, named and combined as TS's WHATWG `Headers` gives them (design §6.6; non-UTF-8 values: RD-30). |
| D07 | Diagnostics content | ADOPT, for the second pass (C05). |
| D08 | Redaction scope | ADOPT: TS scope, K-04 included. |
| D09 | String length unit | ADOPT: UTF-16 code units; a cut inside a surrogate pair drops the whole character. |
| D10 | `error.type` | OVERRIDDEN by R09. |
| D11 | Libraries in the first release | ADAPT: rmcp only. |
| D12 | Language minimum | ADAPT: Rust 1.88 (rmcp's MSRV), edition 2024. |
| D13 | Entry point | ADAPT: free function `agentcat_sdk::track(handler, project_id, options)` plus a cloneable `AgentCat` handle with `.track(handler)` for HTTP factories. The wrapper implements `ServerHandler` (C01). |
| D14 | Options type | ADAPT: an `Options` type with private fields, `Default` = TS defaults, and consuming chained setters. |
| D15 | Copy TS bugs | ADOPT (C15), with the forced differences in the deviation register. |
| D16 | Session block on unknown tools / validation errors | ADAPT: the block goes wherever rmcp produces an `is_error` result, argument deserialization failures included (rmcp 3.6 turns those into `is_error` results). rmcp's JSON-RPC errors (unknown tool, `Err(ErrorData)`) carry no block, as in C# and TS v2. |
| D17 | Tool-injection cache | ADAPT: in-memory registries on the shared handle, merged per tool (C03). |
| D18 | Exporter identity strings | ADOPT, for the second pass. |
| D19 | Backend tickets | ADOPT: no new ticket. Types prevent both bad inputs (identify data is an object; a sessionless event sends `session_id: null`). |
| D20 | API client version constraint | OVERRIDDEN by R11. |
| D21 | Private members of integrated libraries | ADAPT (R18): the owner prefers private access when it makes the code meaningfully cleaner, guarded by monitoring. In Rust this means: use rmcp's `#[doc(hidden)]` and other semver-exempt public items freely when they give cleaner code, from one internals module, guarded by the rmcp version matrix and the nightly canary. Reading truly private fields needs `unsafe` layout assumptions, which is undefined behavior in Rust (no reflection, no stable layout), so it stays excluded (`forbid(unsafe_code)`). |
| D22 | Static analysis | ADAPT: clippy pedantic with the restriction lints of the design, rustdoc with warnings denied, cargo-deny, public-API snapshot. |
| D23 | Testing framework | ADAPT: `cargo test` / nextest, wiremock, insta, proptest, cargo-mutants. |
| D24 | Code style | ADAPT: rustfmt defaults plus clippy. |
| D25 | Enforcement on commits | ADOPT: the same gates run locally (a check script) and in CI. |
| D26 | Merge gates | ADAPT: see the design, section 11. |
| D27 | Automatic framework integration | ADAPT: one-line wrap of the handler; no auto-discovery. |
| D28 | Packaging | ADAPT: one published crate, `agentcat-sdk`. |
| D29 | Namespace root | ADAPT: `agentcat_sdk::`. |
| D30 | Logging | ADAPT (C07). |
| D31 | Customer test suites silent | OVERRIDDEN by R10. |
| D32 | Never throw from runtime telemetry | ADAPT: never panic on runtime paths; building cannot fail (C09); only the custom-event methods return `Result`. |
| D33 | No special disclosure of D06/D07 | ADOPT. |
| D34 | `parameters.extra` shape | ADOPT: `{sessionId?, requestId, _meta?, requestInfo?: {headers, url}}` from `RequestContext`; no `authInfo`. |
| D35 | Hook argument | ADAPT: an owned `ToolCall` snapshot (`Clone + Send + Sync + 'static`). |
| D36 | Missing project ID | ADAPT: `""` means telemetry only; `AgentCat::from_env` reads `AGENTCAT_PROJECT_ID`. |
| D37 | MCP library version constraint | OVERRIDDEN by R05: `rmcp = "3"`, floor 3.0.0. |
| D38 | Logger with several servers | ADOPT: the log destination is process-global, first wins. |
| D39 | SIGTERM / SIGINT drain | OVERRIDDEN by R06: no signal handlers. |
| D40 | Custom event input | ADAPT: a `CustomEvent` type with setters; validation errors carry the TS messages verbatim (C11, C16). |
| D41 | Test fake | ADAPT: an instance-scoped `testing::Fake` behind the `test-util` feature, because `cargo test` runs tests in parallel threads. |
| D42 | `Options::fromArray()` keys | N/A. |
| D43 | `flush()` returns `bool` | ADOPT: `true` when the queue drained before the timeout. |
| D44 | `AGENTCAT_ENABLED` kill switch | ADAPT: a false value turns the SDK off (pass-through wrapper, no thread). A true value has no extra effect, because R10 removes test detection. |
| D45 | Exporter configuration types | Second pass. |
| D46 | Switch for the signal drain | N/A (R06). |
| D47 | Diagnostics delivery under PHP-FPM | N/A. |
| D48 | Diagnostics resource attributes | Second pass. |
| D49 | Diagnostics timeouts | Second pass. |
| D50 | Diagnostics environment names | Second pass. |
| D51–D59 | Laravel, Symfony and PHP runtime items | N/A. |
| D60, D61 | Default log destination | ADAPT (C07). |
| D62 | Content-Length on symfony/mcp-bundle answers | N/A. |
| D63 | First release version | ADOPT (R13): `1.0.0`. |
| D64 | Where the docs live | ADAPT: README, rustdoc on docs.rs, and pages on docs.agentcat.com (R26). |
| D65 | README images | ADOPT: images stay out of the published crate (`include` list). |
| D66 | B10 pagination fix | ADOPT (C03). |
| D67 | Release steps | Second pass: crates.io trusted publishing. |
| D68 | Symfony Flex recipe PR | N/A. |
| P5 | Diagnostics in test environments | OVERRIDDEN by R10; second pass. |
| B09 | `publishCustomEvent` quirks | ADOPT. |
| Bug policy | A Rust fix of a TS bug fixes Rust only | ADOPT. |
