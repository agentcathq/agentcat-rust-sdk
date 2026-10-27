# Items carried forward

Reviews of plans 1 and 2 found these items and deferred them. Each plan's final review sorted them into a later
plan. The person who writes that plan reads this list, checks each item against the code as it stands, and either
puts it in a task or records why not. Line numbers are as of `b9b91e0` (plan 1 items), `e685d14` (plan 2 items),
`dd67f1d` (plan 3 items), `da0b53d` (plan 3 final review items) or `f268db1` (plan 4 items) and can drift. Plan 2 closed every item that plan 1 carried to it.

## Plan 3 (interception, Rust reference server)

Plan 3 closed or moved every item carried to it. The moved items are below, marked "From plan 3".

## Plan 4 (delivery and lifecycle)

Plan 4 closed every item carried to it. Each item and the commit that closed it:

| Item | Closed by |
|---|---|
| The mock API keeps the raw body text | `6b26850` (`rawBody`, normalizer rule `number-source`) |
| The stdio close path and the 2 s SIGTERM | `a160f4c` (the server drains on EOF and SIGTERM), `6b26850` (the drained line) |
| An HTTP server that ignores SIGTERM | `6b26850` (a server that needs SIGKILL fails the run) |
| `TsEvent` key order from a TS probe | `a675047` (`event_order.json`), `f763849` |
| Async string redaction in two passes through `redact_strings` | `1b84def` |
| `new_event_id()` `None` drops the event with the delivery-failure line | `1b84def` |
| `cargo deny check --all-features` | `a160f4c` |
| The wire body with `js::stringify` rules, or an RD | `1b84def` (C40, RD-32) |
| The size guard measures the sent text | `1b84def` (C40: the guard measures the in-memory event, as TS does) |
| The `Object.assign` merge pinned by a test | `1b84def` (`assign_keeps_keys_the_stage_dropped`) |
| The timestamp in `toISOString` form | `d10583d`, `79d9d98` (`js::iso_millis`), `f763849` |
| Performance of truncation, `byte_size` and `own_entries` | `556168d` |
| RD-24 and the typed event | `d10583d`, `1b84def` (C39) |
| `extra-headers-only` hides D06 and D34; `extra` and `_meta` order | `6b26850`, `04ff41c` (`extra-projection`, pinned RD-07 rulings); `a675047`, `f763849` (orders, RD-07) |
| RD-04 whole-second timestamps | `6b26850` (`timestamp-class`) |
| Derived and minted placeholders, the sort tie-breaker, the same-millisecond line | `6b26850`; `04ff41c` (the driver waits for the next millisecond, C46) |
| `parameters.request` key order from a TS probe | `a675047`, `f763849` (RD-07) |
| Drop `resultType` from the response | `f763849` |
| `CallContext` on the `AGENTCAT_ENABLED=false` pass-through | `41e3e30` (C45) |
| Hook settling bounds or aborts a hook; hook panics documented | `f763849` (C37), `6345d82` (unsettled hooks aborted with their event) |
| Render the backtrace in the pipeline | `f763849` |
| The reference server: `emit_custom`, the redaction option, the API URL, the HTTP drain | `a160f4c` |
| `matrix.mjs` stops passing `--expect-publishes 0` for Rust | `a160f4c` |
| Split `rulings.json` for event rulings | `6b26850` (`rulings-events.json`) |
| The feature-unification self-check | `556168d` |
| Unused `cargo deny` licenses | `a160f4c` |
| Build `ToolCall` only when a hook is set | `41e3e30` |
| A drop guard for a dropped call future | `41e3e30` (C41, RD-34) |
| A call-path canary | `3cb757b`, `6cf2416` (`HARNESS_CANARY=call` and `event`) |
| R27: strip the injected arguments with tracing off | `41e3e30`, `8bdfc0b` (RD-29) |
| R27: `capture_error` accepts `?Sized` errors | `d10583d` |

## Plan 5 (finish)

