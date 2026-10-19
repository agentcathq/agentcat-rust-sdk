# 03 — Rust MCP ecosystem and AgentCat Rust SDK design

Scope: a native Rust AgentCat SDK that instruments **Rust MCP servers** (primarily `rmcp`, the
official Rust SDK). This document covers the `rmcp` surface an analytics wrapper must use, the
rest of the Rust MCP ecosystem, interception options with code sketches, lessons from the Go,
Python and Java ports, and a proposed crate layout and public API.

Research date: 2026-10-16. All `rmcp` paths are relative to `crates/rmcp/` in
`github.com/modelcontextprotocol/rust-sdk` at tag **`rmcp-v3.5.1`** (commit `79437f2`,
2026-10-05) unless stated otherwise. The clone is at
`/tmp/claude-1000/-home-ubuntu-agentcat-rust-sdk/78031ef1-7fed-4ae1-94f2-11a86f1fe961/scratchpad/rust-sdk-src`
(read only, nothing built or run). `main` is 13 commits ahead of the tag (none change
`ServerHandler`; the notable one is `718bc60 feat(transport): add non-blocking stdio_pipes transport`).

---

## 0. Recommendations in brief

1. **Target `rmcp` 3.x only for v1** (`rmcp = ">=3.0, <4"`). It is the only Rust MCP crate with
   real adoption (36.6 M downloads, 2,232 reverse dependencies) and it already implements the
   2026-07-28 stateless protocol (30/30 server conformance).
2. **Primary interception: a delegating wrapper `Tracked<H: ServerHandler>`** that implements
   `ServerHandler`, overrides `call_tool`, `list_tools`, `get_tool`, `get_info`, `initialize`,
   `discover`, and forwards the other 22 methods. It works for stdio, streamable HTTP (session
   and stateless), `#[tool_handler]` servers and hand-written handlers, with one line of
   customer code. Add a small **`TrackedService<S: Service<RoleServer>>`** for servers built
   with rmcp's `Router<S>` builder (stdio/custom transports only).
3. **Do not** use a tower `Layer` (no stdio, must re-parse JSON and SSE) or `ToolRouter`
   wrapping (breaks with the default `#[tool_handler]`, misses hand-written servers) for v1.
4. **Biggest risk:** `ServerHandler` has 29 default-implemented methods. A wrapper silently
   shadows any method rmcp adds in a minor release. Mitigate with a CI tripwire that diffs
   rmcp's method list against the forwarded list, and ask upstream for a public delegation macro.
5. **Run the publisher on a dedicated OS thread with its own current-thread tokio runtime**
   (Sentry and OpenTelemetry Rust both converged on this). Flush with `shutdown().await`, with
   a bounded blocking flush in `Drop` as a fallback.
6. **The cross-SDK brief `/home/ubuntu/2026-07-28-cross-sdk-changelog.md` is stale.** TS main,
   Go and Python now use `session_id` (not `task_id`), `mcp_session` (not `_mcp_instructions`),
   a leading (not trailing) mint-back block, validated handles, and TS added token estimates.
   The Rust SDK must pin its constants against TS `main`, not against the brief (see §4.5).
7. Crate names: `agentcat` on crates.io belongs to a third party. Use `agentcat-sdk`
   (placeholder 0.0.1 already owned by `naji247`) with `[lib] name = "agentcat"`, and the
   generated client `agentcat-api` (placeholder 0.0.1 owned by `naji247`).

---

## 1. rmcp — the official Rust MCP SDK

### 1.1 Version, cadence, MSRV, features

