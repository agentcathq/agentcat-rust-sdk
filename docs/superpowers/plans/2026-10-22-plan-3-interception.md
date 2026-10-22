# Plan 3: Interception (`Tracked<H>`, tool-list and tool-call pipelines, Rust reference server) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wrap any rmcp `ServerHandler` so that its tool listings and tool calls look on the MCP wire exactly as they do with the TypeScript SDK at `9b56e9e`, build the record of each tool call up to an internal handoff, and prove the wire behavior against the TS v2 reference server in the differential harness, on every rmcp 3.x minor.

**Architecture:** `Tracked<H>` implements rmcp's `ServerHandler` by hand: it forwards every method and intercepts `list_tools`, `get_tool`, `initialize` and `call_tool`. Pure functions, pinned by TS vectors, do the work: `inject` (the tool-list pipeline), `call` (stripping, mint-back, mirroring, error shapes) and `event` (the fields of the call record). The call path builds a `CallRecord` and hands it to a crate-private `Sink`, which drops it in this plan; plan 4 puts the queue behind the same seam. A Rust reference server joins the harness, which compares the untracked-to-tracked change of the MCP transcript between TS and Rust (the "MCP-wire delta"). Events are compared in plan 4.

**Tech Stack:** Rust 1.88+ (edition 2024); rmcp 3.x (`server` feature, floor 3.0.0, portable 3.0 API); `http` 1; tokio (`rt`, `time`); `futures`; `serde_json` (`preserve_order`). Dev: rmcp client, macros and streamable HTTP, axum 0.8, schemars 1. Node 22: the harness (TS v2 reference server on `@modelcontextprotocol/server` 2.0.0, TS clients), the oracle of plans 1 and 2.