- `agentcat-sdk/Cargo.toml` has no `homepage`; add it with the publish metadata.
- `ci.yml`'s `cancel-in-progress: true` also cancels runs on `main`. Cancel only PR runs once the mutation gate
  needs the previous green commit.
- Windows: the clone-directory rename needs a retry under antivirus, and `shell: true` with an argument array
  warns DEP0190 on Node 24 and later. From plan 2: `oracle/src/lock.mjs` throws on EPERM, EBUSY or EACCES in the
  break path instead of retrying (lines 158-161), the write-then-rename of a fresh directory can fail again and
  again under antivirus (113-114, 137, 163), and release gives up after one failed rename (206-210).
- The msrv job runs nextest only. Add `cargo test --workspace --doc` on 1.88 before the README doctests land.
- `harness/src/contract/extract-schema.mjs` does not compile its output with the validator's strict ajv
  settings.
- Ingest replay README: list `corepack pnpm install` and the first-run oracle build as prerequisites; reword the
  HOME isolation claim (a new HOME hides only HOME-resolved configuration); say precisely that the DNS guard
  covers call-time lookups through the socket module; note that `poetry install` runs with the real HOME.
- After the final PR merges (R21): dispatch the nightly once, so the issue-filing path runs for real.
- From plan 2, lock robustness (`oracle/src/lock.mjs`): the claim holder is identified by pid (compare tokens
  instead); an unreadable claim after a power loss blocks breakers forever; nothing sweeps `.new-*`, `.stale-*` and
  `.done-*` leftovers; only interleaving A has a deterministic test; `isStale` treats a changed host name as another
  host (macOS).
- From plan 2, vector tooling (`oracle/src/vectors/write.mjs`, `oracle/src/strict-json.mjs`): a hole in `cases`
  gives a `TypeError`, not a named error; a cyclic value overflows the stack with no path; extra array properties
  are dropped silently; `$repeat` outside `input` is not rejected; the generator path is hard-coded although
  `genDir` is a parameter; `writeAll` writes files before it checks later sets; tests miss the Set and class
  instance throws, the exact bits of `1e23` and `2**53+2`, and a mulberry32 known answer. A generator that sets a
  `deviation` other than RD-24 must not reuse the JSON round trip of `gen/normalize.mjs` and `gen/sanitize.mjs`.
- From plan 2, oracle tooling: `GIT_REDIRECTS` (`oracle/test/fetch.test.mjs`) duplicates `DROPPED_ENV`
  (`fetch-ts.mjs`); the `oracle-log-*` temp directory leaks on SIGINT or SIGTERM (`logs.mjs`); no test covers an
  absent trace or the trace calls of `run.mjs`, `list-tests.mjs` and `driver.mjs`; `captureLogs` resets the sink to
  `null`, not to the earlier sink; `loadModule` bundles are not log-isolated (say so in its doc);
  `server.test.mjs:63` reuses request id 5.
- From plan 2, `xtask parity`: the `UnclaimedVectorCase` text does not say whether the entry is missing, `todo` or
  another category; a key repeated inside one case's `ts_test` is accepted; no repository-level test covers the
  link problems; the case names written in `covered-by-vector` reasons are not checked against the vector files.
- From plan 2, Rust test support: `OracleError`'s `Debug` puts the stack on one line; oracle error messages hold
  the whole reply line, with no cap; `bmp_string` has a small modulo bias on its any-BMP part.
