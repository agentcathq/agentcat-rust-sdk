# harness

The differential harness. One scripted MCP client (the driver) runs a scenario against a reference
server, a local mock API records every event the server publishes, a normalizer turns the run into a
language-neutral document, and a diff compares two documents. Allowed differences are listed in
`rulings.json` (MCP-wire delta rulings, Rust against TS), `rulings-events.json` (event rulings, Rust
against TS), and their TS-against-TS twins `rulings-ts.json` and `rulings-events-ts.json`.

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
- The driver gives every server `DISABLE_DIAGNOSTICS=true` and a temp `HOME` (both SDKs write
  `~/agentcat.log` there). The TS servers also pass `disableDiagnostics: true`.
- Untracked (`HARNESS_NO_TRACK=1`), the TS servers never publish: `emit_custom` answers without
  calling `publishCustomEvent`.
- The Rust reference server runs no egress guard. It refuses to start the same way (exit 2,
  `refusing to start`), with a stricter check of `AGENTCAT_API_URL`, and passes that URL to the SDK
  as its `api_base_url` option, so the SDK's delivery publishes to the mock only. Untracked, it
  publishes nothing.

## Commands

```sh
# One run: writes harness/out/<scenario>.<label>.<transport>.{raw,transcript,normalized,report}.json
node harness/src/driver.mjs harness/scenarios/basic.json ts stdio \
  -- node --import ./harness/src/egress-guard.mjs harness/src/servers/ts-v1.mjs

# The Rust reference server (build it first)
cargo build -p harness-ref-server
node harness/src/driver.mjs harness/scenarios/basic.json rust stdio \
  -- target/debug/harness-ref-server

# The 2026-07-28 lane against the TS v2 server
node harness/src/driver.mjs harness/scenarios/basic.json ts2 http --lane 2026-07-28 \
  -- node --import ./harness/src/egress-guard.mjs harness/src/servers/ts-v2.mjs

# Compare two runs: here two TS runs, the second with the label ts-b (a TS-vs-Rust document diff also
# compares transcripts, whose host differences only the matrix's delta comparison leaves out)
node harness/src/diff.mjs harness/out/basic.ts.stdio.normalized.json \
  harness/out/basic.ts-b.stdio.normalized.json --rulings harness/rulings-ts.json

# The MCP-wire delta and the published events of TS v2 against Rust on every scenario, transport and
# lane (the L3 gate's first run)
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

### delta.mjs and matrix.mjs (the MCP-wire delta and the event comparison)

The MCP-wire delta is what tracking changes on the wire: the ops (`add`, `remove`, `change`) that turn
an untracked run's transcript into the tracked run's, for one server. `delta.mjs` compares the delta of
TS with the delta of Rust. Owner ruling R20: TS v2 is the answer key.

`node harness/src/delta.mjs <ts-untracked> <ts-tracked> <rust-untracked> <rust-tracked> [--rulings <file>]`
takes four `*.normalized.json` files. Paths look like `$.delta.transcript[4].result.content[0]`; a
tracked-document violation is an `add` at `$.delta.violations[<rule>]@<violation path>`. Output and exit
codes follow `diff.mjs` (`DELTA <path>: ts <x> != rust <y>`, `RULED ...`, ruling lines, a JSON summary).

`node harness/src/matrix.mjs [--servers <a>,<b>] [--scenarios <names>] [--transports <list>] [--lanes <list>] [--jobs <n>] [--rulings <file>] [--event-rulings <file>] [--env2 <NAME=value>]...`
runs, per scenario, transport and lane, both servers untracked (`HARNESS_NO_TRACK=1`) and tracked, then
compares the two deltas and the published events (decision C46). Servers: `ts-v1`, `ts-v2` (default
first) and `rust` (default second; `HARNESS_RUST_SERVER`, else `target/debug/harness-ref-server`). Labels
are `<server>-<lane>-<u|t>`; two equal server names (an A/A run) give the second server the suffix `-b`.
A tracked Rust run gets `--expect-drained` (only the tracked Rust server prints the drained line).
`--env2` sets an environment variable for every run of the second server, untracked and tracked
(repeatable; `matrix.json` records it). It writes `harness/out/matrix.json`.

The event comparison (`src/events.mjs`, the structural diff that `diff.mjs` also uses) compares the
`events` and `nonPublishRequests` of the two tracked normalized documents. Object keys compare unordered,
so a different key order never shows; arrays compare in order. Paths look like
`$.events[3].body.parameters.extra.<keys>` and `$.nonPublishRequests[0]`.

Each combination prints one line,
`ok|FAIL <scenario> <transport> <lane>: delta {"total","ruled","unexpected"} events <total>/<ruled>/<unexpected>`,
and on a FAIL one indented line per unruled difference: `DELTA <path>: ts <x> != rust <y>` or
`EVENT <path>: <first server> != <second server>`. Each entry of `matrix.json` holds `delta` and `events`
(the counts), `differences` (the unruled delta entries) and `eventDifferences` (the unruled event entries,
`{path, a, b, ruledBy}`). After the lines come the ruling lines of the delta rulings and of the event
rulings (these start with `EVENT `).

Rulings: `rulings.json` holds the delta rulings for Rust against TS (`delta.mjs`, `--rulings`), and
`rulings-events.json` the event rulings (`--event-rulings`). When both servers are TS, the defaults are
`rulings-ts.json` and `rulings-events-ts.json` (A/A runs and the nightly drift sweep, so a Rust ruling never
counts as unused there). An A/A run (two equal server names) has no default rulings at all: they are about
TS against Rust, so it prints no `UNUSED RULING` line; a file named with `--rulings` or `--event-rulings`
still applies. Both kinds take the same scope keys (`scenario`, `transports`, `lanes`) and the
same checks; the whole matrix fails on an unused ruling of either kind. An event ruling can also pin the
values it allows (`ts`, `rust`, `rustKeys`, see Rulings); a delta ruling cannot, and a delta rulings file
with one of those keys exits 2.

### The L3 gate

`scripts/check.sh` and the CI job `harness-l3` build the Rust reference server, then run five matrices:

1. `node harness/src/matrix.mjs`: TS v2 against Rust, every scenario, transport and lane. It must exit 0:
   every driver passes (each tracked Rust server drains), every delta and event difference has a ruling,
   and every ruling matches something.
2. The A/A run `node harness/src/matrix.mjs --servers rust,rust --scenarios basic`: it must exit 0 with no
   delta or event difference at all, ruled or not. `node harness/src/l3-expect.mjs none` checks that
   `delta.total` and `events.total` are 0 in every combination of `matrix.json`.
3. The canary run, the A/A run plus `--env2 HARNESS_CANARY=1`: the second server serves the context
   description `"canary"`. It must exit 1. `node harness/src/l3-expect.mjs caught` checks that every
   driver passed, that every combination reports an unruled delta difference, each under
   `$.delta.transcript[1].result.tools[` (basic.json's tracked listing), and that no combination reports
   an event difference (the canary does not change what the servers publish). A canary that passes means
   the delta comparison has gone blind; one caught elsewhere is a different failure.
4. The call canary run, the A/A run plus `--env2 HARNESS_CANARY=call`: the second server wraps its tracked
   handler in an outer handler that changes `[session_id issued` to `[session_id lssued` at the start of a
   result's first text block, after the SDK recorded the call. It must exit 1.
   `node harness/src/l3-expect.mjs caught-call` checks that every driver passed, that every combination
   reports an unruled delta difference, each at `$.delta.transcript[<n>].result`, its `.content[0]` or
   `.content[0].text` (a call whose untracked answer was an error shows the whole tracked result as
   added), each showing the canary's change alone (the Rust text is the TS text with `issued` made
   `lssued`), and that no combination reports an event difference.
5. The event canary run, the A/A run plus `--env2 HARNESS_CANARY=event`: a `redact_event` hook on the
   second server sets the resource name of every event it sees to `canary`. It must exit 1.
   `node harness/src/l3-expect.mjs caught-event` checks that every driver passed, that no combination
   reports a delta difference (the hook runs after the call), and that every combination reports an
   unruled event difference, each at `$.events[<n>].body.resource_name` with the value `canary` on the
   second server. A canary that passes means the event comparison has gone blind.

`l3-expect.mjs <none|caught|caught-call|caught-event> [matrix.json]` reads `harness/out/matrix.json` by
default; exit 0 as expected, 1 not (one line per bad combination; a combination without `delta` or
`events` is bad), 2 on bad arguments.

One full matrix (13 scenarios, 4 transports, 2 lanes: 104 combinations, 416 driver runs) takes a few
minutes at `--jobs 4` on the development machine; the A/A and canary runs about 12 s each.

### driver.mjs

`node harness/src/driver.mjs <scenario.json> <label> <stdio|http|http-sse|http-stateless> [--lane 2025-11-25|2026-07-28] [--expect-publishes <n>] [--expect-drained] -- <server cmd...>`

- `label` tags the output files (letters, digits, `_`, `-`). The lane is not in the file name: give
  runs on different lanes different labels.
- The driver starts the mock API, then the server as `<server cmd...> stdio` (once per scenario
  session), `<server cmd...> http 0`, `<server cmd...> http-sse 0` or `<server cmd...> http-stateless 0`.
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
- `--expect-drained`: every server process (stdio: the server of each session; HTTP: the one server)
  must print the line `{"drained":true}` on stderr before it exits (the tracked Rust server's drain,
  "Server protocol" below). A server that exits without it fails the run: exit 1 with
  `driver: the server of session <s> exited without printing {"drained":true}` (HTTP: `the server`).
- The driver pipes each server's stderr through its own stderr, line by line, and reads it for the
  drained line (stdio: the client transport's `stderr: "pipe"`; HTTP: the child's stderr).
- Before each connect and each step, the driver waits until the wall clock is past the millisecond in
  which the previous request was answered. A server stamps an event between a request's arrival and its
  answer, so the events of two requests in a row never share a millisecond, and the time order of the
  normalizer is the request order on every server and in every run. A `parallel` step waits once, as a
  whole: its calls start together, so their events can share a millisecond (use `"eventOrder":
  "multiset"`).
- A `listTools` step lists one page: with `cursor`, that page; without, the first. On the 2026-07-28 lane
  in a scenario with `pageSize`, the driver sends a cursor-less `tools/list` itself, because the v2
  client's `listTools()` without a cursor follows every `nextCursor` and returns all pages as one.
- The server gets the parent environment, without `AGENTCAT_ENABLED` (the Rust SDK reads it, decision
  C45: a developer shell's `AGENTCAT_ENABLED=false` would turn the tracked Rust server off), plus
  `AGENTCAT_API_URL` (the mock), `HARNESS_PROJECT_ID` (the scenario's `projectId`), `HARNESS_OPTIONS`
  (the scenario's `options` as JSON), `DISABLE_DIAGNOSTICS=true`, and `HOME` and `USERPROFILE` set to
  `harness/out/home/<scenario>.<label>.<transport>/` (recreated per run). `HARNESS_NO_TRACK` passes
  through from the caller's environment. `HARNESS_PAGE_SIZE` is the scenario's `pageSize`; without one,
  the driver removes a `HARNESS_PAGE_SIZE` of the caller's environment.
- In a tracked run (`HARNESS_NO_TRACK` unset), a `saveHandleAs` step whose result carries no handle,
  and a `${NAME}` with no saved value, exit 1 with
  `driver: step <i> of session <s>: no session handle for <NAME>`. An untracked run issues no handles
  and keeps the `${NAME}` text literally. A `saveCursorAs` step whose result has no `nextCursor` exits 1
  in every run: `driver: step <i> of session <s>: no nextCursor for <NAME>`. A `${NAME}` in a `cursor`
  that no `saveCursorAs` saved exits 1 in every run:
  `driver: step <i> of session <s>: the cursor names <NAME>, but no saveCursorAs saved it`.
- The `initialize` transcript entry of each session holds
  `{protocolVersion, serverInfo, capabilities, instructions}` as the client reports them, on both
  lanes (`null` for a part the server did not send).
- On a driver error every connected client is closed before the driver exits, so a stdio server the
  client spawned ends too.
- After the last step the driver:
  1. waits until the mock holds at least `expectPublishes` publish requests (POST
     `/v1/public/events/`) and no request of any kind arrived for `quietMs`, or until
     `settleTimeoutMs` passes;
  2. closes the clients and waits, up to `settleTimeoutMs`, for them to close (a client close that
     takes longer fails the run: `the client close did not finish within <n> ms`), then gives an HTTP
     server SIGTERM and waits up to `settleTimeoutMs` more (`the server did not exit within <n> ms of
     SIGTERM; killing it`, then SIGKILL). A stdio server must exit when its stdin closes (the stdio
     client transport sends SIGTERM after 2 s, then SIGKILL; the driver reads the exit signal of the
     process, `needed SIGKILL`). A server that needs the SIGKILL, over either transport, fails the
     run (exit 1, with or without `--expect-drained`);
  3. keeps the mock open until no request arrived for `quietMs` (at most `settleTimeoutMs`), then
     takes the snapshot. A publish sent on client close or during a shutdown drain is counted.
- It then validates every publish body against the publish contract (`contract/`, below). In
  `report.json`, `contractChecked` is the number of bodies checked, and `contract` is `"ok"`,
  `"no publish bodies"` (nothing to check although the scenario expects publishes), or one entry
  per invalid body (`{seq, id, event_type, resource_name, errors: [{instancePath, keyword, message, params}]}`).
  A run with no publish body passes the contract only when the scenario expects 0 publishes.
  `ts_sha` in `report.json` is the TS commit the run used: `oracle/oracle.lock`, or `ORACLE_SHA`
  when it is set (the driver then writes a trace line to stderr). `drained` lists, per server
  process, whether it printed `{"drained":true}`; `killed` says whether a server needed SIGKILL.
- Exit 0 when the snapshot holds exactly `expectPublishes` publish requests and no other request,
  the contract is `"ok"`, no server needed SIGKILL and, with `--expect-drained`, every server printed
  the drained line. Exit 1 when the count differs, when any other request reached the mock (listed in
  the report as `nonPublishRequests`), when the contract fails (each error on stderr), when a server
  needed SIGKILL or did not drain, or when the run fails. Exit 2 on bad arguments or a bad scenario.

### diff.mjs

`node harness/src/diff.mjs <a.normalized.json> <b.normalized.json> [--rulings <file>]`

The structural diff is the one of `src/events.mjs`. Object keys compare unordered; arrays compare in
order, and arrays of different length give one entry per element (`<missing>` on the shorter side).
Each ordered-check violation in either document is a diff entry, even when both documents share it.
Output lines:

- `DIFF  <path>: <a> != <b>` for a difference no ruling matches;
- `RULED <RD-xx> <path>: <a> != <b>` for a difference a ruling matches (its path, and its pinned values
  when it has them, see Rulings);
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

## Examples check

The examples check runs the customer examples of `agentcat-sdk` (`agentcat-sdk/examples/`) against the local
mock API. It protects a person who copies an example: the server works, and nothing breaks the stdio
channel.

```sh
cargo build -p agentcat-sdk --examples --all-features
node harness/src/examples-check.mjs stdio
node harness/src/examples-check.mjs streamable_http
node harness/src/examples-check.mjs hooks
```

The check accepts `stdio`, `streamable_http`, `stateless_http` and `hooks`. For one example it:

- starts the mock API and builds the example;
- runs the example with `AGENTCAT_API_URL` set to the mock, `AGENTCAT_PROJECT_ID=proj_examples` and a
  temporary `HOME` and `USERPROFILE`, so the SDK does not write the real `~/agentcat.log`. The check
  refuses to start an example when the API URL is empty. The egress guard does not apply to Rust
  examples, so the API URL is the only barrier: the check always sets it to the mock;
- drives `initialize`, `tools/list` and three tool calls with the TS client, then closes the client right
  after the last call. For HTTP examples it then sends SIGTERM;
- asserts that each call published one event, that every stdout line is a JSON-RPC message, that the
  example exits 0 within 5 s of the stdin close or the SIGTERM, and that the publish of the last call
  arrived before the exit.

For `hooks` it also asserts the values of the hooks (`demo-user`, the `example` tag, the redacted
`secret-token`), the error recorded by `capture_error`, and one custom event (`echo-called`).
`EXAMPLES_CHECK_VERBOSE=1` prints the time of the exit. `test/examples-check.test.mjs` tests the check
against a fake example (`test/fixtures/example-fake.mjs`).

## Demo

`harness/demo/run.mjs` runs one fixed scenario (`harness/demo/scenario.json`) through the `hooks` example (Rust)
and through `harness/demo/ts-server.mjs` (the same tools and options on the TS SDK at the pinned commit), over
stdio with the TS client. The scenario has two sessions: `tools/list`, `add` with intent text, two `echo` calls
(one holds `secret-token`) and `fail` in the first, and one `echo` in the second. The `echo` tool publishes one
custom event per call, so the dashboard shows three custom events.

```sh
node harness/demo/run.mjs --mock
```

`--mock` sends to the local mock API. It is part of the gate. The TS server runs under the egress guard
(`node --import`). The Rust child is protected by the mock URL alone: a node `--import` guard cannot cover a
Rust process, so run.mjs refuses to start it without an `AGENTCAT_API_URL` that points at the mock. It prints one
JSON line with the counts, the number of events compared and the number of tools. It exits 1 unless all of these hold:

- Each server answers every scenario call as the scenario expects, and exits 0 within 10 s of the stdin close.
- Every stdout line of each server is JSON-RPC.
- The mock receives publishes from each server, and no other request.
- `tools/list` of the two servers has the same tools, with the same descriptions and input schemas (key order
  ignored).
- The two servers publish the same events. Events are paired after sorting, and these fields must match: event
  type, tool or resource name, user ID and name, tags, `properties.source`, `properties.redacted_by`, `isError`,
  the error type and message, and the session (as the number of the session the client saw).
- No publish holds `secret-token`, and exactly as many publishes hold `[REDACTED]` as the scenario has calls that
  send the secret.

Two differences are known. RD-39 (owner ruling R29: the Rust event keeps its session): the first call of a session
cannot give the TS custom event its session, so the TS custom event of the `echo` in the second session has no
session and the Rust one has it. The comparison expects exactly this, once: it fails if the difference disappears or
moves. The dashboard therefore shows the TS custom event in an anonymous session of its own. The second difference
is not an RD: the error of `fail`. The Rust SDK records `platform: "rust"` and the type `core::io::error::Error`;
the TS SDK records `platform: "javascript"`, a stack and frames. The platform, stack and frames are the SDK's own
and the comparison leaves them out. The type is only the example's choice: `ts-server.mjs` sets `error.name` to the
Rust type text, because the TS SDK records `error.name` as the type.

`--prod --project <id>` is for maintainers only. It sends to the default AgentCat API, for the project
"Testing" only, with no egress guard. Two independent guards refuse it, both before any server starts:

- `parseArgs` refuses any other project, and it refuses when `CI` is set.
- `confirmProd` refuses a server environment with no loopback `AGENTCAT_API_URL` (one that publishes to
  production) unless `AGENTCAT_DEMO_PROD=yes` is in the environment. It runs before the first process of the run
  and again right before each server process.

A maintainer runs it so:

```sh
AGENTCAT_DEMO_PROD=yes node harness/demo/run.mjs --prod --project <the Testing project ID>
```

Never run it from CI, and never set `AGENTCAT_DEMO_PROD` in a test: the tests call `confirmProd` as a pure
function, and `drive` with a command that is not a server. It writes the session IDs and the run time to
`harness/out/demo/sessions.json`.

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
| argv `http-sse <port>` | As `http`, with SSE responses (C59): TS `enableJsonResponse: false` (`ts-v2.mjs` also `createMcpHandler`'s `responseMode: "sse"`), Rust `with_json_response(false)`, rmcp's default. The default most customers keep. It runs every scenario that runs on `http`, on both lanes. |
| argv `http-stateless <port>` | Streamable HTTP with no session ID. Each POST gets a fresh tracked server and transport, both closed when the response closes. GET and DELETE answer 405 with `Allow: POST` and the body `{"jsonrpc":"2.0","error":{"code":-32000,"message":"Method not allowed."},"id":null}`. A 2026-07-28 request is served as in `http` mode. |
| HTTP responses | `http` and `http-stateless` use JSON response mode (`enableJsonResponse: true`; `createMcpHandler`'s `responseMode: "json"`): every POST is answered with one JSON body. `http-sse` uses SSE (`enableJsonResponse: false`; `responseMode: "sse"`): every POST is answered with an SSE stream. Rust answers 2026-07-28 requests as the mode says (`http`, `http-stateless`: JSON; `http-sse`: SSE), as `test/sse.test.mjs` proves. Rust answers a 2025-era POST in `http` and `http-sse` with rmcp's SSE stream (a priming event, then one event per JSON-RPC message), because rmcp serves JSON for a 2025-era request only without session mode (`http-stateless`). |
| ready line | HTTP modes print one line on stdout and nothing else: `{"ready": true, "url": "http://127.0.0.1:<port>/mcp"}`. |
| SIGTERM | An HTTP server exits on SIGTERM (the TS SDK drains its queue first); one that needs SIGKILL fails the driver run. The tracked Rust server also drains on SIGTERM in every mode and on stdin EOF (stdio): it runs `shutdown(5 s)` and prints `{"drained":true}` or `{"drained":false}` on stderr before it exits (the matrix runs it with `--expect-drained`). An untracked server prints nothing. |
| `AGENTCAT_API_URL` | Mock API base URL. Required; must be `http://` on `127.0.0.1`, `localhost` or `[::1]`, else exit 2 with `refusing to start` on stderr. The TS server passes it to `track` as `apiBaseUrl`. |
| `HARNESS_PROJECT_ID` | Project ID passed to `track` and to custom events. Required unless untracked. |
| `HARNESS_OPTIONS` | The options profile (JSON, below). Default `{}`. |
| `HARNESS_NO_TRACK=1` | Serve the same tools untracked: no `track`, and `emit_custom` and `emit_custom_server` publish nothing. |
| `HARNESS_PAGE_SIZE` | A positive decimal integer without a leading zero (else exit 2): `tools/list` answers in pages of that many tools. The cursor is the decimal index of the page's first tool; `nextCursor` is the index after the page while tools remain. A cursor that is not such an index below the tool count answers the JSON-RPC error `-32602` `invalid cursor <cursor as JSON>` (ts-v1's host adds its `MCP error -32602: ` prefix). Unset or empty: one page of every tool. The driver sets it from the scenario's `pageSize`. The TS servers page by replacing the `tools/list` entry of the low-level server's request-handler map before `track` (`pageToolsList` in `common.mjs`, test-server code only), so the TS SDK wraps the paging handler. |
| `HOME`, `USERPROFILE` | A harness temp dir (the TS SDK writes `~/agentcat.log`). |
| `DISABLE_DIAGNOSTICS=true` | Set by the driver. The TS server also always passes `disableDiagnostics: true` to `track`, whatever the profile says. |

Exit 2 on bad arguments, a non-local `AGENTCAT_API_URL`, a bad `HARNESS_PAGE_SIZE` or an invalid
options profile. Logs go to stderr only.

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
| `emit_custom_server` | `{resource: string, message?: string, session?: string, tags?: object}` | Tracked: first awaits the server form `publishCustomEvent(<the tracked server>, projectId, {resourceName: resource, message, sessionId: session, parameters: {from: "emit_custom_server"}, tags})` (K-03: the tracked project, the `enableTracing` gate, the `redactEvent` hook and no string redaction; without `session` the event has no session). Untracked: no publish. Then returns `content: [{type: "text", text: "published"}]`. |

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
  (`AgentCat::track`) for the connection. `http` and `http-sse` serve rmcp's `StreamableHttpService` with a
  `LocalSessionManager` and `legacy_session_mode(true)`, `http-stateless` with
  `legacy_session_mode(false)`; `http` and `http-stateless` use `json_response(true)`, `http-sse` uses
  `json_response(false)`. rmcp honors `json_response` for a 2025-era request only
  where it serves it without a session (`http-stateless`), and for every 2026-07-28 request: JSON in
  `http` and `http-stateless`, SSE in `http-sse`. Each MCP session or request gets a fresh tracked server from one shared `AgentCat`
  handle. rmcp serves 2026-07-28 requests and `server/discover` itself in every mode (over HTTP, a
  fresh server per 2026-07-28 request).
- `src/tools.rs`: the tools. Names, descriptions and schemas come only from
  `contract/tool-contract.json`, embedded with `include_str!`; the behaviors follow `toolHandlers`
  in `common.mjs`. rmcp's `ToolRouter` does the dispatch, `get_tool`, the argument errors and the
  conversion of a tool's `Err` into an `isError` result. `tools/list` keeps the contract's order
  (rmcp's router sorts by name) and, on 2026-07-28, adds `ttlMs: 0` and `cacheScope: "private"` as
  ts-v2 does. With `HARNESS_PAGE_SIZE` it serves the contract's tools in pages, as the TS servers do.
  `boom` records its error with `capture_error`. `emit_custom` publishes its custom event with
  `AgentCat::publish_custom_event` and `emit_custom_server` with `CallContext::publish_custom_event`
  (the server form, C16; the `CallContext` comes from the request's extensions), both only when tracked,
  and answer `published`, as the TS servers do.
- `src/profile.rs`: the options profile and `HARNESS_CANARY`.
- `src/canary.rs`: the outer handler of the call canary (`HARNESS_CANARY=call`).

Where it differs from the TS servers:

- `http` mode answers each 2025-era POST with an SSE stream, not one JSON body (`http-sse` does so on both servers): a priming event
  (`data:` empty, `id`, `retry: 3000`), then one event per JSON-RPC message. In rmcp 3.7.0,
  `json_response` takes effect for a 2025-era request only when `legacy_session_mode` is false; the
  session path always answers with an SSE stream, so Rust `http` differs from TS `http` (JSON) on
  the 2025-11-25 lane. The driver's clients read both. `http-stateless` answers JSON and 2026-07-28
  requests follow the mode (`http` JSON, `http-sse` SSE), as the TS servers do.
- `AGENTCAT_API_URL`: the check is stricter than the WHATWG parse of `common.mjs`: no user info,
  backslash, whitespace or control character; the host spelled `127.0.0.1`, `localhost` (any case)
  or `[::1]`; a port, when given, of 1 to 5 digits below 65536. The value is checked and goes in as
  the SDK's `api_base_url` option, so a tracked Rust server publishes to the mock like the TS
  servers; the matrix expects the scenario's `expectPublishes` from it.
- The profile: a flag must be a boolean (`customContextDescription` a string), where `common.mjs`
  copies any value. A `returns` value must fit the hook's SDK type: `resolveSessionId` a string or
  `null`; `identify` `null` or `{"userId": string, "userName"?: string, "userData"?: object}`;
  `eventTags` and `eventProperties` an object or `null`; `redactSensitiveInformation` a string.
  A `redactEvent` merge field must fit its `Event` field: `resourceName`, `sessionId`, `eventType` and
  `userIntent` a string, `properties` an object (`common.mjs` checks only the field names).
  Anything else exits 2. `redactSensitiveInformation` becomes the SDK's string redaction hook
  (`redact_sensitive_information`), `redactEvent` its event redaction hook (`redact_event`). Every
  hook is async; `sleepMs` sleeps on the tokio timer.
- `HARNESS_CANARY` perturbs the server for an L3 canary run ("The L3 gate" above): `1` sets the context
  description to `"canary"`, whatever the profile says; `call` wraps each tracked server in an outer
  handler that changes `[session_id issued` to `[session_id lssued` at the start of a result's first
  text block; `event` sets a `redact_event` hook, in place of the profile's, that sets every event's
  resource name to `canary`. Unset or empty: no perturbation. Any other value exits 2. The TS servers
  do not read it.
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
`redactSensitiveInformation`, `redactEvent`. A behavior is one of:

| Behavior | The hook... |
|---|---|
| `{"returns": <value or null>}` | returns a fresh copy of the value. |
| `{"throws": "<message>"}` | throws `new Error(message)` (rejects, for a hook that returns a Promise). |
| `{"replace": "<secret>"}` | `redactSensitiveInformation` only: replaces every occurrence of the secret with `[REDACTED]`. |
| `{"merge": {<field>: <value>, ...}}` | `redactEvent` only: returns `{...event, ...merge}` (Rust sets the matching `Event` fields). A field is one of `resourceName`, `sessionId`, `eventType`, `userIntent`, `properties`; another key exits 2. The SDKs restore the event's ID, session, project, type and timestamp after the hook, so a merged `sessionId` or `eventType` does not reach the wire. |

The value each hook's `returns` takes (as in the TS SDK's `src/types.ts`):

| Hook | Arguments it receives (ignored by the canned behavior) | `returns` value |
|---|---|---|
| `resolveSessionId` | the MCP request, the request extra | a string or `null` |
| `identify` | the MCP request, the request extra | `{"userId": string, "userName"?: string, "userData"?: object}` or `null` |
| `eventTags` | the MCP request, the request extra | an object of string values, or `null` |
| `eventProperties` | the MCP request, the request extra | an object of any JSON values, or `null` |
| `redactSensitiveInformation` | one text string | a string (used for every text) |
| `redactEvent` | the whole event | `null` only (drops the event); use `merge` to change it |

Any behavior can add `"sleepMs": <n>`: the hook waits n milliseconds, then answers. The TS server
turns each behavior into a function with the hook's signature from `src/types.ts` at the pinned
commit: `resolveSessionId`, `eventTags`, `eventProperties` and `redactEvent` answer synchronously
unless `sleepMs` is given; `identify` and `redactSensitiveInformation` always return a Promise.

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
  `http-sse`, `http-stateless`). The driver takes `<transport>@<lane>`, then `<transport>`. Any other key exits 2.
- The driver exits 2 on a key it does not know at the top level of the scenario, in a session
  (`client`, `steps`) or in a step (`listTools`: `op`, `cursor`, `saveCursorAs`; `callTool`: `op`,
  `name`, `args`, `saveHandleAs`; `parallel`: `op`, `steps`; a step inside `parallel`: the `callTool`
  keys and `session`).
- `httpHeaders` go on every HTTP request of the client.
- Each session is a new client connection. `saveHandleAs` stores the `ses_` handle a result issued
  (from `structuredContent.mcp_session.session_id` or a `session_id: ses_...` text line); `${NAME}`
  in later arguments is replaced with it.
- `listTools` takes an optional `cursor` (a string; `${NAME}` reads a cursor that `saveCursorAs` saved) and an optional `saveCursorAs`, which
  stores the result's `nextCursor` under that name (no `nextCursor`: exit 1, in every run).
- `pageSize` (optional, a positive integer) starts the servers with `HARNESS_PAGE_SIZE`, so `tools/list`
  answers in pages ("Server protocol" above).
- `{"op": "parallel", "steps": [<callTool steps>]}` runs its calls at once (`Promise.all`) and records
  their transcript entries in step order. A step inside it may name `"session": <index>`, this session or
  an earlier one, to run on that session's client (default: this session's). The `saveHandleAs` of its
  steps take effect after all of them answered.
- `eventOrder` (optional) takes `"multiset"` only (any other value exits 2): the normalizer then sorts
  the events on their normalized body alone, so their order is not compared. Without it they sort by
  time ("Normalized document" below).
- The driver starts each request in a later millisecond than the previous answer ("driver.mjs" above),
  so the events of two steps never share a millisecond. Two events of one step (a call and the custom
  events it publishes) can: the call-first step, the resource name and the body decide their order
  (below), so a step must not publish two events whose order matters unless the scenario compares
  events as a multiset.
- No number in a scenario may be a decimal with more than 15 significant digits: the Rust SDK reads
  such a decimal without `float_roundtrip`, and it can become the neighboring `f64` (RD-23).
- `expectPublishes` is the count the tracked TS v2 server publishes, read from a TS run. The tracked Rust
  server publishes the same count, and the matrix gives it no override.

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
| `redaction` | `identify` returns a user after 200 ms, `eventTags` and `eventProperties` return `SECRET_TOKEN` values, `redactSensitiveInformation` replace, `redactEvent` merge (`resourceName`, `sessionId`, `properties`) | Renamed resources and restored sessions on the call events; the late identity on every call event, the `boom` error event included, and through the `redactEvent` merge; tags keep `SECRET_TOKEN` (string redaction skips them), and the merged `properties` replace the hook's whole value (no `region` key); the session-string custom event of `emit_custom` untouched by both hooks; the server-form custom event of `emit_custom_server` renamed but not string-redacted (K-03). |
| `redact-drop` | `redactEvent` returns `null` | Every event that runs the hook is dropped: only the session-string custom event of `emit_custom` publishes. |
| `hooks-fail` | `identify` throws after 200 ms, `eventTags` returns invalid tags (a bad key, a 33-character key, a number value, a newline in a value) and one valid tag, `eventProperties` throws | Calls in two sessions, each reused by a second call: no identity, no properties, only the valid tag. |
| `hooks-fail-resolve` | the options of `hooks-fail` plus `resolveSessionId` throws | Hook mode with a failing hook: each call is its own silently minted session (no `session_id` is injected, so the steps keep no handles), no identity, no properties, only the valid tag. TS publishes 5 events on every transport and lane. |
| `concurrency` | `enableAgentTracking`, `"eventOrder": "multiset"` | A `parallel` step of 8 `echo` calls with one handle on the first session's client, at the same time as 2 calls on a second session, each call with its own `agent_id`. The actor half (a user per call) is the Rust test `fake::concurrent_calls_keep_their_own_session_agent_and_actor`: one `identify` profile cannot name a user per call. |
| `pagination` | `{}`, `pageSize: 3` | Page 1, a call to a page-2 tool before page 2 is listed (K-01 against RD-12), page 2, the call again, page 3 and `own_context` on it (RD-12 on per-request TS servers). |
| `custom-events` | `{}` | The session-string form (`emit_custom`) with and without a session, the server form (`emit_custom_server`) with and without a session, and invalid custom tags (all dropped, or one valid tag kept). |

Panics have no scenario: a TS tool cannot panic (RD-06), and the Rust tests of Tasks 6 and 9 of plan 4
cover the events of a panicking tool.

## Normalized document

`{"transcript": [...], "events": [...], "nonPublishRequests": [...], "violations": [...]}`

- `transcript`: one entry per session start (`op: "initialize"`, with the negotiated
  `protocolVersion`, on both lanes: on 2026-07-28 no `initialize` request is sent) and per step
  (`op`, `request`, then `result` or `error`).
- `events`: one entry per publish request,
  `{"mock_status": <status>, "content_type": <header>, "query": <query string>, "body": <normalized body>}`.
  The body is read from the raw text the mock recorded (`rawBody`, beside the parsed `body` in
  `raw.json`); a request without `rawBody` uses its parsed `body`.
- `nonPublishRequests`: every other request the mock received, `{"method", "path", "query", "status"}`,
  sorted.
- `violations`: failed ordered checks, `{"rule", "path", ...}`.

The normalizer rules, each listed in `report.rules` of the run report:

| Rule | What it does |
|---|---|
| `request-fields` | A publish keeps its status, `content-type` and query string; its other headers (`user-agent`, `accept`) are dropped as language-specific. Every non-publish request stays as method, path, query and status. |
| `number-source` | The body is parsed from its raw text with source-text access (`JSON.parse` reviver `context.source`, Node 22). An integer number below 1e21 in magnitude whose source text differs from `String(Number(source))` becomes `<non-js-number:<source>>`: it catches `1.0`, `-0`, `1e2` and an exact integer above 2^53, which JavaScript never writes. Every other number compares by value, so a float that serde_json writes as `1e-6` equals TS's `0.000001` (RD-32). |
| `camelCase-alias-drop` | Drops a camelCase body key when its snake_case twin exists (the TS generated client sends both). Dropped names go to `report.camelCaseAliasKeys`. |
| `timestamp-class` | `timestamp` becomes `<iso8601-Z>` when it is ISO 8601 with three millisecond digits or whole seconds (chrono drops a zero fraction, RD-04) and `Z`, else `<bad-timestamp:...>`. |
| `duration-class` | `duration` becomes `<duration-ms>` (non-negative integer), or `<bad-duration:...>`. |
| `language-fields` | `sdk_language` becomes `<sdk_language>` when it is `"TypeScript"` or `"Rust"`, else `<bad-sdk_language:<value>>`; `agentcat_version` becomes `<semver>`, or `<bad-version:...>`. |
| `extra-projection` | `parameters.extra` (the host SDK's request context) becomes `{"<keys>": "<its sorted top-level keys, comma-joined>", "<headers>": {...}, "<url>": "..."}`. `<headers>` holds every header of every `headers` object anywhere under `extra`, by lower-case name, with the values of `host`, `content-length` and `mcp-session-id` as `<host>`, `<content-length>` and `<mcp-session-id>`. `<url>` is `requestInfo.url` (Rust, TS v1) or `http.req.url` (TS v2) with its port as `<port>`. A part `extra` does not have is left out (stdio has no headers and no URL). The keys differ by host (RD-07); one string gives one difference per event. |
| `error-shape-class` | In `error` and each `error.chained_errors` entry: `stack` becomes `<stack:<type>>` (`<stack:empty>` for `""`); `frames` becomes `<frames:array:<sorted, comma-joined union of the frame keys>>` (a frame that is not an object adds `<type>`), or `<frames:<type>>` when not an array; `platform` becomes `<platform:<type>>`. |
| `id-placeholders` | Every `evt_`, `ses_` and `agt_` KSUID becomes `<prefix#n>` by first appearance, the same across transcript and events. A `ses_` ID whose KSUID time is before 2024-03-01 is derived from a hook value (B-065: derived times fall between 2024-01-01 and about 2024-02-19) and becomes `<ses-derived#n>`, with its own count, so a derived ID never matches a minted one. |
| `mcp-session-first` | Every `structuredContent` that has `mcp_session` must have it as its first key; a violation goes to `violations`. |
| `property-order` | For each tool in a `tools/list` result, adds `inputSchema["<properties-order>"]`: the key order of `inputSchema.properties`. |
| `server-info-meta-drop` | In each transcript `result`, drops `_meta["io.modelcontextprotocol/serverInfo"]`, and `_meta` when that leaves it empty. The TS v2 host library adds the key to every 2026-07-28 result and rmcp adds none: host behavior, not AgentCat's (RD-13). Other `_meta` keys, errors and event bodies stay. |
| `literal-derived-ids` | When the profile's `resolveSessionId` returns a non-blank string, the ID `deriveSessionId(value.trim(), projectId)` (computed with the TS `handles` module) stays literal instead of becoming a placeholder. IDs in `report.literalIds`. |

`<type>` is `string`, `number`, `boolean`, `object`, `array` or `null`.

Events are ordered by the time of their raw `timestamp` (so `…:29Z` comes before `…:29.005Z`); in the
same millisecond, a call (an event with a `duration` that is not a custom event: it can enclose events
published during it) comes before the rest, a server-form custom event included (it has a `duration`,
K-03, but encloses nothing); then by resource name, then by the normalized body with its IDs masked and its
keys sorted, so a raw duration or a key order never decides a tie. The TS queue publishes events
concurrently, so arrival order is noise. The call-first step keeps a custom event published during a
tool call after the call event, also when both land in the same millisecond. A scenario with
`"eventOrder": "multiset"` sorts on the masked normalized body alone.

The ordered checks read key order as JavaScript keeps it, so they do not apply to integer-like keys
(JavaScript puts those first).

## Rulings

`rulings.json` (delta rulings) and `rulings-events.json` (event rulings) list each allowed difference:

```json
[{ "path": "<regex over whole diff paths>", "reason": "why the difference is allowed", "deviation": "RD-xx" }]
```

`path` must match the whole diff path: diff.mjs anchors it as `^(?:<path>)$`, so `resource_name`
alone matches nothing. A diff path looks like `$.events[3].body.resource_name` or
`$.transcript[1].result.tools[0].inputSchema.<properties-order>`; a violation's path ends with
`.<rule>`; a delta path starts with `$.delta.`. Write `\$`, `\.`, `\[` and `\]` for the literal characters. `deviation` names the ruled
divergence in the design decision log. A ruling may also carry `"pending": true` (a boolean, nothing
else): a provisional ruling that waits for the owner's answer. It allows differences like any other.

A delta or event ruling may also carry `"scenario": "<name>"`, a scenario name (`scenarios/<name>.json`).
A delta path names the transcript entry by its index only, and an event path the event by its index in
the sorted events, so a ruling for one call of one scenario needs it:
`matrix.mjs` then applies the ruling only to that scenario's combinations, and a run that leaves the
scenario out neither reports the ruling nor counts it as unused. Without `scenario`, a delta ruling
covers its index in every scenario, transport and lane. A `scenario` that is not a string of letters,
digits, `_` and `-` makes the rulings file malformed (exit 2); one that names no scenario file makes
`matrix.mjs` exit 2.

A delta or event ruling may also carry `"transports"` and `"lanes"`: each a non-empty array of names, transports
from `stdio`, `http`, `http-sse` and `http-stateless`, lanes from `2025-11-25` and `2026-07-28`. `matrix.mjs` then
applies the ruling only to the combinations whose transport is in `transports` and whose lane is in
`lanes` (an absent field means every one), so a difference that TS shows on some transports or lanes
only is ruled there and nowhere else. A run that leaves out the ruling's scenario, all its transports or
all its lanes neither reports the ruling nor counts it as unused. A field that is not such an array, or
an unknown transport or lane name, makes the rulings file malformed (exit 2). `harness/rulings.json`
rules RD-29 this way: one ruling for `http-stateless` (both lanes) and one for `http` with
`"lanes": ["2026-07-28"]`, because TS and Rust agree on stdio and on stateful HTTP at 2025-11-25.

`diff.mjs` and `delta.mjs` compare documents that carry no scenario, transport or lane: they ignore
`scenario`, `transports` and `lanes` and match by path alone. The ruling line of a scoped ruling ends
its count with its scope, `in <scenario>`, `on <transport>,...` and `at <lane>,...`
(`RULING (pending) RD-29 matched 2 in tracing-off on http at 2026-07-28: ...`). Keep a ruling's
regex to the exact paths.

An event ruling may also pin the values it allows, so a path regex alone does not let any value pass.
Each difference of the event comparison and of `diff.mjs` carries two values: `a`, the first server's
(TS, the answer key), and `b`, the second server's (Rust). A side that does not have the key reads
`"<absent>"`.

- `"ts": <value>`: the ruling matches only when `a` equals this JSON value (key order aside).
- `"rust": <value>`: the ruling matches only when `b` equals this JSON value.
- `"rustKeys": "<regex>"`: the ruling matches only when `b` is an object with at least one key and every
  key matches the regex, anchored as `^(?:<regex>)$`. A `rustKeys` that is not a non-empty string or not a
  valid regex makes the rulings file malformed (exit 2).

A difference on a ruled path whose values the pins do not allow is unruled (an `EVENT` or `DIFF` line), and
it does not count in the ruling's `matched`. A delta difference carries no values, so a pinned ruling
never matches in `delta.mjs`; delta rulings do not pin. The ruling line shows the pins after the scope:
`ts=<value>`, `rust=<value>` and `rustKeys="<regex>"`.

`rulings.json` holds the delta rulings only, and `rulings-events.json` the event rulings only (the matrix
reads each with its own flag). `rulings-events.json` rules, for TS v2 against Rust: the keys of
`parameters.extra` on every event (RD-07), with one ruling per transport and lane that pins the TS v2 key
set and the Rust D34 key set (`"ts": "mcpReq", "rust": "requestId"` on stdio at 2025-11-25, for example);
the 2026-07-28 envelope that rmcp keeps in `parameters.request.params._meta` and the TS v2 host lifts out
(RD-13), pinned to `"ts": "<absent>"` and `"rustKeys": "io\\.modelcontextprotocol/.+"`, so any other key in
Rust's `_meta` is unruled; the `type`, `stack` and `frames`
of the errors that Rust has no source for (RD-06); and the host's own texts for an unknown tool and an
argument error (RD-13). Each ruling names its scenario and event index where it covers one call.
