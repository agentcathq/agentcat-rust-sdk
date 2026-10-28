# 03 — Instrumenting rmcp (official Rust MCP SDK) for AgentCat

Research date: 2026-10-20. Source: `github.com/modelcontextprotocol/rust-sdk`, cloned to `~/misc/rmcp`.

- **Analyzed tag: `rmcp-v3.6.0`** (commit `74d7895`, released to crates.io 2026-10-20). All `file:line`
  citations below are paths relative to the repo root at this tag.
- `main` is `c960b4b` = 3.6.0 + one CI-only commit ("ci(triage): redact command output", #1380).
  **Nothing unreleased affects a wrapper.**
- TS reference: `/home/ubuntu/agentcat-typescript-sdk/src` @ `main` `9b56e9e` (read-only skim).
- POC: `docs/research/spikes/rmcp-poc`
  (all 6 tests pass on stable 1.99.1 and on Rust 1.88 = rmcp MSRV).

---

## 0. TL;DR

1. **Interception layer: a generic decorator `Tracked<H: ServerHandler>` that itself implements
   `ServerHandler`.** It is the only layer that works for every transport. `StreamableHttpService` takes
   `S: ServerHandler` (not a plain `Service`), and it builds its own transports, so a transport-level or
   `Service`-level wrapper cannot cover HTTP (§4).
2. The macros (`#[tool_router]`, `#[tool_handler]`, `#[prompt_handler]`) only generate inherent trait
   methods on the *inner* type (`call_tool`, `list_tools`, `get_tool`, `get_info`). A decorator calls
   `self.inner.call_tool(..)` and sees everything. Nothing in the macros bypasses the outer type (§3).
3. **POC proved:** schema injection in `list_tools` (input `context` + output `mcp_session` + an extra
   `get_more_tools` tool). `context` is stripped before a `#[serde(deny_unknown_fields)]` tool runs; without
   the wrapper the same call fails. The POC adds `mcp_session` to `structured_content`, prepends a mint-back
   text block, and passes data into tools via `Extension<T>`. It captures client info, protocol version, HTTP
   headers, `Mcp-Session-Id` and panics. It works over the in-process duplex transport, legacy-stateful HTTP,
   stateless HTTP and a 2026-07-28 "discover" client (§6).
4. **Version support:** target **`rmcp = "3"` (`>=3.0.0, <4`)** with `default-features = false,
   features = ["server"]`. The POC decorator compiles unchanged on 3.4.0–3.6.0. A "portable" variant
   (use `InitializeResult` instead of the 3.4+ alias `ServerConfig`; do not override 3.3+
   `negotiate_initialize`) compiles on **every 3.x from 3.0.0 to 3.6.0**. Against 2.2.0 it fails with 27
   errors and against 1.8.0 with 30. Supporting 1.x/2.x would need a separate code line (§9).
5. **Hard risks:**
   - New `ServerHandler` default methods added in rmcp minors are silently not forwarded.
   - Stateless HTTP makes up a fake client identity.
   - A tool that panics never answers the client.
   - `serde_json::Map` key order cannot put `mcp_session` "first" unless `preserve_order` is on.
   - rmcp ships a new major every 1–4 months.
   (§8)

---

## 1. TS SDK interception points (skim) and their rmcp mapping

The TS SDK patches the low-level `Server._requestHandlers` map (`src/engine/index.ts:16-34`). Today it
publishes only `mcp:tools/call` events (`src/engine/callWrap.ts:242`; other event types appear only in the
PostHog exporter). Interception points:

| # | TS point (file:line) | What it does | rmcp 3.6 equivalent |
|---|---|---|---|
| 1 | `tools/list` wrap — `engine/listWrap.ts:91-141`, pipeline `buildInjectedList` `listWrap.ts:33-84` | Appends `get_more_tools`. Injects `session_id`/`agent_id`/`context` into `inputSchema` (skips `oneOf/anyOf/allOf`, removes `additionalProperties:false`, adds to `required`). Injects optional `mcp_session` into object `outputSchema`. Spreads the response so `nextCursor`/`_meta` pass through. | `Tracked::list_tools`: call `inner.list_tools(..)` and mutate `ListToolsResult.tools[i].input_schema` (an `Arc<JsonObject>`, edited with `Arc::make_mut`) and `.output_schema`. Push the extra `Tool`. Other fields (`next_cursor`, `meta`, `ttl_ms`, `cache_scope`) stay as they are because we edit in place. POC `src/lib.rs:205-219`. |
| 2 | `tools/call` wrap — `engine/callWrap.ts:136-357` | Resolves handles and client info. Builds the event with raw args and an input-token estimate. Strips injected args from the handler copy only. Answers `get_more_tools` itself. Mint-back: prepends a text block and mirrors `mcp_session` into `structuredContent`. Records `isError`, output tokens and duration, publishes, rethrows on throw. Tags MRTR `input_required` rounds. | `Tracked::call_tool`: `CallToolRequestParams.arguments: Option<JsonObject>`. Result type is `Result<CallToolResponse, ErrorData>`, where `CallToolResponse::{Complete, InputRequired, Task}` (`crates/rmcp/src/model/mrtr.rs:105-114`). `Err(ErrorData)` is the "throw". POC `src/lib.rs:221-328`. |
| 3 | inner tap — `engine/innerTap.ts:22-71` | Strips leftovers that pass loose validators. Stashes thrown `Error`s so the stack trace survives the SDK turning the throw into an `isError` result. | Stripping: **not needed.** rmcp does not validate arguments against the schema before the handler. The router hands the `JsonObject` to serde (`crates/rmcp/src/handler/server/tool.rs:235-258`), so stripping in `call_tool` is enough. Errors: a `#[tool]` fn's `Err(E)` becomes `CallToolResult{is_error:true}` *inside* the router (`handler/server/tool.rs:125-146`). The typed error is lost, and the wrapper sees only the content text. Panics: the wrapper must `catch_unwind` (§6.4). |
| 4 | `setRequestHandler` re-arm — `engine/registrationPatch.ts:16-40` | Re-wraps if the customer registers handlers after `track()`. | **Not needed.** Dispatch is static. The decorator owns the inner value from construction, and runtime tool changes (`ToolRouter::disable_route`, `crates/rmcp/src/handler/server/router/tool.rs:472-488`) still flow through `list_tools`. |
| 5 | Registry rebuild-on-demand — `engine/callWrap.ts:85-127` | A `tools/call` on an instance that never served `tools/list` (stateless) runs the original list handler with a 5 s timeout to learn which params it injected. | Better in rmcp: `ServerHandler::get_tool(name)` returns the original `Tool` in O(1). `#[tool_handler]` auto-generates it (`crates/rmcp-macros/src/tool_handler.rs:88-95`); the default returns `None` (`crates/rmcp/src/handler/server.rs:545-551`). Fall back to `inner.list_tools` (with a timeout) or the heuristic strip for hand-written handlers. |
| 6 | Client identity — `modules/session.ts:42-56` (`_meta` `io.modelcontextprotocol/clientInfo`, then `getClientVersion()`) | | `RequestContext::client_info()` (`crates/rmcp/src/service.rs:1229-1238`) reads the per-request `_meta` first, then `peer_info()` from `initialize`. **Caveat:** stateless HTTP fabricates the value (§5.4). |
| 7 | Protocol version — `modules/session.ts:58-65` | | `RequestContext::protocol_version()` (`service.rs:1220-1227`). |
| 8 | Server name/version — `modules/session.ts:84-85` (`server._serverInfo`) | | `self.inner.get_info().server_info` (`Implementation`, `crates/rmcp/src/model.rs:1687`). |
| 9 | Hook inputs (`identify`, `eventTags`, `eventProperties`, `resolveSessionId`) get `(request, extra)`; `extra` is projected (headers etc.) — `modules/extra-projection.ts` | | `RequestContext { meta, extensions, peer, id, ct }`. The HTTP request is available as `context.extensions.get::<http::request::Parts>()` (§5.3). |
| 10 | MRTR (2026-07-28 multi-round tools) — `callWrap.ts:53-70, 290-294` | | `CallToolResponse::InputRequired`, plus `request.input_responses` / `request.request_state` (`model.rs:4409-4431`). |

---

## 2. The `ServerHandler` trait (rmcp 3.6.0)

### 2.1 Declaration, async style, bounds

- `pub trait ServerHandler: Sized + Send + Sync + 'static` (`crates/rmcp/src/handler/server.rs:640-644`).
  With the `local` feature the bound is `Sized + 'static` (`server.rs:646-650`).
- Method bodies come from the `server_handler_methods!` macro (`server.rs:308-637`). **Every method has a
  default**, so implementers override only what they need.
- **Async style: RPITIT** (return-position `impl Trait` in traits). No `async_trait`, no `BoxFuture`:
  `fn call_tool(&self, ..) -> impl Future<Output = Result<CallToolResponse, McpError>> + MaybeSendFuture + '_`
  (`server.rs:531-537`).
  - `MaybeSendFuture` is `Send` normally and empty under `local` (`crates/rmcp/src/service.rs:20-40`).
  - An implementer may write `async fn` in the impl (rmcp's own docs do this, `server.rs:347-355`). The
    POC uses `async fn` for intercepted methods and pass-through `fn .. -> impl Future` for forwarded ones.
    Both compile with and without `local`.
- `async-trait` is an optional rmcp dependency, used only by auth and the session store (Cargo.toml
  `async-trait`, `optional = true`; "make async-trait optional" #1119 in 3.1.1).

### 2.2 Methods (29 in 3.6; 28 in 3.0–3.2)

| Method | Signature (abbrev.) | Default | Called from |
|---|---|---|---|
| `ping` | `(ctx) -> Result<()>` | `Ok(())` (`server.rs:310-315`) | `handle_request`, **legacy only**: a 2026-07-28 request gets `-32601` (`server.rs:112-118`). Pre-init pings are answered by the loop and never reach the handler (`crates/rmcp/src/service/server.rs:565-583`). |
| `initialize` | `(InitializeRequestParams, ctx) -> Result<InitializeResult>` | `peer.set_peer_info(req.clone())` then `negotiate_initialize` (`server.rs:317-324`) | handshake (`service/server.rs:673-703`) |
| `negotiate_initialize` *(3.3+, #1247)* | `(&InitializeRequestParams) -> Result<InitializeResult>` | `get_info()` + version negotiation (`server.rs:365-376`) | only from the default `initialize` |
| `supported_protocol_versions` | `-> Cow<'static,[ProtocolVersion]>` | `KNOWN_VERSIONS` (`server.rs:388-390`) | the blanket `Service` impl, handshake, HTTP transport |
| `discover` *(3.0)* | `(ctx) -> Result<DiscoverResult>` | from `get_info` (`server.rs:392-400`) | `server/discover` (2026-07-28 bootstrap) |
| `complete` | `(CompleteRequestParams, ctx)` | empty (`server.rs:401-407`) | |
| `set_level` | `(SetLevelRequestParams, ctx)` | `-32601` (`server.rs:408-414`) | |
| `get_prompt` / `list_prompts` | | `-32601` / empty (`server.rs:415-428`) | |
| `list_resources` / `list_resource_templates` / `read_resource` | | empty / empty / `-32601` (`server.rs:429-453`) | |
| `accepted_subscription_filter` + `listen` *(3.0, SEP-2575)* | | `None` / wait for cancel (`server.rs:460-480`) | `subscriptions/listen` (`server.rs:147-184`) |
| `subscribe` / `unsubscribe` | | `-32601`, `#[deprecated]` (`server.rs:481-502`) | legacy only (`server.rs:185-202`) |
| `call_tool` | `(CallToolRequestParams, ctx) -> Result<CallToolResponse>` | `-32601` (`server.rs:531-537`) | `server.rs:203-218`; it also rejects a `Task` response if the client did not declare tasks |
| `list_tools` | `(Option<PaginatedRequestParams>, ctx) -> Result<ListToolsResult>` | empty (`server.rs:538-544`) | |
| `get_tool` *(sync)* | `(&str) -> Option<Tool>` | `None` (`server.rs:545-551`) | SEP-2243 `Mcp-Param-*` header validation in streamable HTTP (`crates/rmcp/src/transport/streamable_http_server/tower.rs:1596-1601, 2252-2256`) |
| `on_custom_request` | `(CustomRequest, ctx) -> Result<CustomResult>` | `-32601` (`server.rs:552-564`) | |
| `on_cancelled`, `on_progress`, `on_initialized`, `on_roots_list_changed`, `on_custom_notification` | `(.., NotificationContext) -> ()` | no-ops (`server.rs:566-600`) | `handle_notification` (`server.rs:274-297`) |
| `get_info` *(sync)* | `-> ServerConfig` (= `InitializeResult`; the alias exists from 3.4) | default caps (`server.rs:602-604`) | many places: tasks gating (`server.rs:31-48`), subscriptions (`server.rs:157-158`), `serve_directly` (`service.rs:1292`), discover |
| `get_task` / `update_task` / `cancel_task` *(3.0, SEP-2663)* | | `-32601` (`server.rs:606-636`) | gated by `validate_tasks_capability`, which reads `get_info()` (`server.rs:227-244`) |

`Box<T>` and `Arc<T>` implement `ServerHandler` by hand-forwarding every method
(`impl_server_handler_for_wrapper!`, `server.rs:652-873`). **This is the template for our decorator, and
the reference list our CI can diff against.**

### 2.3 Dispatch: the blanket `Service` impl

`impl<H: ServerHandler> Service<RoleServer> for H` (`server.rs:50-306`) turns each `ClientRequest` variant
into one handler call. Around that call it does the following *outside* the handler:

- per-request protocol-version validation (`server.rs:62-72`);
- required `_meta` checks for the inline lifecycle (`server.rs:78-100`);
- the MRTR/cache-hint/`resultType` post-processing (`server.rs:246-262`);
- `RESOURCE_NOT_FOUND` → `INVALID_PARAMS` for 2026 peers (`server.rs:266-271`).

A decorator therefore gets fully validated requests, and its results are post-processed like any handler's.
This impl uses `self.get_info()` and `self.supported_protocol_versions()` on the *decorator*, so both must be
forwarded.

`ClientRequest` is a `#[non_exhaustive]` untagged enum of 19 variants (`crates/rmcp/src/model.rs:4795-4815`;
the attribute comes from `ts_union!` at `model.rs:4765-4774`).

---

## 3. Can a generic decorator intercept everything? What the macros generate

### 3.1 Macros

- `#[tool]` (`crates/rmcp-macros/src/lib.rs:12-40`, `src/tool.rs`) generates a `fn <name>_tool_attr() -> Tool`
  plus a route. The input schema comes from `Parameters<T>` (schemars, draft 2020-12,
  `crates/rmcp/src/handler/server/common.rs:15-49`), with root `title`/`description` stripped
  (`common.rs:51-74`). The output schema comes from a `Json<T>` / `Result<Json<T>,E>` return type
  (`rmcp-macros/src/tool.rs:249-266`).
- `#[tool_router]` generates `fn tool_router() -> ToolRouter<Self>` (`rmcp-macros/src/lib.rs:42-160`). With
  `server_handler` it also emits `#[tool_handler] impl ServerHandler for Self {}`.
- `#[tool_handler]` (`crates/rmcp-macros/src/tool_handler.rs:33-110`) fills in, **only if absent**:
  - `call_tool`: builds a `ToolCallContext` and calls `router.call(tcc)` (`:44-56`);
  - `list_tools`: returns `router.list_all()` with `next_cursor: None`, and for 2026-07-28 peers sets
    `ttl_ms: Some(0)`, `cache_scope: Public` (`:64-86`);
  - `get_tool` (`:88-95`);
  - `get_info`: tools (and prompts, if a sibling `#[prompt_handler]` exists) capability plus
    `Implementation::from_build_env()` or a name/version from the attribute (`:97-169`).
- `#[prompt_router]` / `#[prompt_handler]` do the same for `get_prompt` / `list_prompts`.

**All of this is ordinary trait-method code on the inner type.** The decorator's `call_tool` calls
`self.inner.call_tool(..)` and that runs the macro-generated router. Nothing in rmcp calls the inner type
directly once it is wrapped. The POC (§6) uses `#[tool_router]` + `#[tool_handler]` on `Calc` and wraps it
with no changes.

### 3.2 What the router does (relevant to stripping and errors)

- `ToolRouter::call` (`crates/rmcp/src/handler/server/router/tool.rs:560-579`) looks up the route. A
  disabled or unknown name gives `Err(invalid_params("tool not found"))` (protocol error `-32602`). There is
  no JSON-Schema validation of arguments.
- `Parameters<P>` extraction is plain `serde_json::from_value` (`handler/server/tool.rs:235-250`).
  Unknown fields are ignored unless the struct has `#[serde(deny_unknown_fields)]`, in which case the call
  fails. `into_tool_argument_error` turns that failure into an in-band
  `CallToolResult{is_error:true, "failed to deserialize parameters: unknown field `context`..."}`
  (`router/tool.rs:146-157`). The POC control test reproduces exactly this
  (`tests/poc.rs:91-110`). **So stripping is required for strict tools, and harmless otherwise.**
- The `JsonObject` extractor hands the raw map to the tool (`handler/server/tool.rs:252-258`). Unstripped
  injected params would be visible to such tools.
- `ToolCallContext::new` drops `params.meta` (`handler/server/tool.rs:51-72`). `_meta` lives in
  `RequestContext.meta` anyway (§5.1).
- A `Result<T,E>` return becomes `is_error: Some(true)` with E's content (`handler/server/tool.rs:125-146`).
  The original error value is not recoverable by a wrapper.

### 3.3 What breaks or needs care in a decorator

| Concern | Evidence | Handling |
|---|---|---|
| **Unforwarded methods fall back to defaults, not to the inner override.** If rmcp adds a defaulted method in a minor (it did: `negotiate_initialize` in 3.3.0, #1247; `get_tool`, `on_custom_request` and the old task methods appeared in 1.x/2.x), a decorator built before that silently hides the inner override. | `server.rs:652-873` must be updated by rmcp itself for `Box`/`Arc` each time. | CI canary: diff our method list against rmcp's `impl_server_handler_for_wrapper!` on each rmcp release. Ship a patch promptly. Forwarding `initialize` makes not forwarding `negotiate_initialize` harmless, because only the default `initialize` calls it (`server.rs:323`). |
| `get_info` must be forwarded. | Used for tasks gating, subscriptions, discover, `serve_directly` (§2.2). | Forward. Patch `capabilities.tools` to `Some` if AgentCat adds `get_more_tools` to a server that has no tools (POC `src/lib.rs:330-336`). |
| `get_tool` must be forwarded. | Otherwise SEP-2243 header validation silently turns off (`tower.rs:1596-1601`). | Forward, and also answer for SDK-owned tools (POC `src/lib.rs:323-328`). |
| `supported_protocol_versions` must be forwarded. | The handshake and HTTP layer read it through `Service` (`service.rs:281-293`, `service/server.rs:635, 694`). | Forward. |
| Deprecated `subscribe`/`unsubscribe`. | `#[deprecated]` (`server.rs:481-502`). | `#[allow(deprecated)]` on the impl. |
| `#[non_exhaustive]` model structs (`Tool`, `CallToolResult`, `ListToolsResult` is exhaustive). | `model/tool.rs:16`, `model.rs:4134`. | Build with constructors (`Tool::new`, `CallToolResult::success`) and then assign public fields. Field mutation is allowed. |
| Per-instance construction. | HTTP calls the factory per session (stateful) or **per request** (stateless): POC counted 5 instances for init + list + 2 calls + the initialized notification. | `Tracked::new` must be cheap: an `Arc` clone of a shared core, and no per-instance background tasks. |

---

## 4. Other layers considered (and why not)

| Layer | What it sees | Verdict |
|---|---|---|
| **`Service<RoleServer>` wrapper** (`crates/rmcp/src/service.rs:269-293`; one `handle_request(ClientRequest, ctx)` match) | All requests and notifications, typed. New `ClientRequest` variants could pass through a wildcard arm (future-proof). | **Fails for HTTP.** `StreamableHttpService<S, M>` requires `S: crate::ServerHandler` (`tower.rs:1354-1361`, `1408-1411`, `1611-1619`). The blanket `impl<H: ServerHandler> Service for H` (`server.rs:50`) also prevents one type from implementing both. Usable only with `serve()` (stdio, duplex). |
| **Transport wrapper** (`trait Transport<R>`, `crates/rmcp/src/transport.rs:127-152`) | Raw `ClientJsonRpcMessage` in and `ServerJsonRpcMessage` out; would need request/response id correlation. | **Fails for HTTP.** Streamable HTTP creates transports internally: the session-manager transport for stateful mode and `OneshotTransport` for each stateless request (`tower.rs:1538-1550`, `2275-2290`). There is no user hook in stateless mode. It would also lose `RequestContext`. |
| **tower/axum HTTP layer** around `StreamableHttpService` | Raw HTTP bodies and SSE streams. | Would have to re-parse and rewrite JSON-RPC inside SSE. Fragile. Not needed for headers, because `http::request::Parts` already reach the handler (§5.3). |
| `TowerHandler` (`crates/rmcp/src/service/tower.rs:7-54`) | Adapts a `tower::Service<ClientRequest>` into an rmcp `Service`. | It drops `RequestContext` and all notifications (`:30-46`). Useless here. |
| **`ServerHandler` decorator** | Everything that reaches a handler, with full `RequestContext`. | **Recommended.** Works for stdio, any `AsyncRead+AsyncWrite`, legacy-stateful HTTP, stateless HTTP and 2026-07-28 inline/discover clients (all proven in the POC). |

**What no handler-level layer can see:** pre-`initialize` pings (`service/server.rs:565-583`), 2026 pings
(`server.rs:112-118`), HTTP-level rejections (Host/Origin/Accept, 404 for unknown session, header
mismatch) and `subscriptions/listen` cancellation routing (`service.rs:1612-1645`). None of these matter for
AgentCat events.

Server-side **SSE transport was removed in 0.11.0** ("remove SSE transport support" #562, CHANGELOG
`crates/rmcp/CHANGELOG.md:684`). Transports today: stdio (`transport::io::stdio`, plus `stdio_pipes`, new in
3.6), any `AsyncRead+AsyncWrite` (`transport-async-rw`), Sink/Stream, worker, and streamable HTTP server and
client. `transport/ws.rs` is an empty stub.

---

## 5. Request context, identity, HTTP, sessions

### 5.1 `RequestContext` / `NotificationContext`

`crates/rmcp/src/service.rs:1192-1252`:

```rust
#[non_exhaustive] pub struct RequestContext<R> {
    pub ct: CancellationToken,      // cancelled on notifications/cancelled
    pub id: RequestId,
    pub meta: RequestMetaObject,    // the request's _meta (moved out of params)
    pub extensions: Extensions,     // http::request::Parts lives here for HTTP
    pub peer: Peer<R>,              // send notifications/requests; peer_info()
}
```

- Accessors (feature `server`): `protocol_version()`, `client_info()`, `client_capabilities()`
  (`service.rs:1218-1251`). They prefer the per-request `_meta` keys `io.modelcontextprotocol/{protocolVersion,clientInfo,clientCapabilities}`
  (`crates/rmcp/src/model/meta.rs:390-397, 446-480`) and fall back to `peer.peer_info()`, the stored
  `InitializeRequestParams`, for legacy sessions (`service.rs:1012-1019`).
- `_meta` is stripped from typed params on deserialization and moved into `context.meta` before dispatch
  (`model/meta.rs:15-24`, `service.rs:1574-1586`).
- `NotificationContext { meta, extensions, peer }` (`service.rs:1252-1260`).
- The loop spawns every request on its own tokio task (`service.rs:1569-1600`, `spawn_service_task` `:1308-1323`;
  `spawn_local` under `local`).
- **Passing data to tools:** a decorator can `context.extensions.insert(T)` before forwarding. Tools then
  extract it with `Extension<T>` (`crates/rmcp/src/handler/server/common.rs:179-201`). The POC's `whoami`
  tool reads `Extension<AgentCatCall>` (`tests/poc.rs:58-61`).

### 5.2 Client info and protocol version — observed values

| Path | `initialize` seen by handler? | `context.client_info()` in `tools/call` | `context.protocol_version()` |
|---|---|---|---|
| stdio/duplex, legacy `initialize` (rmcp client default `ClientLifecycleMode::Initialize`, `crates/rmcp/src/service/client.rs:755-771`) | yes | real (`poc-client@9.9.9`) | negotiated `2025-11-25` |
| 2026-07-28 discover lifecycle (no `initialize`) | **no** | real, from `_meta` (`modern-client@2.0.0`) | `2026-07-28` |
| HTTP legacy stateful | yes (once per session) | real (`http-client@1.2.3`) | `2025-11-25` |
| HTTP stateless (`legacy_session_mode=false`) | yes, but on a throwaway instance | **fake: `rmcp@3.6.0`** | from the `MCP-Protocol-Version` header |

- The client asks for `2026-07-28` in `initialize` (`ProtocolVersion::LATEST`, `crates/rmcp/src/model.rs:174`).
  The server negotiates down to `2025-11-25` *after* the handler returns (`service/server.rs:685-703`), because
  2026-07-28 has no `initialize` (`model.rs:181, 199, 218-230`). **So an initialize event must record
  `result.protocol_version`, not `request.protocol_version`.** The POC recorded the request value, and that is
  wrong.
- **Stateless fabrication:** `peer_info_for_stateless_request` builds
  `InitializeRequestParams{ client_info: Implementation::default(), capabilities: default }`
  (`tower.rs:2398-2417`). `Implementation::default()` is `from_build_env()`, which is rmcp's own crate name and
  version (`model.rs:1700-1727`). This cannot be told apart from a real rmcp client that keeps the default. For
  a legacy client in stateless mode, client identity is effectively unknown. 2026-07-28 clients send it in
  `_meta` and are fine.

### 5.3 HTTP request parts and `Mcp-Session-Id`

- `StreamableHttpService` inserts the full `http::request::Parts` (method, URI, headers, and axum extensions
  such as auth state) into the request's (and notification's) `Extensions`:
  - stateful: `tower.rs:2064-2073`, initialize at `:2154`;
  - stateless: `:2275`, negotiated: `:1538`.
  The handler reads it with `context.extensions.get::<http::request::Parts>()`. rmcp documents this pattern, including reading `mcp-session-id`
  (`tower.rs:1281-1331`).
- This needs only the `http` 1.x crate in our SDK, not rmcp's HTTP features. The type must be the same `http`
  major as rmcp's, so `TypeId` matches.
- `Mcp-Session-Id`: in **legacy stateful** mode it appears in `Parts.headers` on every non-initialize request
  (POC: same UUID on both calls). It is absent on the `initialize` POST, because the session is created after
  (`tower.rs:2121-2176`; the initialize request's `Parts` are inserted at `:2154` before `create_session`). It is absent in stateless mode (POC asserted `None`).
- Stateful vs stateless: `StreamableHttpServerConfig.legacy_session_mode` (default `true`) only applies to
  `< 2026-07-28` requests. **2026-07-28 is always served statelessly** (SEP-2567) (`tower.rs:83-90, 2028-2029`).
  The field was renamed from `stateful_mode` in 3.0.0-beta.1 (#999, CHANGELOG `:309`).
  - `json_response` (`:91-95`).
  - `session_store` for cross-instance restore (`:121-140`). A restore replays `initialize` with a
    `SessionRestoreMarker` extension (`crates/rmcp/src/transport/streamable_http_server/session.rs:53-56`,
    `tower.rs:1758-1767`). **The SDK must not count a replayed initialize as a new session.**
- `SessionManager` implementations: `LocalSessionManager` (in-memory), `NeverSessionManager` (stateless)
  (`session.rs:1-20`).

### 5.4 Session model implications

- One `Tracked` instance per stdio connection or per legacy HTTP session (POC: 1 instance for the stateful
  session), so a per-instance "connection session" id works there.
- Stateless HTTP and 2026-07-28 have no server-side session at all. Correlation must come from the
  TS-style injected `session_id` handle plus mint-back. The decorator supports this fully: inject in
  `list_tools`, resolve and strip in `call_tool`, mint back in the result.

---

## 6. Proof of concept

Path: `docs/research/spikes/rmcp-poc`

- `src/lib.rs` (500 lines): the `Tracked<H>` decorator. It overrides `initialize`, `list_tools`, `call_tool`,
  `get_tool` and `get_info`, and forwards the other 24 methods.
- `tests/poc.rs` (408 lines): `Calc` server built with `#[tool_router]` + `#[tool_handler]`, with tools
  `add` (`Json<AddOut>` → output schema + `structured_content`), `strict_echo` (`deny_unknown_fields`),
  `whoami` (`Extension<AgentCatCall>`), `fail` (`Result<String,String>`) and `explode` (panic).
- Dependencies: `rmcp = "=3.6.0"` (features `server, macros, client`; dev adds streamable HTTP server and
  reqwest client), axum 0.8.

### 6.1 Results — what compiled and passed

All compiled on the first attempt with zero warnings. `cargo test`: **6 passed** on rustc 1.99.1, and
**6 passed on rustc 1.88** (rmcp's MSRV) with an MSRV-aware lockfile (see §7).

| Test | Proves |
|---|---|
| `control_unwrapped_strict_tool_rejects_context` (`tests/poc.rs:91`) | Without the wrapper, `{"text":"hi","context":"why"}` gives `is_error: true`, "failed to deserialize parameters: unknown field `context`, expected `text`". |
| `duplex_wrapper_end_to_end` (`:112`), over `tokio::io::duplex` | **list_tools:** `add.input_schema` gains `context` (string) and `required: ["a","b","context"]`; `add.output_schema` gains `mcp_session`; param-less `fail` gets `required:["context"]`; `get_more_tools` is appended. **call_tool:** `strict_echo` now succeeds (context stripped); content[0] is the mint-back text; `add` returns `structured_content = {"mcp_session":{...},"sum":5}`; `whoami` sees `Extension<AgentCatCall>{session_id, intent}`; `fail` is recorded as `is_error`; `get_more_tools` is answered by the wrapper with its intent captured; an unknown tool gives protocol error `-32602 "tool not found"` and is still recorded. **Events** carry intent, raw args (with `context`), client `poc-client@9.9.9`, protocol version and token estimates. |
| `http_legacy_stateful_session` (`:274`), axum + `StreamableHttpService` + rmcp reqwest client with header `x-actor: alice` | Factory `move \|\| Ok(Tracked::new(Calc, rec.clone(), ..))` works. Each call event has `http_header = "alice"` (from `Parts`), the same `Mcp-Session-Id` UUID and the real client info. 1 handler instance per session. |
| `http_stateless` (`:308`), `legacy_session_mode(false)` + `json_response(true)` | Works. Header captured. `Mcp-Session-Id` is `None`. **5 handler instances** (one per POST). Client info is the fabricated `rmcp@3.6.0`. |
| `panic_in_tool` (`:350`) | A panicking tool gives **no response; the client timed out after 3 s**. The server keeps serving. With `AssertUnwindSafe(fut).catch_unwind()` + `resume_unwind` in the wrapper (`src/lib.rs:274-298`), the panic is recorded as an error event and behavior is otherwise unchanged. This compiles with the `Send` bounds. |
| `discover_lifecycle_client` (`:377`), `ClientLifecycleMode::Discover{[2026-07-28]}` | No `initialize` reaches the handler. `tools/call` still has client `modern-client@2.0.0` and protocol `2026-07-28` from `_meta`. Injection and mint-back work. |

### 6.2 Notable observations from the POC

- **Key order:** `structured_content` printed `{"mcp_session":..,"sum":5}` only because `serde_json::Map` is a
  `BTreeMap` (alphabetical). `preserve_order` was not enabled anywhere in the tree (`cargo tree -e features
  -i serde_json`). See §8.
- `Arc::make_mut` on `input_schema` clones the router's shared schema `Arc`. The router's own copy and its
  `thread_local` schema caches (`common.rs:15-49`) stay unmodified, which is correct.
- The decorator also compiled with `rmcp/local` enabled and with `default-features=false, features=["server"]`
  (+ `http`), on 3.0.0 and 3.6.0 (portable variant, §9.3).

### 6.3 What did not work / was not tried

- No compile failures on 3.6.0.
- Not exercised: resources/prompts/completion/custom requests (pure forwarding, the same mechanism),
  MRTR `InputRequired` and tasks (code path present, untested), `session_store` restore replays.

---

## 7. Model types, protocol versions, MSRV, features, dependency weight

### 7.1 Model types (3.6.0)

- `Tool` (`crates/rmcp/src/model/tool.rs:13-40`), `#[non_exhaustive]`, `Default`: `name: Cow<'static,str>`,
  `title`, `description: Option<Cow>`, **`input_schema: Arc<JsonObject>`**,
  **`output_schema: Option<Arc<JsonObject>>`**, `annotations`, `icons`, `meta: Option<MetaObject>`.
  - `Tool::new(name, desc, schema)` (`:163-180`).
  - `JsonObject = serde_json::Map<String, Value>` (`model.rs:43`).
  - Output schemas need not be root `object` (SEP-2106, `common.rs:96-125`). Inject `mcp_session` only when
    `type == "object"`.
- `CallToolRequestParams` (`model.rs:4409-4431`): `meta` (empty after deserialization), `name: Cow`,
  **`arguments: Option<JsonObject>`**, `input_responses`, `request_state` (MRTR).
- `CallToolResult` (`model.rs:4134-4161`): `result_type: Option<ResultType>`, `content: Vec<ContentBlock>`,
  **`structured_content: Option<Value>`** (any JSON value; explicit `null` preserved since 3.5.0 #1295),
  `is_error: Option<bool>`, `meta: Option<MetaObject>`.
  - Constructors: `success`, `error`, `structured`.
  - The custom `Deserialize` needs ≥1 known field (`:4163-4220`).
- `CallToolResponse` enum (`crates/rmcp/src/model/mrtr.rs:103-114`), `#[non_exhaustive]`, so a match needs
  a wildcard.
- `ListToolsResult` from the `paginated_result!` macro (`model.rs:1892-1961`, used at `:4394-4398`):
  `result_type`, `meta`, **`next_cursor: Option<Cursor>`**, `ttl_ms`, `cache_scope` (SEP-2549), `tools`.
  - Request side: `Option<PaginatedRequestParams{ meta, cursor }>` (`model.rs:1755-1766`).
  - The macro-generated `list_tools` never paginates (`tool_handler.rs:77`). Hand-written servers might.
    Add SDK tools when `next_cursor.is_none()`.
- `_meta`: `MetaObject(JsonObject)` (`model/meta.rs:244`), `RequestMetaObject(MetaObject)` with typed accessors
  (`:390-480`), `NotificationMetaObject` (`:609`).
- Errors: `ErrorData{ code: ErrorCode(i32), message: Cow, data: Option<Value> }` (`model.rs:647-665`),
  re-exported as `rmcp::ErrorData`, conventionally `McpError`. `RmcpError` is the transport/service error
  (`crates/rmcp/src/error.rs`).
- `InitializeRequestParams{ meta, protocol_version, capabilities, client_info: Implementation }`
  (`model.rs:1230-1246`).
- `ServerConfig = InitializeResult` (`model.rs:1341`, added 3.4.0). `ServerInfo` is a deprecated alias.

### 7.2 Protocol versions

`KNOWN_VERSIONS = [2024-11-05, 2025-03-26, 2025-06-18, 2025-11-25, 2026-07-28]`, `LATEST = 2026-07-28`,
`LATEST_WITH_INITIALIZE = 2025-11-25`, `NO_INITIALIZE = 2026-07-28` (`crates/rmcp/src/model.rs:163-207`).
2026-07-28 removes `initialize` and sessions and adds `server/discover`, per-request `_meta` identity, MRTR,
cache hints, `subscriptions/listen` and the tasks extension.

### 7.3 MSRV, edition, features

- `rust-version = "1.88"`, `edition = "2024"` (`Cargo.toml` workspace; declared since 3.0.0-beta.2, #1034).
  rmcp's own toolchain pin is 1.96 (`rust-toolchain.toml`).
  - **Transitive deps can exceed the MSRV:** a fresh lockfile pulled `uuid 1.28` (needs 1.89). Only an
    MSRV-aware lockfile (`rust-version` set, resolver 3) picked `uuid 1.26.1`. Our CI must test the MSRV with
    an MSRV-resolved lockfile.
- Features (`crates/rmcp/Cargo.toml:129-197`): `default = ["base64","macros","server"]`.
  - `server = ["transport-async-rw","schemars","dep:pastey","uuid"]`.
  - `macros`, `client`, `local` (drops `Send`/`Sync`; disables `StreamableHttpService`,
    `crates/rmcp/src/transport.rs:114-115`).
  - `transport-io` (stdio), `transport-streamable-http-server` (= session + server-side-http + worker).
  - `transport-streamable-http-client[-reqwest|-unix-socket]`, `auth*`, `elicitation`, `request-state`,
    `tower`.
- SemVer is evaluated against default features plus all features except `local` (`VERSIONING.md`).

### 7.4 Dependency weight (unique crates, `cargo tree -e normal`, rmcp 3.6.0)

| Feature set | Crates |
|---|---|
| default (`base64, macros, server`) | 58 |
| + `transport-io` | 60 |
| + `transport-streamable-http-server` | 70 |
| + `client`, `transport-streamable-http-client-reqwest`, `reqwest` | 125 |

The core always pulls tokio (sync, macros, rt, time), serde/serde_json, schemars 1 (via `server`), futures,
tracing, chrono and indexmap. An AgentCat SDK that depends on `rmcp` with `default-features=false,
features=["server"]` adds nothing beyond what any rmcp server already has. It adds `http` only behind its own
feature flag.

---

## 8. Hard limits and risks

1. **Trait-growth blind spot.** rmcp minors add defaulted `ServerHandler` methods (e.g. `negotiate_initialize`
   in 3.3.0). A decorator compiled against a newer rmcp silently uses the *default* for any method it does not
   forward, which drops the inner override. Mitigations:
   - a CI job on every rmcp release that diffs the method list in `server.rs:652-873` against ours;
   - fast patch releases.
2. **No first-key control for `mcp_session`.** `serde_json::Map` is a `BTreeMap` unless some crate in the build
   enables `serde_json/preserve_order`, and Cargo feature unification makes that a global switch. With
   `preserve_order` on, rebuilding the map with `mcp_session` inserted first does work. Without it, order is
   alphabetical. TS relies on first-key placement for truncation survival (`modules/handles.ts:191-206`).
3. **Stateless HTTP identity.** A legacy client's identity is fabricated as rmcp's own name and version
   (`tower.rs:2398-2417`). Each request gets a new handler instance, so there is no per-instance state.
   Correlation must be handle-based.
4. **Panics.** A tool panic gives no response at all (client hangs until its own timeout). The SDK should
   `catch_unwind` to at least record it (proven). Converting it into an error result would change customer
   behavior, which is a decision.
5. **Tool error detail is lost.** `Err(E)` from a `#[tool]` fn is flattened into content text before the
   wrapper sees it (`handler/server/tool.rs:125-146`). There are no stack traces as in TS `innerTap`.
6. **Output schema strictness.** If an output struct uses `deny_unknown_fields`, schemars emits
   `additionalProperties:false`. Validating clients (the TS client uses ajv; the rmcp client does no validation)
   would then reject an undeclared `mcp_session`. The SDK must inject it into `output_schema` (POC does).
7. **Initialize semantics.** Record the negotiated version from the result. Skip `SessionRestoreMarker`
   replays. 2026-07-28 clients never send `initialize`.
8. **Wrapping is opt-in at construction.** There is no monkey-patching in Rust. Customers must write
   `track(server)`, or `move || Ok(track(MyServer::new()))` inside the HTTP factory. A server that hand-builds a
   `Service` instead of a `ServerHandler` cannot be wrapped (rare).
9. **Churn.** rmcp shipped 0.x weekly. 1.0.0 came 2026-03-03, 2.0.0 2026-06-29 and 3.0.0 2026-07-28, with 14
   commits touching `handler/server.rs` between 2.2.0 and 3.6.0. Expect a 4.0 within months, especially with
   the next spec revision.

---

## 9. Release history, semver churn, version-support recommendation

### 9.1 Breaking changes that hit a wrapper (from `crates/rmcp/CHANGELOG.md`)

| Version | Change hitting a wrapper | CHANGELOG |
|---|---|---|
| 0.4.0 | `Tool.outputSchema` / `CallToolResult.structuredContent` added | `:901` (#316) |
| 0.11.0 | server SSE transport removed | `:684` (#562) |
| 1.0.0 | `get_tool`, `on_custom_request`, `on_custom_notification` and the task methods (`enqueue_task`, `list_tasks`, ...) present. Params renamed `*Param` → `*Params` (0.8.5 had `CallToolRequestParam`). Futures `+ Send` → `+ MaybeSendFuture` by 1.8. | `git show rmcp-v0.8.5 / rmcp-v1.0.0 :crates/rmcp/src/handler/server.rs` |
| 2.0.0 | "align model types with MCP 2025-11-25" (#927): `Content` → `ContentBlock`; structuredContent type relaxed (#919, reverted, then re-landed in 3.0) | `:343-349` |
| 2.2.0 | `on_task_status` added (defaulted) | server.rs at tag |
| **3.0.0** (beta.1–5) | **`call_tool` returns `CallToolResponse` (MRTR, #929/#915)**. `Meta` → `MetaObject`/`RequestMetaObject` (#993). Old task API replaced by `get_task`/`update_task`/`cancel_task` (SEP-2663, #1020). New `discover` and `supported_protocol_versions` (#973). `accepted_subscription_filter`/`listen` (#1000). Cache hints / `result_type` fields (#889, #1038). `Tool.execution` removed. `stateful_mode` → `legacy_session_mode` (#999). Deprecated v3 APIs removed (#1066). `DiscoverResult.server_info` removed (#1065). | `:202-310` |
| 3.3.0 | `ServerHandler::negotiate_initialize` added (defaulted; additive) | `:100-104` (#1247) |
| 3.4.0 | `ServerConfig`/`ClientConfig` aliases added; `ServerInfo` deprecated | `:82-90` (#1266, #1156) |

Migration guide for 2→3: <https://github.com/modelcontextprotocol/rust-sdk/discussions/969> (README:9).

### 9.2 Who uses which version (crates.io)

Downloads per version over roughly the last 30 days (fetched 2026-10-20; partial data, and CI traffic
inflates the numbers):

| Major | Approx. downloads | Share |
|---|---|---|
| 3.x | ~6.9 M (3.1.3: 1.84 M, 3.4.0: 0.92 M, 3.2.0: 0.90 M, 3.4.1: 0.77 M, 3.5.0: 0.72 M, 3.5.1: 0.70 M, ...) | **~70 %** |
| 2.x | ~0.97 M (2.2.0: 0.84 M) | ~10 % |
| 1.x | ~1.5 M (1.8.0: 0.96 M, 1.7.0: 0.41 M) | ~15 % |
| 0.x | ~0.5 M (0.8.5: 0.27 M) | ~5 % |

3.0–3.3 still account for about 3.5 M of the 3.x downloads.

### 9.3 Compile matrix (the POC decorator, `cargo check`)

| rmcp | POC as written | "Portable" variant (`InitializeResult` instead of `ServerConfig`; no `negotiate_initialize` override) |
|---|---|---|
| 3.6.0, 3.5.1, 3.4.0 | ✅ | ✅ |
| 3.3.0 | ❌ 1 error (`ServerConfig` missing) | ✅ |
| 3.2.0, 3.0.0 | ❌ 3 errors (+ `negotiate_initialize` not a member) | ✅ |
| 2.2.0 | ❌ 27 errors (`CallToolResponse`, `SubscriptionFilter`, `ReadResourceResponse`, `GetPromptResponse`, `discover`, `listen`, task API, `client_info()` ...) | ❌ |
| 1.8.0 | ❌ 30 errors (+ `ContentBlock`) | ❌ |

The portable variant also compiles with `rmcp/local` and with server-only features.

### 9.4 Recommendation

- **Support `rmcp = "3"` (`>=3.0.0, <4.0.0`)** as a single range, with `default-features = false,
  features = ["server"]`.
  - Write against the portable 3.0 API: `InitializeResult`; do not override `negotiate_initialize` (safe, see
    §3.3); avoid 3.4+ conveniences.
  - CI matrix: `3.0.0` (minimal versions), `3.4.0`, and the latest `3.x`, plus MSRV 1.88 with an MSRV-aware
    lockfile.
  - Add a canary job that diffs the trait-method list against `impl_server_handler_for_wrapper!` on each new rmcp
    release.
- Do **not** try one crate across majors. Trait identity requires the customer's and our `rmcp` to be the same
  crate instance, and 2.x→3.x changed `call_tool`'s return type and the model types. If 1.x/2.x demand is real
  (~25 % of recent downloads), publish a separate maintenance line (e.g. an `agentcat-sdk` minor series built
  on rmcp 2) rather than cfg/renamed-dependency gymnastics.
- Plan for rmcp 4: a new SDK major or minor that bumps the range, with the CI canary giving early warning from
  rmcp release candidates and betas (rmcp publishes `-beta.N` before majors: 3.0.0-beta.1..5).

---

## 10. Recommended interception design (summary for implementers)

```rust
pub struct Tracked<H> { inner: H, core: Arc<AgentCatCore> }   // cheap to construct

impl<H: ServerHandler> ServerHandler for Tracked<H> {
    // intercept: list_tools, call_tool, get_tool, get_info (+ initialize; optionally
    //            list_prompts/get_prompt/list_resources/read_resource for future event types)
    // forward everything else verbatim (28 methods on 3.0 API), #[allow(deprecated)]
}
```

- **`list_tools`**: forward, then for each tool inject `session_id`/`agent_id`/`context` per config (copy the
  TS rules: skip composed schemas, drop `additionalProperties:false`, add to `required`, never overwrite a
  customer property). Inject `mcp_session` into object output schemas. Append `get_more_tools` on the last
  page. Keep all other result fields.
- **`call_tool`**:
  - Resolve handles from the raw args.
  - Get the original schema via `inner.get_tool(name)`, falling back to `inner.list_tools` with a timeout,
    then to the heuristic. Use it to decide which args are ours, then strip them.
  - Build the event from `context.client_info()` / `protocol_version()` / `extensions.get::<Parts>()`
    (feature `http`) / `inner.get_info().server_info`.
  - Insert an `AgentCatContext` into `context.extensions` for tools.
  - Answer `get_more_tools` itself.
  - Forward under `catch_unwind`.
  - On `Complete`: estimate tokens and mint back (text block first; `mcp_session` in object
    `structured_content` unless present). On `InputRequired`/`Task`: tag only. On `Err`: record and return
    the error unchanged.
  - Publish through a channel to a background tokio task, never awaiting the network on the call path.
- **`get_info`**: forward, and ensure `capabilities.tools` is set if SDK tools are added.
- **`get_tool`**: forward, and answer for SDK tools.
- **Usage:**
  - stdio: `agentcat.track(MyServer::new()).serve(rmcp::transport::stdio()).await?`
  - HTTP: `StreamableHttpService::new({ let ac = ac.clone(); move || Ok(ac.track(MyServer::new())) }, ..)`.

## 11. Decisions for the owner

1. rmcp support range: **3.x only** (recommended), or also a 2.x/1.x maintenance line.
2. Session model: TS-style injected `session_id` + mint-back everywhere (works in stateless and 2026-07-28),
   or also use the per-instance connection session for stdio and legacy-stateful HTTP, and in what fallback
   order.
3. Event scope: copy TS (tools/call only), or also emit initialize/list/prompt/resource events, which rmcp makes
   trivial.
4. Panic policy: record and re-raise (unchanged behavior; client still hangs), or record and return
   `-32603`/`is_error` (fixes the hang but changes behavior).
5. `mcp_session` ordering: accept alphabetical order, or enable `serde_json/preserve_order` (a global
   side effect on the customer's build).
6. Whether `get_tool` returns the augmented schema (consistent with tools/list) or the original. Both are safe
   for SEP-2243.
7. Default-on `http` cargo feature for `Parts` access, and which headers or IP fields go into events
   (redaction policy).
8. MSRV policy: match rmcp (1.88, edition 2024) and add the trait-growth canary to CI.
