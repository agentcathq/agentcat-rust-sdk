# harness

The differential harness. One scripted MCP client (the driver) runs a scenario against a reference
server, a local mock API records every event the server publishes, a normalizer turns the run into a
language-neutral document, and a diff compares two documents. Allowed differences are listed in
`rulings.json` (Rust against TS) and `rulings-ts.json` (TS against TS).

This package holds two TS reference servers, instrumented with the real TS SDK at the commit pinned
in `oracle/oracle.lock`, and the Rust reference server (`rust-server/`, see "The Rust reference
server" below), instrumented with `agentcat-sdk`. `ts-v1.mjs` runs on `@modelcontextprotocol/sdk` 1.31;
`ts-v2.mjs` runs on `@modelcontextprotocol/server` 2.0.0 and also serves protocol 2026-07-28, which
has no `initialize` handshake (design decision C18). Owner ruling R20: `ts-v2.mjs` is the answer key
on both lanes; `ts-v1.mjs` stays as a second TS reference and is not what the Rust server is diffed
against. Both serve the same tools and share
`src/servers/common.mjs`. The Rust reference server follows the same server protocol and loads the
same tool contract, and must match TS on both client lanes.

Run the tests with `corepack pnpm --filter harness test`. The test files run one at a time, and must
not run at the same time as `oracle/test/fetch.test.mjs`, which dirties the shared TS clone on purpose.

## Safety

Nothing in the harness contacts an address other than `127.0.0.1`, `localhost` or `[::1]`:

- Every TS server run preloads `src/egress-guard.mjs` (`node --import`), which loads the egress guard
  of the oracle package (`oracle/src/egress-guard.mjs`). `ts-v1.mjs` and `ts-v2.mjs` also import it
  as their first module. In the server process, the guard refuses, with
  `egress-guard: blocked <target>` and before any packet leaves: a `fetch` to a host other than
  `127.0.0.1`, `localhost` or `[::1]`; a TCP connection (`net`, `http`, `https`, undici) or a TLS
  connection to a host other than `localhost`, `127.0.0.0/8` or `::1`; a DNS lookup of another host
  name; a UDP send to another address. Unix sockets and named pipes pass. It does not cover worker
  threads, child processes, native addons, or `dns.resolve*`.
- `ts-v1.mjs` and `ts-v2.mjs` refuse to start (exit 2, `refusing to start`) unless
  `AGENTCAT_API_URL` is an `http://` URL on `127.0.0.1`, `localhost` or `[::1]`, and pass it to the
  TS SDK as the `apiBaseUrl` option, so a TS SDK that stopped reading the variable still publishes
  to the mock.
- The driver gives every server `DISABLE_DIAGNOSTICS=true` and a temp `HOME` (the TS SDK writes
  `~/agentcat.log` there). The TS servers also pass `disableDiagnostics: true`.
- Untracked (`HARNESS_NO_TRACK=1`), the TS servers never publish: `emit_custom` answers without
  calling `publishCustomEvent`.
- The Rust reference server runs no egress guard. It refuses to start the same way (exit 2,
  `refusing to start`), with a stricter check of `AGENTCAT_API_URL`, and publishes nothing: the SDK
  has no delivery before plan 4.

## Commands

```sh
# One run: writes harness/out/<scenario>.<label>.<transport>.{raw,transcript,normalized,report}.json
node harness/src/driver.mjs harness/scenarios/basic.json ts stdio \
  -- node --import ./harness/src/egress-guard.mjs harness/src/servers/ts-v1.mjs

# The Rust reference server (build it first); it publishes nothing before plan 4
cargo build -p harness-ref-server
node harness/src/driver.mjs harness/scenarios/basic.json rust stdio --expect-publishes 0 \
  -- target/debug/harness-ref-server

# The 2026-07-28 lane against the TS v2 server
node harness/src/driver.mjs harness/scenarios/basic.json ts2 http --lane 2026-07-28 \
  -- node --import ./harness/src/egress-guard.mjs harness/src/servers/ts-v2.mjs

# Compare two runs: here two TS runs, the second with the label ts-b (a TS-vs-Rust document diff
# compares events, which the Rust server publishes from plan 4)
node harness/src/diff.mjs harness/out/basic.ts.stdio.normalized.json \
  harness/out/basic.ts-b.stdio.normalized.json --rulings harness/rulings-ts.json

# The MCP-wire delta of TS v2 against Rust on every scenario, transport and lane (the L3 gate's first run)
node harness/src/matrix.mjs

# Regenerate the tool contract, or check that it is current
node harness/src/dump-contract.mjs
node harness/src/dump-contract.mjs --check

# Check the publish bodies of a run against the publish contract (the driver does this itself);
# add --allow-empty for a run that expects no publish
node harness/src/contract/validate.mjs harness/out/basic.ts.stdio.raw.json

# Regenerate the publish contract from agentcat-server, or check it (local only: needs the private checkout)
git -C ~/agentcat-server fetch origin
node harness/src/contract/extract-schema.mjs ~/agentcat-server origin/main
node harness/src/contract/extract-schema.mjs --check ~/agentcat-server
```

### delta.mjs and matrix.mjs (the MCP-wire delta)

The MCP-wire delta is what tracking changes on the wire: the ops (`add`, `remove`, `change`) that turn
an untracked run's transcript into the tracked run's, for one server. `delta.mjs` compares the delta of
TS with the delta of Rust. Owner ruling R20: TS v2 is the answer key.

`node harness/src/delta.mjs <ts-untracked> <ts-tracked> <rust-untracked> <rust-tracked> [--rulings <file>]`
takes four `*.normalized.json` files. Paths look like `$.delta.transcript[4].result.content[0]`; a
tracked-document violation is an `add` at `$.delta.violations[<rule>]@<violation path>`. Output and exit
codes follow `diff.mjs` (`DELTA <path>: ts <x> != rust <y>`, `RULED ...`, ruling lines, a JSON summary).

`node harness/src/matrix.mjs [--servers <a>,<b>] [--scenarios <names>] [--transports <list>] [--lanes <list>] [--jobs <n>] [--rulings <file>] [--env2 <NAME=value>]...`
runs, per scenario, transport and lane, both servers untracked (`HARNESS_NO_TRACK=1`) and tracked, then
compares the two deltas. Servers: `ts-v1`, `ts-v2` (default first) and `rust` (default second;
`HARNESS_RUST_SERVER`, else `target/debug/harness-ref-server`). Labels are `<server>-<lane>-<u|t>`; two
equal server names (an A/A run) give the second server the suffix `-b`. `--env2` sets an environment
variable for every run of the second server, untracked and tracked (repeatable; `matrix.json` records it).
It writes `harness/out/matrix.json`.

Rulings: `rulings.json` holds the delta rulings for Rust against TS (`delta.mjs`, the matrix). A TS-vs-Rust
document diff with `diff.mjs` compares events, which wait for plan 4, so in plan 3 the file holds delta
rulings only. `rulings-ts.json` is for TS against TS (A/A runs and the nightly drift sweep, so a Rust
ruling never counts as unused there).

### The L3 gate

`scripts/check.sh` and the CI job `harness-l3` build the Rust reference server, then run three matrices:

1. `node harness/src/matrix.mjs`: TS v2 against Rust, every scenario, transport and lane. It must exit 0:
   every driver passes, every delta difference has a ruling, and every ruling matches something.
2. The A/A run `node harness/src/matrix.mjs --servers rust,rust --scenarios basic`: it must exit 0 with no
   delta difference at all, ruled or not. `node harness/src/l3-expect.mjs none` checks that
   `delta.total` is 0 in every combination of `matrix.json`.
3. The canary run, the A/A run plus `--env2 HARNESS_CANARY=1`: the second server serves the context
   description `"canary"`. It must exit 1. `node harness/src/l3-expect.mjs caught` checks that every
   driver passed and that every combination reports an unruled difference, each under
   `$.delta.transcript[1].result.tools[` (basic.json's tracked listing). A canary that passes means the
   delta comparison has gone blind; one caught elsewhere is a different failure.

`l3-expect.mjs <none|caught> [matrix.json]` reads `harness/out/matrix.json` by default; exit 0 as expected,
1 not (one line per bad combination), 2 on bad arguments.

One full matrix (36 combinations, 144 driver runs) took 75 to 100 s at `--jobs 4` on the development
machine; the A/A and canary runs about 12 s each.

### driver.mjs

`node harness/src/driver.mjs <scenario.json> <label> <stdio|http|http-stateless> [--lane 2025-11-25|2026-07-28] [--expect-publishes <n>] -- <server cmd...>`

- `label` tags the output files (letters, digits, `_`, `-`). The lane is not in the file name: give
  runs on different lanes different labels.
- The driver starts the mock API, then the server as `<server cmd...> stdio` (once per scenario
  session), `<server cmd...> http 0` or `<server cmd...> http-stateless 0`.
- `--lane` names the client's protocol lane; any other value exits 2:
  - `2025-11-25` (default): the official TS client 1.31 (`@modelcontextprotocol/sdk`), with the
    `initialize` handshake.
  - `2026-07-28`: the v2 client (`@modelcontextprotocol/client` 2.3.1) with
    `versionNegotiation: { mode: "auto" }`, as `connectV2Client` in the TS repository's
    `src/tests/e2e-http/lanes.ts`. It probes with `server/discover` (over stdio, on a short-lived
    sibling server process) and falls back to `initialize` when the server gives no proof of the
    2026-07-28 era. A session that negotiates a version other than `--lane` fails the run: exit 1
    with `driver: negotiated <version>, expected <lane>`.
- `--expect-publishes <n>` (a non-negative integer) replaces the scenario's `expectPublishes` count.
- The server gets the parent environment plus `AGENTCAT_API_URL` (the mock), `HARNESS_PROJECT_ID`
  (the scenario's `projectId`), `HARNESS_OPTIONS` (the scenario's `options` as JSON),
  `DISABLE_DIAGNOSTICS=true`, and `HOME` and `USERPROFILE` set to
  `harness/out/home/<scenario>.<label>.<transport>/` (recreated per run). `HARNESS_NO_TRACK` passes
  through from the caller's environment.
- In a tracked run (`HARNESS_NO_TRACK` unset), a `saveHandleAs` step whose result carries no handle,
  and a `${NAME}` with no saved value, exit 1 with
  `driver: step <i> of session <s>: no session handle for <NAME>`. An untracked run issues no handles
  and keeps the `${NAME}` text literally.
- The `initialize` transcript entry of each session holds
  `{protocolVersion, serverInfo, capabilities, instructions}` as the client reports them, on both
  lanes (`null` for a part the server did not send).
- On a driver error every connected client is closed before the driver exits, so a stdio server the
  client spawned ends too.
- After the last step the driver:
  1. waits until the mock holds at least `expectPublishes` publish requests (POST
     `/v1/public/events/`) and no request of any kind arrived for `quietMs`, or until
     `settleTimeoutMs` passes;
  2. closes the clients and waits, up to `settleTimeoutMs`, for the servers to exit: a stdio server
     must exit when its stdin closes (the stdio client transport sends SIGTERM after 2 s, then
     SIGKILL); an HTTP server gets SIGTERM, then SIGKILL at the limit;
  3. keeps the mock open until no request arrived for `quietMs` (at most `settleTimeoutMs`), then
     takes the snapshot. A publish sent on client close or during a shutdown drain is counted.
- It then validates every publish body against the publish contract (`contract/`, below). In
  `report.json`, `contractChecked` is the number of bodies checked, and `contract` is `"ok"`,
  `"no publish bodies"` (nothing to check although the scenario expects publishes), or one entry
  per invalid body (`{seq, id, event_type, resource_name, errors: [{instancePath, keyword, message, params}]}`).
  A run with no publish body passes the contract only when the scenario expects 0 publishes.
  `ts_sha` in `report.json` is the TS commit the run used: `oracle/oracle.lock`, or `ORACLE_SHA`
  when it is set (the driver then writes a trace line to stderr).
- Exit 0 when the snapshot holds exactly `expectPublishes` publish requests and no other request,
  and the contract is `"ok"`. Exit 1 when the count differs, when any other request reached the
  mock (listed in the report as `nonPublishRequests`), when the contract fails (each error on
  stderr), or when the run fails. Exit 2 on bad arguments or a bad scenario.

### diff.mjs

`node harness/src/diff.mjs <a.normalized.json> <b.normalized.json> [--rulings <file>]`

Object keys compare unordered; arrays compare in order, and arrays of different length give one entry
per element (`<missing>` on the shorter side). Each ordered-check violation in either
document is a diff entry, even when both documents share it. Output lines:

- `DIFF  <path>: <a> != <b>` for a difference no ruling matches;
- `RULED <RD-xx> <path>: <a> != <b>` for a difference a ruling matches;
- `RULING <RD-xx> matched <n>: ...` or `UNUSED RULING <RD-xx> matched 0: ...` for each ruling
  (`RULING (pending) <RD-xx> matched <n>: ...` for a ruling marked `"pending": true`);
- a JSON summary line `{"total", "ruled", "unexpected", "unusedRulings"}`.

Exit 1 on any difference no ruling matches and on any ruling that matches nothing; exit 2 on bad
arguments or a malformed rulings file.

### dump-contract.mjs

Runs `ts-v2.mjs` untracked over stdio and writes its full `tools/list` result to
`contract/tool-contract.json` (R20: v2 is the answer key). The definitions (name, description, input
and output JSON schemas) live once in `TOOLS` in `src/servers/common.mjs`, next to `toolHandlers`;
`ts-v2.mjs` registers from `TOOLS`, and `ts-v1.mjs` declares the same tools in zod. Adding a tool
means `TOOLS`, `toolHandlers`, ts-v1's zod and the behavior in `rust-server/src/tools.rs`, then
regenerating the file. Never edit it by hand. `--check` regenerates it into a temp
directory and exits 1 on any byte difference (drift between `TOOLS` and the file). A test compares
ts-v1's `tools/list`, minus `execution`, with the file.

## Publish contract (L4)

Every body a reference server publishes must be one the AgentCat backend accepts.

- `contract/publish-event.schema.json` is the request-body schema of `POST /v1/public/events/`
  (operationId `publishEvent`) from agentcat-server's `schema.yml`, converted from OpenAPI 3.0 to
  JSON Schema 2020-12. `contract/SOURCE` names the commit: `agentcat-server <sha> schema.yml`. Both
  files are generated; never edit them by hand.
- `src/contract/extract-schema.mjs <checkout> <sha or ref>` reads `schema.yml` with
  `git show <sha>:schema.yml` (it never checks out or changes the checkout) and writes both files.
  `--check <checkout>` regenerates from the SHA in `SOURCE` into a temp directory and exits 1 on any
  byte difference. agentcat-server is private, so both run locally, never in CI (R14); the test for
  `--check` skips when there is no checkout at `AGENTCAT_SERVER_DIR` (default `~/agentcat-server`).
- The conversion copies `required`, `additionalProperties` and every other validation keyword as
  written. `nullable: true` becomes `type: [<t>, "null"]`, or `anyOf: [{"type": "null"}, <schema>]`
  when the schema has no `type` or uses `$ref`, `allOf`, `oneOf`, `anyOf` or `not`; local `$ref`s are
  inlined; boolean `exclusiveMinimum`/`exclusiveMaximum` take the 2020-12 numeric form; `example`
  becomes `examples`; `x-*` extensions are dropped. Any other keyword (`discriminator`, `readOnly`,
  ...), a recursive or non-local `$ref`, or `nullable` with an `enum` that lacks `null` stops the
  extraction with an error. `schema.yml` is read as YAML 1.2 and as YAML 1.1 (PyYAML writes it), and
  the two readings must agree.
- `src/contract/validate.mjs [--allow-empty] <raw.json>` validates every publish body of a run with
  ajv (strict mode; ajv-formats "full" mode, so `date-time` needs a timezone and a real calendar
  day). Exit 0 when there is at least one publish body and all are valid. Exit 1 with one
  `INVALID publish seq=<n> id=<id> (<event_type> <resource_name>): <path> <message>` line per error,
  or when the file holds no publish body (a wrong path or a wrong file never passes);
  `--allow-empty` accepts zero bodies, for an untracked run. Exit 2 on bad arguments or an
  unreadable file. The driver runs the same check after every run.
- `ingest/` replays the bodies of a run through the backend's real ingest function `process_event`,
  locally, before the final PR (R14). See `ingest/README.md`.

## Server protocol

Every reference server (TS and Rust) follows it. The Rust twin matches each item, except for the
differences listed in "The Rust reference server" below.

### Command line, environment and output

| Item | Meaning |
|---|---|
| argv `stdio` | MCP over stdio. stdout carries JSON-RPC only. The server exits when stdin closes. One tracked server per connection. A server that serves 2026-07-28 picks the era from the opening message (`ts-v2.mjs`: `serveStdio`). |
| argv `http <port>` | Stateful streamable HTTP on `127.0.0.1:<port>` (0: any free port), path `/mcp`. A request without a known `mcp-session-id` gets a new transport and a new tracked server (`initialize` creates the session; anything else is rejected by the transport). One tracked server per MCP session. A 2026-07-28 request (it carries the per-request `_meta` envelope; the protocol has no sessions) gets a fresh tracked server per request (`ts-v2.mjs`: `createMcpHandler`). |
| argv `http-stateless <port>` | Streamable HTTP with no session ID. Each POST gets a fresh tracked server and transport, both closed when the response closes. GET and DELETE answer 405 with `Allow: POST` and the body `{"jsonrpc":"2.0","error":{"code":-32000,"message":"Method not allowed."},"id":null}`. A 2026-07-28 request is served as in `http` mode. |
| HTTP responses | JSON response mode (`enableJsonResponse: true`; `createMcpHandler`'s `responseMode: "json"`): every POST is answered with one JSON body, not an SSE stream. Rust: `http-stateless`, and 2026-07-28 requests in either mode, answer JSON. In `http` mode, a 2025-era POST gets rmcp's SSE stream (a priming event, then one event per JSON-RPC message), because rmcp serves JSON only without session mode. |
| ready line | HTTP modes print one line on stdout and nothing else: `{"ready": true, "url": "http://127.0.0.1:<port>/mcp"}`. |
| SIGTERM | An HTTP server exits on SIGTERM (the TS SDK drains its queue first). |
| `AGENTCAT_API_URL` | Mock API base URL. Required; must be `http://` on `127.0.0.1`, `localhost` or `[::1]`, else exit 2 with `refusing to start` on stderr. The TS server passes it to `track` as `apiBaseUrl`. |
| `HARNESS_PROJECT_ID` | Project ID passed to `track` and to custom events. Required unless untracked. |
| `HARNESS_OPTIONS` | The options profile (JSON, below). Default `{}`. |
| `HARNESS_NO_TRACK=1` | Serve the same tools untracked: no `track`, and `emit_custom` publishes nothing. |
| `HOME`, `USERPROFILE` | A harness temp dir (the TS SDK writes `~/agentcat.log`). |
| `DISABLE_DIAGNOSTICS=true` | Set by the driver. The TS server also always passes `disableDiagnostics: true` to `track`, whatever the profile says. |

Exit 2 on bad arguments, a non-local `AGENTCAT_API_URL` or an invalid options profile. Logs go to
stderr only.

### Server identity

`serverInfo` is `{"name": "harness-ref-server", "version": "1.0.0"}`. Events carry it as
`server_name` and `server_version`.

### Tools

The definitions are `TOOLS` in `src/servers/common.mjs`. `contract/tool-contract.json` is the
`tools/list` result of `ts-v2.mjs` (R20: v2 is the answer key on both lanes; no `execution` field),
generated by `dump-contract.mjs`. The Rust server embeds that file (names, descriptions, input and
output schemas); `ts-v1.mjs` declares the same tools in zod. Arguments are validated as zod objects validate them: a key the schema does not declare is
dropped before the handler runs, not rejected, although the listed schema says
`additionalProperties: false`. Each server instance gets its own handler functions: the TS SDK marks
every function it wraps, so a function shared by two v1 servers is wrapped only on the first, and a
later server's thrown error loses its stack. The behaviors (in `src/servers/common.mjs`):

| Tool | Input | Result |
|---|---|---|
| `echo` | `{msg: string}` | `content: [{type: "text", text: msg}]` |
| `structured` | `{msg: string}`, output schema `{echoed: string}` | `content: [{type: "text", text: msg}]`, `structuredContent: {echoed: msg}` |
| `boom` | `{}` | The handler throws `Error("boom")`; the client receives `isError: true`, `content: [{type: "text", text: "boom"}]`. |
| `tool_error` | `{}` | Returns `isError: true`, `content: [{type: "text", text: "handled failure"}]` (no throw). |
| `big` | `{size: integer}` | `content: [{type: "text", text: "x" repeated size times}]` |
| `emit_custom` | `{session: string, resource: string, message?: string}` | Tracked: first awaits `publishCustomEvent(session, projectId, {resourceName: resource, message, parameters: {from: "emit_custom"}, tags: {kind: "custom"}})`. Untracked: no publish (the TS SDK's API URL is set only by `track`, so a publish would aim at production). Then returns `content: [{type: "text", text: "published"}]`. |
| `own_session` | `{session_id: string, x?: string}` (the tool declares its own `session_id`, so the SDK treats it as foreign) | `content: [{type: "text", text: "session_id=<value received>"}]` |
| `own_context` | `{context: string}` (the tool declares its own `context`) | `content: [{type: "text", text: context}]` |
| `structured_own` | `{msg: string}`, output schema `{echoed: string, mcp_session?: {session_id: string, ...}}` (it declares `mcp_session` itself) | `content: [{type: "text", text: msg}]`, `structuredContent: {echoed: msg}` |

### TS v1 and v2 differences

On the 2025 lane, `ts-v1.mjs` and `ts-v2.mjs` publish the same events on each transport (observed
with `basic.json`). Their transcripts differ where the host MCP SDK differs:

- `tools/list`: v1 adds `execution: {"taskSupport": "forbidden"}` to each tool; v2 does not. The
  contract follows v2.
- Untracked: a call to an unknown tool answers an `isError` result
  (`MCP error -32602: Tool <name> not found`) on v1 and a JSON-RPC error `-32602` on v2.
- Invalid arguments: both answer an `isError` result, with the validator's own text (v1: zod, with
  an `MCP error -32602: ` prefix; v2: ajv).

On the 2026-07-28 lane, the transcript and events also differ from the 2025 lane by protocol: no
`initialize` request (JSON-RPC IDs start one lower), every result carries
`_meta["io.modelcontextprotocol/serverInfo"]`, `tools/list` adds `ttlMs` and `cacheScope`, the
`mcp-protocol-version` header is `2026-07-28`, and the TS SDK adds the event tag
`agentcat_protocol_version: "2026-07-28"`.

### The Rust reference server

`rust-server/` is the Cargo package `harness-ref-server` (a workspace member, `publish = false`).
Build it with `cargo build -p harness-ref-server`. The harness tests and `matrix.mjs` run
`target/debug/harness-ref-server` (`HARNESS_RUST_SERVER` overrides the path); `scripts/check.sh` and
the CI job `oracle-harness` build it before the harness tests.

- `src/main.rs`: the command line, the environment and the modes. `stdio` serves one tracked server
  (`AgentCat::track`) for the connection. `http` serves rmcp's `StreamableHttpService` with a
  `LocalSessionManager` and `legacy_session_mode(true)`, `http-stateless` with
  `legacy_session_mode(false)`, both with `json_response(true)`. rmcp honors `json_response` only
  where it serves a request without a session: in `http-stateless`, and for 2026-07-28 requests in
  either mode. Each MCP session or request gets a fresh tracked server from one shared `AgentCat`
  handle. rmcp serves 2026-07-28 requests and `server/discover` itself in every mode (over HTTP, a
  fresh server per 2026-07-28 request).
- `src/tools.rs`: the tools. Names, descriptions and schemas come only from
  `contract/tool-contract.json`, embedded with `include_str!`; the behaviors follow `toolHandlers`
  in `common.mjs`. rmcp's `ToolRouter` does the dispatch, `get_tool`, the argument errors and the
  conversion of a tool's `Err` into an `isError` result. `tools/list` keeps the contract's order
  (rmcp's router sorts by name) and, on 2026-07-28, adds `ttlMs: 0` and `cacheScope: "private"` as
  ts-v2 does. `boom` records its error with `capture_error`. `emit_custom` answers `published` and
  publishes nothing (the SDK has no custom events before plan 4).
- `src/profile.rs`: the options profile.

Where it differs from the TS servers:

- `http` mode answers each 2025-era POST with an SSE stream, not one JSON body: a priming event
  (`data:` empty, `id`, `retry: 3000`), then one event per JSON-RPC message. In rmcp 3.7.0,
  `json_response` takes effect only when `legacy_session_mode` is false; the session path always
  answers with an SSE stream. The driver's clients read both. `http-stateless` and 2026-07-28
  requests answer JSON, as the TS servers do.
- `AGENTCAT_API_URL`: the check is stricter than the WHATWG parse of `common.mjs`: no user info,
  backslash, whitespace or control character; the host spelled `127.0.0.1`, `localhost` (any case)
  or `[::1]`; a port, when given, of 1 to 5 digits below 65536. The value is checked and kept: the
  SDK has no API URL option and publishes nothing yet (plan 4), so the matrix runs the tracked Rust
  server with `--expect-publishes 0`.
- The profile: a flag must be a boolean (`customContextDescription` a string), where `common.mjs`
  copies any value. A `returns` value must fit the hook's SDK type: `resolveSessionId` a string or
  `null`; `identify` `null` or `{"userId": string, "userName"?: string, "userData"?: object}`;
  `eventTags` and `eventProperties` an object or `null`; `redactSensitiveInformation` a string.
  Anything else exits 2. `redactSensitiveInformation` is checked and kept: the SDK has no redaction
  yet (plan 4). Every hook is async; `sleepMs` sleeps on the tokio timer.
- `HARNESS_CANARY=1` sets the context description to `"canary"`, whatever the profile says: the
  perturbed configuration that the L3 canary run must catch.
- `big` with a negative `size`, or one above V8's longest string, fails with JavaScript's
  `RangeError` text (`Invalid count value: <n>`, `Invalid string length`) instead of allocating.
- Argument errors carry serde's text (`failed to deserialize parameters: ...`) where ts-v2 carries
  ajv's. An unknown tool answers the JSON-RPC error `-32602` `tool not found` (ts-v2:
  `Tool <name> not found`). On 2026-07-28, rmcp adds no `_meta["io.modelcontextprotocol/serverInfo"]`
  to results; the normalizer rule `server-info-meta-drop` drops that key (RD-13).

## Options profile

A JSON object. Every key needs a Rust twin. An unknown key is an error (exit 2).

Flags, copied to the SDK options as given: `enableTracing`, `enableReportMissing`,
`enableToolCallContext`, `enableAgentTracking`, `customContextDescription`.

Hooks, each a canned behavior: `resolveSessionId`, `identify`, `eventTags`, `eventProperties`,
`redactSensitiveInformation`. A behavior is one of:

| Behavior | The hook... |
|---|---|
| `{"returns": <value or null>}` | returns a fresh copy of the value. |
| `{"throws": "<message>"}` | throws `new Error(message)` (rejects, for a hook that returns a Promise). |
| `{"replace": "<secret>"}` | `redactSensitiveInformation` only: replaces every occurrence of the secret with `[REDACTED]`. |

The value each hook's `returns` takes (as in the TS SDK's `src/types.ts`):

| Hook | Arguments it receives (ignored by the canned behavior) | `returns` value |
|---|---|---|
| `resolveSessionId` | the MCP request, the request extra | a string or `null` |
| `identify` | the MCP request, the request extra | `{"userId": string, "userName"?: string, "userData"?: object}` or `null` |
| `eventTags` | the MCP request, the request extra | an object of string values, or `null` |
| `eventProperties` | the MCP request, the request extra | an object of any JSON values, or `null` |
| `redactSensitiveInformation` | one text string | a string (used for every text) |

Any behavior can add `"sleepMs": <n>`: the hook waits n milliseconds, then answers. The TS server
turns each behavior into a function with the hook's signature from `src/types.ts` at the pinned
commit: `resolveSessionId`, `eventTags` and `eventProperties` answer synchronously unless `sleepMs`
is given; `identify` and `redactSensitiveInformation` always return a Promise.

Example (`scenarios/basic.json`):

```json
{
  "identify": { "returns": { "userId": "user-1", "userName": "User One", "userData": { "plan": "pro" } } },
  "eventTags": { "returns": { "team": "core" } },
  "eventProperties": { "returns": { "region": "eu" } },
  "redactSensitiveInformation": { "replace": "SECRET_TOKEN" }
}
```

## Scenario format

```json
{
  "name": "basic",
  "projectId": "proj_...",
  "options": { },
  "httpHeaders": { "x-test-header": "harness" },
  "expectPublishes": 11,
  "settleTimeoutMs": 10000,
  "quietMs": 500,
  "sessions": [
    {
      "client": { "name": "harness-client-a", "version": "1.0.0" },
      "steps": [
        { "op": "listTools" },
        { "op": "callTool", "name": "echo", "args": { "msg": "hi" }, "saveHandleAs": "S1" },
        { "op": "callTool", "name": "echo", "args": { "msg": "again", "session_id": "${S1}" } }
      ]
    }
  ]
}
```

- `expectPublishes` is a number, or an object when the counts differ, keyed by
  `<transport>@<lane>` (for example `http@2026-07-28`) or by transport (`stdio`, `http`,
  `http-stateless`). The driver takes `<transport>@<lane>`, then `<transport>`. Any other key exits 2.
- The driver exits 2 on a key it does not know at the top level of the scenario, in a session
  (`client`, `steps`) or in a step (`listTools`: `op`, `cursor`; `callTool`: `op`, `name`, `args`,
  `saveHandleAs`).
- `httpHeaders` go on every HTTP request of the client.
- Each session is a new client connection. `saveHandleAs` stores the `ses_` handle a result issued
  (from `structuredContent.mcp_session.session_id` or a `session_id: ses_...` text line); `${NAME}`
  in later arguments is replaced with it.
- `listTools` takes an optional `cursor`.
- No number in a scenario may be a decimal with more than 15 significant digits: the Rust SDK reads
  such a decimal without `float_roundtrip`, and it can become the neighboring `f64` (RD-23).
- `expectPublishes` is the count the tracked TS v2 server publishes, read from a TS run. The tracked Rust
  server publishes nothing before plan 4, and the matrix runs it with `--expect-publishes 0`.

The scenario files in `scenarios/` are the scenario names that `parity/manifest.toml` can reference
as `harness:<name>`:

| Scenario | Options profile | What its calls cover |
|---|---|---|
| `basic` | `identify`, `eventTags`, `eventProperties`, `redactSensitiveInformation` | Mint-back (minted, supplied, invalid), context, the structured mirror, a thrown error, an `isError` result, a large result, `get_more_tools`, a custom event, a second client. |
| `schemas` | `{}` (the defaults of B-004) | The tools that declare `session_id`, `context` or an output `mcp_session` themselves, with and without our handles, before and after a listing; the unknown tool `nope`; an argument error (`echo` with `msg: 5`); the mirror on `structured`. |
| `hook-mode` | `resolveSessionId` returns `"customer-abc"`, `enableAgentTracking` | Hook mode with agent tracking: no `session_id` injected, calls with and without `agent_id`, the mirror with `agent_id` only. |
| `agent-tracking` | `enableAgentTracking`, `customContextDescription` | Calls with and without `agent_id` (a two-line value, a number, blanks), with minted, supplied and invalid handles; a call before any listing. |
| `features-off` | `enableReportMissing: false`, `enableToolCallContext: false` | No `get_more_tools` and no `context` injection; a `context` argument that is not ours; a call to `get_more_tools`. |
| `tracing-off` | `enableTracing: false` | Calls with `context`, and `own_context` with its own `context` (RD-29 on per-request TS servers); no handles, no mint-back, no mirror; `get_more_tools` answered by the SDK. |

## Normalized document

`{"transcript": [...], "events": [...], "nonPublishRequests": [...], "violations": [...]}`

- `transcript`: one entry per session start (`op: "initialize"`, with the negotiated
  `protocolVersion`, on both lanes: on 2026-07-28 no `initialize` request is sent) and per step
  (`op`, `request`, then `result` or `error`).
- `events`: one entry per publish request,
  `{"mock_status": <status>, "content_type": <header>, "query": <query string>, "body": <normalized body>}`.
- `nonPublishRequests`: every other request the mock received, `{"method", "path", "query", "status"}`,
  sorted.
- `violations`: failed ordered checks, `{"rule", "path", ...}`.

The normalizer rules, each listed in `report.rules` of the run report:

| Rule | What it does |
|---|---|
| `request-fields` | A publish keeps its status, `content-type` and query string; its other headers (`user-agent`, `accept`) are dropped as language-specific. Every non-publish request stays as method, path, query and status. |
| `camelCase-alias-drop` | Drops a camelCase body key when its snake_case twin exists (the TS generated client sends both). Dropped names go to `report.camelCaseAliasKeys`. |
| `timestamp-class` | `timestamp` becomes `<iso8601-ms-Z>`, or `<bad-timestamp:...>`. |
| `duration-class` | `duration` becomes `<duration-ms>` (non-negative integer), or `<bad-duration:...>`. |
| `language-fields` | `sdk_language` becomes `<sdk_language>` when it is `"TypeScript"` or `"Rust"`, else `<bad-sdk_language:<value>>`; `agentcat_version` becomes `<semver>`, or `<bad-version:...>`. |
| `extra-headers-only` | `parameters.extra` (the host SDK's request context) becomes `{"<headers>": {...}}` with only `user-agent`, `x-anthropic-client`, `x-test-header` and `mcp-protocol-version`. |
| `error-shape-class` | In `error` and each `error.chained_errors` entry: `stack` becomes `<stack:<type>>` (`<stack:empty>` for `""`); `frames` becomes `<frames:array:<sorted, comma-joined union of the frame keys>>` (a frame that is not an object adds `<type>`), or `<frames:<type>>` when not an array; `platform` becomes `<platform:<type>>`. |
| `id-placeholders` | Every `evt_`, `ses_` and `agt_` KSUID becomes `<prefix#n>` by first appearance, the same across transcript and events. |
| `mcp-session-first` | Every `structuredContent` that has `mcp_session` must have it as its first key; a violation goes to `violations`. |
| `property-order` | For each tool in a `tools/list` result, adds `inputSchema["<properties-order>"]`: the key order of `inputSchema.properties`. |
| `server-info-meta-drop` | In each transcript `result`, drops `_meta["io.modelcontextprotocol/serverInfo"]`, and `_meta` when that leaves it empty. The TS v2 host library adds the key to every 2026-07-28 result and rmcp adds none: host behavior, not AgentCat's (RD-13). Other `_meta` keys, errors and event bodies stay. |
| `literal-derived-ids` | When the profile's `resolveSessionId` returns a non-blank string, the ID `deriveSessionId(value.trim(), projectId)` (computed with the TS `handles` module) stays literal instead of becoming a placeholder. IDs in `report.literalIds`. |

`<type>` is `string`, `number`, `boolean`, `object`, `array` or `null`.

Events are ordered by timestamp; in the same millisecond, an event with a `duration` (a call, which
can enclose events published during it) comes before one without; then by resource name, then by the
body with its IDs masked. The TS queue publishes events concurrently, so arrival order is noise. The
`duration` step keeps a custom event published during a tool call after the call event, also when
both land in the same millisecond.

The ordered checks read key order as JavaScript keeps it, so they do not apply to integer-like keys
(JavaScript puts those first).

## Rulings

`rulings.json` lists each allowed difference:

```json
[{ "path": "<regex over whole diff paths>", "reason": "why the difference is allowed", "deviation": "RD-xx" }]
```

`path` must match the whole diff path: diff.mjs anchors it as `^(?:<path>)$`, so `resource_name`
alone matches nothing. A diff path looks like `$.events[3].body.resource_name` or
`$.transcript[1].result.tools[0].inputSchema.<properties-order>`; a violation's path ends with
`.<rule>`. Write `\$`, `\.`, `\[` and `\]` for the literal characters. `deviation` names the ruled
divergence in the design decision log. A ruling may also carry `"pending": true` (a boolean, nothing
else): a provisional ruling that waits for the owner's answer. It allows differences like any other.

A delta ruling may also carry `"scenario": "<name>"`, a scenario name (`scenarios/<name>.json`). A delta
path names the transcript entry by its index only, so a ruling for one call of one scenario needs it:
`matrix.mjs` then applies the ruling only to that scenario's combinations, and a run that leaves the
scenario out neither reports the ruling nor counts it as unused. Without `scenario`, a delta ruling
covers its index in every scenario, transport and lane. A `scenario` that is not a string of letters,
digits, `_` and `-` makes the rulings file malformed (exit 2); one that names no scenario file makes
`matrix.mjs` exit 2. `diff.mjs` and `delta.mjs` compare documents that carry no scenario name: they
ignore `scenario` and match by path alone. The ruling line of a scoped ruling ends its count with
`in <scenario>` (`RULING (pending) RD-29 matched 6 in tracing-off: ...`). Keep a delta ruling's regex
to the exact paths. In plan 3, `rulings.json` holds delta rulings only.
