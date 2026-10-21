# Items carried forward from plan 1

Reviews of plan 1 found these items and deferred them. The final review sorted each one into a later plan. The
person who writes that plan reads this list, checks each item against the code as it stands, and either puts it
in a task or records why not. Line numbers are as of `b9b91e0` and can drift.

## Plan 2 (pure core)

- `oracle/src/vectors/run.mjs` and the generators: one strict serializer for every generator. It throws on
  values that JSON cannot carry (`undefined`, `RegExp`), and it rejects or tags NaN, Infinity and -0 before
  float vectors arrive.
- Require unique case names in every vector set. `oracle/src/drift/compare.mjs` keys cases by name, so a
  duplicate hides drift. Test the drift compare's missing-file branch and the `removed` direction.
- `agentcat-sdk/tests/vector_integrity.rs` passes with zero vector files: assert that at least one set exists.
- The L2 oracle error reply carries `error.stack` with bundle paths. Send the message and the stack separately,
  because D40 needs the TS message verbatim.
- `oracle/src/bundle.mjs`: a symlinked checkout turns off the `import.meta` rewrite. Use `realpathSync` on the
  source directory.
- Oracle tests share cache state, and the test `git()` helper keeps `GIT_DIR` and `GIT_INDEX_FILE`. Scrub them
  as `run()` does.
- `ORACLE_SHA` redirects every consumer without a trace. Print one stderr line when it is set, record the
  effective SHA in `report.json`, and let `support/oracle.rs` compare the server's SHA with `oracle/oracle.lock`.
- Parity: `ported` with `rust = []` gives a misleading message. Manifest order is not enforced, and
  `--init-manifest` cannot append new keys after a SHA bump (add a sorted check or a print-missing mode).
- Pending rulings: `harness/src/diff.mjs` rejects any ruling key except path, reason and deviation, and
  `xtask` rejects a register row written as `| RD-22 (pending) |`. Before the first pending ruling, allow
  `"pending": true` in `rulings.json`, and mark pending in the Why column of design section 9.
- The `cargo hack --no-dev-deps` and minimal-versions steps warn about unused dependencies while the SDK library
  is empty. Make sure that the warnings are gone once the SDK uses its dependencies.
- `oracle/src/egress-guard.mjs`: a custom `lookup` with the host name `localhost` passes the guard. Say so in the
  file header.
- `CONTRIBUTING.md:14` and the `check.sh` hint say Node 22 and later include corepack. Node 25 and later do not.

## Plan 3 (interception, Rust reference server)

- Before the Rust server joins: `extra-headers-only` in `harness/src/normalize.mjs` hides D06 (every header
  verbatim) and D34 (the shape of `extra`). Keep every header except a named volatile set, keep the key set of
  `extra`, keep `requestInfo.url`, and record the known shape difference as an RD-07 ruling.
- Owner ruling R20: make `ts-v2` the default reference server on both lanes, generate
  `harness/contract/tool-contract.json` from v2, and reword `harness/README.md` (the "Tools" paragraph that omits
  `execution`). The planned automated v1-vs-v2 diff on the 2025-11-25 lane is no longer needed as an answer key.
- The nightly diff of pinned runs against TS main counts unused rulings as drift. Give it a TS-only rulings file
  before Rust rulings arrive.
- RD-04: a Rust timestamp with zero milliseconds becomes `<bad-timestamp:…>` about once per 1000 events. Add an
  optional-ruling flag or a class token for RFC 3339 without a fraction.
- The driver records but does not enforce the negotiated protocol version against `--lane`. Exit 1 on a
  fallback. The initialize transcript entry records only `protocolVersion`; add `serverInfo`, `capabilities`
  and `instructions` so the MCP-wire delta sees whether `Tracked<H>` forwards `get_info`.
- Driver and scenarios: `saveHandleAs` with no handle sends the literal `${S1}` (fail the run);
  `checkScenario` accepts unknown keys; unknown `expectPublishes` lane keys are ignored (reject them) and the
  `<transport>` fallback has no test; the sort tie-breaker uses the raw duration (sort on the normalized body);
  the same-millisecond sort limit needs a line in the scenario-authoring rules; placeholders do not tell
  derived `ses_` IDs from minted ones; `diff.mjs` reports only `.length` for arrays of different length, and
  A/A tests (d) and (e) depend on (a).
- On a driver error, `killAll` cannot reach SDK-spawned stdio servers.
- The contract test compares tool names and input schemas only. The Rust server also loads descriptions and
  output schemas.
- Nightly reporting: `head -c` can cut the "Run:" link and split a UTF-8 character (truncate first, then append
  the link); an A/A failure reports no output tail; the drift job prints TS-main output to the log without
  stopping workflow commands (the job has `contents: read` and no secrets).
- The egress guard does not cover worker threads, native addons or `dns.resolve*`. Revisit if TS main moves
  test work to worker threads.
- Found while writing plan 2: TS puts array-index keys of `structuredContent` (for example `"0"`) before
  `mcp_session` on the wire, because JavaScript orders those keys first. Decide how the `mcp-session-first` rule
  (R07) and the Rust mirror treat them.
- Found while writing plan 2: `handles::new_session_id()` returns `None` where TS throws. Take TS's "tracing
  failed" path for that call.
- Found while writing plan 2: harness scenarios with decimals of more than 15 significant digits hit RD-23.
  Avoid them, or add a ruling.

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
  two passes (collect the strings, await the hook for each in order, replay the results).
- Found while writing plan 2: `handles::new_event_id()` returns `None` where TS throws. Drop the event with TS's
  delivery-failure log line.
- Moved from plan 2: `scripts/check.sh` runs `cargo deny check`, and the CI deny action adds `--all-features`.
  Add it when the crate gets features.

## Plan 5 (finish)

- `agentcat-sdk/Cargo.toml` has no `homepage`; add it with the publish metadata.
- `ci.yml`'s `cancel-in-progress: true` also cancels runs on `main`. Cancel only PR runs once the mutation gate
  needs the previous green commit.
- Windows: the clone-directory rename needs a retry under antivirus, and `shell: true` with an argument array
  warns DEP0190 on Node 24 and later.
- The msrv job runs nextest only. Add `cargo test --workspace --doc` on 1.88 before the README doctests land.
- `harness/src/contract/extract-schema.mjs` does not compile its output with the validator's strict ajv
  settings.
- Ingest replay README: list `corepack pnpm install` and the first-run oracle build as prerequisites; reword the
  HOME isolation claim (a new HOME hides only HOME-resolved configuration); say precisely that the DNS guard
  covers call-time lookups through the socket module; note that `poetry install` runs with the real HOME.
- After the final PR merges (R21): dispatch the nightly once, so the issue-filing path runs for real.
