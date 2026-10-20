# harness

The differential harness. One scripted MCP client (the driver) runs a scenario against a reference
server, a local mock API records every event the server publishes, a normalizer turns the run into a
language-neutral document, and a diff compares two documents. Allowed differences are listed in
`rulings.json`.

This package holds the TS half: the TS v1 reference server, instrumented with the real TS SDK at the
commit pinned in `oracle/oracle.lock`. The Rust reference server (plan 3) follows the same server
protocol and loads the same tool contract.

Run the tests with `corepack pnpm --filter harness test`. The test files run one at a time, and must
not run at the same time as `oracle/test/fetch.test.mjs`, which dirties the shared TS clone on purpose.

## Safety

Nothing in the harness contacts an address other than `127.0.0.1`, `localhost` or `[::1]`:

- Every TS server run preloads `src/egress-guard.mjs` (`node --import`), which rejects any `fetch` to
  another host with `egress-guard: blocked <origin>`. `ts-v1.mjs` also imports the guard as its first
  module.
- `ts-v1.mjs` refuses to start (exit 2, `refusing to start`) unless `AGENTCAT_API_URL` is an
  `http://` URL on one of those hosts.
- The driver gives every server `DISABLE_DIAGNOSTICS=true` and a temp `HOME` (the TS SDK writes
  `~/agentcat.log` there).
- Untracked (`HARNESS_NO_TRACK=1`), `ts-v1.mjs` never publishes: `emit_custom` answers without
  calling `publishCustomEvent`.

## Commands

```sh
# One run: writes harness/out/<scenario>.<label>.<transport>.{raw,transcript,normalized,report}.json
node harness/src/driver.mjs harness/scenarios/basic.json ts stdio \
  -- node --import ./harness/src/egress-guard.mjs harness/src/servers/ts-v1.mjs

# Compare two runs
node harness/src/diff.mjs harness/out/basic.ts.stdio.normalized.json \
  harness/out/basic.rust.stdio.normalized.json --rulings harness/rulings.json

# Regenerate the tool contract, or check that it is current
node harness/src/dump-contract.mjs
node harness/src/dump-contract.mjs --check
```

### driver.mjs

`node harness/src/driver.mjs <scenario.json> <label> <stdio|http|http-stateless> [--lane 2025-11-25] -- <server cmd...>`

- `label` tags the output files (letters, digits, `_`, `-`).
- The driver starts the mock API, then the server as `<server cmd...> stdio` (once per scenario
  session), `<server cmd...> http 0` or `<server cmd...> http-stateless 0`.
- `--lane` names the client's protocol lane. `2025-11-25` (the official TS client 1.31) is the
  default and the only value today; any other value exits 2.
- The server gets the parent environment plus `AGENTCAT_API_URL` (the mock), `HARNESS_PROJECT_ID`
  (the scenario's `projectId`), `HARNESS_OPTIONS` (the scenario's `options` as JSON),
  `DISABLE_DIAGNOSTICS=true`, and `HOME` and `USERPROFILE` set to
  `harness/out/home/<scenario>.<label>.<transport>/` (recreated per run). `HARNESS_NO_TRACK` passes
  through from the caller's environment.
- After the last step the driver:
  1. waits until the mock holds at least `expectPublishes` publish requests (POST
     `/v1/public/events/`) and no request of any kind arrived for `quietMs`, or until
     `settleTimeoutMs` passes;
  2. closes the clients and waits, up to `settleTimeoutMs`, for the servers to exit: a stdio server
     must exit when its stdin closes (the stdio client transport sends SIGTERM after 2 s, then
     SIGKILL); an HTTP server gets SIGTERM, then SIGKILL at the limit;
  3. keeps the mock open until no request arrived for `quietMs` (at most `settleTimeoutMs`), then
     takes the snapshot. A publish sent on client close or during a shutdown drain is counted.
- Exit 0 when the snapshot holds exactly `expectPublishes` publish requests and no other request.
  Exit 1 when the count differs, when any other request reached the mock (listed in the report as
  `nonPublishRequests`), or when the run fails. Exit 2 on bad arguments or a bad scenario.

### diff.mjs

`node harness/src/diff.mjs <a.normalized.json> <b.normalized.json> [--rulings <file>]`

Object keys compare unordered; arrays compare in order. Each ordered-check violation in either
document is a diff entry, even when both documents share it. Output lines:

- `DIFF  <path>: <a> != <b>` for a difference no ruling matches;
- `RULED <RD-xx> <path>: <a> != <b>` for a difference a ruling matches;
- `RULING <RD-xx> matched <n>: ...` or `UNUSED RULING <RD-xx> matched 0: ...` for each ruling;
- a JSON summary line `{"total", "ruled", "unexpected", "unusedRulings"}`.