- From plan 2, vector and test coverage: no vector for a key above the `u32` range (`"4294967296"`); the case name
  `stringify:float-one` holds an integer; `constants/tests.rs:288` uses `unwrap_or_default` on the case input and
  reloads the vector file per lookup; the KSUID tests' prefix list differs from TS and 10 distinct event IDs are not
  checked; the `from_parts` proptest almost never reaches the out-of-range margins; `gen/handles.mjs` holds raw
  invisible characters (use `\uXXXX`); an exhaustive `toLowerCase` vector would pin the sentinel equivalence across
  ICU versions; an infallible KSUID encoder would remove an unreachable `Option` branch in `derive_session_id`; the
  tokens divisor test does not assert 3.5; the tags proptest rarely reaches the 50-tag cap or the 200-unit limit
  with astral characters; the normalize RD-24 check derives its expected value from `normalize` and
  `test:same-object-twice` needs a comment; truncation has no RD-24 case and no astral case in the last-resort
  cut, its tests repeat the `"[MaxProperties ~]"` literal, and two `expect` messages in `check_rd27` lack the case
  name; the sanitize proptest needs a comment on the inputs it leaves out, and `ts_event` does not enforce that it
  never makes a `toString` or `__proto__` key; `sanitize_parameters` also walks `structuredContent` (rename);
  redaction has no fixed case for a `""` key inside an array element, and its tests type the hook-field and
  restored-field lists again instead of importing them.
- Owner ruling R26 (2026-10-22): write the Rust pages for docs.agentcat.com. The site is Mintlify, in
  `agentcathq/agentcat-ui` under `apps/docs`. Add a Rust tab next to TypeScript, Python and Go in `quickstart.mdx`
  and in every `sdk/*.mdx` page that shows code (custom events, debug mode, tags and properties, identifying
  users, missing tools, stateless servers, tool-call context). Open it as a pull request to that repository; the
  owner approves it at that time.

- From plan 3: document that a tracked server needs a tokio runtime with the time driver (C32); that rmcp's
  `local` feature is not supported (C27); that Cargo feature unification can turn on serde_json's
  `arbitrary_precision` or `float_roundtrip` in a customer build (number text and RD-23 change with it); and that
  axum's `nest_service` strips the path prefix from `requestInfo.url`.
- From plan 3 (watch item): the egress guard does not cover worker threads, native addons or `dns.resolve*`.
  Revisit if TS main moves test work to worker threads.
- From plan 3: the harness reference server's `cfg(not(unix))` branch (`harness/rust-server/src/main.rs`) has been
  built only on Linux; the final PR's macOS and Windows runs (C17) build it.
- From the plan 3 final review: document (or cap) the registry (C19). It is per handle and handler type, keyed by
  tool name and never pruned, so it grows with per-tenant tool names; and on a server without `get_tool`, one
  session's listing sets the strip set of another session's calls.
- From the plan 3 final review: the CI `rmcp (<version>)` legs pin `rmcp-macros` to the exact rmcp version, so an
  rmcp patch without a matching `rmcp-macros` patch fails its leg (it fails closed). The remedy: take the highest
  `rmcp-macros` of the same minor.

- From the plan 4 final review (lifecycle hardening, one task): a redaction hook has no time budget, so five hung
  `redact_event` or `redact_sensitive_information` hooks stop delivery for that handle (TS parity); document it on
  both options, or add a budget that drops the event (never publishes it unredacted) with a log line. Pin the
  lifecycle edge cases with tests and doc lines: `flush` after a `shutdown` that timed out answers `true`; a
  cancelled async `flush` or `shutdown` stays in the worker's waiting list until the worker is idle; on a handle
  whose worker never started, the C12 "shutting down" line is dropped (`delivery/queue.rs`, `takes_lines`); `shutdown`
  twice; `flush` or `shutdown` before the first event. Add a deterministic test that `stop` awaits the aborted tasks
  before it answers (`worker.rs`), and a `Queue::close` method so `Delivery::drop` stops reaching into the queue
  state (`delivery.rs`).
- From the plan 4 final review: a customer cannot build an `Event` to unit-test a `redact_event` closure (it is
  `#[non_exhaustive]` with no constructor); add `Event::default()` or a `test-util` constructor.
- From the plan 4 final review: the `node` oracle child of the Rust tests (`test_support/oracle.rs`) inherits the
  test's stderr and is never killed or awaited, the likely source of an occasional nextest "leaky" test. Pipe its
  stderr and relay it, or kill it from a guard.
