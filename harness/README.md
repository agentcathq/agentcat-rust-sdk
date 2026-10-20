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
- After the last step, the driver waits until the mock holds at least `expectPublishes` requests and
  none arrived for `quietMs`, or until `settleTimeoutMs` passes.
- Exit 0 when the publish count equals `expectPublishes`, 1 when it differs or the run fails, 2 on
  bad arguments or a bad scenario.

### diff.mjs

`node harness/src/diff.mjs <a.normalized.json> <b.normalized.json> [--rulings <file>]`

Object keys compare unordered; arrays compare in order. Each ordered-check violation in either
document is a diff entry, even when both documents share it. Output lines are `DIFF <path>: a != b`,
`RULED <RD-xx> <path>: a != b` and `UNUSED RULING <RD-xx> ...`, then a JSON summary line. Exit 1 on
any difference no ruling matches and on any ruling that matches nothing; exit 2 on bad arguments or a
malformed rulings file.

### dump-contract.mjs

Runs `ts-v1.mjs` untracked over stdio and writes its full `tools/list` result to
`contract/tool-contract.json`. Never edit that file by hand. `--check` regenerates it into a temp
directory and exits 1 on any byte difference.

## Server protocol

Every reference server (TS now, Rust in plan 3) follows it.

| Item | Meaning |
|---|---|
| argv `stdio` | MCP over stdio. stdout carries JSON-RPC only. |
| argv `http <port>` | Stateful streamable HTTP on `127.0.0.1:<port>` (0: any free port), one tracked server per MCP session. |
| argv `http-stateless <port>` | Streamable HTTP with no session ID: a fresh tracked server and transport per POST; GET and DELETE answer 405. |
| ready line | HTTP modes print one line on stdout and nothing else: `{"ready": true, "url": "http://127.0.0.1:<port>/mcp"}`. |
| `AGENTCAT_API_URL` | Mock API base URL. Required; must be local. |
| `HARNESS_PROJECT_ID` | Project ID passed to `track`. Required unless untracked. |
| `HARNESS_OPTIONS` | The options profile (JSON, below). Default `{}`. |
| `HARNESS_NO_TRACK=1` | Serve the same tools untracked. |
| `HOME` | A harness temp dir. |

Exit 2 on bad arguments, a non-local `AGENTCAT_API_URL` or an invalid options profile.

The tools are those of `contract/tool-contract.json`: `echo`, `structured`, `boom`, `tool_error`,
`big` and `emit_custom` (which publishes a custom event).

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

`{"transcript": [...], "events": [...], "violations": [...]}`

- `transcript`: one entry per session start (`op: "initialize"`, with the negotiated
  `protocolVersion`) and per step (`op`, `request`, then `result` or `error`).
- `events`: one entry per publish request, `{"mock_status": <status>, "body": <normalized body>}`.
- `violations`: failed ordered checks, `{"rule", "path", ...}`.

The normalizer rules, each listed in `report.rules` of the run report:

| Rule | What it does |
|---|---|
| `camelCase-alias-drop` | Drops a camelCase body key when its snake_case twin exists (the TS generated client sends both). Dropped names go to `report.camelCaseAliasKeys`. |
| `timestamp-class` | `timestamp` becomes `<iso8601-ms-Z>`, or `<bad-timestamp:...>`. |
| `duration-class` | `duration` becomes `<duration-ms>` (non-negative integer), or `<bad-duration:...>`. |
| `language-fields` | `sdk_language` becomes `<sdk_language:<type>>`; `agentcat_version` becomes `<semver>`. |
| `extra-headers-only` | `parameters.extra` (the host SDK's request context) becomes `{"<headers>": {...}}` with only `user-agent`, `x-anthropic-client`, `x-test-header` and `mcp-protocol-version`. |
| `error-stack-elided` | `error.stack`, `error.frames` and `error.platform` become `<stack>`, `<frames>`, `<platform>`. |
| `id-placeholders` | Every `evt_`, `ses_` and `agt_` KSUID becomes `<prefix#n>` by first appearance, the same across transcript and events. |
| `mcp-session-first` | Every `structuredContent` that has `mcp_session` must have it as its first key; a violation goes to `violations`. |
| `property-order` | For each tool in a `tools/list` result, adds `inputSchema["<properties-order>"]`: the key order of `inputSchema.properties`. |
| `literal-derived-ids` | When the profile's `resolveSessionId` returns a non-blank string, the ID `deriveSessionId(value.trim(), projectId)` (computed with the TS `handles` module) stays literal instead of becoming a placeholder. IDs in `report.literalIds`. |

Events are ordered by timestamp, then resource name, then the body with its IDs masked: the TS queue
publishes events concurrently, so arrival order is noise.

The ordered checks read key order as JavaScript keeps it, so they do not apply to integer-like keys
(JavaScript puts those first).

## Rulings

`rulings.json` lists each allowed difference:

```json
[{ "path": "<regex over diff paths>", "reason": "why the difference is allowed", "deviation": "RD-xx" }]
```

`deviation` names the ruled divergence in the design decision log. A diff path looks like
`$.events[3].body.resource_name` or `$.transcript[1].result.tools[0].inputSchema.<properties-order>`;
a violation's path ends with `.<rule>`. The list is empty until the Rust half produces differences.
