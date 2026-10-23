# Items carried forward

Reviews of plans 1 and 2 found these items and deferred them. Each plan's final review sorted them into a later
plan. The person who writes that plan reads this list, checks each item against the code as it stands, and either
puts it in a task or records why not. Line numbers are as of `b9b91e0` (plan 1 items), `e685d14` (plan 2 items)
or `dd67f1d` (plan 3 items) and can drift. Plan 2 closed every item that plan 1 carried to it.

## Plan 3 (interception, Rust reference server)

Plan 3 closed or moved every item carried to it. The moved items are below, marked "From plan 3".

## Plan 4 (delivery and lifecycle)

- The mock API keeps parsed JSON only, so `1` and `1.0` and integers above 2^53 look the same. Keep the raw body
  text for the event comparison and C04.
- The stdio exit wait is bounded by the MCP SDK's `close()` (SIGTERM after 2 s), not by `settleTimeoutMs`. With
  RD-09 and R06 (5 s drain), a draining Rust server can be killed. Design the stdio close path.
- An HTTP reference server that ignores SIGTERM only gets a stderr line, and the run still exits 0. Enforce the
  protocol when the Rust server must drain on SIGTERM.
- Found while writing plan 2: build the TS event shape (`pipeline::TsEvent`) in TS's in-memory key order. The
  order decides which of two strings of equal length the last-resort cut of the size guard takes first. Record
  that order from a TS probe.
- Found while writing plan 2: run the async string-redaction hook through `pipeline::redact::redact_strings` in
  two passes (collect the strings with a recording closure, await the hook for each in order, replay the results).
  `redact_strings` is the only definition of the walk order and the protected keys; do not copy its rules.
- Found while writing plan 2: `handles::new_event_id()` returns `None` where TS throws. Drop the event with TS's
  delivery-failure log line.
- Moved from plan 2: `scripts/check.sh` runs `cargo deny check`, and the CI deny action adds `--all-features`.
  Add it when the crate gets features.

- From the plan 2 final review: the wire body. `assert_js_eq` compares `js::stringify` on both sides, so no plan 2
  test catches a serializer that uses `serde_json::to_vec`. That serializer writes `1.0`, `-0.0`, exact integers
  above 2^53, and index-like keys of `tags` and `properties` in insertion order; TS writes `1`, `0`, rounded
  integers, and index keys first. Build the body with `js::stringify` rules, or record an RD.
- From the plan 2 final review: the size guard measures `js::stringify` bytes, so the body that the SDK sends must
  be that same text.
- From the plan 2 final review: TS merges the sanitize and truncate results into the event with `Object.assign`;
  Rust replaces the map. The two are equal only while no stage removes a key. Pin it with a test when the queue
  composes the stages.
- From the plan 2 final review: the event `timestamp` string must have the exact `toISOString` form (three
  millisecond digits and `Z`).
- From plan 2 (performance): `truncate_event` copies the event in `limit_fields` and again for each depth in
  `normalize_fields`, and `byte_size` writes the whole event as text only to count its bytes (up to about 20
  encodings of a 400 KB event). `js::own_entries` allocates a `Vec` for each object. Pass the event by value, count
  bytes with a writer, and give `own_entries` a path that does not allocate.
- From plan 2 (RD-24): Rust's restore keeps a top-level `__proto__` key of an event hook's result, so its strings
  reach the customer's redactor; TS turns that key into the prototype and never redacts them. A typed event has no
  such key; keep it that way, or record the extra redactor calls in RD-24.

- From plan 3, because events are compared in plan 4: `extra-headers-only` in `harness/src/normalize.mjs` hides D06
  (every header verbatim) and D34 (the shape of `extra`). Keep every header except a named volatile set, keep the
  key set of `extra`, keep `requestInfo.url`, and record the known shape difference as an RD-07 ruling. The
  `extra` key order differs from TS v1 (`_meta` before `requestId`), and `request.params` puts `_meta` last
  where B-082 says "as received" (rmcp's own client sends it first): align each, or name it in RD-07.
- From plan 3, because events are compared in plan 4: RD-04: a Rust timestamp with zero milliseconds becomes
  `<bad-timestamp:…>` about once per 1000 events. Add an optional-ruling flag or a class token for RFC 3339 without
  a fraction.
- From plan 3, because events are compared in plan 4: placeholders do not tell derived `ses_` IDs from minted
  ones; the event sort tie-breaker uses the raw duration (sort on the normalized body); the same-millisecond sort
  limit needs a line in the scenario-authoring rules.
- From plan 3: build `parameters.request` in TS's key order, recorded from a TS probe.
- From plan 3: the event builder drops rmcp's `resultType: "complete"` from `CallRecord.response` (TS events have
  none).
- From plan 3: insert `CallContext` on the `AGENTCAT_ENABLED=false` pass-through too (C29).
- From plan 3: hook settling reads `CallRecord.pending`; it must abort or bound a hook that never finishes (today
  it stays a detached task). Document that a panicking hook prints through the process panic hook.
- From plan 3: `capture_error` renders the backtrace on the tool's task when `RUST_BACKTRACE` or
  `RUST_LIB_BACKTRACE` is set. Keep the `Backtrace` and render it in the delivery pipeline.
- From plan 3: the Rust reference server's `emit_custom` publishes a custom event, and its
  `redactSensitiveInformation` profile key reaches the SDK option. It passes `AGENTCAT_API_URL` to the SDK as the API
  base URL (the TS servers' second guard; the `#[expect(dead_code)]` on `Startup.api_url` does not fire if this is
  forgotten), and its HTTP modes drain the SDK queue on SIGTERM before they exit (today they exit at once).
- From plan 3: `harness/src/matrix.mjs` stops passing `--expect-publishes 0` for tracked Rust runs.
- From plan 3: `harness/rulings.json` holds delta rulings only. When event rulings for `diff.mjs` join, split the
  file or make the full-matrix unused-ruling check count only rulings that can match `$.delta.*`
  (`matrix.mjs`).
- From plan 3: the feature-unification self-check (it logs when serde_json's `arbitrary_precision` or
  `float_roundtrip` is on in a customer build).
- From plan 3: check the four unused `cargo deny` licenses again after reqwest and rustls join, and remove each
  license that is still unused.

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
