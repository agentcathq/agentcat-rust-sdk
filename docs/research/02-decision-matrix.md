# PHP decision log → Rust decision matrix

Date: 2026-10-20. Research only; no repository was changed.

Sources:

- PHP decision log: `/home/ubuntu/agentcat-php-sdk/docs/audit/DECISIONS.md` (D01–D68, rounds 1–10), the questions in
  `docs/audit/README.md` §6, and the plans in `docs/superpowers/plans/` where a rationale was missing.
- C# design: `/home/ubuntu/agentcat-csharp-sdk/docs/superpowers/specs/2026-10-19-csharp-sdk-design.md` (§4 carries
  the PHP decisions over) and `/home/ubuntu/agentcat-csharp-sdk/docs/audit/DECISIONS.md` (Q1–Q5).
- TS SDK `main` @ `9b56e9e` (to confirm TS behavior).
- `~/2026-07-28-cross-sdk-changelog.md`: context only. It is stale (it describes `task_id` and `_mcp_instructions`).
- Rust facts: the rmcp source at `~/misc/rmcp` (tag `rmcp-v3.6.0`), the published `agentcat-api` 0.0.3 crate in
  `~/.cargo/registry`, and the crates.io API (queried 2026-10-20).

## Legend

- **Class.** *Behavior* means language-agnostic behavior: what the agent, the wire or the customer sees.
  *PHP-specific* means packaging, runtime or framework. *Process* means how the team works or releases.