- From the plan 4 final review: document that a hook closure that holds an `AgentCat` clone makes a reference
  cycle, so the handle never drops and the last-drop drain never runs; and that the worker's `tracing` events do
  not reach a subscriber set with `with_default` on another thread.
- From plan 4 (tests): `from_ts` has no test for wrong-typed values (non-string tag values, a fractional or negative
  duration, tokens above `u32::MAX`, `null`, an unparsable timestamp); `logging/tests.rs` `no_home_is_silent`
  asserts nothing, `temp_home` never removes its directories, and the "caches the prefix" parity entry maps to a
  test that does not check caching; no test of the event-ID failure line (`process.rs`, needs a seam); no unit test
  of the real `Transport::process` path; no single test runs a failed-build publisher through the worker; no test of
  `Fake::events()` with zero handles, a failing `assert_tracked`, or an empty project on telemetry-only events; no
  direct proptest of `byte_size(event) == js::stringify(event).len()`; `queue_drop_line_comes_before_published_line`
  polls with a bounded sleep; the `Echo` tool of `tests/fake.rs` has no await, so only the hook phase of the
  concurrency test overlaps (TS also delays the tool).
- From plan 4 (code): `config.rs` is about 560 lines (the environment lookup can move to `config/env.rs`); the test
  helper `fn object` is defined in six test files; `publish_string` and `publish_server` repeat a ten-field
  destructure and the empty-project check; the untraced and degraded paths repeat the intercept branch
  (`call/run.rs`); the collect pass copies the untruncated event about three times (`process.rs`);
  `normalize_fields` clones the event once per depth and `own_entries` scans keys twice on its slow path, and no
  speed or allocation measurement exists.
- From plan 4 (parity mapping): `manifest.toml` "hanging resolveSessionId/eventTags/eventProperties do not delay the
  tool call" maps to a test that hangs only `identify` and `eventTags`; the three `rebuild-on-demand` keys cite
  `harness:pagination`, where RD-12 rules the rebuild; the three identity "re-resolve per request" keys are `todo`.
  Plan 5 brings `todo` to zero.
- From plan 4 (oracle generators): `queue.mjs` says its event input is built as `callWrap` builds it, but its error
  is `{message, platform}` and it skips the protocol tags; it advances the mock clock once after one `setImmediate`
  and `recorder.settled()` has no watchdog; the generator tests do not check family content; `bundle.mjs` still says
  "Imports the pure modules".
- From plan 4 (harness): the stdio transport's SIGKILL is not detected (C46 says any server; Rust stdio is caught
  only through the missing drained line); `exitDeadline` in `driver.mjs` covers both the client close and the
  SIGTERM wait, so its message overstates the server's time; no test of an HTTP server that exits cleanly without
  the drained line, and the drained and environment tests do not run on the 2026-07-28 lane; rust-against-rust A/A
  runs print informational `EVENT UNUSED RULING` lines; `loadRulings` accepts pin keys in a delta rulings file
  (refuse them with exit 2); `callCanaryChange` compares truncated summary text; no unit test runs the reference
  server's `redactEvent` hook, and no scenario uses `throws`; two README lines (the `diff.mjs` wrapping, "falls back
  to the `duration`").
- From plan 4: the serde_json feature self-check's float probes were proven on serde_json 1.0.154 only; prove them
  at the floor (1.0.127).
- From the plan 4 final review: plans 3 and 4 were verified on Linux only, because GitHub Actions has refused to
  start jobs since `b78ca55` (billing). Before the PR, run the full CI once, with the Windows and macOS test jobs
  (a Windows-only test panic slipped past the Linux gate in plan 4).
- From the plan 4 fix-wave re-review: `event/assemble.rs` has a private `redact_event` that only prepares `Debug`
  output (rename it); `testing.rs` says that after the `Fake`'s flushes "the drop has nothing left to wait for",
  which is false after a flush that timed out, and names only `AgentCat::new` as a call that sets the log
  destination (`AgentCat::from_env` does too); `delivery.rs:24` is a doc line of 101 columns.