**Spec:** `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (sections 5, 6.1 to 6.6, 7, 9, 11). Behavior contract: `docs/research/01-ts-sdk-behavior.md` B-017 to B-069, B-077 to B-086, B-096, B-097, K-01, K-02, K-05, K-07, K-08. rmcp facts: `docs/research/03-rmcp.md` sections 3 to 10, and the spike `docs/research/spikes/rmcp-poc` (its `src/lib.rs` has every forwarding signature). Decisions: `docs/DECISIONS.md` (C01 to C03, C08, C09, C14 to C16, D06, D16, D34, D35, D66, R05, R07, R09, R18 with D21, R20, R25). Deferred items: `docs/superpowers/plans/carry-forward.md`, section "Plan 3".

## Global Constraints

- Branch `rewrite`; conventional-commit subjects; no AI attribution lines; push only to `origin rewrite`.
- `edition = "2024"`, `rust-version = "1.88"`. Library code: no `unwrap`, `expect`, `panic!`, `todo!`, printing; every lint exception is `#[expect(<lint>, reason = "...")]`. Exception: a lint that fires only on some rmcp 3.x minors (for example `deprecated` on `subscribe`) takes `#[allow(<lint>, reason = "...")]`, with `#[expect(clippy::allow_attributes, reason = "fires on some rmcp 3.x minors only")]` on the same item, because an `#[expect]` would be unfulfilled on the other minors.
- `#![forbid(unsafe_code)]` stays. rmcp's `#[doc(hidden)]` and other semver-exempt items may be used where they make the code cleaner (D21); private fields may not.
- Public API: only the items an Interfaces block marks `pub` become public, each re-exported from the curated `pub use` block in `lib.rs` (no prelude), each with rustdoc and a `Debug` impl. Every other item is `pub(crate)` at most.
- Crate-private items that only plan 4 reads carry `#[cfg_attr(not(test), expect(dead_code, reason = "read by the delivery pipeline (plan 4)"))]`.
- rmcp: `rmcp = { version = "3.0.0", default-features = false, features = ["server"] }` (R05). Write against the portable 3.0 API (`03-rmcp.md` §9.3): `InitializeResult`, never `ServerConfig`; do not override `negotiate_initialize`; no item added after 3.0.0. `cargo +nightly minimal-versions check` builds the crate against rmcp 3.0.0, so it proves this.
- Dependencies: a dependency of library or binary code goes into `[workspace.dependencies]` with `default-features = false`, and its floor is the lowest version that passes `cargo +nightly minimal-versions check --direct --workspace` (R-1). A dependency that only one crate's tests use goes into that crate's `[dev-dependencies]`, never into the workspace table (the `--no-dev-deps` builds report such a workspace entry as unused).
- `serde_json` keeps `features = ["preserve_order"]` (R07). Nothing in the workspace turns on `float_roundtrip` (R22).
- Expected values come from TS only: a vector file written by `node oracle/src/vectors/run.mjs`, a live oracle call, or a harness run of a TS reference server. Never type a TS output into a Rust test. A Rust-only test may assert a shape, a Rust-only path (rmcp mechanics, registries, panics), or a deviation, and it names the deviation or decision ID.
- Vector files are never edited by hand. After a generator change, run `node oracle/src/vectors/run.mjs` and commit the output with the generator.
- Read TS code and tests only in the oracle's clone, `oracle/.cache/ts-9b56e9e920f35f7c37fc2a63ce6520a9ec97edea` (`node oracle/src/fetch-ts.mjs` creates it). Never read `~/agentcat-typescript-sdk`.
- Oracle, harness and Rust reference server processes never contact anything but loopback; never disable the egress guard. Tests never reach a production AgentCat endpoint.
- Lengths are UTF-16 code units (D09): count with `js::utf16_len`, cut with `js::cut_utf16`. Iterate JSON objects in JavaScript order wherever TS iterates them (`js::own_entries`).
- Never panic, never block the host runtime, never print (design §7.3). The SDK's own failures become log lines through `Sink::log`, with the TS message text (plan 4 writes them to the log file).
- The SDK never changes a tool's result beyond the TS injections, and never changes an error (design §7.4).
- Parity rule, for each TS test file a task owns: a pure input-to-output test becomes a vector case (`"ts_test"` in the case; manifest `status = "n/a"`, `category = "covered-by-vector:<file>.json"`, reason naming the case and the Rust test). A behavior pinned by a named Rust test is `ported` with that test name. A wire behavior pinned by a harness scenario's MCP-wire delta is `ported` with `rust = ["harness:<scenario>"]`. A test of a TS mechanism Rust does not have (detection, compatibility errors, re-arm, late registration, the inner tap, Workers) is `n/a`, `ruled-divergence:RD-19` (or RD-17, RD-21 where those match). A test of the JS host (V8 frames, `Error` objects, `AbortSignal`, function stubs, `extra` projection) is `n/a`, `ts-host-only` or the matching RD (RD-06, RD-07). A test whose assertion is on the published event body, hook settling, the queue, logging, exporters, diagnostics or custom events stays `todo` (plans 4 and 5). Entries stay in `parity/ts-tests.json` order.
- Before each commit, run the full local gate: `scripts/check.sh` must exit 0.
- Model per task (owner rule: Opus 5.5 for the hardest half, Sonnet 5.5 for the rest; reviewers use the implementer's model): stated at the top of each task.

## Decisions this plan makes

Task 14 records each one in `docs/DECISIONS.md` (builder decisions table). A reviewer who disagrees raises it; the controller rules.

- **C19 Registries:** one registry per `AgentCat` handle and inner handler type (`TypeId`), merged per tool name across pages and instances (design §6.3, RD-12). A tool found through `inner.get_tool` takes its foreign status from its current schema; the registry's "declared `session_id`" set only grows, as in TS.
- **C20 `get_more_tools` placement:** appended to every non-empty page, as TS does (B-048).
- **C21 Mirror key order:** the mirrored `structured_content` has JavaScript's key order: array-index keys ascending, then `mcp_session`, then the customer's other keys in their order. This copies TS's wire bytes (carry-forward); R07's "first key" holds for every key that is not an array index, which is what the harness rule `mcp-session-first` checks.
- **C22 Stateless identity:** for an HTTP request without an `mcp-session-id` header, the client identity skips rmcp's initialize-time `peer_info` (rmcp fabricates `rmcp@<version>` there, `03-rmcp.md` §5.2). TS's per-request server never saw `initialize` either, so both have no client identity there.
- **C23 Error precedence:** a panic gives `type: "panic"`; otherwise an error recorded with `capture_error` during the call replaces both the result-derived error (B-097) and the JSON-RPC code name (R09). The Rust error shape has no `frames` (RD-06 is reworded to say so).
- **C24 JSON-RPC code names:** -32700 `ParseError`, -32600 `InvalidRequest`, -32601 `MethodNotFound`, -32602 `InvalidParams`, -32603 `InternalError`, -32002 `ResourceNotFound`; any other code `JsonRpcError(<code>)`.
- **C25 `negotiate_initialize`:** not forwarded. rmcp added it in 3.3.0, so forwarding it breaks the 3.0 floor; a customer override of it is lost. The rmcp canary allowlists it with this reason, and design §13 lists the risk.
- **C26 Hooks:** the call path builds the `ToolCall` snapshot and spawns the hooks on the customer's runtime (C14) in this plan; plan 4 settles them.
- **C27 `rmcp/local`:** not supported, because hooks are spawned as `Send` tasks. Plan 5 documents it.
- **C28 Protocol-version tag:** read from the request `_meta` only (B-067), never from `RequestContext::protocol_version()`, whose fallback to the negotiated version TS does not have.

## Review Focus

1. **A strict customer argument type receiving injected arguments.** A `#[serde(deny_unknown_fields)]` parameter struct called with `session_id`, `agent_id` and `context`: after a listing, on a fresh handle before any listing, on a hand-written server without `get_tool`, and for a tool on page 2 of a paginated listing. A person expects the tool to succeed and to receive only its own fields. Pinned by Task 7 tests `strict_tool_gets_stripped_arguments_*` and `page_two_tool_keeps_own_context`.
2. **A tool that declares its own `session_id` or `context`.** A person expects the value to reach the handler untouched, no mint-back for a foreign `session_id`, and `context` still recorded as the intent (K-08). Pinned by Task 7 tests `own_session_param_is_foreign` and `own_context_reaches_handler`.
3. **A panicking tool, a tool that returns `Err(ErrorData)`, an unknown tool name.** A person expects the client to see exactly what it sees without AgentCat (no response for the panic; the same error object), the server to keep serving, and the record to say what happened. Pinned by Task 8 tests `panic_is_recorded_and_resumed` and `server_keeps_serving_after_panic`, and Task 7 tests `error_data_returned_unchanged` and `unknown_tool_error_unchanged`.
4. **`structured_content` that TS treats specially.** An array, a string, `null`, an object that already has `mcp_session`, an object with keys `"0"` and `"10"`, and output schemas that are not objects or that declare `mcp_session`. A person expects TS's mirror decision and TS's wire key order. Pinned by Task 5 `call.json` cases `structured:*` with test `mirror_wire_order_matches_ts`, and Task 7 test `mirror_gate_follows_registry`.
5. **HTTP requests with unusual headers, and stateless HTTP.** Repeated headers, a non-UTF-8 header byte, no `Host` header, and a legacy client on a stateless server. A person expects the D06 projection (values joined with `", "`, lossy text), no `url` without a host, and no fabricated `rmcp@<version>` client. Pinned by Task 6 tests `extra_*` and `stateless_identity_is_unknown`.

---

### Task 1: rmcp joins: options, handle, and a `Tracked<H>` that forwards everything

Model: Opus 5.5.

**Files:**
- Modify: `Cargo.toml`, `agentcat-sdk/Cargo.toml`, `Cargo.lock`, `deny.toml`, `agentcat-sdk/src/lib.rs`, `agentcat-sdk/src/test_support/mod.rs`
- Create: `agentcat-sdk/src/config.rs`, `agentcat-sdk/src/config/tests.rs`, `agentcat-sdk/src/client.rs`, `agentcat-sdk/src/client/tests.rs`, `agentcat-sdk/src/sink.rs`, `agentcat-sdk/src/tracked.rs`, `agentcat-sdk/src/tracked/tests.rs`, `agentcat-sdk/src/test_support/rmcp.rs`

**Interfaces:**
- Dependencies: `rmcp` (as in the Global Constraints), `http` 1, `tokio` 1 (`rt`, `time`), `futures` 0.3 (`std`). Dev: `rmcp` with `client`, `macros`, `transport-streamable-http-server`, `transport-streamable-http-client-reqwest`; `tokio` with `rt-multi-thread`, `macros`, `io-util`, `time`, `test-util`; `schemars` 1 and `serde` with `derive`; `axum` 0.8.
- Produces (`config`), `pub struct Options` (`Clone`, `Default`, `Debug`), with consuming setters `pub fn enable_report_missing(self, on: bool) -> Self`, `enable_tracing`, `enable_tool_call_context`, `enable_agent_tracking`, and `pub fn custom_context_description(self, text: impl Into<String>) -> Self`. Defaults are B-004: report missing on, tracing on, tool-call context on, agent tracking off, no custom description. Crate getters: `report_missing()`, `tracing()`, `tool_call_context()`, `agent_tracking() -> bool`; `context_description(&self) -> &str` (the custom text when non-empty, else `constants::DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`); `hook_mode(&self) -> bool` (always `false` until Task 9).
- Produces (`sink`): `pub(crate) trait Sink: Send + Sync + fmt::Debug { fn log(&self, message: String); }` and `pub(crate) struct DiscardSink;` (drops everything). Task 6 adds a `record` method.
- Produces (`client`):
  - `pub struct AgentCat` (`Clone`, `Debug`) over `Arc<Inner>`: `pub fn new(project_id: impl Into<String>, options: Options) -> Self` (cannot fail, starts nothing; `""` means telemetry only, B-003), `pub fn track<H: ServerHandler>(&self, handler: H) -> Tracked<H>` (an `Arc` clone).
  - `pub(crate) struct Inner { pub(crate) project_id: String, pub(crate) options: Options, pub(crate) sink: Arc<dyn Sink> }`; `pub(crate) fn inner(&self) -> &Inner`; `#[cfg(test)] pub(crate) fn with_sink(project_id: &str, options: Options, sink: Arc<dyn Sink>) -> Self`.
  - `pub fn track<H: ServerHandler>(handler: H, project_id: impl Into<String>, options: Options) -> Tracked<H>`.
- Produces (`tracked`): `pub struct Tracked<H> { inner: H, agentcat: AgentCat }` with `pub fn inner(&self) -> &H`, `pub fn inner_mut(&mut self) -> &mut H`, `pub fn into_inner(self) -> H`, `pub fn agentcat(&self) -> &AgentCat`; `Clone` when `H: Clone`; `Debug` for every `H` (prints `std::any::type_name::<H>()`). `impl<H: ServerHandler> ServerHandler for Tracked<H>` forwards every method of the rmcp 3.0 trait verbatim (the spike's `src/lib.rs` has the signatures). Compile-time assertions: `AgentCat: Send + Sync + 'static`, `Tracked<H>: Send + Sync + 'static` for a `Send + Sync` `H`.
- Produces (`test_support::rmcp`): `pub(crate) async fn connect<S: ServerHandler>(server: S, client_name: &str) -> RunningService<RoleClient, ClientInfo>` over `tokio::io::duplex` (client version `"1.0.0"`); `pub(crate) struct Probe;`, a hand-written `ServerHandler` that answers `get_info` (instructions `"probe"`), `ping`, `complete`, `set_level`, `get_prompt`, `list_prompts`, `list_resources`, `list_resource_templates`, `read_resource` and `on_custom_request` with fixed sentinel values; `pub(crate) struct RecordingSink` (implements `Sink`; `logs() -> Vec<String>`).
- `lib.rs`: a short crate doc (plan 5 writes the guide) and `pub use client::{AgentCat, track}; pub use config::Options; pub use tracked::Tracked;`.

- [ ] **Step 1: Write the failing tests**

`tracked/tests.rs`: `forwards_every_method`: connect `Probe` once directly and once as `track(Probe, "", Options::default())`; each Probe method's result through the client is equal on both connections, and so are the client's `peer_info()` (server info, capabilities, instructions). `tracked_debug_names_the_inner_type`.
`client/tests.rs`: `clones_share_one_inner` (`Arc::ptr_eq` through a crate helper); `track_returns_a_wrapper_around_the_handler` (`into_inner` gives the handler back).
`config/tests.rs`: `setters_chain` (each setter changes only its getter); `empty_custom_description_uses_the_default` (B-004: a falsy description means the default, so `""` gives `DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`). B-004's default values are pinned by the harness profile `{}` (Task 12), not here.

- [ ] **Step 2: Run them to see them fail**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk tracked:: client:: config::`
Expected: FAIL (modules missing).

- [ ] **Step 3: Add the dependencies, the modules and the exports**

Run `cargo deny check` after adding rmcp. Add each license that rmcp's tree needs to `deny.toml`, with a comment naming the crate. Leave the four unused licenses in place: they are checked again when reqwest and rustls join (plan 4).

- [ ] **Step 4: Run them to see them pass; check minimal versions**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk && cargo +nightly minimal-versions check --direct --workspace`
Expected: PASS; the minimal-versions build resolves rmcp 3.0.0 and compiles.

- [ ] **Step 5: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add Cargo.toml Cargo.lock deny.toml agentcat-sdk
git commit -m "feat: track rmcp servers with a forwarding Tracked<H>; options and handle"
```

### Task 2: rmcp canary and the rmcp version matrix in CI (R25)

Model: Sonnet 5.5.

**Files:**
- Create: `xtask/src/rmcp_canary.rs`, `xtask/src/testdata/canary/` (fixtures), `scripts/rmcp-versions.mjs`, `scripts/rmcp-versions.test.mjs`, `agentcat-sdk/tests/rmcp_version.rs`
- Modify: `xtask/src/main.rs`, `xtask/Cargo.toml` (if `serde_json` is needed), `scripts/check.sh`, `.github/workflows/ci.yml`, `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (§13), `docs/superpowers/plans/README.md` (plan 5 row: the canary moves here; plan 5 keeps the rmcp `main` nightly leg)

**Interfaces:**
- Produces (`cargo run -q -p xtask -- rmcp-canary`): finds the resolved rmcp source through `cargo metadata --format-version 1` (the `rmcp` package's `manifest_path`), reads `src/handler/server.rs`, collects the `fn <name>` items of `pub trait ServerHandler` (brace matching), and collects the `fn <name>` items of `impl<H: ServerHandler> ServerHandler for Tracked<H>` in `agentcat-sdk/src/tracked.rs`. Exit 0 when every trait method is implemented or allowlisted; exit 1 with one line `rmcp-canary: ServerHandler::<name> is not forwarded by Tracked<H>` per missing method. Allowlist: `negotiate_initialize` (C25). `--rmcp-src <dir>` overrides the lookup (tests use it with fixtures).
- Produces (`scripts/rmcp-versions.mjs`): `export function latestPatches(versions: {num: string, yanked: boolean}[]): string[]` (major 3, no pre-release, not yanked; the highest patch of each minor; sorted by minor, so `3.10.x` follows `3.9.x`). Run as a script, it fetches `https://crates.io/api/v1/crates/rmcp/versions` with a `User-Agent` naming the repository, follows `meta.next_page`, retries a failed request twice, and prints the JSON array.
- Produces (`tests/rmcp_version.rs`): `compiled_rmcp_version`: `rmcp::model::Implementation::default()` carries rmcp's own build-time version (`03-rmcp.md` §5.2). When `RMCP_EXPECTED_VERSION` is set, the version equals it; otherwise it starts with `"3."`.
- CI: job `rmcp-versions` runs the script and outputs the list; job `rmcp (<version>)`, one per list entry, runs `cargo update -p rmcp --precise <version>`, then `cargo nextest run -p agentcat-sdk --all-features`, `RMCP_EXPECTED_VERSION=<version> cargo nextest run -p agentcat-sdk --test rmcp_version` and `cargo run -q -p xtask -- rmcp-canary`, all with `RUSTFLAGS: -D warnings`. Both jobs join `all-green`. Actions pinned by SHA; `permissions: contents: read`.

- [ ] **Step 1: Write the failing tests**

`xtask` (in `rmcp_canary.rs`): `reports_a_missing_method` (fixture trait with `a`, `b`, `c`; impl with `a`, `b`: one line naming `c`), `allowlisted_method_passes` (`negotiate_initialize` missing: exit 0), `nested_braces_do_not_end_the_trait` (a default method body with blocks).
`scripts/rmcp-versions.test.mjs` (`node:test`): picks `3.4.1` over `3.4.0`; skips a yanked `3.5.2` (keeps `3.5.1`), `3.6.0-beta.1`, `2.2.0` and `4.0.0`; orders `3.10.0` after `3.9.3`.
`tests/rmcp_version.rs`: `compiled_rmcp_version`.

- [ ] **Step 2: Run them to see them fail**

Run: `node --test scripts/ && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p xtask && cargo nextest run -p agentcat-sdk --test rmcp_version`
Expected: FAIL (missing modules), except `compiled_rmcp_version`, which passes already (keep it).

- [ ] **Step 3: Implement the canary, the script and the CI jobs; add `node --test scripts/` and the canary to `scripts/check.sh`**

- [ ] **Step 4: Run them to see them pass**

Run: the Step 2 command, then `cargo run -q -p xtask -- rmcp-canary` and `RMCP_EXPECTED_VERSION=<the rmcp version in Cargo.lock> cargo nextest run -p agentcat-sdk --test rmcp_version`
Expected: PASS; the canary exits 0.

- [ ] **Step 5: Gate, commit, and watch CI**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add xtask scripts agentcat-sdk/tests/rmcp_version.rs .github/workflows/ci.yml docs
git commit -m "ci: test every rmcp 3.x minor and check the forwarded ServerHandler methods"
```

Push. Expected: one green `rmcp (<version>)` leg per 3.x minor, each running `compiled_rmcp_version` with its version.

### Task 3: `inject`: the pure tool-list pipeline

Model: Opus 5.5.

**Files:**
- Create: `agentcat-sdk/src/inject.rs`, `agentcat-sdk/src/inject/tests.rs`, `oracle/src/vectors/gen/inject.mjs`, `agentcat-sdk/tests/vectors/inject.json` (generated)
- Modify: `agentcat-sdk/src/lib.rs`, `agentcat-sdk/src/constants.rs`, `agentcat-sdk/src/constants/tests.rs`, `oracle/src/vectors/gen/constants.mjs`, `oracle/src/functions.mjs`, `agentcat-sdk/src/test_support/strategies.rs`, `parity/manifest.toml`

**Interfaces:**
- Produces (`constants`, checked by `constants.json`): the `get_more_tools` descriptor (`getReportMissingToolDescriptor()` as JSON), the canned result text (B-046) and the report-missing log head.
- Produces (`inject`):
  - `pub(crate) struct InjectConfig { pub(crate) report_missing: bool, pub(crate) inject_session_id: bool, pub(crate) inject_agent_id: bool, pub(crate) inject_context: bool, pub(crate) context_description: String }` and `pub(crate) fn from_options(options: &Options) -> Self` (B-031: `inject_session_id = tracing && !hook_mode`, `inject_agent_id = tracing && agent_tracking`, `inject_context = tool_call_context`).
  - `pub(crate) struct ToolEntry { pub(crate) injected: Vec<&'static str>, pub(crate) output_injected: bool, pub(crate) declares_session: bool }` (`injected` in the order `session_id`, `agent_id`, `context`).
  - `pub(crate) struct InjectState { pub(crate) declared_session_params: BTreeSet<String>, pub(crate) reported_conflicts: BTreeSet<String>, pub(crate) shadow_reported: bool }`: TS's per-tracking-data sets (B-028, B-040), kept across listings.
  - `pub(crate) struct InjectedList { pub(crate) tools: Vec<Tool>, pub(crate) entries: Vec<(String, ToolEntry)>, pub(crate) logs: Vec<String> }`.
  - `pub(crate) fn build_injected_list(config: &InjectConfig, tools: Vec<Tool>, state: &mut InjectState) -> InjectedList`: `buildInjectedList` (B-031 to B-047) on rmcp `Tool`s, schemas edited with `Arc::make_mut`.
  - `pub(crate) fn inject_tool(config: &InjectConfig, tool: Tool, state: &mut InjectState, logs: &mut Vec<String>) -> (Tool, ToolEntry)`: the same per-tool steps; `build_injected_list` uses it.
  - `pub(crate) fn get_more_tools_tool() -> Tool`.
- Produces (`functions.mjs`): `"oracle.buildInjectedList"`: `{options, hookMode, pages}` → one `{tools, registry: [[name, [params]]], outputRegistry: [names], declaredSessionParams: [names], logs}` per page, from one tracking-data object reused across pages (`hookMode: true` sets `resolveSessionId: () => null`).
- Produces (`inject.json`): cases with `input` (`{options, hookMode, pages}`) and `output` (the per-page list). A case whose tool rmcp cannot hold (missing, `null` or non-object `inputSchema`; non-string `name`; non-object `outputSchema`) carries `"deviation": "RD-20"`.

- [ ] **Step 1: Write the generator and the failing tests**

Generator cases, each listing-side input of the owned TS test files with its `ts_test`, plus:
- `flags:` every combination of report missing, tracing, hook mode, agent tracking and context; custom descriptions absent, `""` and `"why"`.
- `schema:` no `properties`; `required` absent, present, and holding `session_id` already; `additionalProperties` `false`, `true` and an object; truthy `oneOf`, `allOf`, `anyOf` (including `[]`, which is truthy) and falsy ones (`null`, `0`); a composed schema that declares `session_id`; customer `session_id`, `agent_id` and `context` with truthy and falsy (`false`) property schemas; customer properties around the injected ones (property order, B-032).
- `output:` absent; an object schema; a composed one; one that declares `mcp_session`; `{"type": "array"}` (TS injects anyway); hook mode with and without agent tracking (B-064).
- `report-missing:` a customer `get_more_tools` (B-028; logged once across two pages); report missing off.
- `pages:` two and three pages where the B-040 and B-041 log lines repeat or dedupe; duplicate names across pages.
- `rd20:` the unrepresentable inputs above.
- 30 from `seeded(8)`.

`inject/tests.rs`: `vectors_match_ts`: for each page, the tool names in order; each tool's `inputSchema` and `outputSchema` with `assert_js_eq` (key order counts); each whole tool as JSON with key order ignored; the registry as a map from name to its parameter list (map order ignored, list order compared); the output registry and declared set as sets; the logs in order. For an RD-20 case, `serde_json::from_value::<Tool>` fails on the tool; for every other case it succeeds. Property test `inject_matches_ts` against the live oracle, with `tool_list()` (new in `strategies.rs`: 1 to 6 tools; names from a set that includes `get_more_tools`; schemas from the generator's shapes) and random flags.

- [ ] **Step 2: Generate; run the tests to see them fail; implement; run to see them pass**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk inject:: constants::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries**

Owned files: `handle-injection.test.ts`, `context-parameters.test.ts`, `context-parameters-edge-cases.test.ts`, `custom-context-description.test.ts`, `handle-conflict-report.test.ts`, `debug-tool-schema.test.ts`, `v2/schema-edges.test.ts`, and the `buildInjectedList` tests of `engine-listwrap.test.ts`. Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk oracle parity
git commit -m "feat(inject): the TS tool-list pipeline on rmcp tools"
```

### Task 4: Listings, `get_tool` and `initialize` on `Tracked<H>`

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/registry.rs`, `agentcat-sdk/src/registry/tests.rs`
- Modify: `agentcat-sdk/src/lib.rs`, `agentcat-sdk/src/client.rs`, `agentcat-sdk/src/tracked.rs`, `agentcat-sdk/src/tracked/tests.rs`, `agentcat-sdk/src/test_support/rmcp.rs`, `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (RD-12 row: the C19 sentence on foreign status), `parity/manifest.toml`

**Interfaces:**
- Consumes: `inject::{InjectConfig, InjectState, InjectedList, ToolEntry, build_injected_list, inject_tool, get_more_tools_tool}`.
- Produces (`registry`): `pub(crate) struct ToolRegistry` holding an `InjectState`, the entries by tool name, and whether any listing was recorded; `pub(crate) fn record(&mut self, list: &InjectedList)` (merges per tool name, RD-12, C19); `pub(crate) fn entry(&self, name: &str) -> Option<&ToolEntry>`; `pub(crate) fn listed(&self) -> bool`; `pub(crate) fn declares_session(&self, name: &str) -> bool`; `pub(crate) fn state_mut(&mut self) -> &mut InjectState`.
- Produces (`client::Inner`): `registries: Mutex<HashMap<TypeId, ToolRegistry>>` and `pub(crate) fn with_registry<H: 'static, R>(&self, f: impl FnOnce(&mut ToolRegistry) -> R) -> R` (a poisoned lock is taken over with `PoisonError::into_inner`; the lock is never held across an `.await`).
- Produces (`Tracked<H>`):
  - `list_tools`: forward; on `Ok` with an empty `tools` list, log `Warning: No tools found in the original list. This is likely due to the tools not being registered before AgentCat.track().` and return the result untouched; otherwise run `build_injected_list` with the registry's state, record the list, send its logs to `Sink::log`, and return the result with `tools` replaced (every other field, `next_cursor` included, untouched). `Err` passes through.
  - `get_tool(name)` (C02): with report missing on and `name == "get_more_tools"`, the injected SDK descriptor; else `inner.get_tool(name)` through `inject_tool` with the registry's state (its logs dropped, so per-request lookups do not repeat listing warnings); else `None`.
  - `initialize`: forward unchanged; on `Ok`, keep the request's `client_info` on this instance (C16; a session-restore replay is handled the same way). `pub(crate) fn client_identity(&self) -> Option<Implementation>`. A clone of a `Tracked` starts with no identity.
- Produces (`test_support::rmcp`): `pub(crate) struct Paged`, a hand-written server whose `list_tools` returns two pages (cursor `"p2"`, a `meta` value on page 1) and whose `get_tool` returns `None`; `pub(crate) struct Sample`, a `#[tool_router]` server with tools `echo {msg}`, `strict {text}` (`deny_unknown_fields`), `structured {msg}` (output schema), `own_context {context}`, `own_session {session_id}`, `fail` (returns `Err(String)`), `explode` (panics). Task 7 adds `whoami` (reads `Extension<CallContext>`).

- [ ] **Step 1: Write the failing tests**

`registry/tests.rs`: `pages_merge_per_tool` (page-1 and page-2 entries both present; a relisted tool's entry is replaced); `declared_session_set_only_grows`.
`tracked/tests.rs`: `listing_equals_the_pure_pipeline` (the client's `list_tools` result equals `build_injected_list` of `Sample`'s own listing); `empty_listing_is_untouched_and_logged`; `cursor_and_other_fields_pass_through` (`Paged`: `next_cursor` and `meta` unchanged); `get_more_tools_on_every_page` (C20); `get_tool_returns_the_injected_schema` (customer tool, `get_more_tools`, unknown name); `initialize_keeps_the_client_identity` (`client_identity()` holds the test client's name and version; a clone holds none); `registries_are_per_inner_type` (`Sample` and `Paged` tracked by one handle keep separate entries).

- [ ] **Step 2: Run them to see them fail; implement; run to see them pass**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk registry:: tracked::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries**

Owned files: the wrapper tests of `engine-listwrap.test.ts`; `detect-shapes.test.ts`, `v2/detect.test.ts`, `compatibility.test.ts`, `edge-runtime-compatibility.test.ts`, `v2/late-registration.test.ts`, `v2/registration-patch.test.ts`, `v2/track-highlevel.test.ts`, `v2/track-lowlevel.test.ts`, `tracing-initialization.test.ts`, `mcp-version-compatibility.test.ts`, `esm-consumer.test.ts`, `handler-property.test.ts`. Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk docs parity
git commit -m "feat(tracked): inject listings, answer get_tool, keep the client identity"
```

### Task 5: `call`: pure tool-call helpers

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/call.rs`, `agentcat-sdk/src/call/tests.rs`, `oracle/src/vectors/gen/call.mjs`, `agentcat-sdk/tests/vectors/call.json` (generated)
- Modify: `agentcat-sdk/src/lib.rs`, `oracle/src/functions.mjs`, `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (RD-26 row: one sentence on the `context length` log line), `parity/manifest.toml`

**Interfaces:**
- Consumes: `inject::ToolEntry`, `constants::*`, `js::{utf16_len, own_entries}`.
- Produces (`call`):
  - `pub(crate) fn strip_arguments(arguments: &JsonObject, entry: Option<&ToolEntry>, tool_name: &str) -> JsonObject`: B-050 with an entry; B-051 without one (all three names, or only `session_id` and `agent_id` for `get_more_tools`).
  - `pub(crate) fn prepend_mint_back(result: &mut CallToolResult, text: String)`: B-056 (`content` is always a list in rmcp).
  - `pub(crate) fn mirror_mint_back(structured: &mut Option<Value>, mint: Map<String, Value>)`: B-058 for an own `mcp_session` key; the new map's key order is C21's.
  - `pub(crate) fn result_error(result: &CallToolResult) -> Map<String, Value>`: B-097's `CallToolResult` branch with `platform: "rust"` (RD-06).
  - `pub(crate) fn error_data_error(error: &ErrorData) -> Map<String, Value>`: `{message, type: code_name(..), platform: "rust"}` (RD-05, RD-06); `pub(crate) fn code_name(code: ErrorCode) -> String` (C24).
  - `pub(crate) fn report_missing(context: Option<&Value>) -> (CallToolResult, String)`: B-046's result and its log line; the context length is the UTF-16 length of a string, the length of an array, and 0 for anything else (an object's own `length` key is a JavaScript duck-typing artifact, R23).
- Produces (`functions.mjs`): `"handles.appendMintBack"`, `"handles.mirrorStructuredMintBack"`, `"exceptions.captureException"` (one argument), `"oracle.strip"` (`{name, arguments, registry: [[name, [params]]] | null}` → the stripped arguments of `cloneRequestWithStrippedArguments`), `"oracle.reportMissing"` (`{context}` → `{value, logs}`).
- Produces (`call.json`): case prefixes `strip:`, `mint-back:`, `structured:`, `result-error:`, `report-missing:`. `structured:` cases also carry `output_text` (`JSON.stringify` of the TS output), because `assert_js_eq` reorders index keys and cannot see wire order.

- [ ] **Step 1: Write the generator and the failing tests**

Generator cases, each owned TS test input with its `ts_test`, plus: `strip:` entries with and without each name, no registry, a tool absent from the registry, `get_more_tools`, customer keys named like ours; `mint-back:` every source with text and without, results with several blocks and with `isError`; `structured:` plain objects, objects with `"0"`, `"10"` and `"x"` keys in mixed order, an own `mcp_session`, arrays, strings, `null`, absent, and mints with one, two and three keys; `result-error:` text blocks only, mixed blocks, no text, whitespace-only text, empty content; `report-missing:` a string, an emoji string, an array, a number, `true`, `null`, absent, `{}`.

`call/tests.rs`: `vectors_match_ts` (each result as rmcp types, compared on `content` and `structuredContent`; `result-error:` outputs compared with `platform` set to `"rust"`, RD-06); `mirror_wire_order_matches_ts` (`serde_json::to_string` of the mirrored value equals `output_text`); property tests `strip_matches_ts` and `mirror_matches_ts`; Rust-only `code_names_follow_c24`.

- [ ] **Step 2: Generate; run to see them fail; implement; run to see them pass**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk call::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries**

Owned files: the `appendMintBack` and `mirrorStructuredMintBack` tests of `handles.test.ts`, `structured-mintback.test.ts`, `exceptions.test.ts`, `report-missing.test.ts`. `invokeSessionHook` stays `todo` (plan 4). Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk oracle docs parity
git commit -m "feat(call): stripping, mint-back, mirroring and error shapes"
```

### Task 6: `event`: the call record and its fields

Model: Opus 5.5.

**Files:**
- Create: `agentcat-sdk/src/event.rs`, `agentcat-sdk/src/event/tests.rs`, `oracle/src/vectors/gen/event.mjs`, `agentcat-sdk/tests/vectors/event.json` (generated)
- Modify: `agentcat-sdk/src/lib.rs`, `agentcat-sdk/src/sink.rs`, `agentcat-sdk/src/test_support/rmcp.rs`, `oracle/src/functions.mjs`, `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (§6.6: replace "the builder decides … records the ruling" with C22), `parity/manifest.toml`

**Interfaces:**
- Produces (`sink`): `Sink` gains `fn record(&self, record: CallRecord);`; `RecordingSink` keeps records (`take_records() -> Vec<CallRecord>`, which drains them: Task 9 adds task handles, which cannot be cloned).
- Produces (`event`):
  - `pub(crate) struct ClientIdentity { pub(crate) name: Option<String>, pub(crate) version: Option<String> }`.
  - `pub(crate) struct CallRecord { started_at: SystemTime, duration_ms: u64, resolution: HandleResolution, resource_name: String, request: Value, extra: Value, input_tokens: Option<i32>, output_tokens: Option<i32>, tags: Map<String, Value>, user_intent: Option<Value>, response: Option<Value>, is_error: bool, error: Option<Map<String, Value>>, server_name: Option<String>, server_version: Option<String>, client: Option<ClientIdentity> }`, all fields `pub(crate)` (B-077; plan 4 assembles the event from it).
  - `pub(crate) fn request_value(params: &CallToolRequestParams, context: &RequestContext<RoleServer>) -> Value`: B-082 with keys `method` (`"tools/call"`), `params` (`serde_json::to_value(params)` without its own `_meta` key, plus `_meta` from `context.meta` when that is non-empty; the arguments are the raw ones, injected keys included), `jsonrpc` (`"2.0"`), `id` (`context.id` as JSON).
  - `pub(crate) fn extra_value(context: &RequestContext<RoleServer>) -> Value`: D34, keys in the order `sessionId` (the `mcp-session-id` header, when present), `requestId`, `_meta` (when non-empty), `requestInfo` (`{headers, url}` when `http::request::Parts` are in the extensions).
  - `pub(crate) fn project_headers(headers: &http::HeaderMap) -> Map<String, Value>` (D06: every header, lower-case names, repeated values joined with `", "` in arrival order, bytes as `String::from_utf8_lossy`); `pub(crate) fn request_url(parts: &http::request::Parts) -> Option<String>` (an absolute request URI as it is; else `http://<Host><path and query>`; `None` without a `Host` header).
  - `pub(crate) fn narrow_client_info(value: Option<&Value>) -> Option<ClientIdentity>` (B-085: an object with a string `name` or a string `version`; non-string fields become `None`); `pub(crate) fn client_identity(context: &RequestContext<RoleServer>) -> Option<ClientIdentity>` (the `_meta` candidate, then `peer_info` unless C22 applies).
  - `pub(crate) fn protocol_version(meta: &JsonObject) -> Option<String>` (C28).
- Produces (`functions.mjs`): `"oracle.clientInfo"` (`{meta, initialize}` → `getClientInfoForRequest` with a fake server whose `getClientVersion()` returns `initialize`), `"session.getProtocolVersion"`.
- Produces (`event.json`): prefixes `client-info:` and `protocol-version:`. TS's v2 envelope branch has no rmcp counterpart (rmcp moves the whole `_meta` into the context): the generator does not use it.

- [ ] **Step 1: Write the generator and the failing tests**

Generator cases: `client-info:` meta candidates with string, number and missing `name` and `version`, arrays, `null`, strings, with and without an `initialize` value of each shape; `protocol-version:` strings, `""`, numbers, missing.

`event/tests.rs`: `vectors_match_ts`; Rust-only shape tests (D34, D06): `request_value_has_the_b082_keys` (key order; raw arguments; `_meta` only when sent), `extra_stdio_has_no_request_info`, `extra_joins_repeated_headers`, `extra_non_utf8_header_is_lossy`, `extra_url_from_host`, `extra_no_host_no_url` (pure tests build `http::request::Parts` with `http::Request::builder()`); integration tests over HTTP (axum with rmcp's `StreamableHttpService` on `127.0.0.1:0`, rmcp's reqwest client, JSON responses): `stateful_http_record_has_session_and_client` (sessionId equals the session header; client is the test client) and `stateless_identity_is_unknown` (`legacy_session_mode(false)`: no sessionId, client `None`, C22). These two use a hand-written test server whose `call_tool` returns `extra_value` and `client_identity` of its own context as structured content; Task 7 adds the same checks on the record.

- [ ] **Step 2: Generate; run to see them fail; implement; run to see them pass**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk event::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries**

Owned files: `client-identity.test.ts`, `extra-projection.test.ts`, `v2/http-context-capture.test.ts`, `e2e-http/header-capture.test.ts`. Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk oracle docs parity
git commit -m "feat(event): the call record, request and extra parameters, client identity"
```

### Task 7: The tool-call pipeline on `Tracked<H>`

Model: Opus 5.5.

**Files:**
- Create: `agentcat-sdk/src/call/run.rs`, `agentcat-sdk/src/call/run/tests.rs`, `agentcat-sdk/src/context.rs`
- Modify: `agentcat-sdk/src/call.rs`, `agentcat-sdk/src/tracked.rs`, `agentcat-sdk/src/event/tests.rs`, `agentcat-sdk/src/handles/mod.rs` (a `#[cfg(test)]` thread-local switch that makes `new_session_id` return `None`), `agentcat-sdk/src/lib.rs`, `agentcat-sdk/src/test_support/rmcp.rs`, `parity/manifest.toml`

**Interfaces:**
- Consumes: Tasks 3 to 6; `handles::{resolve_handles, build_mint_back_text, build_structured_mint_back, build_handle_tags}`; `tokens::{estimate_input_tokens, estimate_output_tokens}`.
- Produces (`context`): `pub struct CallContext` (`Clone`, `Debug`), inserted into `RequestContext::extensions` before the inner call, so a tool can take `Extension<CallContext>`: `pub fn session_id(&self) -> Option<&str>` (`None` when the session is `""`), `pub fn agent_id(&self) -> Option<&str>`, `pub fn agentcat(&self) -> &AgentCat`. Exported from `lib.rs`.
- Produces (`call::run`): `pub(crate) const REBUILD_TIMEOUT: Duration = Duration::from_secs(5);` and `pub(crate) async fn run_call<H: ServerHandler>(tracked: &Tracked<H>, request: CallToolRequestParams, context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData>`; `Tracked::call_tool` calls it. The steps, in B-022's order:
  1. Report-missing intercept: report missing on and the name is `get_more_tools`.
  2. Tracing off: answer the intercept with `call::report_missing`, else forward the raw request. No record (B-023, K-05).
  3. `started_at` (before step 4).
  4. Schema source (C03): `inner.get_tool(name)` through `inject_tool` with the registry's state (its logs dropped); else the registry entry; else, when the registry has never recorded a listing, a rebuild: `inner.list_tools` page by page through `next_cursor` within `REBUILD_TIMEOUT` in total, recorded like a listing, then the entry. Logs: `Rebuilt injection registries on demand (tools/call before tools/list on this instance)`; on failure `Warning: registry rebuild-on-demand failed - Error: <message>` (timeout message `tools/list did not respond within 5000ms`). No entry: the heuristic strip. The mirror gate is the entry's `output_injected`; without an entry it is open only when no listing was ever recorded (B-069).
  5. `session_param_is_ours` = not (the registry's declared set holds the name, or the `get_tool` entry declares `session_id`); `resolve_handles(..)`. `None` (minting failed): log `Warning: AgentCat tracing failed for tool <name>, falling back to original handler - Error: could not generate a session ID` and take the degraded path (answer the intercept, else forward the raw request; no record).
  6. Record fields: resource name (`name`, or `"Unknown Tool"` when empty), `request_value`, `extra_value`, input tokens on the raw arguments, tags (`agentcat_mrtr: "continuation"` when `input_responses` is present, B-025, then `build_handle_tags(.., protocol_version(..))`), user intent (B-083, K-08), server name and version from `inner.get_info().server_info`, `client_identity`.
  7. Intercept: user intent = the raw `context` argument; result from `call::report_missing` (log its line). Else forward the request with `strip_arguments` applied and the `CallContext` inserted.
  8. Finish (`Ok`): `Complete(result)`: the response is the result as JSON before decoration (B-059); mint-back text, then the mirror when the gate is open (C21); `is_error == Some(true)` sets `is_error` and `result_error`. `InputRequired`: tag `agentcat_mrtr: "input_required"`, no decoration. Any other variant (the enum is `#[non_exhaustive]`): no decoration. Output tokens on the response; duration; `Sink::record`. A serialization failure logs `Warning: AgentCat post-handler processing failed for tool <name>; returning the customer's result untouched - <error>` and returns the result undecorated, unrecorded.
  9. `Err(error)`: `is_error`, `error_data_error`, no response, no output tokens, duration, `Sink::record`; return the error unchanged.

- [ ] **Step 1: Write the failing tests**

`call/run/tests.rs`, over `test_support::rmcp::connect` with a `RecordingSink`:
- `strict_tool_gets_stripped_arguments_after_listing`, `strict_tool_gets_stripped_arguments_before_listing` (fresh handle; `get_tool` path), `strict_tool_without_get_tool_rebuilds_registry` (`Paged`; the rebuild log line), `rebuild_times_out_after_5s` (a listing that never answers; `tokio::time::pause`; heuristic strip; the timeout log line), `page_two_tool_keeps_own_context` (RD-12, K-01).
- `own_session_param_is_foreign` (value untouched; no mint-back text; tag `foreign`), `own_context_reaches_handler` (value untouched; recorded as the intent, K-08).
- `mint_back_for_minted_and_invalid_only` (`content[0]` equals `build_mint_back_text` of the resolution; none for a supplied ID), `mirror_gate_follows_registry` (mirrored for `structured`; not for a tool whose output schema was skipped; customer `mcp_session` kept), `record_keeps_the_original_result` (B-059).
- `get_more_tools_answered_and_recorded`, `customer_get_more_tools_is_shadowed` (B-028).
- `tracing_off_forwards_raw_arguments` (K-05; no record; `get_more_tools` still answered).
- `error_data_returned_unchanged` (the client's error equals the inner error; record type per C24), `unknown_tool_error_unchanged`, `is_error_result_gets_mint_back` (D16).
- `input_required_is_tagged_not_decorated` and `continuation_is_tagged` (a hand-written MRTR server).
- `session_id_failure_takes_degraded_path` (the `handles` switch, on a current-thread runtime).
- `call_context_reaches_tool` (`whoami` echoes the resolved session ID and agent ID).
- `record_fields_follow_b077` (resource name, `"Unknown Tool"`, intent rules, tags order, tokens present).
- `concurrent_calls_are_independent` (10 concurrent calls, each with its own handle, each minted back its own ID).
Add record-level twins of Task 6's two HTTP tests (the record's `extra` and `client`). Add to `test_support::rmcp`: `whoami` on `Sample`, a `Paged` variant whose listing never answers, and a hand-written MRTR server.

- [ ] **Step 2: Run them to see them fail; implement; run to see them pass**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk call:: event::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries**

Owned files: `engine-callwrap.test.ts`, `handles-integration.test.ts`, `v2/lowlevel-handles.test.ts`, `v2/session-modes.test.ts`, `v2/agent-tracking.test.ts`, `v2/get-more-tools.test.ts`, `v2/era-2026.test.ts`, `basic-server.test.ts`, `context-preservation.test.ts`, `concurrency.test.ts`, `v2/concurrency.test.ts`, `v2/smoke.test.ts`. Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk parity
git commit -m "feat(call): the TS tool-call pipeline: resolve, strip, mint back, record"
```

### Task 8: Panics and `capture_error`

Model: Opus 5.5.

**Files:**
- Create: `agentcat-sdk/src/capture.rs`, `agentcat-sdk/src/capture/tests.rs`
- Modify: `agentcat-sdk/src/call/run.rs`, `agentcat-sdk/src/lib.rs`, `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (RD-06 row: no `frames`, C23), `parity/manifest.toml`

**Interfaces:**
- Produces (`capture`):
  - `pub fn capture_error<E: std::error::Error + 'static>(error: &E)`: inside a tool call, records `{message: <Display>, type: std::any::type_name::<E>(), platform: "rust", stack?: <backtrace text>, chained_errors?: [{message}]}` for the current call: `stack` when `Backtrace::capture()` is captured; one entry per `source()`, at most 10. Outside a call it does nothing. Exported from `lib.rs`.
  - `pub(crate) struct CaptureSlot` and `pub(crate) async fn scope<F: Future>(slot: CaptureSlot, future: F) -> F::Output` (a `tokio::task_local!`; a task the tool spawns does not inherit it, which the rustdoc says).
  - `pub(crate) fn panic_error(payload: &(dyn Any + Send)) -> Map<String, Value>`: `{message, type: "panic", platform: "rust"}`; the message is a `&str` or `String` payload, else `Box<dyn Any>`.
- Changes (`call::run`): the inner call runs as `scope(slot, AssertUnwindSafe(fut).catch_unwind())`. A panic records `panic_error`, sends the record, then `std::panic::resume_unwind(payload)`. A captured error replaces `result_error` and `error_data_error` (C23).

- [ ] **Step 1: Write the failing tests**

`capture/tests.rs`: `panic_is_recorded_and_resumed` (`explode`: the record has type `"panic"` and the message; the client's request times out after 500 ms), `server_keeps_serving_after_panic`, `panic_payload_kinds` (`&str`, `String`, an integer → `Box<dyn Any>`), `capture_error_records_type_and_chain` (a tool returns `Err` after `capture_error(&e)` on a two-level error: type path, message, one chained entry), `capture_error_outside_a_call_does_nothing`, `captured_error_wins_over_error_data` (C23).

- [ ] **Step 2: Run them to see them fail; implement; run to see them pass**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk capture::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries**

Owned files: `error-capture.test.ts` (V8 stack and frame tests: `ruled-divergence:RD-06`), `engine-innertap.test.ts` (`ruled-divergence:RD-17` or RD-19). Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk docs parity
git commit -m "feat(capture): record panics and captured errors for the current tool call"
```

### Task 9: Hook mode and hooks

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/hooks.rs`, `agentcat-sdk/src/hooks/tests.rs`
- Modify: `agentcat-sdk/src/config.rs`, `agentcat-sdk/src/config/tests.rs`, `agentcat-sdk/src/event.rs`, `agentcat-sdk/src/call/run.rs`, `agentcat-sdk/src/lib.rs`, `parity/manifest.toml`

**Interfaces:**
- Produces (`config`), all exported from `lib.rs`:
  - `pub type HookError = Box<dyn std::error::Error + Send + Sync + 'static>;`
  - `#[non_exhaustive] pub struct UserIdentity { pub user_id: String, pub user_name: Option<String>, pub user_data: Option<Map<String, Value>> }` with `pub fn new(user_id: impl Into<String>) -> Self`, `pub fn user_name(self, name: impl Into<String>) -> Self`, `pub fn user_data(self, data: Map<String, Value>) -> Self` (D19).
  - `Options` setters, each `F: Fn(ToolCall) -> Fut + Send + Sync + 'static`, `Fut: Future<Output = Result<Option<T>, HookError>> + Send + 'static`: `resolve_session_id` (`T = String`), `identify` (`T = UserIdentity`), `event_tags` and `event_properties` (`T = Map<String, Value>`). `hook_mode()` is `true` when `resolve_session_id` is set. `Debug` shows a set hook as `"<hook>"`.
- Produces (`hooks`): `pub struct ToolCall` (`Clone`, `Debug`; D35), with `pub fn tool_name(&self) -> &str`, `arguments(&self) -> Option<&JsonObject>` (raw, injected keys included), `meta(&self) -> &JsonObject`, `headers(&self) -> Option<&Map<String, Value>>` (the D06 projection; `None` on stdio), `client_name()`, `client_version()`, `protocol_version()`, `transport_session_id() -> Option<&str>`, `request_context(&self) -> &RequestContext<RoleServer>`. `pub(crate) struct PendingHooks` (one optional `JoinHandle` per hook) and `pub(crate) fn fire(options: &Options, call: &ToolCall, hook_mode: bool, sink: &dyn Sink) -> PendingHooks`: spawns on `tokio::runtime::Handle::try_current()` in the order `resolve_session_id` (hook mode only), `identify`, `event_tags`, `event_properties` (B-022 step 7, C14, C26). Without a runtime a hook does not start and `Sink::log` gets `Warning: <label> hook not started - no tokio runtime`.
- Changes: `CallRecord` gains `pending: PendingHooks`; `run_call` fires after resolution (step 6) and never awaits the handles (B-026); `InjectConfig::from_options` and `resolve_handles` now see hook mode (B-064).

- [ ] **Step 1: Write the failing tests**

`hooks/tests.rs`: `hooks_receive_raw_arguments`, `hooks_fire_in_ts_order` (current-thread runtime; each hook pushes its label when first polled), `hooks_fire_before_the_tool_runs`, `hooks_never_delay_the_call` (a hook that never finishes; the call returns), `hook_panic_does_not_reach_the_call`, `no_hooks_no_tasks` (all handles `None`), `hook_mode_listing_has_no_session_id`, `hook_mode_resolution_is_provisional` (session `""`, source `hook`, no mint-back text), `hook_mode_mirror_has_only_agent_id`.
`config/tests.rs`: `debug_hides_hooks`.

- [ ] **Step 2: Run them to see them fail; implement; run to see them pass**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk hooks:: config::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries**

Owned files: `v2/hooks.test.ts`, `v2/options.test.ts`, `identify.test.ts`, `event-tags-properties.test.ts` (firing and listing assertions only; settling and event bodies stay `todo`). Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk parity
git commit -m "feat(hooks): hook mode, the ToolCall snapshot, hooks spawned at call start"
```

### Task 10: Harness: driver and diff fixes, v2 as the answer key (R20)

Model: Sonnet 5.5.

**Files:**
- Modify: `harness/src/driver.mjs`, `harness/src/diff.mjs`, `harness/src/dump-contract.mjs`, `harness/contract/tool-contract.json` (regenerated), `harness/README.md`, `harness/test/driver.test.mjs`, `harness/test/aa.test.mjs`, `harness/test/contract.test.mjs`, `harness/test/lane-2026.test.mjs`

**Interfaces:**
- Driver:
  - `--expect-publishes <n>` overrides the scenario's count.
  - In a tracked run (`HARNESS_NO_TRACK` unset), a `saveHandleAs` step that finds no handle, and a `${NAME}` with no saved value, exit 1 with `driver: step <i> of session <s>: no session handle for <NAME>`. An untracked run keeps the `${NAME}` text literally.
  - A negotiated protocol version other than `--lane` exits 1 with `driver: negotiated <version>, expected <lane>`.
  - The `initialize` transcript entry's result is `{protocolVersion, serverInfo, capabilities, instructions}` from the client, on both lanes.
  - The scenario check rejects unknown keys at the top level, in sessions and in steps, and `expectPublishes` keys other than `stdio`, `http`, `http-stateless` and `<transport>@<lane>` for a known lane (exit 2).
  - On a driver error, every connected client is closed before exit, so SDK-spawned stdio servers end.
- Diff: arrays of different length give one entry per element (`<missing>` on the short side); a pending ruling prints `RULING (pending) <RD-xx> matched <n>: ...`.
- R20: `dump-contract.mjs` lists the tools of `ts-v2.mjs`; the contract is regenerated from it; the README's "Tools" paragraph and the v1/v2 section say that v2 is the answer key on both lanes.

- [ ] **Step 1: Write the failing tests**

`driver.test.mjs`: `--expect-publishes 0` with a tracked TS server exits 1 (count mismatch) and with an untracked one exits 0; a tracked `saveHandleAs` on a call that supplies a valid `session_id` (no mint-back text, no structured content) exits 1 with the message; an untracked run sends `${S1}` literally; a fallback lane exits 1 (a v1 server on `--lane 2026-07-28`); the initialize entry has the four keys; unknown scenario, session, step and `expectPublishes` keys exit 2; the `<transport>` fallback of `expectPublishes` picks the transport count. `aa.test.mjs`: tests (d) and (e) build their own inputs instead of reading (a)'s output; a length mismatch in `transcript` gives per-element entries; a pending ruling prints the pending marker. `contract.test.mjs` and `lane-2026.test.mjs`: the contract comes from v2.

- [ ] **Step 2: Run them to see them fail; implement; regenerate the contract; run to see them pass**

Run: `node harness/src/dump-contract.mjs && corepack pnpm --filter harness test`
Expected: FAIL before the change, then PASS; `node harness/src/dump-contract.mjs --check` exits 0.

- [ ] **Step 3: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add harness
git commit -m "test(harness): stricter driver and scenarios; TS v2 is the answer key (R20)"
```

### Task 11: Harness: the MCP-wire delta and the matrix runner

Model: Sonnet 5.5.

**Files:**
- Create: `harness/src/delta.mjs`, `harness/src/matrix.mjs`, `harness/rulings-ts.json`, `harness/test/delta.test.mjs`, `harness/test/matrix.test.mjs`
- Modify: `harness/README.md`, `.github/workflows/nightly.yml`, `harness/package.json` (if a script entry is needed)

**Interfaces:**
- `delta.mjs`:
  - `export function transcriptDelta(untracked, tracked): Op[]`, `Op = {op: "add" | "remove" | "change", path: string, value?: unknown}`: transcript entries pair by index; objects compare by key; arrays of equal length compare by index; arrays of different length align by the longest common subsequence under deep equality (so a prepended mint-back block is one `add` at index 0); `add` and `change` carry the tracked value, `remove` carries none (the untracked value is the host's, not AgentCat's). Paths look like `$.delta.transcript[4].result.content[0]`.
  - `export function compareDeltas(ts, rust, rulings)`: one entry per path whose ops differ; the tracked documents' `violations` count as ops at their own paths.
  - CLI: `node harness/src/delta.mjs <ts-untracked> <ts-tracked> <rust-untracked> <rust-tracked> [--rulings <file>]`, with output lines and exit codes as `diff.mjs` (`DELTA <path>: ts <x> != rust <y>`, `RULED ...`, ruling lines, a JSON summary).
- `matrix.mjs`: `node harness/src/matrix.mjs [--servers <a>,<b>] [--scenarios <names>] [--transports <list>] [--lanes <list>] [--jobs <n>] [--rulings <file>]`. Servers by name: `ts-v1`, `ts-v2` (default first), `rust` (default second; the command is `HARNESS_RUST_SERVER`, else `target/debug/harness-ref-server`). For each scenario, transport and lane it runs both servers untracked (`HARNESS_NO_TRACK=1`, `--expect-publishes 0`) and tracked (the `rust` server tracked also gets `--expect-publishes 0` until plan 4 publishes), with labels `<server>-<lane>-<u|t>`, at most `--jobs` (default 4) drivers at a time; then compares the two deltas. It writes `harness/out/matrix.json` (one entry per combination: driver exit codes and delta summary) and prints one line per combination. Exit 1 on any driver failure or unexpected delta difference; exit 2 on bad arguments.
- Rulings: `harness/rulings.json` is for Rust against TS (`diff.mjs` and `delta.mjs`); `harness/rulings-ts.json` (`[]`) is for TS against TS, used by A/A runs and the nightly drift sweep.
- Nightly (carry-forward): the drift job uses `rulings-ts.json`; reports cut the text first and append the "Run:" link after it (never cutting inside a UTF-8 character); an A/A failure reports the last 50 lines of its output; output from TS main is printed between `::stop-commands::<random token>` and `::<random token>::`.

- [ ] **Step 1: Write the failing tests**

`delta.test.mjs`: a prepended content block is one `add` at index 0; an added key is an `add` with its value; a removed `additionalProperties` is a `remove`; an untracked JSON-RPC error that becomes a tracked result is `remove error` plus `add result`; equal deltas over different untracked values compare equal; a different added value is a `DELTA` line and exit 1; a ruling for it gives exit 0 and a `RULED` line; an unused ruling exits 1. `matrix.test.mjs`: `--servers ts-v2,ts-v2 --scenarios basic --transports stdio --lanes 2025-11-25` exits 0 with one combination and zero delta differences (A/A); an unknown server name exits 2.

- [ ] **Step 2: Run them to see them fail; implement; run to see them pass**

Run: `corepack pnpm --filter harness test`
Expected: FAIL, then PASS.

- [ ] **Step 3: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add harness .github/workflows/nightly.yml
git commit -m "test(harness): MCP-wire delta, matrix runner, TS-only rulings for the nightly"
```

### Task 12: The Rust reference server

Model: Opus 5.5.

**Files:**
- Create: `harness/rust-server/Cargo.toml`, `harness/rust-server/src/main.rs`, `harness/rust-server/src/profile.rs`, `harness/rust-server/src/tools.rs`
- Modify: `Cargo.toml` (workspace member, dependencies), `Cargo.lock`, `harness/test/contract.test.mjs`, `harness/test/lane-2026.test.mjs`, `harness/README.md`

**Interfaces:**
- Package `harness-ref-server` (`publish = false`, workspace lints), binary `harness-ref-server`. It follows every row of the README's "Server protocol": argv `stdio`, `http <port>`, `http-stateless <port>`; JSON responses; the ready line; exit on SIGTERM and on stdin close; `AGENTCAT_API_URL` must be `http://` on a loopback host, else exit 2 with `refusing to start`; `HARNESS_PROJECT_ID`, `HARNESS_OPTIONS`, `HARNESS_NO_TRACK=1`; logs on stderr only; server info `harness-ref-server` `1.0.0`.
- Tools (`tools.rs`): served through rmcp's `ToolRouter`, so dispatch, `get_tool`, argument errors and the `Err`-to-`is_error` conversion are rmcp's own. Each tool's name, description, input schema and output schema come from `harness/contract/tool-contract.json`, embedded with `include_str!`. Behaviors as `src/servers/common.mjs`; `boom` calls `capture_error` and returns `Err("boom")`; `emit_custom` answers `published` without publishing (plan 4 adds custom events). Unknown argument keys are ignored, as zod drops them.
- Profile (`profile.rs`): every key of the README's options profile; an unknown key or a malformed behavior exits 2. Flags map to `Options` setters; `resolveSessionId`, `identify`, `eventTags` and `eventProperties` map to hooks with the canned behavior (`returns`, `throws`, `sleepMs`); `redactSensitiveInformation` is validated and kept for plan 4. `HARNESS_CANARY=1` replaces the context description with `"canary"` (the perturbed configuration that Task 13's canary run must catch).
- Modes: `stdio` serves one tracked server (`agentcat.track(..).serve(stdio())`); `http` uses `StreamableHttpService` with a session manager and `legacy_session_mode(true)`; `http-stateless` uses `legacy_session_mode(false)`; each session or request gets `agentcat.track(RefServer::new())` from one shared `AgentCat`. rmcp serves 2026-07-28 requests and `server/discover` itself; the driver's lane check proves it on both lanes.

- [ ] **Step 1: Write the failing tests**

`contract.test.mjs`: untracked, the Rust server's `tools/list` equals the contract (names, descriptions, input and output schemas) on stdio at 2025-11-25 and on HTTP at 2026-07-28; a non-loopback `AGENTCAT_API_URL` exits 2 with `refusing to start`; an unknown profile key exits 2. `lane-2026.test.mjs`: the Rust server negotiates 2026-07-28 on stdio and HTTP.

- [ ] **Step 2: Run them to see them fail; implement; run to see them pass**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo build -p harness-ref-server && corepack pnpm --filter harness test`
Expected: FAIL, then PASS.

- [ ] **Step 3: Run the first Rust matrix by hand**

Run: `node harness/src/matrix.mjs --scenarios basic`
Expected: every driver exits 0; the matrix may exit 1 on delta differences. Note each one for Task 13; do not add rulings here.

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add Cargo.toml Cargo.lock harness
git commit -m "test(harness): Rust reference server on the tool contract"
```

### Task 13: Scenarios, the L3 gate, A/A and canary, rulings

Model: Opus 5.5.

**Files:**
- Create: `harness/scenarios/schemas.json`, `harness/scenarios/hook-mode.json`, `harness/scenarios/agent-tracking.json`, `harness/scenarios/features-off.json`, `harness/scenarios/tracing-off.json`
- Modify: `harness/src/servers/common.mjs`, `harness/contract/tool-contract.json` (regenerated), `harness/rust-server/src/tools.rs`, `harness/rulings.json`, `harness/README.md`, `scripts/check.sh`, `.github/workflows/ci.yml`, `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (§9 rows for any new difference), `parity/manifest.toml`

**Interfaces:**
- New contract tools (zod in `common.mjs`, mirrored in `tools.rs`): `own_session {session_id: string, x?: string}` (returns `session_id=<value>`), `own_context {context: string}` (returns the context), `structured_own {msg: string}` with an output schema that declares `mcp_session` (returns `structuredContent: {echoed: msg}`).
- Scenarios (each with the TS tracked publish counts the TS server produces, read from a TS run):
  - `schemas.json` (profile `{}`, so B-004's defaults are on the wire): calls to the new tools with and without our handles, an unknown tool `nope`, `echo` with `msg: 5` (an argument error), `structured`.
  - `hook-mode.json`: `resolveSessionId` returns `"customer-abc"`, `enableAgentTracking: true`; calls with `agent_id`.
  - `agent-tracking.json`: `enableAgentTracking: true`, `customContextDescription`; calls with and without `agent_id`.
  - `features-off.json`: `enableReportMissing: false`, `enableToolCallContext: false`.
  - `tracing-off.json`: `enableTracing: false`, calls with `context`.
- The README's scenario rules gain: no decimal with more than 15 significant digits (RD-23).
- L3 gate: `node harness/src/matrix.mjs` over every scenario, the transports `stdio`, `http`, `http-stateless` and both lanes; then an A/A run (`--servers rust,rust --scenarios basic`, zero differences) and a canary run (the same, with `HARNESS_CANARY=1` on the second server, which must report differences). `matrix.mjs` gains `--env2 <NAME=value>` for that. `scripts/check.sh` runs all three after `cargo build -p harness-ref-server`; CI job `harness-l3` (the setup steps of the `oracle-harness` job, plus the Rust build) runs the same, uploads `harness/out` when it fails, and joins `all-green`.
- Rulings: every Rust-vs-TS delta difference is fixed in the SDK, or gets a ruling in `harness/rulings.json` with the deviation ID it falls under (RD-05, RD-13), or a new design §9 row marked `pending` with a `"pending": true` ruling (R17).

- [ ] **Step 1: Add the tools and the scenarios; regenerate the contract**

Run: `node harness/src/dump-contract.mjs && PATH=$HOME/.cargo/bin:$PATH cargo build -p harness-ref-server`
Expected: the contract lists nine tools; the Rust server builds.

- [ ] **Step 2: Run the matrix to see the differences**

Run: `node harness/src/matrix.mjs`
Expected: every driver exits 0; the delta report lists the differences to settle.

- [ ] **Step 3: Settle each difference**

Fix the SDK (with a Rust test that pins the fix, in the module that owns it), or write the ruling. Run the matrix again until it exits 0. Then run the A/A run (exit 0) and the canary run (exit 1).

- [ ] **Step 4: Wire the gate**

Add the three runs to `scripts/check.sh` and the `harness-l3` job to CI.

- [ ] **Step 5: Map the parity entries**

Owned files: `e2e-http/v1-matrix.test.ts`, `e2e-http/v2-matrix.test.ts`, `e2e-http/sse-smoke-v1.test.ts`, `e2e-http/sse-smoke-v2.test.ts`; a wire-only assertion maps to `harness:<scenario>`; an event assertion stays `todo`. Also map to `harness:<scenario>` the wire assertions that Tasks 3 to 9 left `todo` because only a scenario pins them. Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 6: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add harness scripts .github/workflows/ci.yml docs parity agentcat-sdk
git commit -m "test(harness): scenarios and the MCP-wire delta gate against TS v2"
```

### Task 14: Milestone close

The controller runs this task.

- [ ] **Step 1: Full gate and CI**

Run: `scripts/check.sh`, push, and watch the CI run on `rewrite` (every `rmcp (<version>)` leg and `harness-l3` included); dispatch the nightly once (`gh workflow run nightly.yml --ref rewrite`, or push a change it watches) and watch it.
Expected: both green; `cargo run -q -p xtask -- parity` reports its counts.

- [ ] **Step 2: Decisions and carry-forward**

Record C19 to C28 (and any decision a task review added) in `docs/DECISIONS.md`. Remove the carry-forward items this plan closed. Move to plan 4, with the reason "events are compared in plan 4": `extra-headers-only` and the RD-07 shape ruling, the RD-04 timestamp class, minted-versus-derived placeholders, the sort tie-breaker and the same-millisecond line. Move to plan 4: the feature-unification self-check (it logs) and the unused cargo-deny licenses (after reqwest and rustls join). Move to plan 5: the feature-unification and `rmcp/local` (C27) documentation; the egress-guard worker-thread note stays there as a watch item. Add plan 4 items: hook settling reads `CallRecord.pending`; the Rust server's `emit_custom` and `redactSensitiveInformation`; `matrix.mjs` stops passing `--expect-publishes 0` for tracked Rust runs; the `parameters.request` key order from a TS probe.

- [ ] **Step 3: Rulings batch (R17)**

Bring every `pending` row of design section 9 and every pending ruling to the owner as one batch of multiple-choice questions, explained in plain words. Record the answers in `docs/DECISIONS.md`, design section 9 and `harness/rulings.json`.

- [ ] **Step 4: Plan index**

Mark plan 3 done in `docs/superpowers/plans/README.md` with the final commit SHA and the CI run IDs, and commit.