| Fact | Value | Source |
| --- | --- | --- |
| Latest release | `rmcp` / `rmcp-macros` **3.5.1**, 2026-10-05 | https://crates.io/api/v1/crates/rmcp (`max_stable_version`) |
| MSRV | **1.88**, edition 2024 (dev toolchain pinned to 1.96) | `Cargo.toml:14-16` (workspace), `rust-toolchain.toml` |
| Downloads | 36.6 M total, 20.3 M in the last 90 days | crates.io API |
| Downloads by major | 1.x 11.5 M, 3.x 9.2 M, 0.8 4.8 M, 2.x 3.4 M | crates.io `versions[]` aggregated |
| Majors in 2026 | 1.0.0 (2026-03-03), 2.0.0 (2026-06-27), 3.0.0 (2026-07-28) | `CHANGELOG.md:313,496,172` |
| Minors since 3.0 | 3.1 → 3.5 in ~10 weeks; `ServerHandler::negotiate_initialize` added in **3.3.0** (#1247) | `CHANGELOG.md:70-74` |
| Reverse deps on 3.x | 47 of the first 100 reverse deps returned by crates.io; 23 on 1.x, 21 on 0.x, 9 on 2.x | `/api/v1/crates/rmcp/reverse_dependencies` |
| SemVer policy | SemVer 2.0; `local` feature exempt; MSRV bumps only in minor/major | `VERSIONING.md` |

Relevant Cargo features (`Cargo.toml` `[features]`): `server` (default; pulls `schemars`,
`transport-async-rw`), `macros` (default), `transport-io` (stdio), `transport-streamable-http-server`
(+ `-session`), `tower`, `reqwest` / `reqwest-native-tls` / `reqwest-tls-no-provider` (client
side only, reqwest **0.13.2**), `request-state` (SEP-2322 HMAC sealing), `elicitation`, `auth`,
`local` (drops `Send`/`Sync` bounds; outside SemVer). Hard runtime dependency: `tokio`
(`sync, macros, rt, time`).

### 1.2 `ServerHandler`

Defined by a `macro_rules!` body so the `local` and non-`local` variants share it
(`src/handler/server.rs:308-638`); the trait is
`pub trait ServerHandler: Sized + Send + Sync + 'static` (`:640-644`). Every method has a
default. Methods return `impl Future<Output = …> + MaybeSendFuture + '_` (RPITIT; `async fn`
is accepted in impls). At 3.5.1 the method list is (29):

| Group | Methods | Default |
| --- | --- | --- |
| Lifecycle | `ping`, `initialize`, `negotiate_initialize` (3.3+), `supported_protocol_versions`, `discover`, `get_info` | `initialize` stores peer info then negotiates (`:316-324`); `get_info` → `ServerConfig::default()` |
| Tools | `call_tool`, `list_tools`, `get_tool` (sync) | method-not-found / empty list / `None` (`:531-551`) |
| Prompts / resources | `get_prompt`, `list_prompts`, `list_resources`, `list_resource_templates`, `read_resource`, `complete`, `set_level` | not-found or empty |
| Subscriptions | `accepted_subscription_filter`, `listen` (2026-07-28), `subscribe` / `unsubscribe` (`#[deprecated]`, legacy only) | |
| Tasks (SEP-2663) | `get_task`, `update_task`, `cancel_task` | method-not-found |
| Escape hatches | `on_custom_request`, `on_custom_notification` | |
| Notifications | `on_cancelled`, `on_progress`, `on_initialized`, `on_roots_list_changed` | no-op |

The 3.0.0 and 3.2.0 tags have the same list minus `negotiate_initialize` (checked with
`git show rmcp-v3.0.0:crates/rmcp/src/handler/server.rs`). `negotiate_initialize` is only called
from the default `initialize` (`:323`), so a wrapper that forwards `initialize` can skip it and
still compile against every 3.x.

rmcp itself delegates the whole trait for `Box<T>` and `Arc<T>` with a private
`impl_server_handler_for_wrapper!` macro (`:652-873`). That macro is the exact pattern
`Tracked<H>` needs, and it is not exported.

### 1.3 `Service<RoleServer>` and the blanket impl

`Service<R>` has four methods: `handle_request`, `handle_notification`, `get_info`,
`supported_protocol_versions` (`src/service.rs:281-307`). Every `ServerHandler` is a
`Service<RoleServer>` through `impl<H: ServerHandler> Service<RoleServer> for H`
(`src/handler/server.rs:50-306`). The blanket impl does real work around the handler methods:

- **Before dispatch:** per-request protocol-version validation and the 2026-07-28 `_meta`
  requirement (`:56-100`); tasks capability gating (`:22-48`).
- **Dispatch:** a `match` on `ClientRequest` that calls `self.call_tool(...)`,
  `self.list_tools(...)`, etc. (`:103-245`).
- **After dispatch:** rejects `CallToolResponse::Task` if the client did not declare tasks
  (`:203-218`), rejects `InputRequiredResult` for pre-2026 peers, fills cache hints or strips
  `resultType` for legacy peers (`:246-262`), remaps `RESOURCE_NOT_FOUND` (`:264-271`).

Consequence: anything a wrapper does inside `call_tool`/`list_tools` runs *inside* this
validation. Requests rmcp rejects before dispatch never reach a `ServerHandler` wrapper.

`ServiceExt::serve(transport)` (`src/service.rs:330-357`) accepts any `Service<RoleServer>`;
`serve_server` likewise (`src/service/server.rs:420-430`). The streamable HTTP server does
**not**: it requires `S: ServerHandler` (`src/transport/streamable_http_server/tower.rs:1362,
1415, 1636`) because it calls `ServerHandler::get_tool` to validate SEP-2243 `Mcp-Param-*`
headers (`tower.rs:1602-1620`). This bound rules out a `Service`-level wrapper for HTTP.

Each request runs in its own spawned task (`src/service.rs:1581-1617`) inside a
`tokio::task_local!` scope that records the originating request (`:230-257`); cancellation is
cooperative through `RequestContext::ct` (the handler future is not aborted).

### 1.4 Macros and `ToolRouter`

- `#[tool]` marks a method; `#[tool_router]` on an inherent impl generates
  `fn tool_router() -> ToolRouter<Self>` that registers every `#[tool]`
  (`crates/rmcp-macros/src/tool_router.rs:78-91`). `#[tool_router(server_handler)]` also emits
  `#[tool_handler(router = Self::tool_router())] impl ServerHandler for Self {}` (`:97-112`).
- `#[tool_handler]` generates `call_tool`, `list_tools`, `get_tool` and `get_info` if absent
  (`crates/rmcp-macros/src/tool_handler.rs:44-107`). **Its default router expression is
  `Self::tool_router()`** (`:18-31`), i.e. the router is rebuilt from the static function on
  every call. Only `#[tool_handler(router = self.tool_router)]` uses a router stored in the
  struct. In rmcp's own examples and tests the two forms appear equally often (8 each).
- `ToolRouter<S>` (`src/handler/server/router/tool.rs:324-602`) is `#[non_exhaustive]` but has
  **public fields**: `map: HashMap<Cow<'static, str>, ToolRoute<S>>` and
  `transparent_when_not_found`. `ToolRoute<S>` (`:158-163`) has public `call:
  Arc<DynCallToolHandler<S>>` and `attr: Tool`.

Runtime mutability:

| Question | Answer |
| --- | --- |
| Add/remove tools at runtime? | Yes, on a `&mut ToolRouter`: `add_route` (`:436`), `remove_route` (`:456`), `disable_route`/`enable_route` (`:472-489`), `merge` (`:442`). A notifier sends `notifications/tools/list_changed` (`:509-529`). But a router behind `&self` in a handler needs the customer's own interior mutability, and the default macro path rebuilds the router per call, so runtime edits only work with a stored router. |
| Mutate input schemas after registration? | Yes: `route.attr.input_schema` is a public `Arc<JsonObject>` (`src/model/tool.rs:17-40`); `Arc::make_mut` copies on write, so the customer's original schema is not mutated. Routes can be re-wrapped by assigning `route.call`. |
| Does rmcp validate arguments or outputs against schemas? | No. Arguments are only deserialized with serde (`src/handler/server/tool.rs:235-250`); there is no JSON Schema validator in the server (the docs point users to the `jsonschema` crate, `src/model.rs:4203`). Injected properties marked `required` therefore affect only schema-validating clients. |
| What happens to unknown argument keys? | Ignored by serde unless the customer's type has `#[serde(deny_unknown_fields)]`, in which case the call fails with `failed to deserialize parameters`, which `ToolRouter::call` turns into `Ok(CallToolResult::error(...))` (`router/tool.rs:145-156, 560-579`). Stripping before dispatch is therefore mandatory. |

`Router<S>` (`src/handler/server/router.rs:20-122`) is a separate builder
(`Router::new(handler).with_tool(...)`) that implements **only** `Service<RoleServer>`, not
`ServerHandler`, so it cannot be served by `StreamableHttpService`.

### 1.5 `RequestContext`, `Peer`, `Extensions`, `Meta`

```rust
// src/service.rs:1205-1216  (#[non_exhaustive], Clone, all fields pub)
pub struct RequestContext<R: ServiceRole> {
    pub ct: CancellationToken,      // cancelled on notifications/cancelled or HTTP disconnect
    pub id: RequestId,
    pub meta: RequestMetaObject,    // the request's `_meta`, lifted out of params before dispatch
    pub extensions: Extensions,     // type map; HTTP transport inserts http::request::Parts
    pub peer: Peer<R>,              // send notifications/requests to the client; handshake info
}
```

- **`_meta`** is moved from the request into `context.meta` before the handler runs
  (`src/service.rs:1587-1599`; storage via `GetMeta` in `src/model/meta.rs:160-168`). Read
  `_meta` from `context.meta`, not from `CallToolRequestParams.meta`. Reserved keys keep their
  fully qualified names: `io.modelcontextprotocol/protocolVersion`, `…/clientInfo`,
  `…/clientCapabilities`, `…/logLevel` (`meta.rs:392-403`), with typed accessors
  `client_info()`, `protocol_version()`, `client_capabilities()` (`meta.rs:446-489`).
- **`Peer<RoleServer>`** (`service.rs:726-737`) holds the handshake info: `peer_info() ->
  Option<Arc<InitializeRequestParams>>` (`:1029`), which includes `client_info:
  Implementation` and `protocol_version`. It has no session ID.
- **Convenience ladder:** `RequestContext::client_info()` returns `_meta` clientInfo, falling
  back to `peer_info().client_info` only for legacy sessions (`service.rs:1242-1251`);
  `protocol_version()` falls back the same way (`:1233-1240`).
- **`Extensions`** (`src/model/extension.rs`) is an `http::Extensions`-style type map.
  The streamable HTTP server inserts the request's `http::request::Parts` (headers, URI,
  method, axum extensions) for every request (`tower.rs:1273-1331` docs; inserts at `:1544,
  2091, 2176, 2295`). Tools read it with the `Extension<T>` extractor
  (`src/handler/server/common.rs:180-190`). A wrapper can also *insert* values for the inner
  handler.

How client identity reaches the handler, by transport and protocol:

| Situation | `context.meta.client_info()` | `context.peer.peer_info()` |
| --- | --- | --- |
| stdio or HTTP session, legacy `initialize` handshake | absent | real `InitializeRequestParams` |
| any transport, 2026-07-28 inline lifecycle | present (required `_meta`) | `None` or bootstrap; `request_metadata_required` set (`service/server.rs:668`) |
| stateless HTTP, legacy-version request without `_meta` | absent | **synthesized**: `client_info: Implementation::default()` (`tower.rs:2417-2436`) |

**Pitfall:** `Implementation::default()` is `Implementation::from_build_env()`, which expands
`env!("CARGO_CRATE_NAME")` inside rmcp (`src/model.rs:1556-1565`). The synthesized client is
therefore named `"rmcp"` with rmcp's own version, and `RequestContext::client_info()` returns
it. The same placeholder appears as the **server** identity when a customer uses
`ServerConfig::default()` (`get_info` default, `server.rs:602-604`). The SDK must treat an
`Implementation` equal to `Implementation::default()` as "unknown" on the client side and log
a one-time warning on the server side.

### 1.6 Transports

