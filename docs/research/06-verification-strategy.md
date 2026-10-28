# 06: Verification strategy for the Rust SDK (anti-drift)

Date: 2026-10-20. Author: research agent 06. Scope: how to make drift from the TypeScript (TS) SDK impossible to miss in a new Rust SDK for `rmcp`.

Paths abbreviated in this report:

- `TS/` = `/home/ubuntu/agentcat-typescript-sdk` (main, `9b56e9e`; `git fetch` showed origin/main is the same commit, so nothing newer exists)
- `PHP/` = `/home/ubuntu/agentcat-php-sdk` (branch `feat/plan-f-release`)
- `CS/` = `/home/ubuntu/agentcat-csharp-sdk` (branch `feat/delivery`)
- `GO/` = `/home/ubuntu/agentcat-go-sdk`, `PY/` = `/home/ubuntu/agentcat-python-sdk`
- `SRV/` = `/home/ubuntu/agentcat-server`, cited at `origin/main` (the local checkout is 15 commits behind)
- `SPIKE/` = `docs/research/spikes/diff-spike`
- `RMCP/` = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rmcp-3.6.0`

The previous Rust attempt was not read.

---

## 0. Recommendation in one page

Use **four layers**. Each layer catches a class of drift the others cannot.

| Layer | What it compares | Oracle | Cadence |
|---|---|---|---|
| L1 TS-derived vectors | Pure functions: truncation, sanitization, redaction, token estimates, session derivation, KSUID, mint-back text, injected schemas, agent-facing strings, JSON-stringify semantics | JSON files produced by **running the real TS modules** at a pinned SHA (not retyped) | per PR; regenerated nightly from TS main |
| L2 JS-oracle property tests | The same pure functions on random inputs | A long-lived Node process that runs the TS modules (JSON lines over stdio) | per PR (256 cases), nightly (10k+) |
| L3 Differential harness | Whole-system behavior: MCP wire (tools/list, tool results, mint-back) and the HTTP bodies published to the API | A TS reference server with the real TS SDK, run beside a Rust reference server; one scripted MCP client drives both; one mock API records both | per PR (pinned TS SHA, pinned rmcp); nightly (latest TS main × latest rmcp) |
| L4 Contract and end-to-end | Bodies are valid for the backend; examples run per transport | Server OpenAPI schema at a pinned SHA; the server's real `process_event` ingest run locally with testcontainers | schema per PR; ingest replay nightly |

Around these layers, add the Rust-native gates: insta snapshots (review aid), cargo-llvm-cov floors, cargo-mutants on changed code, a machine-checked **parity manifest** over all 1,020 TS test titles, cargo-public-api, and a nightly rmcp version matrix. Section 5 gives each gate with its tool, threshold, cost, and cadence.

The spike in `SPIKE/` shows that L1 and L3 work on this machine today (section 3.6). Two results stand out:

- The TS reference server publishes 11 events per scenario run. Three runs normalized to byte-identical documents (A/A diff = 0). stdio and streamable HTTP differ only in captured HTTP headers.
- All 11 TS bodies passed through the real `agentcat-server` ingest function locally (`ProcessResult.SUCCESS` ×11). This used an ephemeral Postgres container and no network.

---

## 1. TS SDK test suite inventory

### 1.1 Framework and layout

- **vitest 4** (`TS/package.json:96`). There are three projects: `v1`, `v2` and opt-in `e2e-http` (`TS/vitest.config.ts:17-49`). The suite runs sequentially in a single fork because tests share `~/agentcat.log` (`TS/vitest.config.ts:3-13`; `TS/CLAUDE.md` "Commands").
- A separate Cloudflare Workers project runs through `@cloudflare/vitest-pool-workers` (`TS/vitest.workers.config.ts:1-26`).
- `src/tests/setup.ts:6` forces `DISABLE_DIAGNOSTICS=true`. Diagnostics also auto-disable under `VITEST`, `JEST_WORKER_ID` and `NODE_ENV=test` (`TS/src/modules/diagnostics.ts:183`).
- **Size:** 79 test files and 22,539 lines. `npx vitest list --json` reports **1,020 test cases**: v1 806, v2 105, e2e-http 109, plus 3 Workers tests. All 1,020 `(file, full title)` keys are unique, so they work as manifest keys (section 6). The `it(` count is 865. The larger figure includes parameterized and lane-multiplied cases. The listing is saved at `<research scratchpad>/ts-vitest-list.json`.

### 1.2 What the files cover (grouped; counts are vitest cases)

| Area | Files (cases) |
|---|---|
| Handles: session_id/agent_id, mint-back, injection | `handles` (65), `handles-integration` (28), `handle-injection` (28), `handle-conflict-report` (9), `structured-mintback` (9), `context-parameters` (10) + `-edge-cases` (12), `custom-context-description` (8), `context-preservation` (2), `debug-tool-schema` (2) |
| Interception engine | `engine-listwrap` (12, the pure `buildInjectedList`), `engine-callwrap` (25, MRTR helpers), `engine-innertap` (11), `detect-shapes` (2), `handler-property` (3) |
| Event pipeline | `eventQueue` (27), `redaction` (33), `sanitization` (19), `truncation` (49), `tokenEstimate` (8), `token-estimates` (6), `validation` (11), `exceptions` (68), `error-capture` (12), `e2e-sanitization` (4), `e2e-truncation` (4) |
| Hooks and metadata | `identify` (12), `event-tags-properties` (18), `publishCustomEvent` (15), `client-identity` (17), `extra-projection` (8), `report-missing` (12) |
| IDs | `ksuid` (25), `posthog-uuidv7` (12), `constants` (11) |
| Ops | `api-base-url` (11), `signal-shutdown` (4), `logging-*` (9), `diagnostics-*` (11 files, about 37), `tracing-initialization` (2), `posthog-exporter` (27) |
| Host compatibility (TS-only) | `compatibility` (36), `mcp-version-compatibility` (1), `edge-runtime-compatibility` (23), `esm-consumer` (1), `v2/*` (105 over 16 files), `workers/*` (3) |
| Real-HTTP e2e | `e2e-http/v1-matrix` and `v2-matrix` (5 lanes × 22 scenarios = 109 cases), `sse-smoke-v1/v2`, `header-capture` |

### 1.3 Test servers and fixtures

- **Shared in-memory fixture:** `setupTestServerAndClient()` builds a todo McpServer and a Client over `InMemoryTransport.createLinkedPair()` (`TS/src/tests/test-utils/client-server-factory.ts:134`). 16 test files use it.
- **e2e-http matrix:** five lanes (`v1-stateful`, `v1-stateless`, `v2-handler-legacy`, `v2-handler-auto`, `v2-longlived`) on real `node:http` ephemeral ports (`TS/src/tests/e2e-http/lanes.ts:39-44,344-350`).
  - Every lane serves the same five-tool toolkit: echo, loose, strict, structured, boom. A **sink** records what each customer handler actually received. This proves that injected params were stripped (`TS/src/tests/e2e-http/toolkit.ts:1-28`).
  - The 22 scenario configs live in one table with shared fixture constants (`TS/src/tests/e2e-http/scenario-types.ts:31-121`).
  - **This table is the best seed for the Rust differential scenarios.** Its configs are already declarative, and only its scripts are TS code.

### 1.4 How TS tests observe published events

- **`EventCapture`** monkeypatches `eventQueue.add` to capture each event. It also stubs `sendEvent` to a no-op, so **no HTTP is sent** (`TS/src/tests/test-utils.ts:36-110`, stub at `:56-60`). 38 test files use it. Events are captured as in-memory objects, before serialization.
- `eventQueue.test.ts` and `api-base-url.test.ts` use `vi.mock("agentcat-api")` and assert calls on a mocked `EventsApi.publishEvent` (`TS/src/tests/api-base-url.test.ts:20,30-58`).
- Only the Workers test stubs `fetch` and sees real request bodies (`TS/src/tests/workers/event-delivery.test.ts:20-31`).
- **Gap that matters for Rust:** no Node test asserts the serialized wire body. The body comes from the generated client `agentcat-api`. Its `PublishEventRequestToJSONTyped` spreads the camelCase model and then adds the snake_case keys, so **every scalar is sent twice** (`SPIKE/ts-sdk-src/node_modules/agentcat-api/dist/models/PublishEventRequest.js:69`; the spike captured it, see `SPIKE/captured-event.ts.stdio.json`). The TS suite cannot act as the oracle for wire shape. The differential harness's mock API is the first place where the real TS wire body is observed.

### 1.5 TS CI

- `ci.yml` runs on Node 20 and 22: typecheck, lint, `pnpm test` (v1 and v2), build (`TS/.github/workflows/ci.yml:16-45`). Coverage is uploaded to Codecov with **no threshold** (`:47-74`; `TS/vitest.config.ts` has no `thresholds`).
- `mcp-compatibility.yml` runs on PR, push and a daily 02:00 cron (`:10`). It runs the v1 suite against the latest patch of every MCP SDK minor ≥ 1.11 (`:35-77`), the v2 equivalent, the real-HTTP e2e matrix (`:132-157`), and packaging probes.

### 1.6 Hazards in the local TS checkout (do not use it as the oracle)

- `TS/package.json` has an **uncommitted** version edit (`2.1.1` → `2.1.1-beta.1`).
- `TS/node_modules` is **stale**: `agentcat-api@1.0.1` is installed, but `pnpm-lock.yaml:1058` resolves `1.0.3`. The 1.0.1 serializer has no `input_tokens`/`output_tokens` mapping.
- An oracle built from this checkout would differ from main. The spike used `git archive origin/main` plus `pnpm install --frozen-lockfile`, which resolved `agentcat-api@1.0.3`. CI must do the same.

---

## 2. How PHP, C#, Go, Python and Java verified parity

### 2.1 PHP (`PHP/`), the most rigorous port

- **Seven PHPUnit suites:** Unit, Golden, Integration, Contract, E2E, Runtime, Architecture (`PHP/phpunit.xml.dist:13-37`). They hold about 2,132 test methods. Order is random, warnings count as failures, and any test slower than 100 ms fails (`:5-10,50-52`).
- **Goldens generated by running real TS** at `9b56e9e`, bundled with esbuild, on Node 22 (`PHP/docs/audit/fixtures/README.md:3-10`). The generators are `handles/entry.ts`, `listing.ts`, `exporters/golden.ts`, `golden2.ts`, `probes/p1..p9`.
  - The copies in `tests/Fixtures/golden/` are pinned by sha256. `FixtureIntegrityTest` recomputes the hashes (`PHP/tests/Golden/FixtureIntegrityTest.php:12-38`).
  - Coverage: agent-facing strings, mint-back blocks, derive vectors (UTF-8, empty, `":"` collision), tools/list injection for 7 option sets, and exporter and diagnostics payloads.
- **Hand-copied (drift risk):** token-estimate, KSUID, UUIDv7, truncation, sanitization, redaction and exception vectors. The wire bodies G1-G4 were hand-written from reading `eventQueue.ts` (`PHP/docs/audit/07-api-client-and-backend.md:476-478`).
  - One assertion was loosened. TS gave 20130, but PHP asserts only `16389 < b < 32771` (`PHP/tests/Unit/Internal/Pipeline/TruncatorTest.php:245-246`).
- **Traceability enforced by a test:** 413 hand-maintained acceptance criteria (`PHP/tests/Traceability/criteria.php:5`) plus `gaps.php`.
  - `TraceabilityTest` fails when a criterion has no `#[Group('ac-…')]` and no stated gap (`PHP/tests/Architecture/TraceabilityTest.php:32-56,139-150`).
  - It is shallow. The test checks that a group exists, not what the test asserts. Criteria link to TS behaviors through audit docs, not to TS test IDs.
- **Decision log:** D15 "Copy TS for now. No group of bugs is fixed" (`PHP/docs/audit/DECISIONS.md:60`).
  - D09: lengths in UTF-16 code units (`:61`). D10: `error.type` is the native fully qualified class name (`:62`). D66 fixes the pagination bug B10.
  - The wire is snake_case only (`PHP/docs/audit/07-api-client-and-backend.md:210`). It drops the TS camelCase duplicates on purpose.
- **Gates:**
  - PHPStan at level max with no baseline.
  - Infection: `minMsi 90`, `minCoveredMsi 95` (`PHP/infection.json5:12-13`). On PRs it mutates changed lines only. The full run happens on main and nightly.
  - 95% line coverage from clover (`PHP/.github/workflows/ci.yml:112-123`).
- **Nightly** (`17 3 * * *`, `PHP/.github/workflows/nightly.yml:5`): dev-main of upstream MCP libraries, full mutation run, live schema check, runtime images, and private-member snapshot diffs of upstream. A failing job opens an issue.
- **No job ever runs TS.** The goldens are frozen at `9b56e9e`, and the regeneration job was planned but never built.
- **E2E** uses hand-written JSON-RPC clients over stdio and `php -S`, not a real MCP client (`PHP/tests/E2E/Support/StdioMcpClient.php:13`).
- **Caution:** the nightly `network` job GETs the production schema endpoint (`PublishEventSchemaTest.php:29`). Under this task's rules a Rust job must use the schema at a pinned `SRV` SHA instead.

### 2.2 C# (`CS/`)

- xUnit v3 on net8 and net10, plus net472 on Windows. There are 526 Facts and 75 Theories, and the last full run had 2,402 cases.
- Analyzers turn warnings into errors. `BannedSymbols.txt` bans Console and Task.Run. PublicApiAnalyzers is wired but has no shipped baseline.
- **Fixtures are byte-for-byte copies of the PHP audit goldens** (`CS/tests/AgentCat.Tests/Fixtures/README.md:1-12`; all 5 verified with `cmp` by the survey).
- One new idea: `json/stringify-vectors.json` is **generated by Node** to pin JS `JSON.stringify` semantics (46 strings, 25 numbers as IEEE bits). It settled a plan contradiction.
- Parity map:
  - `CS/docs/audit/03-ts-sdk-index.md` indexes all 79 TS files. It tags each test `[B]`, `[B/TS]` or `[TS]` and has a behavior→test checklist (`:723-941`).
  - Plan 3 requires every ported TS test to keep its TS title. Each task report lists every TS test with its C# name or a reason for dropping it (`CS/docs/superpowers/plans/2026-10-19-plan-3-delivery.md:36`).
  - **These ledgers live in gitignored `.superpowers/sdd/`.** The ledgers for plans 1-2 are gone, and no test enforces the map.
- Observation tools:
  - `RecordingSender`, and a `ManualClock` so retries never sleep.
  - `FakeAgentCatApi`, a loopback `TcpListener` that records raw bytes. `DeliveryEndToEndTests` runs pipeline → HTTP → fake.
  - There is no real-MCP-client end-to-end test yet, and no differential test.
- CI tests 4 MCP versions with a proof that the requested version was the one loaded (`BuildGateTests.cs:41-54`). It has **no coverage or mutation gate, though the spec asks for 95% lines and 90/95 mutation, no TS job, and no nightly.**
- Process that worked:
  - Every plan has a "Review Focus" of adversarial scenarios, each tied to a named test.
  - TDD with RED/GREEN evidence.
  - Opus implements and reviews the risky half, Sonnet the rest. Fix rounds are capped at 5.
  - Every ruling records its cost if wrong.

### 2.3 Go, Python, Java

- **No shared fixture files exist anywhere.** Cross-language agreement rests on hand-copied literals:
  - Derive vectors such as `ses_2cOHEO0LYGADMzRvWTXXVbbgxgm` (`GO/internal/handles/handles_test.go:13-30`, `PY/tests/test_handles.py:37-41`).
  - Agent-facing copy (`GO/internal/constants/constants_test.go:6-40`, `PY/tests/test_constants_copy.py:1-20`). Python adds an AST scan that fails on a second copy of the text (`:169-188`).
- Event observation: Go uses an injected publish function plus an `httptest` capture API (`GO/mcpgo/redact_event_test.go:18-60`). Python monkeypatches `event_queue.add` (`PY/tests/test_lowlevel_v2_handles.py:51-57`).
- **Good nightly templates:**
  - Go discovers upstream versions with `git ls-remote --tags`, keeps the latest patch per minor at or above a floor, and runs `go get pkg@vX && go test -race` (`GO/.github/workflows/mcp-go-compatibility.yml:24-55`).
  - A separate prerelease nightly runs too, and "pinned tripwire" tests fail when an upstream rename silently disables a shim (`GO/mcpgo/compat_pinned_test.go:1-25`).
  - Python discovers versions from PyPI with PEP 440 ranges and emails failures.
- **Lessons:**
  - Python's `uv run` without `--no-sync` silently reinstalled the lockfile version, so its matrix was "green by construction". Always assert the loaded version.
  - The cross-SDK changelog drifted from the code: it says the derivation trims, but Go pins "does NOT trim" (`/home/ubuntu/2026-07-28-cross-sdk-changelog.md:188` vs `GO/internal/handles/handles_test.go:36`). Prose specs drift; executable oracles do not.
  - Python's `identify` hook was never awaited (`PY/2026-07-28-cross-sdk-changelog.md:278-286`). That is a behavior drift that only a whole-system comparison would catch.
- Java ported the 22 e2e-http scenarios over every lane (`agentcat-java-sdk-private/.../E2EMatrixTest.java:18-20`), with "ported from" comments per class.

### 2.4 What worked, what was shared, what was missing

- **Worked:**
  - Goldens generated from the real TS modules and pinned by sha256 (PHP, copied into C#).
  - Node as the oracle for JS semantics (C#).
  - Traceability enforced by a test (PHP).
  - Fake clocks, loopback fake APIs, and API-guard env vars.
  - Upstream version matrices with a proof of the loaded version.
- **Shared across languages:** only the PHP audit's generated goldens (PHP→C#) and hand-copied literals (Go, Python, Java). None of them is regenerated.
- **Missing in every SDK:**
  1. Nothing ever compares a running SDK with the running TS SDK.
  2. No fixture is regenerated from TS HEAD.
  3. No machine check that every TS test is ported.
  4. The real TS wire body is never captured.
  5. Coverage and mutation gates are specified but unenforced (C#).
  6. Prose specs drift (the changelog).

  These six gaps are what the Rust plan must close.

---

## 3. Differential testing as the main anti-drift gate

### 3.1 Verdict

**Feasible now, cheap (about 1.7 s per scenario per transport), and the strongest single gate.** It compares behavior at the two boundaries the customer and the backend see: the MCP wire and the publish body. It catches drift that no unit vector can. Examples:

- An un-awaited hook (Python's bug).
- A missing tag or a field that is null on one side.
- Mint-back placed in the wrong content position.
- A wrong `session_id` on the event after an invalid handle.
- Custom events missing session info.
- Token estimates computed after redaction instead of before.

### 3.2 Tools on this machine (checked)

- Node `v22.23.2` (`/usr/bin/node`), npm, and pnpm 10 (`~/.local/bin/pnpm`).
- Docker 29.7.2, working: the testcontainers-based server ingest ran.
- Rust (add `~/.cargo/bin` to PATH): `rustc 1.99.1`. Toolchains: stable, nightly, nightly-2026-10-18 and 1.88 (useful for an MSRV leg). `llvm-tools` and `clippy` are installed.
- Installed cargo tools: cargo-llvm-cov 0.9.1, cargo-mutants 27.1.0, cargo-deny 0.20.2, cargo-public-api 0.52.0, cargo-hack 0.6.45, cargo-minimal-versions, miri.
- **Not installed:** cargo-nextest and cargo-insta (both `cargo install`-able). insta and proptest are ordinary dev-dependencies.
- crates.io latest: `rmcp = 3.6.0`. Several 3.x versions are cached locally (3.0, 3.3, 3.5.1, 3.6.0), so the API is moving fast. The nightly must track it.

### 3.3 Pointing the TS SDK at a mock API

- Use the `apiBaseUrl` option, or `AGENTCAT_API_URL`, then the legacy `MCPCAT_API_URL`, then the default `https://api.agentcat.com` (`TS/src/index.ts:229-234`; `TS/src/modules/eventQueue.ts:43-50`). The publish path is `POST {base}/v1/public/events/` (`agentcat-api/dist/apis/EventsApi.js:44`).
- **Safety in the spike:**
  - The server refuses to start unless the base URL is `127.0.0.1` (`SPIKE/ts-server.mjs:22-25`).
  - `disableDiagnostics: true` plus `DISABLE_DIAGNOSTICS=true` keep OTLP diagnostics off. The default collector is `https://otel.agentcat.com` (`TS/src/modules/constants.ts:6`).
  - An **egress guard** preloaded with `node --import` refuses every non-loopback `fetch` (`SPIKE/egress-guard.mjs`). It was verified: a fetch to `api.agentcat.com` is refused before any connection.
  - `HOME` is redirected so `~/agentcat.log` lands in `SPIKE/home/` (`SPIKE/driver.mjs:50`; the log path comes from `os.homedir()`, `TS/src/modules/logging.ts:49-54`).
  - The Rust side needs the same three controls: a loopback-only base URL, diagnostics off, and a log path override.

### 3.4 Determinism: normalize, do not stub

Nondeterministic sources in TS:

- Random KSUIDs for `evt_` and minted `ses_` IDs (`TS/src/modules/eventQueue.ts:155`, `TS/src/modules/handles.ts:22-24`).
- `new Date()` timestamps and durations (`TS/src/engine/callWrap.ts:168,322,352`).
- Up to 5 concurrent publishes, so arrival order varies (`TS/src/modules/eventQueue.ts:37,89-95`).
- Versions and the language name.

**Hook-mode session IDs are deterministic.** `deriveSessionId` is sha256 of `id:projectId` (`TS/src/modules/handles.ts:35-49`), so those IDs must be compared **literally**.

The spike normalizer (`SPIKE/normalize.mjs`) applies explicit, reported rules:

1. Drop TS camelCase alias keys that have a snake_case twin (`:50-56`). The aliases are listed in the run report: 18 keys, from `actorId` to `userIntent`. PHP and C# ruled snake_case-only, and the server reads only snake_case.
2. `timestamp` → format class `<iso8601-ms-Z>`, after validating the format (`:58`).
3. `duration` → `<duration-ms>`, only if it is a non-negative integer (`:60`).
4. `sdk_language` and `agentcat_version` → class tokens (`:62`).
5. `parameters.extra` → only the backend-relevant headers by name (`:64-70`). The server finds `user-agent` and `x-anthropic-client` by **nested search** anywhere in `parameters` (`SRV/apps/analytics/models/events.py:327-341`; `find_nested_value` use at `:574`), so the path does not matter but the name and value do.
6. Elide `error.stack`, `frames` and `platform`. Keep `message` and `type` (`:71-74`).
7. Random IDs → first-appearance placeholders `<ses#1>`, `<evt#3>`, mapped consistently across the MCP transcript and the events (`:78-90`). This keeps the cross-check "the ses_ the client was issued is the ses_ on the event".

Order is restored by sorting on the original timestamp, then `resource_name`, then canonical JSON, before the timestamps are blanked (`:39-45`).

**Evidence:**

- Three TS stdio runs produced byte-identical normalized documents (`diff.mjs` total 0).
- stdio vs HTTP differs in exactly 30 places. All are the HTTP-only headers (`user-agent`, `x-test-header`, `mcp-protocol-version`) on the 10 tool-call events.

The normalizer is tight enough to show real differences and stable enough to have no noise.

Recommended additions for the real harness:

- (a) Ordered comparison at specified paths, for example "`mcp_session` is the first key of `structuredContent`" (`TS/src/modules/constants.ts:56-58`). Plain JSON diff is order-insensitive for objects.
- (b) Invariant checks before blanking: duration ≤ wall time between steps, timestamps monotonic with step order, and ID regexes.
- (c) A **canary**: a deliberately perturbed Rust config must make the diff fail. This proves the normalizer is not hiding everything. It is a mutation test for the harness.

### 3.5 One language-neutral client, many servers

- **Driver:** `SPIKE/driver.mjs` uses the official TS MCP client, which is also what most real hosts run.
  - It spawns any server command over stdio, or starts it in HTTP mode and reads a JSON ready line.
  - It runs a **JSON scenario file** (`SPIKE/scenarios/basic.json`) with steps `listTools` and `callTool`, `${VAR}` substitution, and `saveHandleAs`, which extracts the issued `ses_` from the mint-back text or `structuredContent.mcp_session` (`SPIKE/driver.mjs:83-92`).
  - It waits until the expected publish count is reached and the stream is quiet for 500 ms (`:122-136`).
  - It writes raw, transcript, normalized and report files.
- **Options profile:** each scenario carries a JSON options profile that both reference servers must interpret the same way (`SPIKE/ts-server.mjs:27-42`): flags, `sessionHook`, `identify`, `eventTags`, `eventProperties`, `redactSecret`. Hooks with arbitrary code cannot be serialized. The profile enumerates canned behaviors instead: returns X, returns null, throws, sleeps N ms.
- **Tool contract:** to keep customer-schema noise out, the TS server is run once **without** `track()` and its `tools/list` is dumped to `SPIKE/tool-contract.json`. The Rust reference server loads that file and serves those exact schemas, because `rmcp::model::Tool` deserializes raw JSON Schema (`RMCP/src/model/tool.rs:17-40`). The contract is generated, not hand-copied.
- **Recommended lanes:**
  - A v1 TS client (protocol 2025-11-25).
  - A v2 TS client with `versionNegotiation: auto`, which reaches the 2026-07-28 per-request envelope (`TS/src/tests/e2e-http/lanes.ts:146-155`). rmcp 3.6 implements 2026-07-28 (`RMCP/src/model.rs:4136-4147`).
  - Optionally an rmcp client lane later.
- **"Delta" comparison:** run each scenario four ways: TS untracked, TS tracked, Rust untracked, Rust tracked. Then assert both of these:
  - Events: `TS tracked` == `Rust tracked`, after normalization and the ruled list.
  - MCP wire: `patch(TS untracked → TS tracked)` == `patch(Rust untracked → Rust tracked)`.

  The second check compares *what AgentCat changes*: injected properties, required entries, the get_more_tools tool, mint-back blocks and the structured mirror. Host-SDK differences then cancel out.

### 3.6 Spike results (all under `SPIKE/`)

| Step | Result |
|---|---|
| Build TS SDK from `git archive origin/main` | pnpm frozen install plus tsup build OK; resolved `agentcat-api@1.0.3` |
| `basic` scenario: 2 clients, 10 tool calls (handle mint → supplied → invalid → `start`, thrown error, `isError` result, 120 KB payload, get_more_tools, custom event, identify/tags/properties/redaction) | 11 publishes = 11 expected, over **stdio and streamable HTTP**; 1.65 s and 1.75 s |
| A/A (3 stdio runs) | 0 diffs |
| stdio vs HTTP | only HTTP header capture differs (30 diffs, all expected) |
| Captured event JSON | `SPIKE/captured-event.ts.stdio.json` (raw body: camelCase + snake_case duplicates, `user-agent: node`, `parameters.extra = {signal:{}, requestId}`) |
| Server ingest of the 11 TS bodies | `process_event` → SUCCESS ×11 on a testcontainers Postgres; created 2 error issues and 1 `oversized_tool_response` issue (`SPIKE/ingest-check/test_ingest_capture.py`) |
| Rust reference server skeleton (rmcp 3.6, uninstrumented) driven by the same driver | works over stdio (`SPIKE/rust-ref/src/main.rs`) |
| Host baseline TS SDK vs rmcp (no AgentCat on either side) | 14 diffs, listed below |
| TS-derived vectors | `SPIKE/vectors/gen-vectors.ts`, bundled with esbuild, runs the real `tokenEstimate.ts`, `handles.ts` and `truncation.ts` → `vectors.json` with `ts_sha` |

Behaviors the spike exposed. Each would need a ruling, and each would have been invisible to unit tests:

- **TS transforms the publish body in ways no test asserts:**
  - camelCase duplicates.
  - Custom events carry no `sdk_language`, `agentcat_version`, server or client fields. The server logs "No client identifiers found" for them.
  - An invalid handle (`ses_invented`) publishes `session_id: null` with tag `agentcat_session_id_source: invalid`.
  - Token estimates are computed on raw args **before** redaction.
  - Redaction touches the event but **not** the wire result the client sees.
  - A 120,000-char response is truncated to 32,771 UTF-16 units (32,768 + `...`), and `output_tokens` = 34,286 is computed before truncation.
- **Host-SDK differences between rmcp 3.6 and TS MCP SDK 1.31** (`out/host-baseline.*`):
  1. TS advertises `execution: {taskSupport: "forbidden"}` on every tool. rmcp's `Tool` has no `execution` field (`RMCP/src/model/tool.rs:17-40`).
  2. rmcp always emits `isError: false`. TS omits it.
  3. A thrown handler error becomes an `isError` result in TS. In rmcp, an `Err(McpError)` becomes a JSON-RPC error (-32603). rmcp's docs say protocol errors are opaque to callers (`RMCP/src/handler/server.rs:520-537`).
  4. An unknown tool is an `isError` result in TS and a JSON-RPC error in rmcp.
  5. TS validates input against zod. rmcp skips validation when `get_tool` returns None (`RMCP/src/handler/server.rs:545-551`).
- **JS-vs-Rust semantic traps the vectors exposed:**
  - `{"n":1.0}` gives 2 input tokens in TS, because `JSON.stringify(1.0)` is `"1"`. serde_json would write `1.0` and give 3.
  - `1e21` serializes as `1e+21` in JS.
  - A lone surrogate `\ud800` is accepted by JS but **rejected by serde_json at parse time**, so rmcp cannot even receive it.
  - Truncation counts UTF-16 units. An emoji string truncates to 32,771 units but 65,539 UTF-8 bytes.

  All of these need L1 and L2 vectors. Rust's natural `String::len()` or `serde_json::to_string` would drift.

### 3.7 What the differential harness cannot cover (needs L1, L2 or Rust-native tests)

1. **Timing and delivery policy:** retries (3, with backoff 1/2/4 s), retries on 4xx, queue cap 10,000 with drop-oldest, concurrency 5, shutdown drain 5 s (`TS/src/modules/eventQueue.ts:35-37,213-245,261-283`). Test these with a fake clock in Rust. Add one slow nightly harness scenario that uses the mock's `/__fail?n=K` (`SPIKE/mock-api.mjs`) to compare retry counts.
2. **Inputs the wire cannot carry or rmcp rejects:** lone surrogates, NaN or Infinity, cyclic or non-JSON TS values, and customer `Serialize` types in custom events. Use L1 vectors over `serde_json::Value`, plus Rust-only tests.
3. **Arbitrary hook code** (a throwing identify, hook timeouts, `redactEvent` forgery as in Python 8846cf5). The profile covers canned cases; the rest needs unit tests.
4. **Error capture details:** stack frames, and native error type names (PHP D10). These are a ruled difference, pinned with insta snapshots.
5. **Exporters** (OTLP, Datadog, Sentry, PostHog) and SDK diagnostics, if in scope. Either point their endpoints at the mock (the mock records any path) or use the PHP audit's exporter goldens (`PHP/docs/audit/fixtures/exporters/golden*.ts`).
6. **Log lines in `agentcat.log`.** This can be added later as a normalized log diff.
7. **API ergonomics and lifecycle:** `track` never panics, idempotence, signal re-raise (`TS/src/modules/eventQueue.ts:299-319`), and a clean stdout on stdio. A stray stdout line breaks the harness's stdio lane, which partly covers the last point.
8. **TS-host-only behavior:** v1 and v2 adapter detection, Workers, ESM packaging. 96 cases are out of scope by nature (section 6).

---

## 4. Backend test or sandbox ingestion path

- **No sandbox, test mode or dry-run exists for ingest.**
  - `POST /v1/public/events/` is `AllowAny`. It checks only that the body is non-empty and that `project_id` is present, then queues `request.data` raw to SQS and returns `202 {"status":"accepted"}` (`SRV/apps/analytics/api/public.py:38,222-283`).
  - The repo has no test projects or sandbox flags. The only `dry_run` is on issue rules (`SRV/apps/analytics/api/issues/rules.py:42-120`).
- **Real validation happens in the SQS consumer `process_event`** (`SRV/apps/analytics/models/events.py:464+`):
  - Required `project_id`, `id` and `event_type` (`:211-237`).
  - `project_id` = `proj_` plus 27 characters (`:260-272`).
  - Project exists; timestamp present.
  - Event type canonicalized from `agentcat:*` to `mcpcat:*`.
  - `agentcat_version` copied to `mcpcat_version`.
  - Tags and properties sanitized (`:385-460`).
  - Session upsert, token counts, redaction, issue matching.
- **Recommended "real end-to-end" path without production:** replay the bodies captured by the harness through `process_event` in the server's own pytest setup.
  - The setup uses testcontainers Postgres and ClickHouse (`SRV/conftest.py:19-31`, `SRV/utils/testing/postgres.py`). Precedent: `SRV/apps/analytics/tests/test_agentcat_wire_compat.py:24-40`.
  - **Spike:** `SPIKE/ingest-check/test_ingest_capture.py` ran from outside the repo with `-p conftest`, `PYTHONDONTWRITEBYTECODE=1` and `-p no:cacheprovider`. All 11 TS bodies passed in 13.8 s, and the server repo stayed unmodified.
  - In CI it fits a nightly job that needs an `agentcat-server` checkout (private). A cleaner alternative is a consumer-driven contract: the Rust CI uploads its captured bodies as an artifact, and the server repo's CI replays them.
- **Schema gate:**
  - Validate every captured body (TS and Rust) against the publishEvent request schema in `SRV/schema.yml:5621+` at a pinned SHA. That schema is the same source the generated API clients are built from (`SRV/sdk-generator/generate-rust-sdk.sh`).
  - `SRV/apps/analytics/tests/test_public_event_schema.py:8-30` already reads it.
  - Do not fetch the production schema endpoint, as PHP's nightly does.

---

## 5. Rust verification gates (tools, thresholds, cadence)

Costs are rough: CI minutes on a 4-core runner, plus maintenance.

| # | Gate | Catches | Tool and threshold | Cost | Cadence |
|---|---|---|---|---|---|
| G1 | **TS-derived unit vectors** | Pure-function drift (token rounding, UTF-16 truncation, derive collisions, mint-back wording, injected schema shape and key order, agent-facing strings, JSON number formatting) | `tools/ts-oracle/` (Node): checks out TS at the SHA pinned in `oracle.lock`, `pnpm install --frozen-lockfile`, bundles `src/modules/*.ts` with esbuild, and writes `tests/vectors/*.json` with `ts_sha`. A sha256 manifest is verified by an integrity test (as in PHP). Start by porting the PHP generators (`PHP/docs/audit/fixtures/`). Rust tests load JSON with `include_str!`. **Threshold: 100% of vectors equal.** | Low CI (seconds). Medium one-time cost (about 2 days) | per PR (pinned); nightly regenerates from TS main and diffs (drift alarm opens an issue) |
| G2 | **JS-oracle property tests** | Edge cases nobody thought to list (Unicode, number formats, deep or wide structures, cut points) | `proptest` generates `serde_json::Value` and strings. A Node oracle process (JSON lines over stdin/stdout) runs the same TS function, and the test asserts equality. Plus pure invariants: never panics, idempotent sanitize, output ≤ caps in UTF-16 units, valid UTF-8, `tokens == ceil(bytes/3.5)` clamped to int32. **256 cases per PR, 10k nightly. Commit proptest regression files.** | Low to medium | per PR (small), nightly (large) |
| G3 | **Differential harness** | Whole-system drift on both boundaries: injection, mint-back, hooks, tags, null fields, event counts, header capture, custom events, error paths, multi-session, large payloads | Section 3 harness in the Rust repo: `harness/` (Node driver, mock API, normalizer, TS reference server) plus `examples/ref-server` (Rust). Scenarios seeded from the TS e2e table (`scenario-types.ts:58-121`) plus pagination (low-level list handler with cursors), identify, custom events, concurrency (compared as a multiset), and a 2026-07-28 lane. **Threshold: 0 unexpected diffs.** Ruled differences go in `harness/rulings.json` (path regex, reason, decision ID). Any ruling that matches nothing fails, so stale rulings cannot linger. Includes the canary. | About 2 s per scenario per transport per server: 25 scenarios × 2 transports × 4 runs ≈ 7 min sequential, about 2 min with 4-way parallelism. Medium maintenance (2 reference servers) | **per PR, blocking** (pinned TS SHA, pinned rmcp); nightly: {pinned, latest TS main} × {pinned, latest rmcp} |
| G4 | **insta snapshots** | Unreviewed changes to agent-facing strings, injected schemas, tools/list output, error-capture shapes, log lines | `insta` with JSON or YAML snapshots. CI runs `INSTA_UPDATE=no cargo insta test --check --unreferenced=reject`. **Rule:** a snapshot never stands alone for TS-owned text. The same value must also equal the G1 vector, otherwise accepting a snapshot would launder drift. | Low | per PR |
| G5 | **Coverage floor** | Untested code paths, often ported code with no TS test mapped | `cargo llvm-cov --workspace --fail-under-lines 90`; `--fail-under-lines 95` for the pure crates or modules (sanitize, truncate, tokens, ids, handles, injection). Branch coverage (`--branch`, nightly toolchain) reported nightly, with a target ≥ 85%. Harness runs of the Rust reference server can be included with `cargo llvm-cov run` / `show-env`. | Low | per PR (lines), nightly (branches) |
| G6 | **Mutation testing** | Tests that run code but assert nothing (PHP's loosened truncation assertion is the classic case) | `cargo mutants --in-diff <(git diff origin/main...HEAD)` per PR on changed code: **0 missed mutants in core modules**, and `#[mutants::skip]` only with a written reason. Full run nightly: **mutation score ≥ 90% caught of viable mutants** (PHP used `minMsi 90` and `minCoveredMsi 95`). Use `--test-tool nextest` for speed. | PR: minutes on the diff. Nightly: 30-90 min | per PR (diff), nightly (full) |
| G7 | **Parity manifest** | Silently unported TS behavior; a new TS test with no Rust counterpart | Section 6. `cargo xtask parity` checks every TS `(file, title)` against a Rust test name or a written reason, and checks that the named Rust tests exist (`cargo test -- --list`). **Threshold: 0 unmapped, 0 dangling.** | Low CI. Medium upkeep, but it is the porting checklist anyway | per PR (pinned list); nightly (list regenerated from TS main) |
| G8 | **Wire contract** | Bodies the backend rejects or misreads | JSON Schema validation of every captured body against `SRV/schema.yml` at a pinned SHA (per PR). Nightly replay of captured TS and Rust bodies through `process_event` (section 4). **Threshold: 100% valid, 100% SUCCESS.** | Low (schema). Medium (replay needs the server checkout and Docker) | per PR, nightly |
| G9 | **End-to-end example per transport** | Broken integration paths: stdio stdout pollution, streamable HTTP session handling, shutdown flush | `examples/stdio.rs` and `examples/streamable_http.rs` run in CI against the mock: assert N events arrive, no non-JSON stdout lines, and a clean exit within 5 s of SIGTERM with the queue drained | Low | per PR |
| G10 | **rmcp version matrix** | rmcp API or behavior changes; the floor version still works | Discover versions with the crates.io index (or `cargo search`), keeping the latest patch per minor ≥ floor. Run `cargo update -p rmcp --precise X`, then tests plus harness. A **proof-of-version test** asserts that the compiled rmcp is the requested one (C# `BuildGateTests` lesson; Python "green by construction" lesson). Also run rmcp git main and a prerelease leg. | Medium | nightly (and per PR for floor plus latest) |
| G11 | **API surface and supply chain** | Accidental public API changes; bad dependencies | `cargo public-api diff` against a committed baseline; `cargo-semver-checks` after 1.0; `cargo deny check`; `cargo hack --each-feature`; MSRV leg with the installed 1.88 toolchain; `clippy -D warnings` | Low | per PR |
| G12 | **Nightly drift sweep** | Any change in TS main that Rust has not followed | Rebuild the oracle from TS main, regenerate G1 vectors, regenerate the G7 test list, run G3 with latest TS. Any diff opens or updates one issue with the diff attached (PHP nightly pattern, `PHP/.github/workflows/nightly.yml:55-59`). | Medium | nightly |

Notes:

- Run tests with `cargo nextest` (`cargo install cargo-nextest`). It gives per-test timeouts (catches hangs) and JUnit output for the manifest checker.
- Use a fake clock and a fake ID source behind a test-only seam (a `test-util` feature or `#[cfg(test)]` injection). Rust unit tests on retry and backoff then never sleep (C# `ManualClock`, PHP simulated clock).
- Add a closed-port API guard env var in the test bootstrap (PHP `EventApiGuard`). A stray real publish then fails loudly.

---

## 6. Parity manifest design

- **Source of truth.** `npx vitest list --json` at the pinned TS SHA, plus the Workers config. This gives 1,020 unique `(file, full title)` keys today. Commit the result as `parity/ts-tests.json` together with the TS SHA. It is generated, never hand-written.
- **Mapping file.** `parity/manifest.toml` has one entry per key, of two kinds:
  - `rust = ["crate::module::tests::name", "harness:scenario-name"]`, or
  - `status = "n/a", reason = "<category>: <one line>", decision = "Dxx"`.

  Allowed categories: `ts-host-only` (v1/v2 adapter, Workers, ESM), `out-of-scope` (exporters, if ruled), `ruled-divergence`, `covered-by-vector:<file>`.
- **Checker** (`cargo xtask parity`, per PR):
  - (1) Every TS key has an entry.
  - (2) Every named Rust test exists in `cargo nextest list` output, and every `harness:` name exists in `harness/scenarios/`.
  - (3) No Rust test is claimed by zero entries in the ported-test directories. This is optional, to avoid orphans.
  - (4) Entries for TS keys that no longer exist fail.
  - Nightly regenerates `ts-tests.json` from TS main, so a new TS test shows up as unmapped the next morning.
- **Naming.** Keep the TS title in the Rust test, either as a `#[doc = "TS: <title>"]` attribute or in the manifest. C#'s mixed naming made the map hard to audit.
- **First-cut layer split** of the 1,020 cases, by file:
  - about **443** go to G1/G2 vectors (truncation, sanitization, redaction, tokens, ksuid, handles, injection, listwrap, constants, validation, exceptions, …);
  - about **325** go to G3 harness scenarios (all 109 e2e-http cases, identify, tags and properties, custom events, report-missing, error-capture, handles-integration, …);
  - about **156** go to Rust-native unit tests with a fake clock (eventQueue, base URL, signal shutdown, diagnostics, logging, exporters, callwrap MRTR helpers);
  - about **96** are `ts-host-only` (compatibility, edge runtime, ESM, detect, innertap, v2 adapter internals).

  The script and numbers are in this report's working notes. Treat them as a starting estimate, not a ruling.
- **Why this beats PHP's criteria list.** PHP's `criteria.php` is hand-maintained prose, so a new TS test never appears anywhere. A generated list keyed by TS test identity cannot silently miss one.

---

## 7. Risks

1. **The normalizer hides real drift.** Mitigations: every rule is explicit and its counts are reported; A/A tests; the canary perturbation must fail; ordered checks at specified paths; hook-mode IDs compared literally.
2. **Copying TS bugs on purpose (D15) means the harness enforces "same as TS", not "correct".** The G8 ingest replay adds a correctness check from the backend side. Each TS bug the Rust SDK keeps should be pinned by name in `rulings.json` or in tests, as PHP B01-B20 are.
3. **Oracle staleness vs flakiness.** A pinned SHA keeps PRs deterministic, and the nightly against TS main catches drift. Someone must triage nightly diffs within a set SLA, or the alarm becomes noise.
4. **Host-SDK divergence (rmcp vs TS SDK) is large and growing.** rmcp 3.x already differs on errors, validation and the tool `execution` field, and it moves fast. The "delta" comparison and an explicit host-difference ruling list keep this from swamping AgentCat diffs.
5. **Two reference servers and an options profile add maintenance.** Keep the reference servers tiny, keep the tool contract generated, and cap hooks to enumerated behaviors.
6. **Using the developer TS checkout as the oracle.** It is dirty and has stale `node_modules` (section 1.6). Always build from `git archive <sha>` with a frozen lockfile.
7. **Private repos in CI.** The TS source (needed for module-level vectors) and `agentcat-server` (needed for ingest replay) need read tokens in the Rust repo CI. An npm tarball of `agentcat` is enough for the reference server, but not for internal-module vectors.
8. **Timing flakes in the harness.** Use expected counts plus a quiet window, not sleeps. Per-test timeouts through nextest. A retry-on-failure policy is **not** allowed, because it would mask races.

---

## 8. Decisions the owner must make

1. **Is the differential harness PR-blocking?** Recommended: yes, at a pinned TS SHA. The alternative is nightly-only.
2. **TS pin policy.** Pin `9b56e9e`, which is current main and the PHP and C# audit commit. Who bumps the pin, and how fast must nightly drift be triaged (port it, or rule a divergence)?
3. **Bug-compat policy (D15 carry-over).** Copy TS quirks by default? Decide the known ones explicitly:
   - camelCase duplicate keys (PHP and C# went snake_case-only);
   - retries on 4xx;
   - custom events without session info;
   - `identifyActorData` redacted;
   - `error.type` naming: PHP D10 uses the native type name.
4. **Host-difference rulings for rmcp.** When a Rust handler returns `Err(McpError)` or a tool is unknown, rmcp answers with a JSON-RPC error where TS answers with an `isError` result. Choose what the Rust event records:
   - (a) a synthesized TS-shaped `response {content:[{text}], isError:true}`, or
   - (b) a native shape (`response` null, `error` from the code and message).

   Also decide whether to advertise `execution.taskSupport`.
5. **Shape of `parameters.extra` in Rust events.** TS dumps the host SDK's `extra` (`signal`, `requestId`, `requestInfo.headers`). The backend only does a nested header search. Choose a fixed Rust shape. Also decide whether inbound `Authorization` and `Cookie` headers are published (PHP D06 kept them for parity).
6. **`sdk_language` value and version format** for Rust (TS sends `"TypeScript"`).
7. **Scope of exporters and diagnostics in v1.** This decides whether the harness gets exporter lanes and whether exporter goldens are ported.
8. **Thresholds.** Lines 90% workspace / 95% core; mutation 0 missed on diff and ≥ 90% nightly; proptest case counts. Confirm or adjust.
9. **CI access.** Grant read tokens for the TS and server repos, or use the consumer-driven alternative (the server CI replays Rust-captured bodies).
10. **Protocol eras.** Is a 2026-07-28 lane (v2 TS client with `auto`) required at launch?

---

## Appendix A: spike file index (`SPIKE/` =

`docs/research/spikes/diff-spike`)

- `run-spike.sh`: reproduces every step.
- `mock-api.mjs`: records every request. It answers publishes with `202 {"status":"accepted"}` (mirrors `SRV public.py:280-283`) and has `/__requests`, `/__reset` and `/__fail` control routes.
- `egress-guard.mjs`: a loopback-only `fetch` guard.
- `ts-server.mjs`: the TS reference server (real TS SDK). It supports stdio and streamable HTTP (one tracked server per session), the JSON options profile, and `SPIKE_NO_TRACK=1` for host-baseline runs.
- `driver.mjs`: the scenario driver. `normalize.mjs`: the normalizer. `diff.mjs`: a structural diff with a `--allow` rulings file.
- `scenarios/basic.json`, `scenarios/host-baseline.json`.
- `out/`: raw, transcript, normalized and report files per run. `captured-event.ts.stdio.json`: one raw TS wire body.
- `tool-contract.json`: the generated tool contract.
- `rust-ref/`: the rmcp 3.6 uninstrumented reference server skeleton (stdio).
- `vectors/gen-vectors.ts` and `vectors/vectors.json`: TS-module-generated vectors.
- `ingest-check/test_ingest_capture.py`: the server ingest replay.
- `ts-sdk-src/`: the TS SDK built from `git archive origin/main` (`9b56e9e`).
