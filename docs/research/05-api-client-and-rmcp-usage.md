# 05 - agentcat-api crate and real-world rmcp usage

Date: 2026-10-20. Paths: `SRC` = `<research scratchpad>/agentcat-api-src/agentcat-api-0.0.3`;
`SRV` = `/home/ubuntu/agentcat-server` branch `feat/rust-sdk` (read with `git show`; main has no Rust generator);
`TS` = `/home/ubuntu/agentcat-typescript-sdk` (main); `SURVEY` = `~/misc/rust-mcp-survey`.

---------------------------------------------------------------------
## PART 1 - agentcat-api

### 1.1 Generator and publishing (all on branch `feat/rust-sdk` of agentcat-server)

- Script: `SRV:sdk-generator/generate-rust-sdk.sh` (172 lines). Runs `openapi-generator-cli` with generator `rust`, library `reqwest`, input `sdk-generator/public-schema.json` (line 85-89; the spec is produced by `pre-generate-script.sh`, not committed on this branch), copies `rust-sdk-README.md` over the README (line 93), patches the `[package]` table of Cargo.toml (line 98-149), then builds (line 151-169).
- Generator pin: `SRV:sdk-generator/rust-openapitools.json` pins openapi-generator 7.26.0. Comment in the script (line 69-72): the shared older pin emits reqwest 0.12 and no TLS backend; 7.26.0+ emits reqwest 0.13 (same as rmcp) with `native-tls` and `rustls` features.
- Config: `SRV:sdk-generator/openapi-config.yaml:388-430` (`rust:` block): `packageName: agentcat-api`, `library: reqwest`, `supportAsync: true`, `reqwestDefaultFeatures: "native-tls"`, `useSingleRequestParameter: true`, `hideGenerationTimestamp: true`. Build gate (line ~483-492): `cargo build`, `cargo build --no-default-features --features rustls`, `cargo package --allow-dirty`.
- CI: `SRV:.github/workflows/generate-sdk-manual.yml` - manual `workflow_dispatch`, input `rust` + `publish`. Steps "Setup Rust" (stable minimal), "Authenticate with crates.io" via `rust-lang/crates-io-auth-action@v1` (trusted publishing, OIDC), "Publish Rust SDK": `cd sdk-generator/rust-public-sdk-output && cargo publish --allow-dirty`. crates.io metadata confirms trusted publishing: repo `agentcathq/agentcat-server`, sha `3f652c21a4d1...`, `trustpub_only: true`.
- Version is the shared SDK version (`CRATES_VERSION="$SDK_VERSION"`, pre-generate-script.sh:175).

### 1.2 Published versions (crates.io API)

| Version | Published | Features |
|---|---|---|
| 0.0.1 | 2026-08-23 | none (empty feature table) |
| 0.0.2 | 2026-10-19 17:31 | default=[native-tls], native-tls, rustls |
| 0.0.3 (latest, not yanked) | 2026-10-19 18:33 | same |

86 downloads, MIT, edition 2021, `rust_version: null` (no MSRV declared), 15 files, ~676 code lines, crate size 24 KB. `repository` field is null. Description says "consumed by agentcat-sdk". Source downloaded and read (not built in place) at `SRC`.

### 1.3 Public API surface (`SRC/src/lib.rs`)

`pub mod apis; pub mod models;` (lib.rs:7-8).

apis:
- `apis::events_api` - the only one the SDK needs. `publish_event` at `SRC/src/apis/events_api.rs:37`:
  `pub async fn publish_event(configuration: &configuration::Configuration, params: PublishEventParams) -> Result<models::EventAcceptedResponse, Error<PublishEventError>>`
  - `PublishEventParams { pub publish_event_request: Option<models::PublishEventRequest> }` (line ~13). Request is an `Option`, so `None` posts JSON `null`.
  - POST `{base_path}/v1/public/events/` (line 39), JSON body (line 53).
  - `PublishEventError` (line 27, `#[serde(untagged)]`): `Status400(serde_json::Value)`, `Status404(serde_json::Value)`, `Status401(models::Error)`, `Status403(models::Error)`, `UnknownValue(serde_json::Value)`.