Exit 1 on any difference no ruling matches and on any ruling that matches nothing; exit 2 on bad
arguments or a malformed rulings file.

### dump-contract.mjs

Runs `ts-v1.mjs` untracked over stdio and writes its full `tools/list` result to
`contract/tool-contract.json`. Never edit that file by hand. `--check` regenerates it into a temp
directory and exits 1 on any byte difference.

## Server protocol

Every reference server (TS now, Rust in plan 3) follows it. A Rust twin must match each item.

### Command line, environment and output

| Item | Meaning |
|---|---|
| argv `stdio` | MCP over stdio. stdout carries JSON-RPC only. The server exits when stdin closes. |
| argv `http <port>` | Stateful streamable HTTP on `127.0.0.1:<port>` (0: any free port), path `/mcp`. A request without a known `mcp-session-id` gets a new transport and a new tracked server (`initialize` creates the session; anything else is rejected by the transport). One tracked server per MCP session. |
| argv `http-stateless <port>` | Streamable HTTP with no session ID. Each POST gets a fresh tracked server and transport, both closed when the response closes. GET and DELETE answer 405 with `Allow: POST` and the body `{"jsonrpc":"2.0","error":{"code":-32000,"message":"Method not allowed."},"id":null}`. |
| HTTP responses | JSON response mode (`enableJsonResponse: true`): every POST is answered with one JSON body, not an SSE stream. |
| ready line | HTTP modes print one line on stdout and nothing else: `{"ready": true, "url": "http://127.0.0.1:<port>/mcp"}`. |
| SIGTERM | An HTTP server exits on SIGTERM (the TS SDK drains its queue first). |
| `AGENTCAT_API_URL` | Mock API base URL. Required; must be `http://` on `127.0.0.1`, `localhost` or `[::1]`, else exit 2 with `refusing to start` on stderr. |
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

The schemas are those of `contract/tool-contract.json` (generated from the TS server; the Rust
server loads it). The behaviors:

| Tool | Input | Result |
|---|---|---|
| `echo` | `{msg: string}` | `content: [{type: "text", text: msg}]` |
| `structured` | `{msg: string}`, output schema `{echoed: string}` | `content: [{type: "text", text: msg}]`, `structuredContent: {echoed: msg}` |
| `boom` | `{}` | The handler throws `Error("boom")`; the client receives `isError: true`, `content: [{type: "text", text: "boom"}]`. |
| `tool_error` | `{}` | Returns `isError: true`, `content: [{type: "text", text: "handled failure"}]` (no throw). |
| `big` | `{size: integer}` | `content: [{type: "text", text: "x" repeated size times}]` |
| `emit_custom` | `{session: string, resource: string, message?: string}` | Tracked: first awaits `publishCustomEvent(session, projectId, {resourceName: resource, message, parameters: {from: "emit_custom"}, tags: {kind: "custom"}})`. Untracked: no publish (the TS SDK's API URL is set only by `track`, so a publish would aim at production). Then returns `content: [{type: "text", text: "published"}]`. |

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

- `expectPublishes` is a number, or an object keyed by transport (`stdio`, `http`,
  `http-stateless`) when the counts differ.
- `httpHeaders` go on every HTTP request of the client.
- Each session is a new client connection. `saveHandleAs` stores the `ses_` handle a result issued
  (from `structuredContent.mcp_session.session_id` or a `session_id: ses_...` text line); `${NAME}`
  in later arguments is replaced with it.
- `listTools` takes an optional `cursor`.

The scenario files in `scenarios/` are the scenario names that `parity/manifest.toml` can reference
as `harness:<name>`.

## Normalized document

`{"transcript": [...], "events": [...], "nonPublishRequests": [...], "violations": [...]}`

- `transcript`: one entry per session start (`op: "initialize"`, with the negotiated
  `protocolVersion`) and per step (`op`, `request`, then `result` or `error`).
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
| `literal-derived-ids` | When the profile's `resolveSessionId` returns a non-blank string, the ID `deriveSessionId(value.trim(), projectId)` (computed with the TS `handles` module) stays literal instead of becoming a placeholder. IDs in `report.literalIds`. |

`<type>` is `string`, `number`, `boolean`, `object`, `array` or `null`.

Events are ordered by timestamp, then resource name, then the body with its IDs masked: the TS queue
publishes events concurrently, so arrival order is noise.

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
divergence in the design decision log. The list is empty until the Rust half produces differences.