- **C#.** *Same*: C# applied the decision as written. *Adapted*: same intent, different mechanism. *Deviated*: different
  outcome. *N/A*: the C# design marks the decision as PHP-only. *Not addressed*: the C# design is silent, or it was
  written before the decision existed (D63–D68 are from 2026-10-20; the C# design is from 2026-10-19).
- **Rust verdict.** *ADOPT*: take the decision as it is. *ADAPT*: keep the intent with a Rust mechanism, which the
  section describes. For a PHP-only decision with no Rust counterpart, the verdict is "ADAPT (N/A: drop)".
  *OWNER DECISION NEEDED*: Rust forces a real choice. The section gives options and a recommendation.
- **[API]** marks a decision that the owner's Rust input changes: the SDK uses the generated `agentcat-api` crate,
  which depends on reqwest, and not a models-only client.

## 0. Rust facts this matrix relies on

1. **rmcp 3.6.0** (released 2026-10-20): edition 2024, `rust-version = "1.88"`. `ServerHandler: Sized + Send + Sync +
   'static` (unless rmcp's `local` feature is on). rmcp has a blanket `impl<H: ServerHandler> Service<RoleServer> for H`
   (`handler/server.rs:50`). `Service<RoleServer>` has four methods: `handle_request`, `handle_notification`,
   `get_info` and `supported_protocol_versions` (`service.rs:269`). `RequestContext { ct, id, meta, extensions, peer }`
   is `Clone` and `#[non_exhaustive]` (`service.rs:1192`). `StreamableHttpService` puts `http::request::Parts` into
   `extensions` (`transport/streamable_http_server/tower.rs:1276`). The tool router answers an unknown tool with the
   JSON-RPC error `invalid_params("tool not found")` (`handler/server/router/tool.rs:566`). The `#[tool_handler]` macro
   lists every tool in one page (`next_cursor: None`). rmcp never calls `catch_unwind`.
   `rmcp::model::Implementation::from_build_env()` returns rmcp's own crate name and version, because `env!` expands
   inside rmcp (`model.rs:1718`).
2. **rmcp cadence and adoption.** rmcp 1.0.0 shipped on 2026-03-03, 2.0.0 on 2026-06-29 and 3.0.0 on 2026-07-28: three
   majors in five months. Downloads in the 30 days to 2026-10-20 (sampled per version, about 10.2 M in total): 3.x at
   least 6.5 M (about 63 %), 1.x at least 1.4 M, 2.x at least 0.97 M, 0.x at least 0.5 M. rmcp follows SemVer and
   counts an MSRV raise as breaking (`VERSIONING.md`).
3. **`agentcat-api` 0.0.3** (crates.io, 2026-10-19), the owner's chosen client [API]:
   - Dependencies: `reqwest ^0.13` (default features off; `json`, `multipart`, `query`, `form`), `chrono`,
     `serde_with`, `serde_repr`, `serde_json`. Its features are `default = ["native-tls"]` and `rustls`.
   - `Configuration` has public fields. By default, `user_agent` is `"OpenAPI-Generator/v1/rust"` and `client` is
     `reqwest::Client::new()`, which has no timeout.
   - `events_api::publish_event` builds the body with `.json()`, so the body has a `Content-Length` and is never
     chunked. It sets `Authorization` only when `api_key` is set. It returns `Error::Reqwest` for a network error or
     timeout and `Error::ResponseError { status, .. }` for an HTTP error.
   - `PublishEventRequest` fields:
     - `session_id: Option<Option<String>>` (serde_with `double_option`): it can send an explicit `null`.
     - `timestamp: Option<DateTime<FixedOffset>>`: chrono writes RFC 3339 with `Z` for a zero offset and
       `SecondsFormat::AutoSi`, so `….123Z`, but no fraction at all when the milliseconds are 0.
     - `tags: Option<HashMap<String, String>>`: the key order on the wire is random.
     - `duration`, `input_tokens` and `output_tokens` are `Option<i32>`.
     - The free-form fields are `serde_json::Value`.
     - `EventType` has `AgentcatColonCustom` (`"agentcat:custom"`).
   - rmcp's client features use reqwest 0.13.2 too, so the build has one reqwest major.
4. **crates.io names.** `agentcat` belongs to an unrelated project (rpodgorny/agentcat 0.1.0). `agentcat-sdk` and
   `agentcat_sdk` are reserved by `agentcathq` (0.0.1 placeholders, 2026-08-23).
5. **serde_json.** `Map` is a `BTreeMap` (sorted keys) unless the `preserve_order` feature is on, which makes it an
   `IndexMap` (insertion order). Cargo unifies features, so that feature is global to the customer's build. rmcp
   types tool schemas and `structuredContent` as `serde_json::Map`. Rust strings are always valid UTF-8, a lone
   surrogate cannot exist, and `serde_json::Value` cannot hold NaN or infinity. So most of PHP's wire-normalization
   layer (P4) disappears.
6. **Runtime.** rmcp needs tokio.
   - A tokio signal listener replaces the default action for that signal for the rest of the process.
   - `cfg(test)` applies only to the crate under test, never to a dependency.
   - `cargo test` runs the tests of one binary on parallel threads of one process.
   - Test binaries live at `target/<profile>/deps/<name>-<hash>`.
   - `std::process::exit` and `panic = "abort"` skip destructors.
   - Rust has no "before exit" hook.

## 1. Summary table

| ID | Title | Class | Rust verdict |
|---|---|---|---|
| D01 | `sdk_language` value | Behavior | ADAPT (`Rust` / `rust`) |
| D02 | `agentcat:custom` in the API enum | Behavior (backend) | ADOPT (already in `agentcat-api` 0.0.3) [API] |
| D03 | Transport and API client | PHP-specific (packaging) + Behavior | ADAPT (generated reqwest client, SDK-owned policy) [API] |
| D04 | When hooks run | PHP-specific (runtime) | ADAPT (TS semantics: spawned at call start) |
| D05 | Public `flush()` / `shutdown()` | Behavior + runtime | OWNER DECISION NEEDED (shutdown model) |
| D06 | HTTP headers in events | Behavior (privacy) | ADOPT |
| D07 | Diagnostics content | Behavior (privacy) | ADOPT |
| D08 | Redaction scope | Behavior | ADOPT |
| D09 | String length unit (UTF-16) | Behavior | ADOPT |
| D10 | `error.type` | Behavior | OWNER DECISION NEEDED (no exception classes) |
| D11 | Libraries in the first release | PHP-specific (framework) | ADAPT (rmcp only) |
| D12 | PHP minimum | PHP-specific (runtime) | ADAPT (MSRV = rmcp's, 1.88) |
| D13 | Entry point | Behavior (public API) | OWNER DECISION NEEDED (`track` returns a wrapper) |
| D14 | Options type | Behavior (public API) | ADAPT (typed struct + chained setters) |
| D15 | Copy TS bugs | Behavior | ADOPT (with listed forced differences) [API] |
| D16 | Session block on unknown tools / validation errors | Behavior | ADAPT (rmcp's errors are JSON-RPC errors) |
| D17 | Tool-injection cache under PHP-FPM | PHP-specific (runtime) | ADAPT (shared in-memory registry, rebuild on demand) |
| D18 | Exporter identity strings | Behavior | ADOPT |
| D19 | Backend tickets | Process | ADOPT [API] |
| D20 | `agentcat/api` version constraint | PHP-specific (packaging) | OWNER DECISION NEEDED (pin vs range) [API] |
| D21 | Private members of integrated libraries | PHP-specific (technique) | ADAPT (public seams only; same CI guard) |
| D22 | Static analysis | Process | ADAPT (clippy, rustdoc, cargo-deny, semver checks) |
| D23 | Testing framework | Process | ADAPT (cargo test, cargo-mutants, mock HTTP) |
| D24 | Code style | Process | ADAPT (rustfmt + clippy) |
| D25 | Enforcement on commits | Process | ADOPT |
| D26 | Merge gates | Process | ADAPT (no type coverage; llvm-cov, mutants) |
| D27 | Automatic framework integration | PHP-specific (framework) | ADAPT (one-line wrap; env var) |
| D28 | Packaging | PHP-specific (packaging) | ADAPT (one crate `agentcat-sdk`) [API] |
| D29 | Namespace root | PHP-specific (packaging) | ADAPT (`agentcat_sdk::`) |
| D30 | Logging | Behavior + PHP-specific | ADAPT (own file default, opt-in `tracing`) |
| D31 | Customer test suites silent | Behavior | OWNER DECISION NEEDED (Rust test detection) |
| D32 | Never throw from runtime telemetry | Behavior | ADAPT (never panic; `Result` only for custom events) |
| D33 | No special disclosure of D06/D07 | Process (product) | ADOPT |
| D34 | `parameters.extra` shape | Behavior | ADOPT (mapped from `RequestContext`) |
| D35 | `ToolCall` hook argument | Behavior (public API) | ADAPT (owned `Clone + Send + Sync` snapshot) |
| D36 | `track()` without a project ID | Behavior | ADAPT (same meaning; `from_env` constructor) |
| D37 | `mcp/sdk` version constraint | PHP-specific (packaging) | OWNER DECISION NEEDED (which rmcp majors) |
| D38 | Logger with several servers: first wins | Behavior | ADOPT |
| D39 | SIGTERM / SIGINT drain | Behavior + runtime | OWNER DECISION NEEDED (recommend no handlers) |
| D40 | `publishCustomEvent()` input | Behavior (public API) | ADAPT (`CustomEvent` + `Result` with TS messages) |
| D41 | `AgentCat::fake()` | Behavior (public API) | ADAPT (instance-scoped fake; parallel tests) |
| D42 | `Options::fromArray()` keys | PHP-specific (framework) | ADAPT (N/A: drop) |
| D43 | `flush()` returns `bool` | Behavior (public API) | ADOPT |
| D44 | `AGENTCAT_ENABLED` kill switch | Behavior | ADOPT |
| D45 | Exporter configuration types | Behavior (public API) | ADAPT (four structs, snake_case fields) [API] |
| D46 | Switch for the signal drain | PHP-specific (runtime) | ADAPT (follows D39; drop if no handlers) |
| D47 | Diagnostics delivery under PHP-FPM | PHP-specific (runtime) | ADAPT (N/A: keep TS 2 s window) |
| D48 | Diagnostics resource attributes | Behavior | ADAPT (Node vocabulary, Rust values) |
| D49 | Diagnostics timeouts | Behavior | ADOPT [API] |
| D50 | Diagnostics environment names | Behavior | ADOPT |
| D51 | `laravel/mcp` versions | PHP-specific (framework) | ADAPT (N/A: see D37) |
| D52 | Laravel configuration | PHP-specific (framework) | ADAPT (N/A: drop) |
| D53 | Server exclusion and per-server projects | PHP-specific (framework) | ADAPT (N/A: one handle per project) |
| D54 | Laravel install command | PHP-specific (framework) | ADAPT (N/A: README `cargo add`) |
| D55 | Symfony configuration | PHP-specific (framework) | ADAPT (N/A: drop) |
| D56 | Symfony Flex recipe | PHP-specific (framework) | ADAPT (N/A: drop) |
| D57 | Output buffers on SAPIs without finish-request | PHP-specific (runtime) | ADAPT (N/A: drop) |
| D58 | Delivery wait after each request in workers | PHP-specific (runtime) | ADAPT (N/A: background delivery) |
| D59 | Content-Length on laravel/mcp answers | PHP-specific (framework) | ADAPT (N/A: drop) |
| D60 | Default log destination (Laravel) | Behavior + framework | ADAPT (see D30) |
| D61 | Default log destination (Symfony) | Behavior + framework | ADAPT (see D30) |
| D62 | Content-Length on symfony/mcp-bundle answers | PHP-specific (framework) | ADAPT (N/A: drop) |
| D63 | First release version `1.0.0` | Process | OWNER DECISION NEEDED (0.1.0 vs 1.0.0) |
| D64 | Where the docs live | Process | ADAPT (README + docs.rs + Mintlify tabs) |
| D65 | README images | Process | ADOPT (excluded from the crate) |
| D66 | B10 pagination fix | Behavior | ADOPT (low trigger in rmcp, cheap) |
| D67 | Release steps | Process | ADAPT (crates.io trusted publishing) |
| D68 | Symfony Flex recipe PR | Process (framework) | ADAPT (N/A: drop) |
| P5 | Diagnostics in test environments | Behavior | ADAPT (follows D31) |
| B09 | `publishCustomEvent` quirks | Behavior | ADOPT |
| Bug policy | A Rust fix of a TS bug fixes Rust only | Process | ADOPT |
| N1 (new) | `serde_json` `preserve_order` | Behavior (Rust-only) | OWNER DECISION NEEDED [API] |
| N2 (new) | Default TLS backend | Packaging (Rust-only) | OWNER DECISION NEEDED [API] |

## 2. Counts

There are 71 PHP rows: D01–D68, P5, B09 and the bug policy. N1 and N2 are new Rust-only questions. They are listed
separately and not counted in the PHP total.

| Verdict | PHP rows | Of which N/A drops | New (N1, N2) |
|---|---|---|---|
| ADOPT | 20 | — | — |
| ADAPT | 43 | 13 (D42, D47, D51–D59, D62, D68) | — |
| OWNER DECISION NEEDED | 8 (D05, D10, D13, D20, D31, D37, D39, D63) | — | 2 |

D05 and D39 make one question for the owner (the shutdown model), so the owner gets 9 questions in total.

---

## 3. One section per decision

### D01: `sdk_language` value

- **Question.** What does the SDK put in `sdk_language`, and in `error.platform`?
- **PHP decision.** `PHP`.
- **Rationale.** `sdk_language` is a free string with exact-match dashboard filters. The value follows the TS
  (`TypeScript`) and Go (`Go`) style: the language name with no version.
- **Class.** Behavior (a per-language value under a shared rule).
- **C#.** Adapted (Q3). It sends `C#`. `error.platform` and the diagnostics `agentcat.sdk.language` stay lower case
  (`csharp`).
- **Rust: ADAPT.** Send `sdk_language = "Rust"`, `error.platform = "rust"` and `agentcat.sdk.language = "rust"`.
  `rust` is not `javascript|node|js`, so the server groups errors by `module|function`, which fits Rust paths. Ask the
  owner to confirm the one word, and check that the dashboard's language filter lists `Rust`.

### D02: `agentcat:custom` in the API enum

- **Question.** Add `agentcat:custom` to the server's `event_type` enum, or work around it?
- **PHP decision.** README §6 recommended adding it to the schema. The decided table never lists D02, but the work is
  done: PHP now requires `agentcat/api` 0.0.4, which lists the value (commit `d1629d0`).
- **Rationale.** The server accepts the value, and only the generated enum lacked it.
- **Class.** Behavior (backend contract).
- **C#.** Same (milestone M5a adds the value to the schema).
- **Rust: ADOPT [API].** `agentcat-api` 0.0.3 has `EventType::AgentcatColonCustom`, so no workaround is needed.

### D03: Transport and API client

- **Question.** Send events through the generated client, a hybrid, or a custom sender?
- **PHP decision.** `agentcat/api` becomes a models-only package with zero dependencies. The SDK owns a small typed
  curl sender (blocking with timeouts after the response; non-blocking `curl_multi` on stdio). A dev-only contract test
  checks request bodies against the live schema.
- **Rationale.** The old Guzzle-based client cannot install next to Guzzle 8, and it calls functions that Guzzle 8
  removed. The SDK must control timeouts and non-blocking delivery.
- **Class.** PHP-specific (packaging). The part that carries over is behavior: the SDK, not the generated client,
  owns the delivery policy (queue, retries, timeouts, body rules).
- **C#.** Same (Q1): models-only `AgentCat.Api` and an SDK-owned `HttpClient`.
- **Rust: ADAPT [API].** The owner already chose the full generated client, `agentcat-api` with reqwest. Keep the PHP
  intent by using the crate only as a typed request builder and sender:
  - Build one SDK-owned `reqwest::Client` with a connect timeout and a per-attempt total timeout. C# uses 10 s per
    attempt; TS has none, and reqwest's default has none either. Put that client into
    `Configuration { client, .. }`.
  - Set `user_agent` to an SDK value, for example `agentcat-rust/<version>`, in place of `OpenAPI-Generator/v1/rust`.
  - Never set `api_key` or `bearer_access_token`. An invalid `Authorization: Token` gets 401 (README §4.1 A7).
  - Call `events_api::publish_event` once per attempt. Map `Error::Reqwest` and `Error::ResponseError` onto the TS
    retry rule (D15: retry everything, 3 retries at 1 s, 2 s and 4 s).
  - Build `PublishEventRequest` with `new()` and field assignments, never a struct literal, so that a regenerated
    field does not break the build. This is the Rust form of C#'s "set model properties by name".
  - Handle the model's wire gaps in the SDK:
    - Make the timestamp UTC and cut it to milliseconds. When the milliseconds are 0, chrono drops the fraction
      (`…:00Z`, not `…:00.000Z`). Accept that, or fix it in the generator with a serde_with format.
    - Always set `session_id = Some(None)` (an explicit `null`), never `""`.
    - Round and clamp `duration` and the token fields to `i32`.
    - `tags` is a `HashMap`, so its order on the wire is random. See D15, B13.
  - Keep `agentcat-api` types out of the SDK's public API, so that a regenerated `agentcat-api` never breaks the
    SDK's own SemVer (see D20 and D63).
  - Reuse the same reqwest for exporters and diagnostics, so the build has one HTTP stack.
  - Add a contract test that serializes `PublishEventRequest` and validates it against the committed public OpenAPI
    schema and the golden JSON bodies.

### D04: When hooks run

- **Question.** In synchronous PHP, do hooks run before the handler (adding latency) or after the response?
- **PHP decision.** Hooks receive an immutable `ToolCall` snapshot and run after the response where the runtime allows
  it.
- **Rationale.** PHP has no concurrency, and running hooks before the handler would add latency to every call.
- **Class.** PHP-specific (runtime).
- **C#.** N/A. C# keeps the TS semantics: hooks start when the call starts and run concurrently with the tool, and the
  queue awaits each one with a 30 s budget (§7.3).
- **Rust: ADAPT.** Use the TS semantics, as C# does:
  - At the start of `tools/call`, build the owned `ToolCall` (D35) and `tokio::spawn` each configured hook
    (`identify`, `event_tags`, `event_properties`, `resolve_session_id`) on the customer's runtime, because the hooks
    are customer code.
  - The delivery stage awaits each `JoinHandle` under `tokio::time::timeout(30 s)`. A `JoinHandle` can be awaited from
    another runtime, so this also works if delivery runs on an SDK thread (D05).
  - A hook that panics returns a `JoinError`, which counts as a hook failure: the event loses data, and the tool
    response is unchanged.
  - Hook bounds: `Fn(ToolCall) -> impl Future<Output = …> + Send + 'static`, plus `Send + Sync + 'static` on the
    closure.

### D05: Public `flush()` / `shutdown()`

- **Question.** Expose public `flush()` and `shutdown()`?
- **PHP decision.** Yes, as a PHP-only addition. The docs call them rarely needed, because every supported PHP runtime
  flushes on its own.
- **Rationale.** Short-lived and worker runtimes need a deterministic drain.
- **Class.** Behavior (public API) + runtime.
- **C#.** Adapted: `FlushAsync(TimeSpan) -> bool`. An `IHostedService` drains for at most 5 s in `StopAsync`. A
  non-DI `AppDomain.ProcessExit` handler drains, and an option turns it off.
- **Rust: OWNER DECISION NEEDED** (one question together with D39). Rust has no `beforeExit` hook. When `main`
  returns, the tokio runtime drops and cancels every SDK task. A stdio server (one process per client session) ends
  as soon as `service.waiting().await` returns, so without a drain the last events are lost. Options:
  1. **Explicit only.** Async `flush(timeout) -> bool` and `shutdown(timeout) -> bool`. Delivery tasks run on the
     customer's runtime. The README tells stdio servers to call `shutdown` after `waiting()`. This is the simplest
     option, but a forgotten call silently loses the tail of every session.
  2. **SDK thread and automatic bounded drain.** Delivery, diagnostics and exporters run on one SDK-owned OS thread
     with its own current-thread tokio runtime. This is the pattern of the `sentry` crate and of `opentelemetry_sdk`'s
     default batch processors. `flush` and `shutdown` exist, in both async and blocking forms. When the last
     `AgentCat` handle or `Tracked` service drops, the SDK drains for at most 5 s on its own thread. For stdio, that
     is when `waiting()` returns; for HTTP, it is when the server's factory closure drops. Then the SDK re-arms: it
     does not destroy the pipeline, which avoids TS B19. This works under `current_thread` runtimes and while the
     customer's runtime shuts down.
  3. **Sentry-style guard.** `let _guard = agentcat_sdk::init(…)` flushes when it drops. Rust users know the
     pattern, but it is a second line and breaks the one-line promise (D27).
  - **Recommendation: option 2.** It is the closest analogue of TS's automatic drain. It does not depend on the
    customer's runtime still being alive, and it makes a blocking drain in `Drop` safe, because the drain runs on the
    SDK's thread and not on a customer worker. Document that `std::process::exit` and `panic = "abort"` skip it; an
    explicit `shutdown()` covers those paths.

### D06: HTTP headers in events

- **Question.** Send headers verbatim (TS parity), or apply an allowlist?
- **PHP decision.** All headers, verbatim, `Authorization` and `Cookie` included.
- **Rationale.** TS parity. The backend reads `user-agent` from `parameters` to name legacy clients on stateless
  servers. The owner chose parity over the audit's privacy recommendation.
- **Class.** Behavior (privacy).
- **C#.** Same, through runtime detection of ASP.NET Core (Q2).
- **Rust: ADOPT.**
  - Read `ctx.extensions.get::<http::request::Parts>()`. rmcp's streamable HTTP server inserts it, so no runtime
    detection is needed. This needs `http = "1"`, the same major as rmcp's.
  - `HeaderMap` names are already lower case. Join repeated values with `", "`, and decode non-UTF-8 values lossily.
  - On stdio there is no `Parts`, so the event has no `requestInfo`.

### D07: Diagnostics content

- **Question.** Diagnostics: TS parity, or errors and beacons only, with no IDs?
- **PHP decision.** Every log line (TS parity), including the per-event line with the Actor ID and the session ID.
- **Rationale.** TS parity. The owner chose it over the privacy recommendation.
- **Class.** Behavior (privacy).
- **C#.** Same.
- **Rust: ADOPT.** The README must not copy the TS sentence that says diagnostics never include anything about users
  (DECISIONS "What these decisions mean" item 3).

### D08: Redaction scope

- **Question.** Protect `identifyActorData` from string redaction, and redact custom events?
- **PHP decision.** TS parity: `identifyActorData` goes through `redactSensitiveInformation`, and custom events skip it.
- **Rationale.** TS parity.
- **Class.** Behavior.
- **C#.** Same.
- **Rust: ADOPT.**

### D09: String length unit

- **Question.** Measure the length limits in UTF-16 units, code points, or bytes?
- **PHP decision.** UTF-16 code units (TS parity).
- **Rationale.** The limits must cut at the same place as TS: 32 768 per string, the 200-unit tag values and
  `agent_id` clamp, and the 256 + `...` fields.
- **Class.** Behavior.
- **C#.** Same (native `string.Length`).
- **Rust: ADOPT.**
  - Count with `char::len_utf16()` (or `encode_utf16().count()`). Never use `str::len()` (bytes) or
    `chars().count()`.
  - Cut at the last character boundary that fits. When the cut falls inside a surrogate pair, drop the whole
    character, as PHP does (DECISIONS note 4). TS sends a lone surrogate that the server strips, so the stored result
    is the same.
  - Token estimates use another unit: UTF-8 bytes of compact JSON. `serde_json::to_vec` matches `JSON.stringify` for
    strings (no escaping of non-ASCII or `<>&`). Floats differ: serde writes `1.0` and `1e21`, where JS writes `1`
    and `1e+21`. For byte-exact parity, format floats like JS (for example with the `ryu-js` crate), or accept
    one-byte differences in rare estimates.

### D10: `error.type`

- **Question.** What shape do errors take: the `type` string, the frame order, `in_app`, `module`?
- **PHP decision.** `type` is the fully qualified class name, for example `App\Exceptions\PaymentFailed`.
- **Rationale.** The class name is the most useful label, and non-JS platforms group by `module|function`.
- **Class.** Behavior.
- **C#.** Same (`System.InvalidOperationException`), with frames from `StackTrace` and causes from
  `InnerException`.
- **Rust: OWNER DECISION NEEDED.**
  - Rust has no exception classes. A tool returns `Result<CallToolResult, McpError>`.
  - A customer's own error type `E` becomes an `is_error: true` result inside rmcp (`IntoCallToolResult`) before
    AgentCat sees it, so the SDK never sees `E`.
  - `Err(McpError)` is always the same type (`rmcp::model::ErrorData`).
  - Backtraces exist only if the error captured one where it was created.
  - A panic carries only a `&str` or `String` payload, and rmcp does not catch panics.
  - Options:
    1. **Minimal.** For `Err(McpError)`, `type` is the JSON-RPC error code name in rmcp's constant style
       (`INVALID_PARAMS`, `INTERNAL_ERROR`, or the number for a custom code). `is_error` results get no `type` (the TS
       `CallToolResult` rule). There are no frames.
    2. **Option 1 plus a capture helper.** The customer calls `agentcat_sdk::capture_error(&err)`, or a `Result`
       extension, inside a tool. The SDK stores the following in a task-local for that call, and the event uses them
       in place of the opaque result:
       - `std::any::type_name::<E>()` (the full path, the Rust analogue of an FQCN, best effort per std),
       - the `Display` text,
       - the `source()` chain as `chained_errors` (at most 10),
       - any captured `std::backtrace::Backtrace` as frames.
    3. **Option 1 plus a global panic hook.** The hook chains to the previous one and captures the panic location
       and backtrace. It is intrusive: it changes process-global panic state, and it does nothing under
       `panic = "abort"`.
  - In every option, the wrapper runs the inner future under `catch_unwind(AssertUnwindSafe(…))`, publishes an error
    event (`type = "panic"`, message = the payload), and then calls `resume_unwind`. rmcp and the customer see exactly
    the panic they would have seen.
  - **Recommendation: option 2, without a panic hook.** It gives D10's FQCN where the customer opts in, keeps cause
    chains like TS and C#, and never touches process-global state.

### D11: Libraries in the first release

- **Question.** Which MCP libraries does v1 support, and in which order?
- **PHP decision.** `mcp/sdk` (which also covers `symfony/mcp-bundle` and Drupal) and `laravel/mcp`. `logiscape` and
  WordPress are out of v1.
- **Rationale.** These libraries cover most PHP servers, and each one has a feasible seam.
- **Class.** PHP-specific (framework).
- **C#.** Adapted: only the official `ModelContextProtocol` (1.x and 2.x). Other .NET MCP libraries are out.
- **Rust: ADAPT.** Support rmcp only, the official SDK. Other Rust MCP crates are out of v1. Which rmcp majors to
  support is D37.

### D12: PHP minimum

- **Question.** Which minimum PHP version?
- **PHP decision.** `^8.3`. PHP 8.2 security support ends on 2026-12-31, and the current tools, Laravel 13 and typed
  class constants need 8.3.
- **Rationale.** Follow the floor of the host frameworks and tools.
- **Class.** PHP-specific (runtime).
- **C#.** Adapted: `netstandard2.0;net8.0;net10.0`, the targets of the MCP SDK itself.
- **Rust: ADAPT.**
  - Set `rust-version` to rmcp's MSRV (1.88 for rmcp 3.x) and use edition 2024.
  - Raise the MSRV only when rmcp does, in a minor release, as rmcp's own policy says.
  - `reqwest` 0.13 needs 1.85, so it fits. `agentcat-api` declares no `rust-version`; check it in CI [API].
  - CI: `cargo +1.88 check` and `cargo minimal-versions check` (both tools are installed).

### D13: Entry point

- **Question.** A static facade, namespaced functions, or both?
- **PHP decision.** One static class `AgentCat\AgentCat` over a swappable client, plus an injectable `AgentCat\Client`.
  No namespaced functions, and no Laravel facade.
- **Rationale.** The developer-experience research (09c): one obvious call, and testable through dependency injection.
- **Class.** Behavior (public API).
- **C#.** Adapted: `.WithAgentCat("proj_…")` on the builder, the static facade `AgentCatSdk.Track(...)`, and an
  injectable `IAgentCat`. No class is named `AgentCat`, because that would clash with the namespace.
- **Rust: OWNER DECISION NEEDED.**
  - Rust cannot change the customer's handler in place, so `track()` must return a wrapper. PHP's "returns the server
    it was given" (D32) cannot hold.
  - With streamable HTTP, rmcp calls a factory per session or per request, so a per-instance tracker loses the
    registries (D17).
  - Free functions are the idiomatic Rust facade; PHP's "no namespaced functions" reason (autoloading) does not apply.
  - Options:
    1. **Free function plus a shareable handle.** `agentcat_sdk::track(service, "proj_…", Options)` returns
       `Tracked<S>`. A cloneable `AgentCat::new("proj_…", Options)` has `.track(service)` for factories, so
       registries and configuration are shared. A process-wide default backs `publish_custom_event`, `flush` and
       `shutdown`.
    2. **Extension trait in C# style.** `service.with_agentcat("proj_…", Options)`. It reads well, but hides the
       shared-handle need.
    3. **A tower `Layer` around `StreamableHttpService`.** It covers HTTP only (no stdio), and it would parse raw HTTP
       and SSE bodies. Reject.
  - In options 1 and 2, `Tracked<S>` should implement `rmcp::Service<RoleServer>` around any `S: Service<RoleServer>`,
    not `ServerHandler`.
    - A `ServerHandler` wrapper must forward about 25 methods by hand. A method that rmcp adds later with a default
      body would silently skip the customer's override.
    - A `Service` wrapper matches `ClientRequest::{ListToolsRequest, CallToolRequest}` and forwards everything else
      unchanged.
  - **Recommendation: option 1** (option 2 can come later as sugar). It keeps the cross-SDK verb `track`, makes the
    per-request factory case cheap and correct, and has the smallest surface that rmcp changes can break.

### D14: Options type

- **Question.** Arrays, a value object, or both?
- **PHP decision.** A typed `final readonly class Options` with named arguments is the only option type. Arrays enter
  only through framework config, via `Options::fromArray()`.
- **Rationale.** Typed options catch mistakes and work well with IDEs (09c).
- **Class.** Behavior (public API).
- **C#.** Adapted in form: a mutable `AgentCatOptions` set through `Action<AgentCatOptions>` (the .NET options
  pattern), read once at install. Same intent: typed only.
- **Rust: ADAPT.**
  - A `#[non_exhaustive]` `Options` struct whose `Default` gives the TS defaults, with chained setters in rmcp's
    style (`Implementation::with_title`), for example `Options::default().enable_agent_tracking(true).identify(|c|
    async move { … })`. Rust has no named arguments, and `#[non_exhaustive]` lets fields be added in a minor release.
  - Hooks are generic closures that are boxed inside (`Arc<dyn Fn(ToolCall) -> BoxFuture<'static, _> + Send + Sync>`).
  - There is no untyped map input in v1 (see D42).

### D15: Copy TS bugs (README §5)

- **Question.** Copy or fix each TS defect B01–B20?
- **PHP decision.** Copy TS for now. No group of bugs is fixed. D66 later overrode this for B10 only.
- **Rationale.** Cross-SDK parity, and a TS fix belongs in TS.
- **Class.** Behavior.
- **C#.** Same, plus the C#-only Q5 fix (error frames keep 5 depth levels in truncation) and the differences that C#
  forces (§10).
- **Rust: ADOPT [API]**, with these differences that Rust or `agentcat-api` forces. Document them as PHP did.
  - **B02.** Keep the TS retry rule, but add a per-attempt timeout. reqwest's default has no timeout, and a hung POST
    would hold one of the 5 delivery slots for good. C# made the same choice.
  - **B04.** OTLP fails on a fractional duration. It cannot happen if `CustomEvent::duration` is an integer number of
    milliseconds, which the `i32` wire field suggests. Either type it as an integer and accept the divergence, or
    type it as `f64` and copy the failure.
  - **B13.** In TS, SDK tags come last and can drop past 50. With `agentcat-api`'s `HashMap`, the order is random, so
    with more than 50 tags *which* tags drop is random. It is only reachable with 50 valid customer tags plus SDK
    tags.
  - **B15, B17, B18.** No-ops. `serde_json::Value` cannot hold a cycle or a function, Rust has no non-Error throws,
    and Rust cannot split a surrogate (D09).
  - **B16.** A non-string `userIntent` loses the event in TS. Copy it, as PHP did.
  - **B19.** TS's permanent `destroy()` comes from its signal path. It depends on D05 and D39; the recommended design
    re-arms after a drain.
  - Everything else (B01, B03, B05–B12, B14, B20) can be copied as is. B10 follows D66.

### D16: Session block on unknown tools and validation errors

- **Question.** Prepend the session block to unknown-tool and validation-error results?
- **PHP decision.** TS parity (a default applied without a question).
- **Rationale.** "The retry must carry the same task."
- **Class.** Behavior.
- **C#.** Same intent. An unknown tool gives a JSON-RPC error with no block, because the C# SDK throws (§10 item 4),
  which is also what TS v2 does.
- **Rust: ADAPT.** Prepend the block wherever rmcp produces an `is_error` result. Where rmcp produces a JSON-RPC
  error (an unknown tool, or argument deserialization in `Parameters<T>`, both `invalid_params`), there is no block,
  and the event has no `response`. This is the same as TS v2 and C#.

### D17: Tool-injection cache under PHP-FPM

- **Question.** Under PHP-FPM, cache the registries in APCu or a file, or keep no cache?
- **PHP decision.** No cache. Rebuild on every `tools/call` that arrives before a `tools/list`.
- **Rationale.** Simplicity. A cache keyed by the tools and options was judged not worth it.
- **Class.** PHP-specific (runtime).
- **C#.** N/A in name. C# solved the same problem: registries live on one tracker shared by every options instance,
  and merge per tool. A rebuild on demand runs the handler and collection listing, raced against 5 s.
- **Rust: ADAPT.** Rust servers are long-running, so no cross-process cache is needed.
  - Registries live in memory on the shared `AgentCat` handle (an `Arc`) and are used by every service instance it
    wraps.
  - Rebuild on demand when `tools/call` arrives first: call the inner service with a `ListToolsRequest` and a context
    cloned from the current call, raced against `tokio::time::timeout(5 s)`. TS has the same 5 s bound.
  - Merge per tool name (see D66).

### D18: Exporter identity strings

- **Question.** Keep TS's hard-coded `sentry_client=agentcat/1.0.0`, or send an SDK-specific string?
- **PHP decision.** TS parity.
- **Rationale.** Parity.
- **Class.** Behavior.
- **C#.** Same.
- **Rust: ADOPT.**

### D19: Backend tickets

- **Question.** Which server-side fixes to request?
- **PHP decision.** One ticket: coerce `identify_data` that is not an object, and treat `session_id: ""` as null. The
  `agentcat/api` regeneration is separate work in `agentcat-server`.
- **Rationale.** These two inputs pass the 202 check and then damage data during ingestion.
- **Class.** Process.
- **C#.** Not addressed. C#'s server work is the enum (D02).
- **Rust: ADOPT [API].** No new ticket. Let the types prevent both inputs:
  - `Identity::data` is a `serde_json::Map` (an object only), even though `agentcat-api` types `identify_data` as a
    free `Value`.
  - A sessionless event always sends `session_id: Some(None)` (`null`).

### D20: `agentcat/api` version constraint

- **Question.** `~0.0.2`, or an exact pin?
- **PHP decision.** `~0.0.3`, which means 0.0.3 and later, below 0.1.0. The owner had removed version discipline and
  released the models-only client as 0.0.3.
- **Rationale.** Let generator patch releases arrive.
- **Class.** PHP-specific (packaging).
- **C#.** Adapted: `AgentCat.Api` from 0.0.3. NuGet resolves the *lowest* match, so a later 0.0.x never arrives by
  surprise.
- **Rust: OWNER DECISION NEEDED [API].**
  - Cargo resolves the *highest* compatible version, the opposite of NuGet.
  - Cargo treats every `0.0.x` as incompatible with the others: `"0.0.3"` means exactly 0.0.3.
  - `agentcat-api` is regenerated without semver checks, so a new 0.0.x can change field types and break every
    customer's fresh build.
  - Options:
    1. **Pin.** `agentcat-api = { version = "0.0.3", default-features = false }`. Bump it on purpose with each SDK
       release. A CI job builds against the newest `agentcat-api` to warn early.
    2. **Range.** `">=0.0.3, <0.1"` (the PHP `~0.0.3`). Customers pick up regenerations automatically, and a breaking
       regeneration breaks them at once.
    3. **Owner publishes `agentcat-api` 0.1.0** and promises additive-only changes within 0.1.x, then the SDK uses
       `"0.1"`.
  - **Recommendation: option 1** now, and option 3 when the generator gains a SemVer check. Keep `agentcat-api` a
    private dependency (no re-exports, see D03), so a pin bump never breaks the SDK's public API.

### D21: Private and protected members of integrated libraries

- **Question.** May the SDK use the private members of the libraries it integrates with?
- **PHP decision.** Yes, preferred whenever they give cleaner code or a shorter integration. Guardrails: one internals
  registry, private-API contract tests, version gates, and a log line plus no-op on a mismatch.
- **Rationale.** The owner runs aggressive CI and monitoring for private-member changes.
- **Class.** PHP-specific (technique).
- **C#.** N/A: C# has public filter seams for everything, and private access would break Native AOT.
- **Rust: ADAPT.** Rust cannot reach private fields at all: there is no reflection and no `Closure::bind`.
  - Keep the intent (pick the simplest seam and guard it with CI) using only rmcp's public API: the
    `Service<RoleServer>` trait and the public model types.
  - Treat any `#[doc(hidden)]` rmcp item as private API under the same guardrails: one internals module, and tests on
    every supported rmcp minor plus a nightly run against the newest release and rmcp's main branch.

### D22: Static analysis

- **Question.** Which static analysis?
- **PHP decision.** PHPStan max with strict, deprecation and PHPUnit rules, bleeding edge, larastan, and 100 % native
  type coverage. No baseline; ignores only by identifier, path and reason.
- **Rationale.** The best-practice research (09, 09a).
- **Class.** Process.
- **C#.** Adapted (§11.1): nullable on, warnings as errors, latest analyzers, banned APIs, public-API baselines.
- **Rust: ADAPT.**
  - `cargo clippy -D warnings` with `pedantic`, plus these restriction lints: `unwrap_used`, `expect_used`, `panic`,
    `indexing_slicing`, `print_stdout`, `print_stderr`, `dbg_macro`, `todo`. The two print lints are the Rust form of
    "never write to stdout".
  - `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`, and `RUSTDOCFLAGS=-D warnings`.
  - `cargo deny` (advisories, licenses, duplicates).
  - `cargo public-api` or `cargo semver-checks` from the first release.
  - Exceptions only as `#[expect(lint, reason = "…")]`, the analogue of "by identifier and reason". Type coverage is
    N/A in Rust.

### D23: Testing framework

- **Question.** Which test tools?
- **PHP decision.** PHPUnit, Infection (mutation), ParaTest, PHPat (layering rules), Orchestra Testbench.
- **Rationale.** The best-practice research (09b).
- **Class.** Process.
- **C#.** Adapted: xUnit, `FakeTimeProvider`, a scripted HTTP handler, Stryker.NET.
- **Rust: ADAPT.**
  - `cargo test` (nextest in CI is optional).
  - `cargo-mutants` for mutation testing.
  - `#[tokio::test(start_paused = true)]` for the timers (retries, the 30 s hook budget, the 2 s diagnostics window).
  - A mock HTTP server (`wiremock` or `httpmock`) for 202, 400, 401, 503 and timeouts.
  - Golden-fixture comparisons, raw or with `insta`.
  - End-to-end tests with a real rmcp client (in-process duplex, child-process stdio, and streamable HTTP in
    stateful and stateless modes).
  - Layering rules come from module privacy (`pub(crate)`), not from a tool.

### D24: Code style

- **Question.** Which style tools?
- **PHP decision.** PHP-CS-Fixer with PER-CS 3.0 and typing rules, plus Rector as a dry-run check.
- **Rationale.** The best-practice research (09a).
- **Class.** Process.
- **C#.** Adapted (analyzers and `.editorconfig`).
- **Rust: ADAPT.** `cargo fmt --check` with the default style. Clippy covers what Rector does; there is no Rector
  equivalent to run.

### D25: Enforcement on commits

- **Question.** How are the gates enforced?
- **PHP decision.** GitHub Actions on every push and pull request, one aggregate "all green" job as the only required
  check, branch protection, `composer check` locally, and an optional git hook of about 5 seconds.
- **Rationale.** One required check survives matrix changes.
- **Class.** Process.
- **C#.** Same (§11.3).
- **Rust: ADOPT.** Use `just check` or `cargo xtask check` as the local equivalent of `composer check`.

### D26: Merge gates

- **Question.** Which thresholds block a merge?
- **PHP decision.** 100 % native type coverage, 95 % lines, mutation score 90/95 (core) and 80/90 (adapters), 95 %
  covered MSI on changed lines, golden fixtures 100 %, zero architecture violations, private-API contract tests green.
- **Rationale.** Strict quality for an SDK that runs inside customer processes.
- **Class.** Process.
- **C#.** Adapted: 95 % lines, Stryker thresholds like D26, golden fixtures, an AOT job.
- **Rust: ADAPT.**
  - 95 % lines (`cargo llvm-cov`).
  - Mutation: caught ÷ (caught + missed) at least 90 % for the core and 80 % for the rmcp adapter. `cargo-mutants`
    has no covered-MSI split, so a small script computes the gate.
  - Golden fixtures at 100 %, zero clippy and fmt findings.
  - `cargo hack --feature-powerset check`, an MSRV build, and a minimal-versions build.
  - rmcp compatibility tests replace the private-API contract tests.
  - Type coverage is N/A.

### D27: Laravel and Symfony integration

- **Question.** How much code must a framework user write?
- **PHP decision.** Automatic: install the package and set `AGENTCAT_PROJECT_ID`, and every MCP server is tracked.
  The Boost dev server is excluded, and more servers can be excluded. Plain `mcp/sdk` users call `AgentCat::track()`.
- **Rationale.** A near-zero-code integration (09c).
- **Class.** PHP-specific (framework).
- **C#.** Adapted: one line, `.WithAgentCat("proj_…")`. No automatic install.
- **Rust: ADAPT.** Rust has no auto-discovery: no container, and no code runs before `main` without the `ctor` crate.
  The integration is one wrap, `agentcat_sdk::track(server, …)` (D13). When no project ID is given, the SDK reads
  `AGENTCAT_PROJECT_ID` (D36). Exclusion lists are N/A, because only wrapped services are tracked.

### D28: Packaging

- **Question.** How many packages?
- **PHP decision.** One Composer package, `agentcat/sdk`, with the core, the adapters and both framework bridges.
- **Rationale.** One install step.
- **Class.** PHP-specific (packaging).
- **C#.** Same intent: one NuGet package `AgentCat`, plus `AgentCat.Api`.
- **Rust: ADAPT [API].**
  - One crate, `agentcat-sdk` (reserved by agentcathq on crates.io; `agentcat` belongs to an unrelated crate). It
    holds the core, the rmcp adapter, the exporters, diagnostics and the testing fake.
  - It depends on `agentcat-api` with `default-features = false`.
  - Cargo features only where they remove a real cost: the TLS backend (N2), and possibly a `tracing` log sink
    (D30). Do not put the exporters behind features in v1.

### D29: Namespace root

- **Question.** Which namespace root?
- **PHP decision.** `AgentCat\`, with the API models under `AgentCat\Api\`.
- **Rationale.** A short brand root.
- **Class.** PHP-specific (packaging).
- **C#.** Adapted: package `AgentCat`, the static class `AgentCatSdk`, and the builder extension in
  `Microsoft.Extensions.DependencyInjection`.
- **Rust: ADAPT.**
  - The crate root is `agentcat_sdk::` (the default library name), with a flat surface: `track`, `AgentCat`,
    `Options`, `ToolCall`, `Identity`, `CustomEvent`, `flush`, `shutdown`, `exporters::*`, `testing::*`.
  - `agentcat_api` types stay in their own crate and are not re-exported.
  - `[lib] name = "agentcat"` would allow `agentcat::track`, but it collides with the unrelated `agentcat` crate's
    library name and breaks Cargo naming norms. Recommend the default; users can write `use agentcat_sdk as agentcat;`.

### D30: Logging

- **Question.** Where do the SDK's own log lines go?
- **PHP decision.** PSR-3 first: the framework log channel or the Builder's logger. Fall back to `~/agentcat.log`.
  Never STDOUT. D60 and D61 later changed the framework default to the SDK's own file.
- **Rationale.** Use the host's logging. D60 and D61: per-request setup lines flooded the app channels and reached
  error trackers.
- **Class.** Behavior (never stdout) + PHP-specific (PSR-3).
- **C#.** Adapted: `~/agentcat.log` by default; `LogToHost = true` routes the lines to `ILogger`.
- **Rust: ADAPT.**
  - By default, append to `~/agentcat.log` in the TS line format with Rust keys, for example
    `[sdk=0.1.0 rust=1.88.0 rmcp=3.6.0]`. Write off the request path. If there is no home directory, stay silent.
  - An option sends the same lines as `tracing` events (target `agentcat`) instead. rmcp itself logs through
    `tracing`.
  - `tracing` is not the default, because many stdio servers install `tracing_subscriber::fmt()`, which writes to
    stdout, and every SDK line would then corrupt the JSON-RPC stream.
  - Keep the TS message texts: diagnostics infer severity from the text.

### D31: Customer test suites

- **Question.** Is the SDK active in the customer's tests?
- **PHP decision.** No instrumentation and no delivery in test environments unless the test calls `AgentCat::fake()`.
  `AGENTCAT_ENABLED=true` forces the SDK on.
- **Rationale.** Customer tests that count tools or read `content[0]` must keep passing after the customer adds
  AgentCat, and tests must not send events.
- **Class.** Behavior (a divergence from TS, which keeps the engine on in tests and turns off only diagnostics).
- **C#.** Same. C# detects a loaded xUnit, NUnit, MSTest or TUnit assembly, or a `testhost` process.
- **Rust: OWNER DECISION NEEDED.** A dependency cannot see the customer's `cfg(test)`, and Rust has no loaded-assembly
  check. Options:
  1. **Runtime heuristic.** Treat the process as a test when the executable path matches
     `target/<profile>/deps/<name>-<hex hash>` (unit and integration tests under `cargo test` and nextest), or when a
     `NEXTEST*` variable is set. `AGENTCAT_ENABLED` overrides in both directions.
     - It misses end-to-end tests that spawn the server binary (`target/<profile>/<bin>`) as a child process. C#'s
       check misses those too.
     - False positives are unlikely.
  2. **TS behavior.** No detection: the engine and delivery stay on in tests, and only diagnostics turn off.
     Customers set `AGENTCAT_ENABLED=false` in CI.
  3. **Compile-time opt-out.** A cargo feature or `--cfg agentcat_test` that the customer sets for tests. It is
     clumsy, and features are global to a build.
  - **Recommendation: option 1.** It keeps the owner's intent at almost no cost, and the override covers the
    exceptions. The same check drives P5 and D44.

### D32: Errors (never throw)

- **Question.** May the SDK's runtime telemetry throw?
- **PHP decision.** Never throw from runtime telemetry. PHP's own type errors still fail fast. A bad option value logs
  one line. `track()` returns the server it was given.
- **Rationale.** The SDK runs inside the customer's server and must never break it.
- **Class.** Behavior.
- **C#.** Same: it never throws from runtime paths. The custom-event methods are the only public methods that throw.
- **Rust: ADAPT.**
  - Never panic in runtime paths. The clippy lints in D22 enforce this.
  - Hook panics end in a `JoinError` (D04), and tool panics are re-raised unchanged (D10).
  - `track()` cannot fail. When the SDK is disabled or misconfigured, it returns a pass-through `Tracked<S>` instead
    of the service itself (D13).
  - Only `publish_custom_event` returns `Result` (D40).
  - The compiler replaces PHP's runtime type errors. A bad option value (for example an exporter with an invalid
    DSN) logs one line.

### D33: Disclosure of D06 and D07

- **Question.** Add a special notice for the header and diagnostics behavior?
- **PHP decision.** No special disclosure. Document the behavior only in the reference docs.
- **Rationale.** Product choice.
- **Class.** Process (product).
- **C#.** Same.
- **Rust: ADOPT.**

### D34: `parameters.extra` shape

- **Question.** Which shape does `parameters.extra` have?
- **PHP decision.** The TS v1 and Python shape: `{sessionId?, requestId, _meta?, requestInfo?: {headers, url?},
  authInfo?}`, with lower-case headers that have string values, a full URL, and no `method` key.
- **Rationale.** Ingestion and the `x-anthropic-client` backfill read `extra.requestInfo.headers`.
- **Class.** Behavior.
- **C#.** Adapted: the same shape without `authInfo` (C# has a `ClaimsPrincipal` instead). `requestInfo` appears only
  under ASP.NET Core.
- **Rust: ADOPT.** Map the fields from `RequestContext`:
  - `sessionId` is the `Mcp-Session-Id` header from `Parts` (stateful streamable HTTP only).
  - `requestId` is `ctx.id`, as a number or a string, exactly as sent.
  - `_meta` is the request's `_meta` when the client sent one.
  - `requestInfo` is `{headers, url}` from `Parts`. The server sees an origin-form URI, so rebuild the absolute URL
    from `Host` and the path and query. Document the scheme rule.
  - There is no `authInfo`, as in C#: rmcp has no standard server-side auth object.
  - The key order needs N1, or an ordered build by hand.

### D35: `ToolCall` (hook argument)

- **Question.** What do hooks receive?
- **PHP decision.** Flat fields: `toolName`, the raw `arguments`, `meta`, `headers`, `clientName`, `clientVersion`,
  `protocolVersion`, `transportSessionId`, and the library's `request` and `context`.
- **Rationale.** An immutable snapshot is stable, while the library's request objects can change.
- **Class.** Behavior (public API).
- **C#.** Same (`AgentCatToolCall`), plus `User` (a `ClaimsPrincipal`).
- **Rust: ADAPT.** A `#[non_exhaustive] #[derive(Clone, Debug)] pub struct ToolCall`:
  - It is owned and `Send + Sync + 'static`, with an `Arc` inside so clones are cheap. Spawned hooks (D04) need this.
  - Fields: `tool_name`, `arguments: Option<JsonObject>` (raw, injected keys included), `meta`,
    `headers: HashMap<String, String>`, `client_name`, `client_version`, `protocol_version`, `transport_session_id`.
  - `context: RequestContext<RoleServer>` (a clone) gives advanced users the `extensions` (for example `Parts` or an
    auth extension set by their middleware) and the `peer`.
  - Exposing rmcp types makes rmcp a public dependency. That is unavoidable anyway, because `Tracked` implements
    rmcp's trait (see D63).

### D36: `track()` without a project ID

- **Question.** What does a missing project ID mean?
- **PHP decision.** `null` reads `AGENTCAT_PROJECT_ID`. `''` means telemetry only: exporters, and no AgentCat
  delivery. This diverges from TS, where `null` means telemetry only.
- **Rationale.** The framework bridges and twelve-factor apps configure through the environment.
- **Class.** Behavior.
- **C#.** Same.
- **Rust: ADAPT.** Keep the same meanings with a Rust-shaped API:
  - `track(svc, "proj_…", opts)` and `AgentCat::new("proj_…", opts)`, where `""` means telemetry only.
  - `AgentCat::from_env(opts)` (or `track_from_env`) for the PHP `null` case. An `Option<&str>` parameter would make
    every call write `Some(…)`.
  - A missing ID with no exporters logs the TS warning and turns delivery off.

### D37: `mcp/sdk` version constraint

- **Question.** Which `mcp/sdk` versions may install?
- **PHP decision.** `conflict: <0.8.1` only, with no upper bound. A startup check of private members turns the SDK
  off with one log line, and a nightly upstream job catches breaks.
- **Rationale.** New library releases should install, with detection in place of blocking.
- **Class.** PHP-specific (packaging).
- **C#.** Adapted: `ModelContextProtocol >= 1.0.0` with no upper bound. It compiles against 1.0.0, never references
  2.x-only APIs, tests 1.0.0, 1.4.1, 2.0.0 and 2.2.0, and runs a nightly job against the newest release.
- **Rust: OWNER DECISION NEEDED.**
  - Cargo can only depend on rmcp majors that AgentCat names.
  - The customer's rmcp and AgentCat's rmcp must unify. If they do not, `Tracked<S>` does not implement the
    customer's `Service` trait, and the build fails with a type error. That failure is loud and early, better than
    PHP's runtime check.
  - rmcp shipped three majors in five months, with trait changes between them.
  - Options:
    1. **rmcp 3.x only.** `rmcp = { version = "3.<floor>", default-features = false, features = ["server"] }`. The
       floor is the lowest 3.x minor that passes the end-to-end suite. CI runs the floor and the latest, plus a
       nightly job against the newest release and rmcp's main branch. 3.x has about 63 % of 30-day downloads and
       implements protocol 2026-07-28 (stateless, MRTR).
    2. **2.x and 3.x (and maybe 1.x).** Additive features `rmcp-2` and `rmcp-3` with renamed dependencies
       (`rmcp2 = { package = "rmcp", version = "2" }`) and one adapter module per major. It reaches about 25 % more
       downloads, at the cost of duplicated adapters and test matrices.
    3. **Newest major only, always.** Each new rmcp major drops the previous one in a new AgentCat release.
  - **Recommendation: option 1.** Add a 2.x adapter later only if customer demand shows up. When rmcp 4 ships, decide
    between options 2 and 3 (see D63 for versioning).

### D38: PSR-3 logger with several servers

- **Question.** When several servers are tracked, which logger gets the SDK's lines?
- **PHP decision.** The first tracked Builder's logger receives every SDK line for the process (first wins, like the
  exporters).
- **Rationale.** The pipeline is process-wide, so its logger is too.
- **Class.** Behavior.
- **C#.** Not addressed (C# uses one option, `LogToHost`). The process-global settings keep TS precedence:
  exporters and diagnostics first-wins, the API URL last-wins.
- **Rust: ADOPT.** The log destination (file or `tracing`) is process-global, set once in a `OnceLock`. The first
  `AgentCat` handle that sets it wins.

### D39: SIGTERM / SIGINT on CLI servers

- **Question.** Drain queued events on SIGTERM and SIGINT?
- **PHP decision.** With ext-pcntl, deliver the queued events, then call the earlier handler or re-raise. Never install
  over `SIG_IGN`. Turn on async signals (TS parity).
- **Rationale.** TS drains on SIGINT, SIGTERM and `beforeExit` (5 s), then re-raises.
- **Class.** Behavior + runtime.
- **C#.** Deviated: no signal handlers, because the .NET host owns Ctrl+C and SIGTERM. Drains come from `StopAsync`
  and `ProcessExit`.
- **Rust: OWNER DECISION NEEDED** (asked together with D05). Registering SIGTERM with tokio or signal-hook replaces
  the default action for the rest of the process. Unless a handler exits, the process no longer dies on SIGTERM. A
  library cannot tell whether the app registered its own handler first. Options:
  1. **No signal handlers** (C#). The app owns its signals. The drain comes from D05 (the drop drain or an explicit
     `shutdown()`), and the README shows a `tokio::signal::ctrl_c()` example that calls `shutdown`.
  2. **An opt-in `signals` option or feature.** It uses `signal-hook`: drain at most 5 s, then
     `emulate_default_handler` to restore the default and re-raise.
  3. **TS parity, on by default.** Reject: it changes process-wide signal behavior for every customer.
  - **Recommendation: option 1.** With option 1, D46 drops.

### D40: `publishCustomEvent()` input

- **Question.** What input does `publishCustomEvent` take?
- **PHP decision.** A typed `AgentCat\CustomEvent` value object with named arguments. The TS validation errors throw
  `\InvalidArgumentException` with the TS messages.
- **Rationale.** Typed input, and the same failure texts as TS.
- **Class.** Behavior (public API).
- **C#.** Same: `CustomEventData`, and `ArgumentException` with the TS texts.
- **Rust: ADAPT.**
  - `CustomEvent` is `#[non_exhaustive]` and `Default`, with chained setters.
  - `async fn publish_custom_event(session_id: &str, project_id: &str, event: CustomEvent) -> Result<(), Error>` for
    the session form. The server form takes a `Tracked<S>` or the `AgentCat` handle.
  - `Error`'s `Display` carries the TS validation messages verbatim.
  - The TS quirks follow B09.

### D41: `AgentCat::fake()`

- **Question.** What does the test fake look like?
- **PHP decision.** `AgentCat::fake()` returns `AgentCatFake` with `events()`, `assertTracked()`,
  `assertTrackedCount()`, `assertNothingTracked()` and `assertCustomEventPublished()`.
- **Rationale.** The Laravel-style fake idiom (09b, 09c).
- **Class.** Behavior (public API).
- **C#.** Same: `AgentCatSdk.Fake()`. Disposing it restores the previous state.
- **Rust: ADAPT.** `cargo test` runs tests in parallel threads of one process, so a process-global fake would mix
  events across tests.
  - Make the fake instance-scoped: `let fake = agentcat_sdk::testing::Fake::new(); let svc = fake.track(MyServer);`.
    Only the services it wraps record into it.
  - Methods: `events()`, `assert_tracked(..)`, `assert_tracked_count(n)`, `assert_nothing_tracked()`,
    `assert_custom_event_published(..)`.
  - For the global facade, a `fake::global()` guard that restores on `Drop` can exist, behind a mutex that
    serializes the tests that use it.
  - Ties to D31: the fake turns the engine on even in a detected test.

### D42: `Options::fromArray()` keys

- **Question.** Which key style does `Options::fromArray()` accept?
- **PHP decision.** camelCase only. The bridges map their own config style.
- **Rationale.** One name per option.
- **Class.** PHP-specific (framework).
- **C#.** N/A.
- **Rust: ADAPT (N/A: drop).** No map input in v1. If a config-file path is ever added, use `serde` with snake_case
  keys, the Rust convention.

### D43: `AgentCat::flush()` return

- **Question.** What does `flush()` return?
- **PHP decision.** `bool`: `true` when every queued event was sent, `false` when the timeout ended first.
- **Rationale.** The caller can react.
- **Class.** Behavior (public API).
- **C#.** Same (`FlushAsync` returns `bool`).
- **Rust: ADOPT.** `async fn flush(timeout: Duration) -> bool`, plus a blocking twin if D05 picks option 2.

### D44: `AGENTCAT_ENABLED`

- **Question.** What does `AGENTCAT_ENABLED` do?
- **PHP decision.** A false value turns the SDK off everywhere (a kill switch). A true value forces it on, also in
  tests. When unset, the SDK is on, except in test environments.
- **Rationale.** Operators need a switch that does not need a code change.
- **Class.** Behavior.
- **C#.** Same.
- **Rust: ADOPT.** Read it once, when the handle is built. When off, `track()` returns a pass-through wrapper and
  spawns nothing. The test environment is as D31 defines it for Rust.

### D45: Exporter configuration types

- **Question.** How does a customer configure the exporters?
- **PHP decision.** Four public `final readonly` classes, `OtlpExporter`, `DatadogExporter`, `SentryExporter` and
  `PostHogExporter`, whose named arguments use the TS key names. `fromArray()` takes `['type' => 'otlp', …]` like TS.
- **Rationale.** Typed configuration with TS names.
- **Class.** Behavior (public API).
- **C#.** Same.
- **Rust: ADAPT [API].**
  - Four public structs with the same names. Their fields map one to one onto the TS keys, in snake_case as Rust
    requires, each with `#[doc(alias = "<tsKey>")]`.
  - An `Exporter` enum, added with `Options::exporter(name, exporter)`.
  - Optionally, `serde::Deserialize` with `#[serde(tag = "type", rename_all = "camelCase")]` accepts the TS-shaped
    config.
  - The exporters send through the SDK's reqwest client, which is already in the build because of `agentcat-api`.

### D46: Switch for the SIGTERM/SIGINT drain

- **Question.** How does a customer turn off AgentCat's signal handlers?
- **PHP decision.** `Options(disableSignalHandlers: true)` turns them off and logs a WARN line if they are already
  installed. There is no environment variable. PHP-only (TS has no switch).
- **Rationale.** Some apps own their signals.
- **Class.** PHP-specific (runtime).
- **C#.** N/A (no signal handlers). The non-DI `ProcessExit` drain has an off option.
- **Rust: ADAPT.** This follows D39. With no handlers (the recommendation), drop it. If signals are opt-in, the opt-in
  is the switch.

### D47: Diagnostics delivery under PHP-FPM

- **Question.** When do diagnostics go out under PHP-FPM?
- **PHP decision.** One diagnostics POST per request, at shutdown, after `fastcgi_finish_request()`, inside the 5 s
  budget. Long-running servers keep the TS 2 s window.
- **Rationale.** FPM has no timers.
- **Class.** PHP-specific (runtime).
- **C#.** N/A.
- **Rust: ADAPT (N/A).** Keep the TS batching (at most 1 000 lines, a 2 s flush) on the delivery thread or task.
  `shutdown()` and the drop drain flush diagnostics too.

### D48: Diagnostics resource attributes

- **Question.** Which resource attributes do diagnostics carry?
- **PHP decision.** PHP values in the Node vocabulary: `agentcat.sdk.language=php`, `process.runtime.name=php`, the
  runtime version, `os.type` mapped to `linux`/`darwin`/`win32`, `os.version`, `host.arch` mapped to `x64`/`arm64`,
  no `host.cpu.count`, `deployment.environment=APP_ENV`, `agentcat.mcp_sdk.version`, and `agentcat.install_id` = the
  first 16 hex of `sha256(hostname | file://<package root>)`.
- **Rationale.** The dashboards query the Node vocabulary.
- **Class.** Behavior.
- **C#.** Adapted: `csharp` and `dotnet` values, plus `host.cpu.count` and `process.pid`, and
  `DOTNET_ENVIRONMENT`/`ASPNETCORE_ENVIRONMENT`.
- **Rust: ADAPT.**
  - `agentcat.sdk.language=rust`, `agentcat.sdk.version`, `process.runtime.name=rust`.
  - `process.runtime.version` is the rustc version, captured at build time by a tiny `build.rs` that runs
    `$RUSTC -V`. Or omit it to avoid a build script.
  - `os.type` comes from `std::env::consts::OS` (`macos` → `darwin`, `windows` → `win32`, otherwise as is).
  - `os.version`: on Linux read `/proc/sys/kernel/osrelease`; elsewhere omit it unless a small uname dependency is
    acceptable.
  - `host.arch` comes from `std::env::consts::ARCH` (`x86_64` → `x64`, `aarch64` → `arm64`, `x86` → `ia32`).
  - `host.cpu.count` comes from `std::thread::available_parallelism()` (TS and C# send it).
  - `process.pid` is `std::process::id()`.
  - `deployment.environment`: Rust has no convention. Recommend omitting it, or reading `APP_ENV` like PHP.
  - `agentcat.mcp_sdk.version` is `rmcp::model::Implementation::from_build_env().version`, which is rmcp's own
    version at runtime. No `mcp_sdk_v2`.
  - `agentcat.install_id` is the first 16 hex of `sha256(hostname + "|file://" + current_exe())`. A package root has
    no runtime meaning in Rust. The hostname needs `gethostname` (a small crate, because of `forbid(unsafe_code)`) or
    `/proc/sys/kernel/hostname`.

### D49: Diagnostics timeouts

- **Question.** Which timeouts do diagnostics requests use?
- **PHP decision.** 250 ms to connect and 1 s in total. Exporters keep 2 s and 10 s.
- **Rationale.** A blocked `otel.agentcat.com` must not hold a PHP worker.
- **Class.** Behavior.
- **C#.** N/A (listed as PHP-only).
- **Rust: ADOPT [API].** The PHP reason (worker hold) is weaker in Rust, because delivery is in the background. The
  same numbers still keep the shutdown drain short and match PHP. Set them as per-request timeouts on the shared
  reqwest client.

### D50: Diagnostics environment names

- **Question.** Which environment variables configure diagnostics?
- **PHP decision.** `AGENTCAT_DISABLE_DIAGNOSTICS`, `AGENTCAT_DIAGNOSTICS_ENDPOINT` and `AGENTCAT_DIAGNOSTICS_TOKEN`.
  The unprefixed `DISABLE_DIAGNOSTICS` can only turn diagnostics off. The unprefixed endpoint and token are not read.
- **Rationale.** An application's own variables must never reach AgentCat.
- **Class.** Behavior.
- **C#.** Same.
- **Rust: ADOPT.**

### D51: `laravel/mcp` versions

- **Question.** Which `laravel/mcp` versions?
- **PHP decision.** `^1.0` only, with `conflict: <1.0`.
- **Rationale.** Only 1.x has the needed seams.
- **Class.** PHP-specific (framework).
- **C#.** N/A.
- **Rust: ADAPT (N/A).** The rmcp version question is D37.

### D52: Laravel configuration

- **Question.** How is the Laravel bridge configured?
- **PHP decision.** `config/agentcat.php` with snake_case keys, `AGENTCAT_*` environment names, and hooks as invokable
  classes.
- **Rationale.** Laravel conventions.
- **Class.** PHP-specific (framework).
- **C#.** N/A.
- **Rust: ADAPT (N/A: drop).**

### D53: Server exclusion and per-server projects

- **Question.** How are servers excluded, and how does each server get its own project?
- **PHP decision.** A deny-list and a per-server project map.
- **Rationale.** Auto-tracking needs opt-outs.
- **Class.** PHP-specific (framework).
- **C#.** N/A.
- **Rust: ADAPT (N/A).** Nothing is auto-tracked. A per-server project is a separate `AgentCat::new(project)` handle
  per server.

### D54: Laravel install command

- **Question.** Does the Laravel bridge need an install command?
- **PHP decision.** `php artisan agentcat:install`: it validates the ID, writes `.env`, and sends nothing.
- **Rationale.** Laravel conventions.
- **Class.** PHP-specific (framework).
- **C#.** N/A.
- **Rust: ADAPT (N/A).** The README shows `cargo add agentcat-sdk` and the environment variable. The ID format check
  (`proj_` + 27 characters) happens at `track()` and logs a line.

### D55: Symfony configuration

- **Question.** How is the Symfony bundle configured?
- **PHP decision.** Root key `agentcat`, snake_case keys, `%env()%` defaults, and hooks as service IDs.
- **Rationale.** Symfony conventions.
- **Class.** PHP-specific (framework).
- **C#.** N/A.
- **Rust: ADAPT (N/A: drop).**

### D56: Symfony Flex recipe

- **Question.** Ship a Flex recipe?
- **PHP decision.** A `recipes-contrib` recipe that registers the bundle and copies the config.
- **Rationale.** A zero-step Symfony install.
- **Class.** PHP-specific (framework).
- **C#.** N/A.
- **Rust: ADAPT (N/A: drop).**

### D57: Output buffers on SAPIs without a finish-request function

- **Question.** How does the client get its answer before hooks and delivery on mod_php, `php -S` and php-cgi?
- **PHP decision.** Send and close the removable output buffers at the start of the last shutdown pass.
- **Rationale.** The client gets its response before hooks and delivery run.
- **Class.** PHP-specific (runtime).
- **C#.** N/A.
- **Rust: ADAPT (N/A: drop).** hyper and axum write responses; nothing in the SDK delays them.

### D58: Delivery wait after each request in long-running workers

- **Question.** How long may delivery hold a worker after each request?
- **PHP decision.** Up to 1 s after each request. The owner chose this over the recommended 5 s.
- **Rationale.** Free the workers sooner.
- **Class.** PHP-specific (runtime).
- **C#.** N/A.
- **Rust: ADAPT (N/A).** Delivery is in the background, and requests never wait. The owner's preference for short
  waits does inform the drop-drain bound in D05: keep 5 s at the end only.

### D59: Content-Length on laravel/mcp answers

- **Question.** Must the Laravel bridge set `Content-Length` on JSON answers?
- **PHP decision.** Yes, on SAPIs without a finish-request function, so that with D57 the client gets its answer
  before hooks and delivery.
- **Rationale.** laravel/mcp sends no `Content-Length`, and clients waited up to 5 s.
- **Class.** PHP-specific (framework).
- **C#.** N/A.
- **Rust: ADAPT (N/A: drop).**

### D60: Default log destination of the Laravel bridge

- **Question.** Where does the Laravel bridge write AgentCat's own lines by default?
- **PHP decision.** Its own file, `storage/logs/agentcat.log`. A config key routes the lines elsewhere.
- **Rationale.** Per-request setup lines flooded the app channel and reached error trackers.
- **Class.** Behavior (keep SDK noise out of the host's logs) + framework.
- **C#.** Adapted (carried over): `~/agentcat.log`, and `LogToHost` routes to the host.
- **Rust: ADAPT.** See D30: the SDK's own file by default, with an opt-in `tracing` sink.

### D61: Default log destination of the Symfony bundle

- **Question.** Where does the Symfony bundle write AgentCat's own lines by default?
- **PHP decision.** The analogue of D60: `%kernel.logs_dir%/agentcat.log`.
- **Rationale.** The same as D60.
- **Class.** Behavior + framework.
- **C#.** Adapted (carried over).
- **Rust: ADAPT.** See D30.

### D62: Content-Length on symfony/mcp-bundle answers

- **Question.** Must the Symfony bundle set `Content-Length` on JSON answers?
- **PHP decision.** The analogue of D59.
- **Rationale.** The same as D59.
- **Class.** PHP-specific (framework).
- **C#.** N/A.
- **Rust: ADAPT (N/A: drop).**

### D63: First release version

- **Question.** Which version is the first release?
- **PHP decision.** `1.0.0`. The public API follows SemVer, with a BC check on every pull request.
- **Rationale.** The repo README already promised 1.0.0.
- **Class.** Process.
- **C#.** Deviated: the first release is `0.1.0-preview.1` (M11). The C# design predates D63.
- **Rust: OWNER DECISION NEEDED.** rmcp is necessarily a *public* dependency: `Tracked<S>` implements rmcp's trait,
  and `ToolCall` carries `RequestContext`. So every rmcp major is a breaking change for AgentCat, and rmcp shipped
  three majors in five months. Options:
  1. **`0.1.0`.** This is Cargo's convention for young crates: a minor bump is breaking, so rmcp 4 support becomes
     0.2.0. Move to 1.0 when rmcp's major cadence slows.
  2. **`1.0.0`, as PHP.** AgentCat's major then follows rmcp's (2.0, 3.0, …), with `cargo semver-checks` on every pull
     request.
  3. **`1.0.0` and hide rmcp from the public API.** Not possible, because the wrapper must implement rmcp's trait.
  - **Recommendation: option 1.** Rust users read 0.x as "may break on a minor", which matches reality. Keep
    `agentcat-api` private either way (D20).

### D64: Where the PHP docs live

- **Question.** Where do the docs live?
- **PHP decision.** The README matches the TS README and adds short quickstarts. The reference pages go to
  docs.agentcat.com (Mintlify `apps/docs` in `~/agentcat-ui`): language tabs on the existing pages, plus
  language-only pages, through a pull request.
- **Rationale.** One docs site for every SDK.
- **Class.** Process.
- **C#.** Not addressed (the C# design predates D64; M11 lists the README and CHANGELOG).
- **Rust: ADAPT.**
  - The README mirrors the TS README with Rust quickstarts (stdio, and streamable HTTP with a shared handle).
  - docs.rs builds the API reference automatically from rustdoc, which is the Rust norm.
  - Rust tabs and Rust-only pages (rmcp versions, shutdown, tests) go to Mintlify through a pull request, as in PHP.

### D65: README images

- **Question.** How do the README images ship?
- **PHP decision.** Copy the five TS README images to the same paths and link them by absolute GitHub URLs. Keep
  them out of the package. A CI check compares them byte for byte with the TS repo.
- **Rationale.** They render on GitHub and on Packagist without bloating the package.
- **Class.** Process.
- **C#.** Not addressed.
- **Rust: ADOPT.** Absolute URLs render on crates.io too. Keep the images out of the crate with `include`/`exclude` in
  `Cargo.toml` (crates.io has a 10 MB limit).

### D66: B10 (pagination)

- **Question.** Fix B10 (pagination) in PHP?
- **PHP decision.** Fix it: AgentCat reads every `tools/list` page and merges the registries, so no tool falls back
  to the heuristic strip, which would remove the tool's own `session_id` or `context`. This overrides D15 for B10
  only.
- **Rationale.** `laravel/mcp` pages 15 tools, so every Laravel server with 16 or more tools hit the bug.
- **Class.** Behavior.
- **C#.** Not addressed (the design predates D66). C#'s per-tool registry merge already covers the "last page only"
  half.
- **Rust: ADOPT.** rmcp's `#[tool_handler]` lists every tool in one page, so the trigger is rare. Customers who write
  their own paginated `list_tools` still hit it. The fix is cheap in Rust: merge registries per tool name on the
  shared handle, and follow `next_cursor` during the rebuild. It also matches PHP.

### D67: Release steps

- **Question.** How does the release happen?
- **PHP decision.** The owner reviews and merges the PRs. After approval, Claude tags `v1.0.0`, checks that Packagist
  lists it, and checks that `composer require` installs it in a clean project.
- **Rationale.** The owner controls the release.
- **Class.** Process.
- **C#.** Adapted (M11): NuGet trusted publishing and a tag-equals-version check.
- **Rust: ADAPT.**
  - The owner merges. After approval, Claude tags `v<version>`.
  - A release workflow publishes with crates.io trusted publishing (GitHub OIDC) and checks that the tag equals the
    `Cargo.toml` version.
  - Claude checks that crates.io and docs.rs list the release, and that `cargo add agentcat-sdk` builds in a clean
    project.
  - Note: a crates.io publish is permanent. Only a yank is possible, so run `cargo publish --dry-run` and a package
    probe before.

### D68: Symfony Flex recipe pull request

- **Question.** Who opens the Flex recipe pull request?
- **PHP decision.** After approval, Claude opens the `recipes-contrib` PR.
- **Rationale.** The owner controls external posts.
- **Class.** Process (framework).
- **C#.** N/A.
- **Rust: ADAPT (N/A: drop).**

### P5: Diagnostics in test environments

- **Question.** Are diagnostics on in test environments?
- **PHP decision.** Off when D31 test mode is active. `DISABLE_DIAGNOSTICS` keeps the TS meaning.
- **Rationale.** TS turns diagnostics off under `VITEST`, `JEST_WORKER_ID` or `NODE_ENV=test`.
- **Class.** Behavior.
- **C#.** Same: no diagnostics in tests.
- **Rust: ADAPT.** Use the test detection chosen in D31.

### B09: `publishCustomEvent` quirks

- **Question.** Copy the quirks of TS `publishCustomEvent`?
- **PHP decision.** TS parity, misleading log line included: the server form ignores its `projectId` argument, and it
  drops events silently when tracing is off but still logs "Published".
- **Rationale.** D15.
- **Class.** Behavior.
- **C#.** Same.
- **Rust: ADOPT.**

### Bug policy

- **Question.** When an SDK fixes a TS bug, who else must fix it?
- **PHP decision.** When PHP fixes a TS bug, it fixes PHP only. No tickets for the other SDKs.
- **Rationale.** Owner process.
- **Class.** Process.
- **C#.** Same (Q5 was fixed in C# only).
- **Rust: ADOPT.**

---

## 4. Rust-only questions that the PHP log does not cover

### N1: `serde_json` `preserve_order` [API]

- **Why it is a question.** rmcp types tool schemas and `structuredContent` as `serde_json::Map`, and so does
  `agentcat-api`'s free-form `Value`. Without `preserve_order`, `Map` sorts keys. Several TS behaviors depend on key
  order:
  - `mcp_session` must be the *first* key of `structuredContent` (TS `handles.ts:200`).
  - The injected schema properties sit at TS positions, which the golden listing fixtures check.
  - The truncation breadth limit (100) keeps the *first* 100 keys.
  - `parameters.extra` has a fixed key order.
  - Worse, if the SDK does not choose, the order depends on whether some other crate in the customer's build turns
    the feature on, so behavior would differ between customers.
- **Options:**
  1. Turn on `serde_json/preserve_order` in `agentcat-sdk`. It is global to the customer's build: their `Map` becomes
     an `IndexMap`, which is slightly slower and iterates in insertion order. Many crates already turn it on.
  2. Leave it off and accept sorted order, which diverges from TS and the fixtures.
  3. A default-on AgentCat feature that customers can turn off. It adds a test-matrix leg, and turning it off needs
     `default-features = false`, which also affects TLS (N2).
- **Recommendation: option 1.** It gives deterministic, TS-identical, agent-facing output. Document the global effect.

### N2: Default TLS backend [API]

- **Why it is a question.** `agentcat-api` defaults to `native-tls` (OpenSSL headers and `pkg-config` at build time
  on Linux, `libssl` at run time). reqwest 0.13's own default, and rmcp's `reqwest` feature, are `rustls` with
  aws-lc-rs and the platform verifier, which needs a C toolchain at build time.
- **Options:**
  1. Default to `rustls`. Offer `native-tls` and `rustls-no-provider` as features, the names rmcp uses. Depend on
     `agentcat-api` with `default-features = false` and forward the features.
  2. Default to `native-tls`, `agentcat-api`'s own choice. It often breaks musl, static and distroless images.
  3. No default, so every customer must pick. This adds friction.
- **Recommendation: option 1.** It matches reqwest and rmcp, and it avoids OpenSSL runtime dependencies in
  containers.

## 5. C#-only decisions (Q1–Q5) and their Rust relevance

| C# ID | Decision | Rust |
|---|---|---|
| Q1 | Models-only `AgentCat.Api` and an SDK-owned sender | Superseded: the owner chose the full `agentcat-api` for Rust. D03 keeps the SDK-owned policy. |
| Q2 | One package; detect ASP.NET Core at runtime for header capture | No detection needed: rmcp passes `http::request::Parts` in `extensions` (D06). |
| Q3 | `sdk_language = "C#"` | D01: `Rust`. |
| Q4 | Public repo `agentcathq/agentcat-csharp-sdk` | Same pattern; the `agentcat-sdk` crate already points at `agentcathq/agentcat-rust-sdk`. |
| Q5 | Error frames keep 5 depth levels during truncation (a C#-only fix) | Adopt it if D10 option 2 produces frames, so that the backend can parse `frames`. It only matters when frames exist. |

## 6. PHP decisions the owner may want to revisit for Rust

1. **D63 (1.0.0).** rmcp's major cadence makes 1.0 costly. Recommend 0.1.0, as C# did with its preview.
2. **D31, D44, P5 (silent in tests).** These are a divergence from TS, and Rust detection is only a heuristic. Accept
   the heuristic, or return to TS behavior.
3. **D15 and B02.** Rust has to add a per-attempt timeout anyway. The owner may want to confirm this divergence, as C#
   made it.
4. **D21 (private members).** Moot in Rust. The monitoring requirement becomes "CI on every supported rmcp minor and
   nightly on the newest".
5. **D39 and D46 (signal drain).** TS and PHP drain on signals. For Rust, the recommendation is C#'s choice: no signal
   handlers, plus a drop drain (D05).
6. **D20 (`~0.0.3`).** Cargo resolves the highest version, so a range lets every regeneration reach every customer.
   Pin instead.
7. **D66.** The trigger is rare in rmcp. Adopting it is still recommended, because it is cheap.
8. **D49.** The PHP reason does not apply in Rust. Keeping the values is harmless.
9. **D45 and D14.** Rust uses snake_case fields and chained setters in place of TS camelCase named arguments. Confirm
   that the docs may show different spellings from TS.
10. **D48 `deployment.environment`.** Rust has no standard variable. Omit it, or read `APP_ENV`.