| Transport | API | Notes |
| --- | --- | --- |
| stdio | `handler.serve(rmcp::transport::stdio()).await?.waiting().await` (`src/transport/io.rs:1-6`, examples `counter_stdio.rs`) | one handler instance per process |
| any `AsyncRead + AsyncWrite` | `transport-async-rw` | used for in-process tests (duplex) |
| streamable HTTP server | `StreamableHttpService::new(factory, session_manager, config)` (`tower.rs:1413-1438`); a `tower_service::Service<http::Request<B>>` (`:1359-1383`), mounted with `axum::Router::nest_service` | `factory: Fn() -> Result<S, io::Error>` |
| session managers | `LocalSessionManager` (in-memory), `NeverSessionManager` (no sessions) (`session/local.rs:33`, `session/never.rs:19`); optional `SessionStore` for cross-instance restore (replays `initialize`) (`tower.rs:133-154`) | |
| stateless mode | `StreamableHttpServerConfig { legacy_session_mode: false, json_response, stateless_protocol_metadata_required, .. }` (`tower.rs:76-211`) | 2026-07-28 requests are **always** stateless regardless of `legacy_session_mode` (`:83-90`); default `legacy_session_mode = true` |
| unix socket | client only (`transport-streamable-http-client-unix-socket`) | |
| third party | `rmcp-actix-web` 0.13.1 (depends on `rmcp ^3.0.0`, 257 k downloads) | actix is not tower |

**Factory call frequency matters for a wrapper.** In session mode the factory runs once per
session (`spawn_session_worker`, `tower.rs:1629-1641`). In stateless mode it runs **once per
HTTP request** (`tower.rs:2276-2278`), and it also runs to read `get_tool` for header
validation (`:1611-1614`). Wrapper construction must therefore be cheap and side-effect free,
and all shared state must live in an `Arc` captured by the factory closure.

### 1.7 2026-07-28 support

