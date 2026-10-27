# Items carried forward

Reviews of plans 1 and 2 found these items and deferred them. Each plan's final review sorted them into a later
plan. The person who writes that plan reads this list, checks each item against the code as it stands, and either
puts it in a task or records why not. Line numbers are as of `b9b91e0` (plan 1 items), `e685d14` (plan 2 items),
`dd67f1d` (plan 3 items) or `da0b53d` (plan 3 final review items) and can drift. Plan 2 closed every item that plan 1 carried to it.

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
