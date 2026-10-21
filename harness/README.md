# harness

The differential harness. One scripted MCP client (the driver) runs a scenario against a reference
server, a local mock API records every event the server publishes, a normalizer turns the run into a
language-neutral document, and a diff compares two documents. Allowed differences are listed in
`rulings.json`.

This package holds the TS half: two TS reference servers, instrumented with the real TS SDK at the
commit pinned in `oracle/oracle.lock`. `ts-v1.mjs` runs on `@modelcontextprotocol/sdk` 1.31;
`ts-v2.mjs` runs on `@modelcontextprotocol/server` 2.0.0 and also serves protocol 2026-07-28, which
has no `initialize` handshake (design decision C18). Both serve the same tools and share
`src/servers/common.mjs`. The Rust reference server (plan 3) follows the same server protocol and
loads the same tool contract, and must match TS on both client lanes.

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

## Commands

```sh
# One run: writes harness/out/<scenario>.<label>.<transport>.{raw,transcript,normalized,report}.json
node harness/src/driver.mjs harness/scenarios/basic.json ts stdio \
  -- node --import ./harness/src/egress-guard.mjs harness/src/servers/ts-v1.mjs

# The 2026-07-28 lane against the TS v2 server
node harness/src/driver.mjs harness/scenarios/basic.json ts2 http --lane 2026-07-28 \
  -- node --import ./harness/src/egress-guard.mjs harness/src/servers/ts-v2.mjs

# Compare two runs
node harness/src/diff.mjs harness/out/basic.ts.stdio.normalized.json \
  harness/out/basic.rust.stdio.normalized.json --rulings harness/rulings.json

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

### driver.mjs

`node harness/src/driver.mjs <scenario.json> <label> <stdio|http|http-stateless> [--lane 2025-11-25|2026-07-28] -- <server cmd...>`

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
    2026-07-28 era. The driver does not fail on a fallback: the transcript records the negotiated
    version, so a diff against a 2026-07-28 run shows it.
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
- It then validates every publish body against the publish contract (`contract/`, below). In
  `report.json`, `contractChecked` is the number of bodies checked, and `contract` is `"ok"`,
  `"no publish bodies"` (nothing to check although the scenario expects publishes), or one entry
  per invalid body (`{seq, id, event_type, resource_name, errors: [{instancePath, keyword, message, params}]}`).
  A run with no publish body passes the contract only when the scenario expects 0 publishes.
- Exit 0 when the snapshot holds exactly `expectPublishes` publish requests and no other request,
  and the contract is `"ok"`. Exit 1 when the count differs, when any other request reached the
  mock (listed in the report as `nonPublishRequests`), when the contract fails (each error on
  stderr), or when the run fails. Exit 2 on bad arguments or a bad scenario.

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

Every reference server (TS now, Rust in plan 3) follows it. A Rust twin must match each item.

### Command line, environment and output

| Item | Meaning |
|---|---|
| argv `stdio` | MCP over stdio. stdout carries JSON-RPC only. The server exits when stdin closes. One tracked server per connection. A server that serves 2026-07-28 picks the era from the opening message (`ts-v2.mjs`: `serveStdio`). |
| argv `http <port>` | Stateful streamable HTTP on `127.0.0.1:<port>` (0: any free port), path `/mcp`. A request without a known `mcp-session-id` gets a new transport and a new tracked server (`initialize` creates the session; anything else is rejected by the transport). One tracked server per MCP session. A 2026-07-28 request (it carries the per-request `_meta` envelope; the protocol has no sessions) gets a fresh tracked server per request (`ts-v2.mjs`: `createMcpHandler`). |
| argv `http-stateless <port>` | Streamable HTTP with no session ID. Each POST gets a fresh tracked server and transport, both closed when the response closes. GET and DELETE answer 405 with `Allow: POST` and the body `{"jsonrpc":"2.0","error":{"code":-32000,"message":"Method not allowed."},"id":null}`. A 2026-07-28 request is served as in `http` mode. |
| HTTP responses | JSON response mode (`enableJsonResponse: true`; `createMcpHandler`'s `responseMode: "json"`): every POST is answered with one JSON body, not an SSE stream. |
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

The schemas are those of `contract/tool-contract.json`: `ts-v1.mjs` declares them in zod and
generates the file; `ts-v2.mjs` and the Rust server load it (names, descriptions, input and output
schemas). Arguments are validated as zod objects validate them: a key the schema does not declare is
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

### TS v1 and v2 differences

On the 2025 lane, `ts-v1.mjs` and `ts-v2.mjs` publish the same events on each transport (observed
with `basic.json`). Their transcripts differ where the host MCP SDK differs:

- `tools/list`: v1 adds `execution: {"taskSupport": "forbidden"}` to each tool; v2 does not.
- Untracked: a call to an unknown tool answers an `isError` result
  (`MCP error -32602: Tool <name> not found`) on v1 and a JSON-RPC error `-32602` on v2.
- Invalid arguments: both answer an `isError` result, with the validator's own text (v1: zod, with
  an `MCP error -32602: ` prefix; v2: ajv).

On the 2026-07-28 lane, the transcript and events also differ from the 2025 lane by protocol: no
`initialize` request (JSON-RPC IDs start one lower), every result carries
`_meta["io.modelcontextprotocol/serverInfo"]`, `tools/list` adds `ttlMs` and `cacheScope`, the
`mcp-protocol-version` header is `2026-07-28`, and the TS SDK adds the event tag
`agentcat_protocol_version: "2026-07-28"`.

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
  `http-stateless`). The driver takes `<transport>@<lane>`, then `<transport>`.
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
divergence in the design decision log. The list is empty until the Rust half produces differences.