rmcp 3.x implements the stateless revision: per-request `_meta` validation and inline version
negotiation (`server.rs:56-100`), `server/discover` (`:108-111`), always-stateless HTTP for
2026-07-28 (`tower.rs:83-90`, CHANGELOG #999), `subscriptions/listen` (`server.rs:147-184`),
MRTR `InputRequiredResult` (`server.rs:246-252`, `src/model/mrtr.rs:105-115`), SEP-2549 cache
hints (`server.rs:253-259`), SEP-2243 standard headers (`tower.rs:2273`), SEP-2663 tasks
extension, SEP-2322 `request-state` sealing. `ROADMAP.md` reports 30/30 server conformance on
both the 2025-11-25 and 2026-07-28 suites. Still open upstream: **SEP-2567 sessionless MCP
via explicit state handles (#870)** (`ROADMAP.md:69`) — the same SEP AgentCat's injected
`session_id` follows, so watch for overlap.

### 1.8 Hook points and middleware

rmcp has **no built-in middleware, interceptor or hook API** for servers (a search for
`middleware|interceptor|hook` in `src/` finds nothing relevant). The available seams are:
(1) implementing/wrapping `ServerHandler`; (2) implementing/wrapping `Service<RoleServer>`;
(3) tower layers around `StreamableHttpService` (HTTP only); (4) mutating a `ToolRouter`
before it is used; (5) `RequestContext.extensions` for passing data to handlers.

### 1.9 Error representation

- `ErrorData` (re-exported as `rmcp::ErrorData`, aliased `McpError`) is
  `{ code: ErrorCode, message: Cow<'static, str>, data: Option<Value> }` (`src/model.rs:642-653`).
- `call_tool` returns `Result<CallToolResponse, McpError>`. The trait docs spell out the
  contract (`server.rs:503-530`): `Err(McpError)` is a JSON-RPC protocol error that clients
  usually render opaquely; `Ok(CallToolResult::error(...))` is a tool failure the user sees.
- `CallToolResult { result_type, content, structured_content, is_error: Option<bool>, meta }`
  (`src/model.rs:3972-4000`, `#[non_exhaustive]`, constructors `success`/`error`/`structured`
  at `:4074-4155`).
- `CallToolResponse` (`src/model/mrtr.rs:105-115`, `#[non_exhaustive]`) is `Complete(..)`,
  `InputRequired(InputRequiredResult)` (MRTR intermediate round) or `Task(CreateTaskResult)`.
- Unknown or disabled tool through `ToolRouter`: `Err(invalid_params("tool not found"))`
  (`router/tool.rs:565-571`). Argument deserialization failure: flattened to
  `Ok(CallToolResult::error(...))` (`:145-156`). A `#[tool]` returning `Err(E)` is flattened to
  an `is_error` result by `IntoCallToolResult` (`src/handler/server/tool.rs:125`), so the typed
  error is gone by the time a wrapper sees it.
- A **panic** in a handler kills the spawned request task; no response is sent.

### 1.10 Gotchas for an analytics wrapper (summary)

1. Forward every `ServerHandler` method; a missed one silently reverts to rmcp's default.
2. `StreamableHttpService` needs `ServerHandler`, not `Service`.
3. Stateless HTTP constructs the handler per request; also for `get_tool` probes.
4. Read `_meta` from `context.meta`; never rebuild `CallToolRequestParams` (it carries
   `input_responses` and `request_state` for MRTR).
5. Reject the `"rmcp"` placeholder `Implementation` as client identity.
6. Capabilities appear in three places: `get_info`, the `initialize` result, the `discover`
   result. Adding `get_more_tools` must patch all three.
7. `serde_json::Map` is a `BTreeMap` unless something in the build enables
   `serde_json/preserve_order`; injected properties then sort alphabetically.
8. Panics and runtime shutdown drop events unless handled.

---

## 2. Other Rust MCP server crates

Data from the crates.io API on 2026-10-16 (`/api/v1/crates/{name}` and
`/reverse_dependencies`).

| Crate | Downloads (90 d) | Rev. deps | Latest | Notes |
| --- | --- | --- | --- | --- |
| **rmcp** | 36.6 M (20.3 M) | **2,232** | 3.5.1 (2026-10-05) | official; Tier 1; 2026-07-28 |
| rust-mcp-sdk (+ `rust-mcp-schema`, `-transport`, `-macros`) | 314 k (118 k) | 52 | 2.0.0 (2026-08-27) | own `ServerHandler` and `ServerHandlerCore` traits; README claims full 2026-07-28 |
| rmcp-actix-web | 257 k (69 k) | 5 | 0.13.1 | actix transport **for rmcp** (`rmcp ^3.0.0`) |
| rmcp-openapi | 137 k (13 k) | — | 0.32.0 | OpenAPI → rmcp tools (`rmcp ^3.4.0`) |
| pmcp | 131 k (56 k) | 23 | 2.22.7 | serves 2025-11-25 and 2026-07-28 per request |
| turbomcp | 36 k (10 k) | 8 | 4.0.0-alpha.5 | dispatcher is a `tower::Service<JsonRpcMessage>` |
| tower-mcp | 34 k (22 k) | 15 | 0.24.0 | "an MCP server is a `tower::Service`" |
| poem-mcpserver | 55 k (1 k) | 1 | 0.3.1 (2025-10) | Poem framework |
| mcp-core, mcp-server, mcp-spec, mcpr, mcp-sdk, ultrafast-mcp, mcp-attr, … | < 105 k each, mostly stale since mid-2025 | | | abandoned or niche |
| smg-mcp | 1.8 M | | | MCP **client**; not relevant |

**Recommendation:** v1 supports **only rmcp 3.x**. rmcp has ~40× the reverse dependencies of
the next server SDK, and two of the next five crates are rmcp add-ons that a
`ServerHandler` wrapper covers automatically (rmcp-actix-web, rmcp-openapi). Keep the core
crate library-neutral (as Go's root module is) so later adapters stay small. The most
promising second target is a **JSON-RPC-message-level tower `Layer`**, which would cover
tower-mcp and turbomcp together because both expose `tower::Service` over MCP messages; add it
only on customer demand. Older rmcp majors (about half of the top 100 dependents) are a
separate decision; 1.x and 2.x lack the MRTR and 2026-07-28 types, so an adapter would be a
partial port.

---

## 3. Interception design

### 3.1 What must be intercepted

From the TS SDK (`src/engine/callWrap.ts`, `listWrap.ts`, `registrationPatch.ts`) and the
current v2 behavior (§4.5):

| Concern | Required behavior |
| --- | --- |
| `tools/call` | resolve handles (`session_id`, optional `agent_id`), strip injected args on a copy, time the call, capture raw args + original result or error, decorate the wire result (mint-back text block first, `mcp_session` mirror), tag MRTR rounds, publish one `mcp:tools/call` event |
| `tools/list` | inject `session_id`, `agent_id` (opt-in), `context` into each tool's input schema (skip collisions and composed schemas, drop `additionalProperties: false`), declare `mcp_session` in output schemas, append `get_more_tools`, record registries; publish **no** event |
| rebuild on demand | a `tools/call` on an instance that never listed must recompute the registry deterministically |
| identity | per request: client name/version ladder, protocol version tag, `identify` hook on every call, no caches |
| custom events | `publish_custom_event` |
| shutdown | flush queued events |

### 3.2 Option (a) — `Tracked<H: ServerHandler>` (recommended)

```rust
// crates/agentcat-rmcp/src/tracked.rs  (sketch)
use std::{panic::AssertUnwindSafe, sync::Arc, time::Instant};
use futures::FutureExt;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::*,
    service::{NotificationContext, RequestContext},
};

pub struct Tracked<H> {
    inner: H,
    shared: Arc<Shared>,   // config, hooks, engine, publisher sender: one per track()/track_factory()
    listed: Registry,      // per instance (TS WeakMap<server> analogue): what THIS instance advertised
}

impl<H: ServerHandler> ServerHandler for Tracked<H> {
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut page = self.inner.list_tools(request, context).await?;
        if let Some(engine) = self.shared.engine() {
            let last_page = page.next_cursor.is_none();
            // Pure + deterministic; Arc::make_mut copies each schema before editing it.
            let (tools, reg) = engine.inject(std::mem::take(&mut page.tools), last_page);
            page.tools = tools;            // includes get_more_tools on the last page
            self.listed.merge(reg);
        }
        Ok(page)                           // ttl_ms / cache_scope untouched
    }

    async fn call_tool(
        &self,
        mut request: CallToolRequestParams,
        mut context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(engine) = self.shared.engine() else {
            return self.inner.call_tool(request, context).await;
        };
        // listed → inner.get_tool(name) → inner.list_tools(None, ctx.clone()) → heuristic
        let reg = self.registry_for(&request.name, &context).await;
        // Resolve handles, MRTR continuation (request.input_responses.is_some()),
        // client-info ladder, protocol version, identify/tags/properties hooks (catch_unwind),
        // http::request::Parts projection. Clones raw args once for the event.
        let call = engine.begin(&request, &context, &reg);
        if call.is_get_more_tools() {
            return Ok(call.answer_get_more_tools(&self.shared).into());
        }
        request.arguments = call.strip(request.arguments.take()); // never rebuild the params
        context.extensions.insert(call.public_info());          // tools may read Extension<agentcat::CallInfo>

        let started = Instant::now();
        let outcome = AssertUnwindSafe(self.inner.call_tool(request, context))
            .catch_unwind()
            .await;
        match outcome {
            Ok(mut result) => {
                let event = call.finish(&result, started.elapsed()); // serializes the ORIGINAL result
                engine.decorate(&mut result, &call);                  // then edits the wire result in place
                self.shared.publish(event);                           // try_send, never blocks
                result
            }
            Err(panic) => {
                self.shared.publish(call.finish_panic(&panic, started.elapsed()));
                std::panic::resume_unwind(panic) // keep rmcp's behavior
            }
        }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        // Advertised (injected) definition; also answers "get_more_tools".
        self.shared.advertised_tool(&self.inner, name)
    }

    fn get_info(&self) -> InitializeResult {             // `ServerConfig` is an alias (model.rs:1178)
        let mut info = self.inner.get_info();
        self.shared.patch_capabilities(&mut info.capabilities); // tools capability for get_more_tools
        info
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        let mut result = self.inner.initialize(request, context).await?; // inner sets peer info
        self.shared.patch_capabilities(&mut result.capabilities);
        Ok(result)
    }

    // discover(): same "forward, then patch capabilities" shape.

    // The remaining 21 methods are one-line forwards, generated by our own macro modeled on
    // rmcp's private impl_server_handler_for_wrapper! (handler/server.rs:652-873):
    forward_server_handler! { inner =>
        ping, supported_protocol_versions, complete, set_level, get_prompt, list_prompts,
        list_resources, list_resource_templates, read_resource, accepted_subscription_filter,
        listen, subscribe, unsubscribe, on_custom_request, on_cancelled, on_progress,
        on_initialized, on_roots_list_changed, on_custom_notification, get_task, update_task,
        cancel_task
    }
}
```

Customer code: `agentcat.track(MyServer::new()).serve(stdio())`, or
`StreamableHttpService::new(agentcat.track_factory(|| Ok(MyServer::new())), ..)`.

Why it fits rmcp:

- One type serves stdio (via the blanket `Service` impl), streamable HTTP (session and
  stateless), `rmcp-actix-web`, and custom transports.
- It sees `#[tool_handler]` servers in both router forms and hand-written `call_tool`
  implementations, including unknown-tool errors.
- No re-serialization: arguments are already a `serde_json::Map`; stripping is
  `Map::remove`, so large integers are never re-encoded (Go needed `json.Number`, §4.3).
- Per-instance registry state lives in the wrapper itself, so per-request factories need no
  weak maps (Go needed `runtime.AddCleanup` and `weak.Make`, §4.3). Ownership ends the state
  when rmcp drops the instance.
- `get_tool` gives a cheap, synchronous, per-tool rebuild for macro-generated servers; the
  pure engine computed on one tool equals the full-list result for that tool.

### 3.3 Option (b) — tower `Layer` on the streamable HTTP service

```rust
#[derive(Clone)]
pub struct AgentCatLayer { shared: Arc<Shared> }

impl<S> tower::Layer<S> for AgentCatLayer {
    type Service = AgentCatHttp<S>;
    fn layer(&self, inner: S) -> AgentCatHttp<S> { AgentCatHttp { inner, shared: self.shared.clone() } }
}

impl<S, B> tower::Service<http::Request<B>> for AgentCatHttp<S>
where
    S: tower::Service<http::Request<Full<Bytes>>, Response = http::Response<BoxBody<Bytes, Infallible>>>
        + Clone + Send + 'static,
    S::Future: Send,
    B: http_body::Body + Send + 'static,
{
    // call():
    // 1. Buffer the body (respect max size) and parse the JSON-RPC message.
    // 2. tools/call: resolve handles, strip args, re-serialize, fix Content-Length.
    //    initialize (legacy): remember clientInfo per Mcp-Session-Id for later requests.
    // 3. Forward. Response is application/json (patch whole body) or text/event-stream
    //    (map the SSE stream, find the frame whose JSON-RPC id matches, patch it).
    // 4. tools/list: inject schemas into the matching frame. Publish when the frame passes.
}
// usage: Router::new().nest_service("/mcp", ServiceBuilder::new().layer(agentcat.http_layer()).service(mcp))
```

Strengths: independent of rmcp's Rust types (wire-level), sees requests rmcp rejects (bad
`_meta`, header mismatch), sees raw HTTP headers. Weaknesses: **no stdio** (most local MCP
servers), not actix; parses and re-serializes every body and SSE frame; must re-implement
legacy session tracking for client info; must correlate request IDs across SSE streams,
resumable streams (event store) and `json_response` mode; timing includes transport time.

### 3.4 Option (c) — wrap the `ToolRouter`

```rust
pub fn instrument_router<S: Send + Sync + 'static>(ac: &AgentCat, mut router: ToolRouter<S>) -> ToolRouter<S> {
    let engine = ac.engine();
    for route in router.map.values_mut() {          // pub field: keeps disabled set + notifier
        let (advertised, injected) = engine.inject_one(&route.attr);
        route.attr = advertised;                     // list_all()/get() now advertise injected params
        let inner = route.call.clone();
        let engine = engine.clone();
        route.call = Arc::new(move |mut tcc: ToolCallContext<'_, S>| {
            let call = engine.begin_router(&tcc, &injected);
            tcc.arguments = call.strip(tcc.arguments.take());
            let fut = inner(tcc);
            Box::pin(async move {
                let started = Instant::now();
                let mut out = fut.await;
                engine.finish_and_decorate(call, &mut out, started.elapsed());
                out
            })
        });
    }
    router.add_route(get_more_tools_route::<S>(&engine));
    router
}
// Customer must store the router AND switch the macro:
//   tool_router: agentcat.instrument(Self::tool_router()),
//   #[tool_handler(router = self.tool_router)]
```

Strengths: no `ServerHandler` drift; schemas are injected once at registration.
Weaknesses: silently does nothing with the **default** `#[tool_handler]` (router rebuilt from
`Self::tool_router()` each call) and with `#[tool_router(server_handler)]`; no coverage for
hand-written `call_tool`; never sees unknown-tool calls; requires the customer to edit two
places; couples to `ToolRoute`/`ToolCallContext` field layout (`#[non_exhaustive]` types with
public fields, which rmcp may change in a minor release).

### 3.5 Option (d) — `TrackedService<S: Service<RoleServer>>`

Same engine, wrapping the 4-method `Service` trait and matching on
`ClientRequest::CallToolRequest` / `ListToolsRequest`. No drift risk, but it cannot be used
with `StreamableHttpService` (needs `ServerHandler`) and has no `get_tool`, so rebuild-on-demand
must send an internal `ListToolsRequest`. Ship it as the entry point for rmcp `Router<S>`
users and custom `Service` implementations (stdio and other `serve()` transports).

### 3.6 Comparison

| Criterion | (a) `Tracked<H>` | (b) tower Layer | (c) router wrap | (d) `TrackedService<S>` |
| --- | --- | --- | --- | --- |
| stdio | yes | **no** | yes | yes |
| rmcp streamable HTTP (session + stateless) | yes | yes | yes | **no** |
| rmcp-actix-web | yes | no | yes | no |
| `#[tool_handler]` default router | yes | yes | **no** | yes |
| hand-written `ServerHandler` | yes | yes | **no** | yes |
| rmcp `Router<S>` builder | no | n/a | yes | yes |
| unknown-tool calls observed | yes | yes | no | yes |
| requests rejected by rmcp before dispatch | no | yes | no | partly |
| typed `RequestContext` (meta, peer, extensions) | yes | no | yes | yes |
| extra parsing per request | none | full JSON + SSE | none | none |
| coupling to rmcp API | high (29-method trait) | low (wire) | medium | low |
| customer change | wrap one value | add one layer | constructor + macro attribute | wrap one value |

### 3.7 Recommendation and per-concern mapping

Ship **(a)** as the default (`agentcat.track(h)` / `agentcat.track_factory(f)`) and **(d)** for
`Router<S>`/custom services, both over one rmcp-agnostic engine in the core crate. Keep (b) as
a possible later "zero-code / multi-library" adapter; reject (c).

| TS behavior | How `Tracked<H>` does it with rmcp |
| --- | --- |
| time `tools/call`, capture args/result/errors | `Instant` around `inner.call_tool`; classify `Err(ErrorData)` (protocol error: code, message, data), `Ok(Complete(r))` with `r.is_error == Some(true)` (tool error: join text blocks, as Go does at `officialsdk/middleware.go:351-366`), `Ok(InputRequired)` (tag `agentcat_mrtr=input_required`, no decoration), `Ok(Task)` (open question, §6) |
| inject params into schemas | edit `ListToolsResult.tools[*].input_schema` / `output_schema` (`Arc<JsonObject>`, copy-on-write); same engine behind `get_tool` so SEP-2243 header validation and rebuilds see one definition |
| append `get_more_tools` | append on the last page (`next_cursor == None`) unless the inner server defines that name; answer it in `call_tool`; add the tools capability in `get_info`/`initialize`/`discover` |
| strip injected args | `request.arguments: Option<JsonObject>`, remove only registry-listed keys; heuristic strip of all three names (except `get_more_tools`' own `context`) only when no registry can be built |
| per-request client identity | `context.meta.client_info()`, then `context.peer.peer_info().client_info` unless it equals `Implementation::default()`; narrow each field; never cache |
| protocol version tag | `context.meta.protocol_version()` |
| identify / event tags / properties | sync callbacks over a `ToolCall<'_>` view (name, raw args, `&RequestContext`, helpers for `http::request::Parts` headers); run under `catch_unwind`; on every call |
| request "extra" | `context.extensions.get::<http::request::Parts>()` → `{ method, uri, headers }` (TS `src/modules/extra-projection.ts`) |
| server name/version | `inner.get_info().server_info`, read once at `track()`; warn if it is the `"rmcp"` placeholder |
| sessions | transport session IDs ignored (v2); correlation only through the injected `session_id` |
| flush on shutdown | `agentcat.shutdown(timeout).await` after `RunningService::waiting()` (`service.rs:1099`) or after axum graceful shutdown; bounded flush in `Drop` as fallback |
| handler panics | `catch_unwind`, publish an error event, `resume_unwind` |

### 3.8 Limits of the recommended design

1. **Trait drift.** Any `ServerHandler` method added in a future 3.x minor is not forwarded and
   silently falls back to rmcp's default for tracked servers. Mitigations: a CI tripwire that
   extracts the method list from the resolved rmcp source (`cargo metadata` → `src/handler/server.rs`
   `server_handler_methods!`) and fails on a mismatch, run weekly against the newest rmcp; a
   probe-handler test that overrides every method and asserts each request type reaches it
   through `Tracked`; an upstream request for a public delegation macro or for
   `StreamableHttpService` to accept `S: Service<RoleServer>` plus a `get_tool` hook.
2. **Pre-dispatch rejections** by rmcp (missing 2026 `_meta`, unsupported version, Host/Origin
   checks, SEP-2243 header mismatch) produce no event.
3. **`Router<S>` users** need `TrackedService`, which cannot serve streamable HTTP (neither can
   `Router<S>` itself).
4. **Hand-written handlers without `get_tool`**: the first call on a fresh instance pays one
   `inner.list_tools` to rebuild the registry; paginated lists may need several pages.
5. **Typed tool errors** are flattened by rmcp before the wrapper sees them; only text
   content and `ErrorData` are available. An opt-in "inner tap" (a task-local slot that
   customer code fills via `agentcat::capture_error(&e)`) could recover detail later, like TS
   `src/engine/innerTap.ts` and Python `adapters/_inner_tap.py`.
6. **`local` feature** (non-`Send` handlers) is not supported; **wasm** (Cloudflare Workers) is
   not supported because the publisher needs a thread.
7. **Nested calls**: a tool that calls another tool through `self` bypasses the wrapper, so no
   nested event is produced (the opposite of Python's FastMCP Code Mode problem, §4.3).

---

## 4. Lessons from the Go, Python and Java ports

### 4.1 How each port mapped the TS design without monkey-patching

| | Go (`/home/ubuntu/agentcat-go-sdk`, v2.1, HEAD `7e71df8`) | Python (`/home/ubuntu/agentcat-python-sdk`, HEAD `8533bdf`) | Java (`/home/ubuntu/agentcat-java-sdk-private`, 0.1.0, HEAD `99a29e8`) |
| --- | --- | --- | --- |
| Libraries | mark3labs/mcp-go v0.53–0.57; official go-sdk v1.4.1–1.7.0 (`CLAUDE.md:99`) | official `mcp` 1.x and 2.x lowlevel, FastMCP 3/4 (`pyproject.toml:20-35`) | MCP Java SDK 0.18.x, 1.x, 2.x (`pom.xml` profiles; CI `99a29e8`) |
| Seam | official go-sdk: one **receiving middleware** (`mcp.Server.AddReceivingMiddleware`) switching on `tools/list` / `tools/call` (`officialsdk/officialsdk.go:247`, `middleware.go:37-78`). mcp-go: `tools/list` **after-hook** + tool-handler **middleware** (`server.Use`) + **failure hooks** (`mcpgo/failure_hooks.go:15-45`) | replace the entry in the server's `_request_handlers` dict with a wrapper, keeping the registration seam armed (`adapters/lowlevel_v2.py:1-30, 566`); FastMCP v3 middleware | reflection: read the private `tools` list, wrap each tool spec, write back a `CopyOnWriteArrayList` subclass that wraps later `addTool` calls (`AgentCat.java:304-327, 347-367`, `StatelessToolList.java`) |
| Where reflection remains | reading the unexported server `impl` field for name/version (`officialsdk.go:298-325`); compat shims (`compat.go`) | private attribute access by design | private fields; `McpSdkBridgeV1` reflects 1.x `Tool` builders (`McpSdkBridgeV1.java:1-60`) |
| Structure | root module (shared engine in `internal/`, re-exported integration API in `mcpcat.go`, `types.go`) + one Go module per adapter (`CLAUDE.md:71`) | one package, `modules/` core + `modules/adapters/*` per library era, `detection.py` | one jar: `sdk/engine` (bridges), `sdk/modules` (handles, events, pipeline), plus `spring/`, `micronaut/`, `quarkus/` auto-config |
| API client | generated `go.agentcat.com/api` v1.0.0: `apiClient.EventsAPI.PublishEvent(ctx).PublishEventRequest(req).Execute()` (`internal/publisher/publisher.go:254`) | generated `agentcat-api==1.0.0`: `EventsApi.publish_event(publish_event_request=...)` (`event_queue.py:238`) | generated `com.agentcat:api:0.0.2`: `EventsApi.publishEvent(req)` (`EventQueue.java:97, 471`) |
| Queue | global singleton, 10,000 events, 5 workers, 3 retries at 1 s/2 s/4 s, `Shutdown(ctx)` default 5 s (`internal/publisher/config.go:7-24`, `publisher.go:212, 311`) | 10,000-event queue, daemon worker, 10 s per attempt; exit hook **stops** the worker and never sends (`event_queue.py:1-40`) | async queue (`EventQueue.java`) |

All three generated clients come from one generator setup in
`/home/ubuntu/agentcat-server/sdk-generator/` (`openapi-config.yaml`, scripts per language,
OpenAPI Generator pinned to **7.14.0** in `openapitools.json`). The endpoint is
`POST /v1/public/events/`, 202 Accepted (`/home/ubuntu/agentcat-go-api/api/openapi.yaml:101-116`).

### 4.2 What Rust should copy

- **Core/adapter split with a pure engine.** Go's `inject.Build(cfg, tools)` is deterministic
  and idempotent so rebuilds match listings (`CLAUDE.md:79`); handles, constants and event
  building live in the core; adapters only translate library types. Rust: `agentcat-core`
  (library-neutral) + `agentcat-rmcp`.
- **`track()` never fails the server.** Go returns `(nil, err)` only for nil server/empty
  project and is idempotent per server (`officialsdk.go:181-216`); Java catches `Throwable`.
  Rust: `build()` is infallible and degrades to pass-through with a `tracing::warn!`.
- **Synchronous capture, one bounded queue.** Go builds the event on the request goroutine and
  hands it to a non-blocking queue that drops with a warning (`middleware.go:17-36`).
- **Registries before handle resolution** (ownership decides whether `session_id` is ours),
  and "a tool missing from the registries counts as ours" (`CLAUDE.md:77`).
- **Compatibility range + tripwires.** Go supports a version range per library and pins shims
  with tripwire tests (`CLAUDE.md:99`). Rust: support `rmcp >=3.0, <4`, test the floor and the
  newest, and add the trait-drift tripwire (§3.8).

### 4.3 Pitfalls the ports hit, and the Rust translation

| Port pitfall (source) | Rust translation |
| --- | --- |
| Double `Track()` installed two middlewares: two mint-back blocks, two events, and a rebuild that read already-injected tools so stripping stopped (`officialsdk.go:206-216`) | `track()` on an already-`Tracked` value is a type error; for `TrackedService<Tracked<H>>` detect via a marker in `Extensions` and pass through |
| Decoding arguments as float64 corrupts integers > 2^53; fix with `json.Number` (`middleware.go:102-111, 319-331`) | never re-serialize; mutate the existing `serde_json::Map` |
| Per-request factories leaked one registry entry per request; fixed with weak refs and `runtime.AddCleanup` (`CLAUDE.md:85`) | state per instance lives in the wrapper; only config/publisher are shared via `Arc` |
| mcp-go `Use()` wraps only the handler: unknown tools and invalid args are rejected before it, output validation replaces results after it; needed after-hooks and `OnError` (`mcpgo/failure_hooks.go:15-45`) | `Tracked` wraps the whole `call_tool`; rmcp has no server-side schema validation to replace results |
| Reading the server name needed reflection on an unexported field (`officialsdk.go:298-325`) | `ServerHandler::get_info()` is public |
| Identify gets the live request context; a cancelled call yields no identity (`officialsdk.go:94-99`) | same: run hooks before dispatch; document "no network calls" |
| Stateless mode leaked one user's client info into another's events; fixed by per-request extraction (Python `1bdfd6d`, #28) | never cache client info; resolve from `context` every call |
| Deep-copying whole tool objects failed on non-copyable fields (RLock), silently skipping injection (Python `e167664`, #38) | copy only the schema (`Arc::make_mut`) |
| Non-JSON-serializable payloads dropped events after 3 retries (Python `bb1e9bc`, #40) | everything is `serde_json::Value`; still cap sizes and handle serialization errors per event |
| Nested tool calls in FastMCP Code Mode rebuilt registries from the raw catalog and broke stripping; fix: nested lists verbatim, nested calls join the outer session (Python `c8803b5`, #43) | rmcp self-calls bypass the wrapper; document; no nested events |
| `redact_event` hook could forge or erase actor, client and server identity; restored fields were aliased (Python `1da7a0b`, `8846cf5`, #50/#51) | `redact_event: Fn(Event) -> Option<Event>`; after it runs, restore protected fields from the original by value |
| Generated model's `event_type` enum predates `agentcat:custom`, so custom events failed validation; Python overrides the validator by name (`types.py:89-110`) | the Rust generator emits a **closed enum**; `agentcat:custom` is unrepresentable. Add it to the server's OpenAPI enum before generating, or post-process `event_type` to `String` |
| Generated model marks `project_id` required, but events are built before it is known (`types.py:83-87`) | own `Event` struct; convert at send time |
| `addTool`/`removeTool` triggered `notifyToolsListChanged` and could block; tool capability missing made `addTool` throw (Java `AgentCat.java:301-347`) | never mutate the customer's registration; patch capabilities in results |
| Agent ID trimmed in Java (`fdcc0df`) but kept verbatim in Go (`internal/handles/handles.go:82-88`) | follow TS `main`; add a cross-SDK fixture test |
| Docs drift: Go's own `CLAUDE.md:75` still describes the pre-#13 behavior (optional `session_id`, trailing block, `_mcp_instructions`) | generate the constants fixture from TS source, not from docs |

### 4.4 Event model and API client lessons

- Go embeds the generated `PublishEventRequest` in its `Event`; Python subclasses it and had
  to override validators. Rust should define its **own public `Event` struct** (stable for
  redaction hooks) and convert with an exhaustive destructuring of the generated struct, so a
  new spec field is a compile error (a free tripwire).
- The Rust generator maps `nullable` optional fields to `Option<Option<T>>` with
  `serde_with::rust::double_option` (generator `rust/model.mustache:98,143`, v7.26.0).
  `session_id` is nullable (`openapi.yaml`, `publishEvent_request.session_id`); sessionless
  events must send an explicit JSON `null` (`Some(None)`), as Go does (`CLAUDE.md:77`).
- The spec already has `input_tokens` / `output_tokens` (TS `e4b2190`, #71), which Go has not
  adopted yet.

### 4.5 Cross-SDK brief versus current `main`

| Topic | Brief (`2026-07-28-cross-sdk-changelog.md`) | Current TS `main` (`1c4f8b1` #67, `e4b2190` #71), Go `7e71df8`, Python `8533bdf` |
| --- | --- | --- |
| handle parameter | `task_id` | `session_id` (TS `src/modules/constants.ts:20-32`) |
| required? | optional; omission mints | required, pattern `^(start\|ses_[0-9A-Za-z]{27})$` (`constants.ts:27`); `start` begins a task; servers still accept omission |
| supplied values | trusted verbatim | validated; unknown values publish sessionless and get an "unrecognized" block (Go `CLAUDE.md:77`) |
| mint-back text | trailing block | **leading** block (first content element) |
| structured mirror | `_mcp_instructions` with prose | `mcp_session` `{session_id?, agent_id?, status}` (`constants.ts:58-72`) |
| source tag | `agentcat_task_id_source` | `agentcat_session_id_source` (`constants.ts:81`) |
| token counts | — | `input_tokens` / `output_tokens` on tool-call events (TS #71) |
| hook-mode `agent_id` copy | separate string | single description (Go `7e71df8` message) |

The Rust SDK should treat TS `main` as the spec and port its byte-pinned constants with a
fixture test.

---

## 5. Proposed Rust workspace

### 5.1 Layout

```
agentcat-rust-sdk/
├── Cargo.toml                 # [workspace] resolver = "3"; edition = "2024"; rust-version = "1.88"
├── crates/
│   ├── agentcat-core/         # library-neutral engine; "integration API", semver-exempt like Go's root module
│   │   └── src/
│   │       ├── config.rs      # Options shared by all adapters
│   │       ├── constants.rs   # byte-pinned agent-facing copy, param names, tags, meta keys
│   │       ├── handles/       # KSUID, mint, derive (SHA-256 → ses_ KSUID), validate, resolve
│   │       ├── inject/        # pure engine: build(cfg, tools) -> (tools, Registries); strip; mirror
│   │       ├── event/         # public Event, builder, SDK tags, token estimates, client-info narrowing
│   │       ├── pipeline/      # redact (string + event-level), sanitize, truncate
│   │       ├── publisher/     # bounded queue, worker thread + own runtime, retry, flush/shutdown
│   │       ├── exporters/     # otlp / datadog / sentry / posthog (hand-written JSON over reqwest)
│   │       ├── diagnostics.rs # anonymized setup/runtime errors (opt-out)
│   │       └── log.rs         # `tracing` target "agentcat"; optional ~/agentcat.log parity
│   ├── agentcat-rmcp/         # rmcp 3.x adapter: Tracked<H>, TrackedService<S>, ToolCall view,
│   │                          # identity ladder, Parts projection, forward macro, factory helper
│   └── agentcat-sdk/          # published facade, [lib] name = "agentcat"; features pick adapters/TLS/exporters
├── examples/                  # stdio, streamable HTTP (session), stateless HTTP, Router<S>, identify
├── tests/                     # rmcp client ↔ tracked server over tokio duplex and real HTTP
├── fixtures/                  # constants + derive-ID golden vectors exported from TS main
└── xtask/                     # rmcp trait tripwire, fixture refresh, regenerate-api check
```

The generated client lives in its own repo, `agentcathq/agentcat-rust-api`, published as
`agentcat-api` (placeholder 0.0.1 already exists on crates.io), generated by a new
`generate-rust-sdk.sh` in `agentcat-server/sdk-generator/` like the other languages.

Why three crates: a breaking rmcp major then means a new adapter crate or feature
(`rmcp-3`, `rmcp-4`, using renamed dependencies `rmcp3 = { package = "rmcp", version = "3" }`)
without a core release, and customers depend only on `agentcat-sdk`.

### 5.2 Runtime and publisher

- **tokio** is a given (rmcp requires it). The wrapper's futures run on the customer's
  runtime; `publish()` is `tokio::sync::mpsc::Sender::try_send` (works without a runtime,
  never blocks, drops with a warning when full).
- The **worker runs on a dedicated OS thread with its own current-thread tokio runtime**,
  driving the generated async reqwest client with bounded concurrency (5 in-flight, like Go),
  3 retries at 1 s/2 s/4 s, 10 s per attempt. Reasons: it works whatever runtime flavor the
  customer uses (a `current_thread` runtime blocked by customer code would starve a spawned
  task); `AgentCat::new` does not need to run inside a runtime; and `Drop` can flush by
  joining the thread with a timeout even after the customer's runtime is gone. Precedents:
  sentry-rust's transport thread (`sentry/src/transports/tokio_thread.rs`: `thread::Builder`
  + `new_current_thread`, `flush(timeout)`), and opentelemetry-sdk 0.28, which redesigned its
  batch processors "to no longer require an async runtime. They create its own background
  thread instead." (`opentelemetry-sdk/CHANGELOG.md`, 0.28.0).
- Redaction, sanitization, truncation and token estimation run on the worker thread, so
  customer redaction hooks must be `Send + Sync + 'static`.
- Shutdown: `async fn shutdown(&self, timeout)` closes the queue and awaits the drain
  (oneshot from the worker). `Drop` of the last handle does a bounded blocking flush
  (default 2 s, configurable, 0 disables). Document: `process::exit` and SIGKILL lose queued
  events; the SDK installs no signal handlers.

### 5.3 MSRV and edition

**MSRV 1.88, edition 2024**, equal to rmcp 3.x (`Cargo.toml:14-16`); rmcp cannot compile
below it anyway, and 1.88 stabilizes `let` chains for edition 2024 (rmcp uses them). CI:
`cargo +1.88 check --all-features` and `cargo msrv verify`. Treat an MSRV bump as a minor
release, as rmcp does (`VERSIONING.md`).

### 5.4 Feature flags (facade `agentcat-sdk`)

```toml
[features]
default = ["rmcp", "rustls", "http-capture", "exporters", "diagnostics"]
rmcp = ["dep:agentcat-rmcp"]                       # rmcp 3.x adapter
http-capture = ["agentcat-rmcp?/http"]             # read http::request::Parts (headers for identify, parameters.extra)
# TLS for the AgentCat API and exporters (mirrors rmcp's own reqwest features)
rustls = ["agentcat-core/rustls"]                  # reqwest/rustls (aws-lc-rs provider)
rustls-no-provider = ["agentcat-core/rustls-no-provider"]  # bring your own crypto provider (e.g. ring)
native-tls = ["agentcat-core/native-tls"]
exporters = ["otlp", "datadog", "sentry", "posthog"]
otlp = ["agentcat-core/otlp"]
datadog = ["agentcat-core/datadog"]
sentry = ["agentcat-core/sentry"]
posthog = ["agentcat-core/posthog"]
diagnostics = ["agentcat-core/diagnostics"]        # also DISABLE_DIAGNOSTICS env var at runtime
file-log = ["agentcat-core/file-log"]              # ~/agentcat.log parity with other SDKs
preserve-schema-order = ["agentcat-core/preserve-schema-order"]  # serde_json/preserve_order; global side effect, opt-in
```

`agentcat-core` emits `compile_error!` when no TLS feature is enabled. Exporters are
hand-written JSON over the same reqwest client, so they add no heavy dependency (no
`opentelemetry-sdk`).

`agentcat-rmcp` depends on `rmcp = { version = ">=3.0, <4", default-features = false,
features = ["server"] }` and, behind `http`, on `http = "1"` (the same major as rmcp, so the
`TypeId` of `http::request::Parts` matches).

### 5.5 Generating `agentcat-api`

Add a `rust` section to `agentcat-server/sdk-generator/openapi-config.yaml`:

```yaml
rust:
  generator: "rust"
  library: "reqwest"                 # async reqwest client
  output_dir: "rust-public-sdk-output"
  additional_properties:
    packageName: "agentcat-api"
    packageVersion: "1.0.0"
    supportAsync: true               # default true (RustClientCodegen.java:57)
    avoidBoxedModels: true
    reqwestDefaultFeatures: ""       # no default TLS; agentcat-core selects it (RustClientCodegen.java:67,89,249)
    hideGenerationTimestamp: true
```

- Use **OpenAPI Generator ≥ 7.26.0** for Rust (latest release, 2026-10-06). Its template uses
  `reqwest ^0.13` with `native-tls`/`rustls` features and a configurable default
  (`rust/Cargo.mustache:70-99` at v7.26.0). The shared pin, 7.14.0, still emits `reqwest ^0.12`,
  a hard-coded `default = ["native-tls"]` and `rustls = ["reqwest/rustls-tls"]`
  (`Cargo.mustache:66-102` at v7.14.0), which would pull a second reqwest and OpenSSL into
  servers that use rmcp's reqwest 0.13.
- Fix the spec first: add `agentcat:custom` to the `event_type` enum (§4.3).
- The generated call is roughly
  `apis::events_api::publish_event(&configuration, Some(request)).await`.

### 5.6 Public API sketch

```rust
// main.rs — stdio server
use std::time::Duration;
use agentcat::{AgentCat, UserIdentity};
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let agentcat = AgentCat::builder("proj_0000000")
        .identify(|call: &agentcat::ToolCall<'_>| {
            // Runs on every tool call; keep it cheap (no network calls).
            let user = call.header("x-user-id")?;          // None on stdio
            Some(UserIdentity::new(user).name("demo_user"))
        })
        .redact(|text: &str| scrub_secrets(text))
        .enable_agent_tracking(true)
        .build();                                          // infallible; misconfig → warn + pass-through

    let running = agentcat.track(Counter::new()).serve(stdio()).await?;
    running.waiting().await?;
    agentcat.shutdown(Duration::from_secs(5)).await;       // Drop also flushes (bounded) as a fallback
    Ok(())
}
```

```rust
// main.rs — streamable HTTP (session or stateless)
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};

let agentcat = AgentCat::builder("proj_0000000").build();
let ct = tokio_util::sync::CancellationToken::new();
let mcp = StreamableHttpService::new(
    agentcat.track_factory(|| Ok(Counter::new())),         // runs per session, or per request when stateless
    LocalSessionManager::default().into(),
    StreamableHttpServerConfig::default().with_cancellation_token(ct.child_token()),
);
let app = axum::Router::new().nest_service("/mcp", mcp);
let listener = tokio::net::TcpListener::bind("127.0.0.1:8000").await?;
axum::serve(listener, app)
    .with_graceful_shutdown(async move { tokio::signal::ctrl_c().await.ok(); ct.cancel(); })
    .await?;
agentcat.shutdown(std::time::Duration::from_secs(5)).await;
```

```rust
// Facade surface (agentcat-sdk, lib name `agentcat`)
pub struct AgentCat { /* Arc<core::Inner> */ }               // Clone; cheap
impl AgentCat {
    pub fn builder(project_id: impl Into<String>) -> AgentCatBuilder;
    pub fn telemetry_only() -> AgentCatBuilder;               // exporters only, no project ID
    #[cfg(feature = "rmcp")]
    pub fn track<H: rmcp::ServerHandler>(&self, handler: H) -> Tracked<H>;
    #[cfg(feature = "rmcp")]
    pub fn track_factory<H, F>(&self, factory: F)
        -> impl Fn() -> std::io::Result<Tracked<H>> + Send + Sync + 'static
    where H: rmcp::ServerHandler, F: Fn() -> std::io::Result<H> + Send + Sync + 'static;
    #[cfg(feature = "rmcp")]
    pub fn track_service<S: rmcp::Service<rmcp::RoleServer>>(&self, svc: S) -> TrackedService<S>;
    pub fn publish_custom_event(&self, data: CustomEventData); // session_id used verbatim if given
    pub async fn flush(&self, timeout: Duration) -> FlushOutcome;
    pub async fn shutdown(&self, timeout: Duration) -> FlushOutcome;
}
impl AgentCatBuilder {
    pub fn identify<F>(self, f: F) -> Self
        where F: Fn(&ToolCall<'_>) -> Option<UserIdentity> + Send + Sync + 'static;
    pub fn resolve_session_id<F>(self, f: F) -> Self          // hook mode: no session_id injected
        where F: Fn(&ToolCall<'_>) -> Option<String> + Send + Sync + 'static;
    pub fn event_tags<F>(self, f: F) -> Self;                 // Fn(&ToolCall) -> Option<BTreeMap<String, String>>
    pub fn event_properties<F>(self, f: F) -> Self;           // Fn(&ToolCall) -> Option<serde_json::Map<..>>
    pub fn redact<F: Fn(&str) -> String + Send + Sync + 'static>(self, f: F) -> Self;
    pub fn redact_event<F: Fn(Event) -> Option<Event> + Send + Sync + 'static>(self, f: F) -> Self;
    pub fn enable_agent_tracking(self, on: bool) -> Self;
    pub fn disable_tool_call_context(self, off: bool) -> Self;
    pub fn custom_context_description(self, text: impl Into<String>) -> Self;
    pub fn disable_report_missing(self, off: bool) -> Self;   // no get_more_tools
    pub fn disable_tracing(self, off: bool) -> Self;
    pub fn exporter(self, name: impl Into<String>, cfg: ExporterConfig) -> Self;
    pub fn api_base_url(self, url: impl Into<String>) -> Self; // else AGENTCAT_API_URL, MCPCAT_API_URL
    pub fn debug(self, on: bool) -> Self;
    pub fn disable_diagnostics(self, off: bool) -> Self;
    pub fn flush_on_drop(self, timeout: Duration) -> Self;
    pub fn build(self) -> AgentCat;
}
pub struct ToolCall<'a> {                                     // view passed to hooks
    pub name: &'a str,
    pub arguments: Option<&'a serde_json::Map<String, serde_json::Value>>, // raw, unstripped
    pub context: &'a rmcp::service::RequestContext<rmcp::RoleServer>,
}
impl ToolCall<'_> {
    pub fn header(&self, name: &str) -> Option<&str>;        // needs `http-capture`
    pub fn http_parts(&self) -> Option<&http::request::Parts>;
    pub fn client_info(&self) -> Option<ClientInfo>;          // same ladder the events use
}
```

Tools can read the resolved handles through rmcp's extractor:
`async fn my_tool(&self, Extension(call): Extension<agentcat::CallInfo>, ...)`.

### 5.7 Test and CI plan

- In-process end-to-end tests with an rmcp client over `tokio::io::duplex` and over real HTTP
  (session, stateless, `json_response`), modeled on rmcp's own tests.
- Probe-handler test: every `ServerHandler` method reaches the inner handler through `Tracked`.
- Trait tripwire (`xtask`): rmcp method list equals the forwarded list; weekly job against the
  newest rmcp release and rmcp `main`.
- Version matrix: lowest supported rmcp 3.x (`cargo update -p rmcp --precise 3.0.1`) and newest.
- Cross-SDK fixtures exported from TS `main`: constants byte equality; `derive_session_id`
  golden vectors; injection snapshots (input tool list → advertised list + registries).
- Concurrency test: parallel calls with distinct `session_id`s never cross-attribute.
- MRTR test: an `InputRequired` round publishes undecorated with `agentcat_mrtr=input_required`;
  the continuation carries `input_responses` and `request_state` unchanged.

---

## 6. Open risks and questions

1. **`ServerHandler` drift** (§3.8 item 1). rmcp ships a minor about every two weeks and added a
   trait method in 3.3.0. Without the tripwire, a new method would silently break tracked
   servers. Ask upstream for a public delegation macro or a `Service`-based HTTP bound.
2. **rmcp major churn**: three majors in five months; about half of the top dependents are
   still on 0.x–2.x. Decide whether any pre-3 adapter is worth building.
3. **Stale cross-SDK brief** (§4.5). The Rust port must follow TS `main`; confirm with the TS
   audit which commit is canonical (token estimates #71 included).
4. **Generator and spec fixes** before the first release: `agentcat:custom` in the enum,
   OpenAPI Generator ≥ 7.26.0 for Rust, nullable `session_id` as `Option<Option<String>>`.
5. **Identity placeholders**: `Implementation::default()` reads as `"rmcp"` for stateless legacy
   clients and for servers using `ServerConfig::default()`; real rmcp-based clients that never
   set `client_info` look identical.
6. **Coverage gaps**: requests rejected by rmcp before dispatch; `Router<S>` over HTTP; the
   `local` feature; wasm targets.
7. **Tasks extension**: `CallToolResponse::Task` results arrive later through `tasks/get`.
   Decide whether the event records the task handle, waits, or is tagged; TS treats the
   extension as a non-goal.
8. **Schema property order**: without `serde_json/preserve_order`, injected properties sort
   alphabetically instead of "customer params, session_id, agent_id, context". Confirm with the
   TS audit whether order is load-bearing for agents.
9. **Panic policy**: catching and re-raising panics keeps behavior, but some customers build
   with `panic = "abort"`; then no event is possible.
10. **Overlap with rmcp's own SEP-2567 work (#870)**: if rmcp adds first-class explicit state
    handles, the injected `session_id` could collide in name or meaning.
11. **Crate naming**: `agentcat` is owned by a third party (`rpodgorny/agentcat`, "Universal
    stream formatter for AI coding agent output"); `use agentcat::…` still works through
    `[lib] name = "agentcat"` in `agentcat-sdk`, but docs.rs and search will show the other crate.