- `apis::alerts_api` (`unsubscribe_alert_get/post`), `apis::notifications_api` (`unsubscribe_notification`, `verify_email_channel`): dashboard-side operations, not needed by the SDK, but they ship in the crate.
- `apis::configuration::{Configuration, ApiKey, BasicAuth}` and `apis::{Error<T>, ResponseContent<T>}`.
- models: `PublishEventRequest` (+ nested `publish_event_request::EventType`), `EventAcceptedResponse { status: Option<String> }`, `Error { detail: String }` (name collides with `apis::Error`), `EmailChannelVerifyRequest/Response`, `UnsubscribeRequestRequest`, `QueryTimeout`, `ValidationErrorValue`.

### 1.4 Request model: `PublishEventRequest` (`SRC/src/models/publish_event_request.rs`)

Derives `Clone, Default, Debug, PartialEq, Serialize, Deserialize`. Constructor `PublishEventRequest::new(project_id: String)`. Field order is the serialization order (no `preserve_order`; serde_json emits struct fields in declaration order). All fields use `skip_serializing_if = "Option::is_none"` except `project_id`.

| Field | Rust type | Notes |
|---|---|---|
| id | Option<String> | SDK-generated event id (TS uses KSUID with `evt` prefix) |
| project_id | String | only required field |
| session_id | Option<Option<String>> | double option, see 1.8; `Some(None)` -> `"session_id":null` (stateless mode); `None` -> omitted |
| actor_id | Option<String> | legacy |
| event_id | Option<String> | legacy custom id |
| event_type | Option<EventType> | enum, below |
| is_error | Option<bool> | |
| error | Option<serde_json::Value> | |
| resource_name | Option<String> | |
| duration | Option<i32> | ms; i32 (TS number) |
| input_tokens / output_tokens | Option<i32> | |
| timestamp | Option<chrono::DateTime<chrono::FixedOffset>> | needs chrono (public type leak) |
| user_intent | Option<String> | |
| parameters / response | Option<serde_json::Value> | |
| identify_actor_given_id / identify_actor_name | Option<String> | |
| identify_data | Option<serde_json::Value> | |
| tags | Option<HashMap<String,String>> | std HashMap: serialization order random |
| properties | Option<serde_json::Value> | |
| ip_address, sdk_language, agentcat_version, server_name, server_version, client_name, client_version | Option<String> | |

`EventType` (`#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]`, line 138), 19 variants with ugly names: `McpColonPing` (`mcp:ping`), `McpColonInitialize`, `McpColonCompletionSlashComplete` (`mcp:completion/complete`), `McpColonLoggingSlashSetLevel`, `McpColonPromptsSlashGet`, `...PromptsSlashList`, `McpColonResourcesSlashList`, `McpColonResourcesSlashTemplatesSlashList`, `McpColonResourcesSlashRead`, `...ResourcesSlashSubscribe`, `...ResourcesSlashUnsubscribe`, `McpColonToolsSlashCall`, `McpColonToolsSlashList`, `McpColonTasksSlashGet/Result/List/Cancel`, `AgentcatColonIdentify`, `AgentcatColonCustom`. `Default` is `McpColonPing` (line 181). Closed enum: an unknown wire value (for example a new MCP method) cannot be represented, and rmcp methods such as `notifications/*`, `sampling`, `elicitation` or `roots` have no variant.

### 1.5 Configuration (`SRC/src/apis/configuration.rs`)

```rust
#[derive(Clone)] pub struct Configuration {
  pub base_path: String,                 // default "https://api.agentcat.com" (line 68)
  pub user_agent: Option<String>,        // default "OpenAPI-Generator/v1/rust" (line 69)
  pub client: reqwest::Client,           // default reqwest::Client::new() (line 70)
  pub basic_auth: Option<BasicAuth>, pub oauth_access_token: Option<String>,
  pub bearer_access_token: Option<String>, pub api_key: Option<ApiKey> }
pub struct ApiKey { pub prefix: Option<String>, pub key: String }
```
- All fields public; custom `reqwest::Client` is injected by struct update: `Configuration { client, base_path, ..Default::default() }`. `pub use reqwest::Client;` is re-exported (line 12).
- Only `api_key` is applied by `publish_event` (events_api.rs:45-52): header `Authorization: {prefix} {key}`. `bearer_access_token`, `oauth_access_token` and `basic_auth` fields exist but are ignored by this operation. README example uses prefix `"Token"`. Spec security scheme is `tokenAuth`.
- `Debug` redacts secrets (lines 44-56).
- `Default::default()` calls `reqwest::Client::new()` every time (no timeout, builds a TLS context, panics on TLS init failure); wasteful when the SDK injects its own client.
- No timeout, no retry, no middleware, no proxy knob in the crate; all must come from the injected `reqwest::Client`.

### 1.6 Errors (`SRC/src/apis/mod.rs`)

`pub enum Error<T> { Reqwest(reqwest::Error), Serde(serde_json::Error), Io(std::io::Error), ResponseError(ResponseContent<T>) }`; `ResponseContent<T> { status: reqwest::StatusCode, content: String, entity: Option<T> }`. Implements `Display`, `std::error::Error` (requires `T: Debug`), and `From` for the three source errors. Leaks `reqwest::Error` and `reqwest::StatusCode`. No response headers are kept, so `Retry-After` on 429/503 is lost. Display of `ResponseError` prints only the status code, not the body.

### 1.7 Async/sync, reqwest, TLS, features, MSRV

- Async only (`async fn`), reqwest 0.13 `Client` (not blocking). Needs a tokio runtime (reqwest's async client spawns on tokio).
- `Cargo.toml:40-43`: `default = ["native-tls"]`, `native-tls = ["reqwest/native-tls"]`, `rustls = ["reqwest/rustls"]`. reqwest dep: `^0.13`, `default-features = false`, features `json, multipart, query, form`. Other deps: `chrono ^0.4 (serde)`, `serde ^1 (derive)`, `serde_json`, `serde_repr ^0.1`, `serde_with ^3.8 (base64, std, macros; no default)`, `url ^2.5`.
- Users can choose TLS: `default-features = false, features = ["rustls"]`. Features are NOT mutually exclusive in Cargo: enabling both pulls both stacks. With `default-features = false` and no TLS feature, HTTPS requests fail at runtime (verified: builds fine, 90 deps, zero TLS crates).
- Edition 2021, no `rust-version`. rmcp 3.6.0 requires edition 2024 and `rust-version = 1.88`, so the effective MSRV of anything that includes rmcp is 1.88.
- Dead weight: `multipart`, `form`, `serde_repr`, `Io` error, `parse_deep_object`, `urlencode`, and the alerts/notifications modules compile into every user.

### 1.8 Awkward for an SDK

1. `session_id: Option<Option<String>>` with `serde_with::rust::double_option` (models/publish_event_request.rs:23). Correct on the wire (verified: `Some(None)` -> `"session_id":null`, `None` -> omitted) but easy to misuse.
2. `reqwest` types in the public API (`Configuration.client`, `Error::Reqwest`, `StatusCode`); reqwest 0.13 major bump would be a semver break for agentcat-sdk consumers if re-exported. Keep it private inside agentcat-sdk.
3. `chrono` in the public model (`timestamp`) forces chrono (and its serde feature) on everyone.
4. Success path requires `Content-Type` containing `application/json` (events_api.rs:59-72): a 202 with an empty body or `text/plain` becomes `Err(Serde(...))` even though the event was accepted. Treat `Error::Serde` on a 2xx as "probably delivered" or avoid the typed call.
5. Request param is `Option<PublishEventRequest>`; no builder; 28 public fields, struct literal needs `..Default::default()` (Default exists, but defaults `project_id` to empty string).
6. `Error<T>` generic over per-operation error; `models::Error` name clash with `apis::Error`.
7. `tags` uses `HashMap` (non-deterministic order); `properties`/`parameters`/`response` are `serde_json::Value` (fine; matches rmcp's JSON).
8. Default User-Agent `OpenAPI-Generator/v1/rust` is not AgentCat-branded; set `user_agent` (e.g. `agentcat-sdk-rust/<ver>`).
9. `Configuration::default()` creates a throwaway `reqwest::Client` (see 1.5).
10. One request per event; no batch endpoint in the spec.
11. 0.0.x semver: every minor-looking change can break; pin exact version (`=0.0.3`) in agentcat-sdk.

### 1.9 Dependency weight (scratch projects in `.../scratchpad/api-weight/{a..f}`; `cargo 1.99.1`, rustc 1.99.1; `cargo tree -e normal`, unique package names)

| Project | Dependencies | Notes |
|---|---|---|
| a: agentcat-api 0.0.3 (default, native-tls) | 103 normal (109 incl. build) | openssl, native-tls, hyper-tls, tokio-native-tls |
| b: agentcat-api, `rustls` only | 103 normal (112 incl. build) | rustls, aws-lc-rs/aws-lc-sys (C build), rustls-platform-verifier, hyper-rustls; no openssl |
| f: agentcat-api, no TLS feature | 90 | plain HTTP only |
| c: rmcp 3.6.0 `server, transport-io, transport-streamable-http-server` | 70 | no reqwest, no hyper (uses axum-free `tower` service) |
| d: rmcp + agentcat-api (default) | 133 normal (139 incl. build) | +63 over rmcp alone |
| e: rmcp + agentcat-api (rustls) | 135 normal (144 incl. build) | |

Duplicate versions in the build graph (d and e): `syn` 2.0.119 and 3.0.9 (both proc-macro side: rmcp-macros/async-trait/darling use syn 3, serde_derive etc. use syn 2). There is NO duplicate reqwest, hyper, http, tokio, tower or rustls: rmcp's server features do not use reqwest, and rmcp's optional reqwest dep is `0.13.2` (same major as agentcat-api, so it unifies if a user also enables an rmcp reqwest client feature). Cargo.lock also shows duplicate `schemars` 0.9 / 1.2, `bitflags`, `hashbrown`, `indexmap`, `windows-sys`, `getrandom`, but those entries are platform- or dev-only (not in the `cargo tree -e normal,build` output).

### 1.10 Comparison with what the TypeScript SDK sends

TS builds the request in `TS:src/modules/eventQueue.ts:154-209` (`toPublishEventRequest`) and sends it with `EventsApi.publishEvent({ publishEventRequest })` at line 227; client built at lines 43-49 as `new Configuration({ basePath })` with default `https://api.agentcat.com` (configurable via `configure(apiBaseUrl)`). The TS generated model is in npm `agentcat-api@1.0.3`.

Behavioural facts from TS:
- No Authorization header or api key is configured at all; the project is identified by `project_id` in the body. Rust must NOT require `api_key` (leave `Configuration.api_key = None`).
- Retries: up to 3 retries, exponential backoff 1s/2s/4s (`maxRetries = 3`, line ~136-145 of sendEvent), concurrency 5, queue cap 10000 dropping oldest (lines 35-40, 66-70), graceful destroy waits up to 5 s.
- `sessionId: event.sessionId || null` (line 162) - explicit null when no session, mapped to `Some(None)` in Rust.
- `id`: KSUID with prefix `evt` generated if missing (line ~165 of processEvent).
- `actorId: event.actorId || event.identifyActorGivenId`, `identifyData: event.identifyActorData`, `tags/properties: x ?? undefined`.
- `sdkLanguage` is hardcoded `"TypeScript"` (`TS:src/modules/session.ts:78`) -> Rust must send `"Rust"`.

Mismatches / gaps:
1. `agentcat:custom`: TS emits it (`TS:src/modules/constants.ts:3`, `index.ts:449`) although its generated TS enum (npm 1.0.3) lacks it; the Rust crate (0.0.3) has `AgentcatColonCustom`. OK in Rust, but the enum is closed.
2. Timestamp: TS sends JS `Date` (ISO-8601 with `Z`); Rust needs `chrono::DateTime<FixedOffset>` (RFC 3339), so use `Utc::now().fixed_offset()` or parse. Serialization format differs slightly (chrono emits `2026-...+00:00` or `Z` depending on serde impl; both RFC 3339 - check server accepts offsets; flag as a verify item).
3. `duration` / `input_tokens` / `output_tokens` are `i32` in Rust vs JS number; clamp or saturate u128/u64 durations.
4. Redaction, truncation and `pending` hook results happen before the model is built in TS (eventQueue.ts:90-150); the Rust crate does none of this, the SDK must implement it.
5. No event types exist for rmcp-specific traffic beyond the 17 `mcp:*` methods; notifications, sampling/createMessage, elicitation, roots/list, `mcp:tasks/*` (present) and `logging` have no variants.
6. Wire key order: Rust emits declaration order; TS emits its own order. Not significant to the server (JSON object).
7. Both treat `tags` and `properties` as optional objects; the server's constraints (tag key <=32 chars, value <=200, <=50 entries) are documented in the field docs only, not enforced by either client.
8. TS never reads the response body (202 `{status}`); Rust's content-type requirement (1.8 #4) is stricter.

### 1.11 Generator changes worth requesting (not done; generator is read-only for this task)

1. Make `rustls` the default (or drop the default TLS feature entirely) and document "choose exactly one"; add a compile-time `compile_error!` or CI check for neither/both. SDK users on musl/Alpine/static builds struggle with openssl; rmcp itself is TLS-free.
2. Add a `default-features = false` friendly layout: make `multipart`/`form` reqwest features optional or off (`reqwestDefaultFeatures` only covers TLS), and drop `serde_repr`.
3. Declare `rust-version` (1.75 is plenty for the crate itself, even though rmcp needs 1.88) so cargo reports a clear MSRV.
4. Set a branded `user_agent` default via a template (`agentcat-api/<version>`) and make `Configuration::default()` use `reqwest::Client::builder().timeout(...)`, or make `client` constructible without building a throwaway client (`Configuration::with_client(client)`).
5. Add `repository` (and `rust-version`) to the cargo metadata block in `openapi-config.yaml` so crates.io links to source.
6. Spec level: make `session_id` non-double-option (`nullable` but modeled as `Option<String>` is a template/typeMapping issue; OpenAPI Generator emits double option only when the field is both nullable and optional). Either make it required-nullable, or add a custom serializer template.
7. Spec level: make `event_type` an open enum (string with `x-enum-varnames`/`x-extensible-enum`), or add an `Other(String)` variant; today a server enum addition breaks deserialization for `Deserialize` users and cannot express new MCP methods.
8. Spec level: declare publish-event 202 response with `application/json` always, and make `EventAcceptedResponse.status` required; consider a 429/5xx schema with `Retry-After` header; consider a `publish_events` batch endpoint.
9. Keep only the events tag in the Rust crate (`--global-property`/tag filter) so alerts and notifications modules are not compiled.
10. Generate `#[non_exhaustive]` on models/enums (`enumUnknownDefaultCase` in the Rust generator) so adding fields/variants is not a semver break.
11. Pin the crate version policy: publish `0.1.x` before the SDK goes 1.0; agentcat-sdk must pin `=` exact version because 0.0.x is breaking on every bump.
12. Produce a `#[serde(default)]`-safe `Deserialize` for responses (already ok) and a documented `Error::Serde`-on-2xx note.

---------------------------------------------------------------------
## PART 2 - how real projects use rmcp (`SURVEY`, all clones `git pull`ed 2026-10-20)

rmcp latest on crates.io: 3.6.0 (2026-10-20); previous: 3.5.1 (10-05), 3.4.1, 3.3.0, 3.2.0, 3.1.x (July-Aug). Versions below are the Cargo.toml requirement (manifest) with Cargo.lock in parentheses when it differs. All servers run on tokio (`tokio::main`/multi-thread; tokio 1.45-1.54 in lockfiles). None wraps the handler in a generic instrumentation layer; wrappers seen are per-product (metrics recorder on `McpService`, bearer-auth axum middleware around the `StreamableHttpService`).

| Project | rmcp | Handler style | Transports | Start call (file:line) |
|---|---|---|---|---|
| vibe-kanban | 1.2.0 (lock 0.15.0) | `#[tool_router(router=..., vis="pub")]` x11 + `#[tool_handler] impl ServerHandler` (`crates/mcp/src/task_server/handler.rs:9-10`) | stdio | `server.init().await?.serve(stdio()).await` (`crates/mcp/src/bin/vibe_kanban_mcp.rs:42`) |
| Govcraft rust-docs-mcp-server | 0.1.5 | old `#[tool(tool_box)] impl ServerHandler` (`src/server.rs:288-289`), no router | stdio (features also list `transport-sse-server`) | `service.serve(stdio()).await` (`src/main.rs:327`) |
| PostHog (rust/pgapi) | 3 | `#[tool_router]` + `#[tool_handler] impl ServerHandler for PgMcp` (`src/mcp.rs:407-408`) | streamable HTTP in axum | `StreamableHttpService::new(move || Ok(PgMcp::new(state.clone())), LocalSessionManager::default().into(), config)` (`src/mcp.rs:423`), mounted `.nest_service("/mcp", ...)` (`src/main.rs:151`) |
| goose (goose-mcp) | 3.4.1 (workspace, default-features=false + per-crate features) | `#[tool_router]` + `#[tool_handler(router = self.tool_router)]` x4 (`tutorial/mod.rs:110`, `memory/mod.rs:510`...) | stdio; in-process duplex streams | `server.serve(stdio())` (`mcp_server_runner.rs:43`); `server.serve(duplex_transport)` (`lib.rs:65`); StreamableHttp only in test fixture (`goose-test-support/src/bin/mcp_fixture_server.rs:28`) |
| agentgateway | 3.1.0 manifest (lock 0.10.0) | no handler; proxy/relay with its own `StreamableHttpService` type | streamable HTTP (own impl), SSE | `StreamableHttpService::new(sessions, config)` own type (`crates/agentgateway/src/mcp/router.rs:194`); rmcp used mainly for model types |
| apache iggy (core/ai/mcp) | 3.5.1 | `#[tool_router]` + `#[tool_handler] impl ServerHandler for IggyService` (`service/mod.rs:771`) | stdio + streamable HTTP (axum) | `.serve(stdio())` (`src/main.rs:152`); `StreamableHttpService::new(...)` (`src/api.rs:91`), `.nest_service(&config.path, service)` (`api.rs:119`) |
| apollo-mcp-server | 3.5.1 | hand-written `impl ServerHandler for McpService` (`server/states/running.rs:701`); tools generated dynamically from GraphQL | stdio + streamable HTTP + auth middleware | `running.for_service().serve(stdio())` (`server/states/starting.rs:245`); `StreamableHttpService::new(move || Ok(running.for_service()), LocalSessionManager::default().into(), config)` (`starting.rs:283`) |
| devenv | 2.0 | `#[tool_router]` (no `tool_handler`) + hand-written `impl ServerHandler for DevenvMcpServer` (`devenv/src/mcp.rs:307`) | streamable HTTP (axum `fallback_service`) and stdio over `(ThreadedStdin, stdout)` | `StreamableHttpService::new(move || Ok(server.clone()), ...)` (`mcp.rs:608`); `server.serve((stdin, tokio::io::stdout()))` (`mcp.rs:651`) |
| chroma foundation-api | 1.8.0 | `#[tool_router]` + `#[tool_handler] impl ServerHandler for FoundationMcpServer` (`routes/mcp/server.rs:432`) | streamable HTTP, stateless (`with_stateful_mode(false)`, `with_json_response(true)`, `NeverSessionManager`) behind bearer gate | `StreamableHttpService::new(factory, Arc::new(NeverSessionManager::default()), cfg)` (`routes/mcp/mod.rs:124`) |
| elastic mcp-server-elasticsearch | 0.2.1 (pinned old) | `#[tool_router]` + `#[tool_handler] impl ServerHandler for EsBaseTools` (`servers/elasticsearch/base_tools.rs:292`) | stdio, streamable HTTP, legacy SSE (`SseServer::new`) | `handler.serve(stdio())` (`src/lib.rs:48`); `StreamableHttpService::new(move || Ok(server_provider()), config.session_manager, sh_config)` (`src/protocol/http.rs:76`); `SseServer::new(sse_config)` (`http.rs:91`) |
| wassette | 3.1.3 (lock 3.5.1) | hand-written `impl ServerHandler for McpServer` (`crates/wassette-mcp-server/src/server.rs:171`) | stdio + streamable HTTP | `serve(stdio_transport())` path (`wassette-mcp-server/src/main.rs:168`); `StreamableHttpService::new(move || Ok(server.clone()), LocalSessionManager::default().into(), http_config)` + `.nest_service("/mcp", service)` (`main.rs:321-328`) |
| rust-sdk (official repo, v3.6.0) | 3.6.0 (path) | 24 files with `#[tool_router]`, 21 with `#[tool_handler]`; 62 hand-written `impl ServerHandler` (examples/tests) | everything | examples throughout `examples/servers/`; `tool_router(server_handler)` shortcut shown in readyset |
| bevy_brp | 3.4.1 (lock 3.5.0) | hand-written `impl ServerHandler for McpService` (`mcp/src/mcp_service.rs:74`), custom `call_tool` dispatch | stdio | `mcp_service.serve(transport::stdio()).await?` (`mcp/src/main.rs:33`) |
| nushell (nu-mcp) | 3.4.0 | `#[tool_router]` + `#[tool_handler] impl ServerHandler for NushellMcpServer` (`src/server.rs:84`) | stdio + streamable HTTP (hyper, `TowerToHyperService`) | `NushellMcpServer::new(..).serve(stdio())` (`src/lib.rs:156`); `TowerToHyperService::new(StreamableHttpService::new(move || Ok(NushellMcpServer::new(...)), session_manager, http_config))` (`lib.rs:202`) |
| openai codex | `=3.5.1` git rev, default-features=false | client-heavy (`rmcp-client`); servers only as test/helper binaries and the TUI dynamic-tool handler: hand-written `impl ServerHandler` (`tui/src/dynamic_tools_mcp.rs:248`, `rmcp-client/src/bin/test_stdio_server.rs:511`) | stdio, streamable HTTP | `serve(stdio())`, `StreamableHttpService::new` in test bins |
| readyset (adapter + readyset-mcp) | 2.2 | `#[tool_router(server_handler)] impl ReadysetMcpServer` (`readyset-mcp/src/server.rs:17`, new one-attribute form) | stdio (`readyset-mcp`) and streamable HTTP with `axum::middleware::from_fn_with_state(auth_state, bearer_auth)` wrapped around the service | `server.serve(stdio()).await` + `service.waiting()` (`readyset-mcp/src/main.rs:33-38`); `StreamableHttpService::new(...)` + `.nest_service("/mcp", service)` (`readyset-adapter/src/mcp_http.rs:100-111`) |
| rustfs | none | no rmcp dependency anywhere in the clone (grep for `rmcp`/`modelcontextprotocol` in toml/rs/md finds nothing); nothing to survey | n/a | n/a |
| spiceai | 3.3.0 (default-features=false) | hand-written `impl ServerHandler for RuntimeServer` (`crates/runtime-tools/src/mcp/server.rs:802`); also `OriginCheckServer` test handler | streamable HTTP via axum (`.nest_service("/v1/mcp", ...)`, `crates/runtime/src/http/routes.rs:578-603`); client too | `StreamableHttpService::new(...)` (`routes.rs:578`) |
| surrealdb | 1.4.0 (lock 1.7.0) | `#[tool_router]` + `#[tool_handler] impl ServerHandler for McpService` (`surrealdb/mcp/src/service.rs:781`); mutates router at construction (`service.rs:268`) | stdio + streamable HTTP via axum | `StreamableHttpService::new(move || { McpService::new(..).with_metrics_recorder(..) }, Arc::new(LocalSessionManager::default()), config)` (`service.rs:972`); `config.stateful_mode = true` |
| windmill | `=3.1.0` (client features) | hand-written generic `impl<B: McpBackend> ServerHandler for Runner<B>` (`windmill-mcp/src/server/runner.rs:449`) | streamable HTTP, sessionless, `.route_service("/", service)` | `StreamableHttpService::new(move || Ok(runner.clone()), session_manager, service_config)` (`windmill-api/src/mcp/core.rs:636`) with `.with_legacy_session_mode(false)`, `.disable_allowed_hosts()`, `.with_max_request_body_bytes(...)` |

(Some `file:line` values above come from grep over the pulled clones; line numbers drift with `git pull`.)

### 2.1 Version spread

rmcp requirements in use: 0.1.5 (Govcraft), 0.2.1 (elastic), 1.2.0, 1.4.0, 1.8.0, 2.0, 2.2, 3.x (3.1.0, 3.1.3, 3.3.0, 3.4.0, 3.4.1, 3.5.1, "3", 3.6.0). Lockfile pins sometimes lag the manifest (vibe-kanban 0.15.0, surrealdb 1.7.0, agentgateway 0.10.0). Most projects are on 3.x; 2.x and 1.x each have 2-3 projects; two are on pre-1.0 (counts approximate, from manifests). The major version moves every few weeks (1.x -> 3.x in months), so the SDK should target current 3.x and state a supported range; a generic `T: ServerHandler` bound compiles across majors only if the SDK depends on the same `rmcp` major as the app (Cargo allows two majors side by side, but traits would not match).
Note: handler construction API changed across majors (`ServerInfo::new(...).with_instructions(..)` in 3.x vs struct literal `ServerInfo { ... }` in 1.x; `ServerConfig` alias in devenv/bevy).

### 2.2 Common integration shapes (what the AgentCat SDK must support)

Server construction (handler):
- A. Macro handler: `#[tool_router] impl S {..}` + `#[tool_handler] impl ServerHandler for S { fn get_info.. }` (PostHog, goose, iggy, chroma, nushell, surrealdb, elastic, vibe-kanban). Dominant (about 9 projects, approximate).
- B. One-attribute: `#[tool_router(server_handler)]` (readyset; newest form).
- C. Hand-written `impl ServerHandler` with custom `list_tools`/`call_tool` (apollo, bevy_brp, wassette, windmill, spiceai, codex, devenv hybrid).
- D. Generic handlers `impl<B> ServerHandler for Runner<B>`, Arc/Box wrappers (rmcp provides `impl ServerHandler for Arc<T>/Box<T>`, `handler/server.rs:654`).

Serving (transports):
1. stdio: `handler.serve(stdio()).await?.waiting().await?` (most projects; `ServiceExt::serve`).
2. Streamable HTTP in axum: `StreamableHttpService::new(move || Ok(handler.clone()), LocalSessionManager::default().into(), StreamableHttpServerConfig::default())` mounted via `Router::nest_service("/mcp", svc)` / `route_service` / `fallback_service` (PostHog, iggy, apollo, devenv, wassette, readyset, spiceai, surrealdb, chroma, windmill).
3. Streamable HTTP over hyper directly (`TowerToHyperService`, nushell).
4. Duplex/in-process or `(stdin, stdout)` tuple transports (goose, devenv): any `IntoTransport`.
5. Legacy `SseServer::new` (elastic, Govcraft features) - rare and gone from new code.
6. The HTTP factory closure creates a handler per session (or per request when stateless); the SDK must be able to wrap inside the factory (`move || Ok(agentcat::instrument(handler.clone()))`) and see request headers/extensions (Host, auth) for actor identity and session id (`Mcp-Session-Id`). Stateless mode (chroma, windmill: `with_stateful_mode(false)`, `NeverSessionManager`) gives no session id: this is the TS "stateless mode (server assigns session)" case -> `session_id: Some(None)`.

Where the wrapper can attach (all shapes funnel through `ServerHandler` / `Service<RoleServer>`):
- `impl<H: ServerHandler> Service<RoleServer> for H` with a single `handle_request` choke point (`rmcp crates/rmcp/src/handler/server.rs:50-51`); about 25 overridable methods (`call_tool`, `list_tools`, `initialize`, ... `handler/server.rs:310-630`). So a generic `Instrumented<H: ServerHandler>` that forwards every method and times it covers shapes A-D, for both stdio (`Instrumented::new(h).serve(stdio())`) and HTTP (inside the factory closure).
- One-line setups the SDK should offer:
  - stdio: `agentcat::track(handler, opts).serve(stdio()).await?`
  - streamable HTTP: `StreamableHttpService::new(move || Ok(agentcat::track(h.clone(), opts.clone())), sessions, cfg)`
  - macro handlers work unchanged because `track` takes `impl ServerHandler`; no macro changes needed.
  - global one-time setup (project id, shutdown flush) plus an explicit flush/shutdown handle, since stdio servers exit when stdin closes (`service.waiting().await` then `agentcat.shutdown().await`).
- Required runtime: tokio multi-thread everywhere; sending must be spawned off the request path (the SDK may not assume `#[tokio::main]` flavor, but all surveyed projects use tokio and rmcp itself needs it).
- Dependency-sensitive consumers: codex and spiceai use `default-features = false` on rmcp, goose also; the SDK must not enable rmcp default features (default = base64, macros, server) beyond the minimum, and should let `rmcp` be a version-flexible dependency. TLS choice matters (not verified per project): a native-tls default in agentcat-api forces openssl into static or musl builds.
