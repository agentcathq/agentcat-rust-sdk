# Plan 5: Finish (docs, examples, API and quality gates, parity zero, backend proof, the PR) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `agentcat-sdk` ready for its one review PR: green CI on Linux, macOS and Windows; the lifecycle edge cases pinned; zero `todo` in the parity manifest; customer docs, examples and a CHANGELOG; the public API, coverage and mutation gates of design §11; the ingest replay and the dashboard demo as backend proof; the docs.agentcat.com pages; and the PR from `rewrite` into `main`.

**Architecture:** No new SDK subsystem. The SDK gets one public addition (`impl Default for Event`), small internal refactors and many tests. The gates are new CI jobs and `scripts/check.sh` steps backed by `xtask` subcommands (`public-api`, `coverage`) and `.cargo/mutants.toml`. The harness gets an SSE lane. Backend proof runs locally: the ingest replay through a clone of `agentcat-server`, and the dashboard demo into project "Testing", which the controller reads back with the AgentCat MCP server.

**Tech Stack:** Rust 1.88+ (edition 2024); rmcp 3.x (floor 3.0.0); `agentcat-api` `=0.0.3`; cargo-nextest, cargo-llvm-cov 0.9, cargo-mutants 27, cargo-public-api 0.52 (nightly); Node 22 (oracle, harness); Python 3.12 with Poetry 2 and Docker (ingest replay, local only); Mintlify MDX (docs site).

**Spec:** `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (sections 2, 3, 5, 8, 10, 11, 12 milestone 6, 13). Practices: `docs/research/04-rust-sdk-practices.md` sections 10 (documentation), 15 (metadata), 16 (public API), 20.3 (CI gates). Gates: `docs/research/06-verification-strategy.md` G5, G6, G9, G11. Decisions: `docs/DECISIONS.md` (R04, R12, R14 to R17, R21, R26, C08, C17, C19, C27, C32, D41). Deferred items: `docs/superpowers/plans/carry-forward.md`, section "Plan 5" (every item maps to a task below; the map is at the end of this plan).

## Global Constraints

- Branch `rewrite`; conventional-commit subjects; no AI attribution lines; push only to `origin rewrite`. Two exceptions: Task 18 pushes branch `docs/rust-sdk` to `agentcathq/agentcat-ui` and opens a draft PR there (R26); Task 19 opens the PR from `rewrite` into `main` (R15). Never force-push; never push to `main` or tags; never change repository settings.
- The repository is public since 2026-10-27. Nothing committed may hold a credential, a token, a private hostname or customer data. Project IDs in docs are placeholders (`proj_…`), except the "Testing" project ID that R12 names.
- `edition = "2024"`, `rust-version = "1.88"`. Library code: no `unwrap`, `expect`, `panic!`, `todo!`, printing; every lint exception is `#[expect(<lint>, reason = "...")]`. The `testing` module's assertion helpers keep their `clippy::panic` exceptions.
- `#![forbid(unsafe_code)]` stays. rmcp's `#[doc(hidden)]` items may be used where they make the code cleaner (D21); private fields may not.
- Public API: the only new public item is `impl Default for Event` (C49). Any other change to the public API is a finding. `reqwest`, `tokio`, `chrono` and `agentcat-api` types never appear in a public signature (C08).
- Dependencies: library dependencies go into `[workspace.dependencies]` with `default-features = false`; floors pass `cargo +nightly minimal-versions check --direct --workspace`. Test-only and example-only dependencies go into the crate's `[dev-dependencies]`. No new library dependency.
- `serde_json` keeps `preserve_order` (R07); nothing turns on `float_roundtrip` (R22) or `arbitrary_precision`.
- Expected values come from TS only: a vector file, a live oracle call, or a harness run of a TS reference server. A Rust-only test asserts a shape, a Rust-only path or a deviation, and names the deviation or decision ID. Vector files are never edited by hand.
- Read TS code only in `oracle/.cache/ts-9b56e9e920f35f7c37fc2a63ce6520a9ec97edea`. Never read `~/agentcat-typescript-sdk` or attempt-1 files and tags. Never modify `~/agentcat-server` (Task 16 only clones it).
- No test, harness run, example check or ingest replay reaches a production AgentCat endpoint. The only production publish is the dashboard demo, run by the controller in Task 19, to project "Testing" (`proj_2yAR5QHJ3CE3icTpJlKJqu1ZiXG`, R12). Never print or copy tokens or credentials. Never disable the egress guard.
- Never panic, never block the host runtime on the call path, never print (design §7.3).
- Time-dependent tests shrink the budgets through `DeliveryConfig`; a test that needs real time keeps its wall-clock wait under 3 s.
- Parity rule (unchanged from plan 4): a pure input-to-output TS test is a vector case (`n/a`, `covered-by-vector:<file>.json`); a behavior pinned by a named Rust test is `ported` with that name; a harness scenario is `ported` with `rust = ["harness:<scenario>"]`; a TS mechanism Rust lacks is `n/a` with its RD or category. A mapping must assert the TS test's behavior, not only touch the same code. Run `cargo run -q -p xtask -- parity` (exit 0) after each mapping.
- Docs style: customer docs (README, rustdoc, examples, CHANGELOG, MDX pages) use the plain style of `CONTRIBUTING.md`: short sentences, active voice, present tense, one term per concept, every code sample compiled or doctested.
- Disk: the machine has about 13 GB free and `target/` holds about 35 GB. Before a coverage or mutation run, make sure at least 15 GB are free; delete `target/debug/incremental`, `target/llvm-cov-target` or run `cargo clean` (the gate rebuilds them). Never delete `oracle/.cache` or `node_modules`.
- Before each commit, run the full local gate: `scripts/check.sh` must exit 0. CI mirrors it (`.github/workflows/ci.yml`); a change to `scripts/check.sh` gets the same change in the matching CI job. The controller pushes to `origin rewrite` after each task's review passes and watches the CI run (`gh run watch`); a red run on any of the three systems sends the task back to its fix loop. Tasks 1, 14 and 15 push themselves, because their steps need CI results.
- Model per task (owner rule: Opus 5.5 for the hardest half, Sonnet 5.5 for the rest; reviewers use the implementer's model): stated at the top of each task.

## Decisions this plan makes

Task 19 records each one in `docs/DECISIONS.md`. A reviewer who disagrees raises it; the controller rules.

- **C48 Three operating systems on every push.** The repository is public, so hosted runners cost nothing: the `test` job runs on `ubuntu-latest`, `macos-latest` and `windows-latest` for every push and pull request, and builds `harness-ref-server` on each. This replaces C17's "the final PR and nightly add macOS and Windows". `cancel-in-progress` applies to pull-request runs only, so every push to `rewrite` and `main` keeps a finished run for the mutation gate (C53).
- **C49 `Event` implements `Default`.** All `Option` fields `None`, empty `id`, `project_id` and `event_type`, `timestamp` = `SystemTime::UNIX_EPOCH`. A customer builds one with `Event::default()` and sets fields, to unit-test a `redact_event` closure. Not behind `test-util`: it is harmless and the hook is not a test feature.
- **C50 Redaction hooks keep TS's unbounded time.** TS has no budget for `redactSensitiveInformation` or a `redactEvent` hook; Rust keeps that (parity, no RD). The rustdoc of both `Options` setters says that a hook that never completes holds one of the 5 delivery slots of the handle for good, and that five such hooks stop its delivery.
- **C51 Public API gate.** `agentcat-sdk/public-api.txt` is the committed output of `cargo +nightly public-api -p agentcat-sdk --all-features -ss` (simplified); `cargo run -p xtask -- public-api` regenerates it with `--bless` and otherwise diffs it (exit 1 on any difference). The same command checks every crate path in that text against an allowlist in `xtask/src/public_api.rs`: `std`, `core`, `alloc`, `agentcat_sdk`, `rmcp`, `serde_json`, and `http` only if the current API uses it. No `cargo-check-external-types`: it pins its own nightly per release.
- **C52 Coverage floors.** `cargo llvm-cov nextest -p agentcat-sdk --all-features` with test files excluded (`--ignore-filename-regex '(/tests\.rs$|/test_support/|/tests/)'`); `cargo run -p xtask -- coverage <summary.json>` reads `--json --summary-only` output and fails below 90 % lines for the crate, or below 95 % lines for the pure set: `constants.rs`, `js.rs`, `handles/**`, `tokens.rs`, `inject.rs`, `pipeline/{tags,normalize,truncate,sanitize,redact}.rs`. Every push runs it; the nightly also reports branch coverage (`--branch`, nightly toolchain; report only, target 85 %).
- **C53 Mutation gates.** `.cargo/mutants.toml` sets nextest, `--all-features`, the SDK crate only, and timeouts. Every exclusion is an `exclude_re` entry with a comment that gives its reason (no `mutants` crate dependency in the published crate). Every push and pull request runs `cargo mutants --in-diff` on the diff since the last commit of the same branch whose CI run succeeded (GitHub API, `actions: read`; on a branch with no green run, the last commit only); zero missed mutants. The nightly runs the full set in 8 shards and fails below 90 % caught of viable mutants. `scripts/check.sh` runs the diff gate against `origin/rewrite`.
- **C54 Examples.** `agentcat-sdk/examples/stdio.rs`, `streamable_http.rs` (stateful, rmcp's default SSE responses), `stateless_http.rs` and `hooks.rs` (identify, tags, properties, both redaction hooks, a custom event, `capture_error`). Each builds with the crate's dev-dependencies and logs only to stderr. `harness/src/examples-check.mjs` runs `stdio`, `streamable_http` and `hooks` against the local mock: every publish arrives, stdout carries only JSON-RPC lines, and the server exits 0 within 5 s after the client closes (stdio EOF or SIGTERM) with its last event delivered.
- **C55 Docs layout.** `agentcat-sdk/src/lib.rs` is the guide (what the SDK records, quickstarts for stdio and streamable HTTP, `Options`, environment variables, features, shutdown, logging and stdout, MSRV policy, known limits, links to examples). `agentcat-sdk/README.md` is short (badges, install, one quickstart, links) and doctested through `#[cfg(doctest)] #[doc = include_str!("../README.md")] pub struct ReadmeDoctests;`. A root `README.md` describes the repository (the crate, the verification layers, how to contribute) for GitHub.
- **C56 CHANGELOG.** One `## [Unreleased]` section lists the first release's content under `Added` (Keep a Changelog 1.1.0) and links the deviation register. The release pass (second pass, R04) names the version.
- **C57 Docs site PR stays a draft.** The Rust tabs on docs.agentcat.com show `cargo add agentcat-sdk`, which works only after the crates.io release (R04 puts the release in the second pass). The PR to `agentcat-ui` stays a draft until then; its body says so.
- **C58 Dashboard demo.** `harness/demo/run.mjs` runs one fixed scenario (`harness/demo/scenario.json`) through the `hooks` example (Rust) and through `harness/demo/ts-server.mjs` (the same tools and options on the TS SDK at the pinned SHA), over stdio with the TS client. `--mock` sends to the local mock (part of the gate); `--prod` sends to the default AgentCat API and requires project `proj_2yAR5QHJ3CE3icTpJlKJqu1ZiXG`, refuses any other project, refuses when `CI` is set, and writes the two session IDs to `harness/out/demo/sessions.json`. The controller runs `--prod` and reads the events back (Task 19).
- **C59 SSE lane.** A fourth transport, `http-sse`: stateful streamable HTTP with SSE responses (TS `enableJsonResponse: false`, rmcp `with_json_response(false)`, rmcp's default). It runs every scenario that runs on `http`, on both lanes.
- **C60 Ingest replay of every Rust capture.** `harness/ingest/replay.sh <capture>...` runs the README steps once for a list of capture files; `test_ingest_capture.py` reads every file named in `CAPTURE_FILES` (colon-separated). Task 16 replays the Rust tracked captures of every scenario, transport and lane of one full matrix run.

New pending deviations: none planned. A task that finds an unforced difference adds a pending RD row (RD-39 onward) and its ruling; Task 19 brings them to the owner (R17).

## Review Focus

1. **A stdio customer whose logs reach stdout.** A person who copies the README or an example expects a working stdio server: the SDK and the example code write nothing to stdout, and the docs tell them to send their own `tracing` output to stderr. Pinned by Task 10: `examples-check.mjs` fails on any stdout line that is not JSON-RPC.
2. **A streamable HTTP server with rmcp's default SSE responses.** Most customers keep `json_response` off. A person expects the same tool lists, results and events as with JSON responses. Pinned by Task 8: the `http-sse` lane runs every `http` scenario in the matrix.
3. **A process that ends right after the client disconnects, with no `shutdown` call.** A person expects the last call's event delivered by the last-drop drain within 5 s. Pinned by Task 10: `examples-check.mjs` closes the client right after the last call and requires that call's publish before the exit.
4. **Copied docs code on the floor versions.** A person on rmcp 3.0.x or Rust 1.88 expects the README, the rustdoc examples and the examples to compile. Pinned by Task 11 (doctests on the `msrv` job) and Task 10 (`cargo build --examples` and `cargo test --doc` in every `rmcp (<version>)` leg).
5. **A customer unit test of a redaction hook.** A person expects to build an `Event` and run their `redact_event` closure on it, and to test with `Fake`. Pinned by Task 2: integration test `tests/customer_redaction.rs` (outside the crate, so `#[non_exhaustive]` applies) with `customer_builds_event_for_redact_hook` and `fake_records_redacted_event`.

---

### Task 1: Three operating systems: oracle locking, bundle race, CI matrix

Model: Opus 5.5.

**Files:**
- Modify: `oracle/src/bundle.mjs`, `oracle/src/lock.mjs`, `oracle/src/fetch-ts.mjs`, `oracle/test/lock.test.mjs`, `oracle/test/bundle.test.mjs` (create if missing), `.github/workflows/ci.yml`, `.github/workflows/nightly.yml`, `scripts/check.sh` (only if a CI command changes)

**Interfaces:**
- Produces: `oracle/src/fsretry.mjs` with `renameWithRetry(from, to, {attempts = 8, delayMs = 50})`: retries `EPERM`, `EBUSY`, `EACCES` with doubling delays; when the error persists and `to` exists with the same bytes as `from`, it removes `from` and returns (another process won the race); otherwise it throws the last error with both paths.

- [ ] **Step 1: Write the failing tests.** `oracle/test/bundle.test.mjs`: `parallel bundle builds converge` starts 6 child processes that build the same bundle into one empty cache directory at once and asserts every child exits 0 and the bundle loads. `oracle/test/lock.test.mjs`: `claim holder is identified by token, not pid`; `unreadable claim is broken after the stale timeout`; `sweep removes .new-*, .stale-* and .done-* leftovers older than the stale timeout`; `interleaving B (release during break) is deterministic` and `interleaving C (two breakers)`; `a changed host name does not make a live claim stale` (macOS renames hosts); `rename retries EPERM then succeeds` (a stub `fs` that fails twice).
- [ ] **Step 2: Run them to see them fail.** Run: `corepack pnpm --filter oracle test`. Expected: FAIL.
- [ ] **Step 3: Implement.** `bundle.mjs` (line 145, `renameSync` of `core.mjs.tmp-<pid>`) and every write-then-rename in `lock.mjs` (lines 113-114, 137, 163, 206-210) go through `renameWithRetry`; `lock.mjs` compares claim tokens, treats an unreadable claim as stale after the timeout, retries the break path on `EPERM`/`EBUSY`/`EACCES` (lines 158-161) and sweeps leftovers. `fetch-ts.mjs` renames the clone directory with `renameWithRetry`, and passes a command string (no argument array) where it spawns with `shell: true` (DEP0190).
- [ ] **Step 4: CI matrix (C48).** `ci.yml`: the `test` job matrix is always `["ubuntu-latest", "macos-latest", "windows-latest"]` with `fail-fast: false`, and builds `harness-ref-server` (`cargo build -p harness-ref-server`) on each; `concurrency.cancel-in-progress` is `${{ github.event_name == 'pull_request' }}`. The `rmcp (<version>)` legs pin `rmcp-macros` to the highest `rmcp-macros` of the same minor (`scripts/rmcp-versions.mjs` gains the lookup and its test), not to the exact rmcp patch.
- [ ] **Step 5: Run the gate, commit, push, watch CI.** Run: `scripts/check.sh` (exit 0). Push, then `gh run watch` the CI run and the nightly push run: every job green, `rust (windows-latest)` included.

```bash
git add oracle .github scripts
git commit -m "fix(oracle): retry renames and converge parallel bundle builds on Windows; ci: test on three systems on every push"
git push origin rewrite
```

---

### Task 2: Lifecycle hardening and a constructible `Event`

Model: Opus 5.5.

**Files:**
- Modify: `agentcat-sdk/src/delivery.rs`, `agentcat-sdk/src/delivery/queue.rs`, `agentcat-sdk/src/delivery/worker.rs`, `agentcat-sdk/src/delivery/tests.rs`, `agentcat-sdk/src/event/public.rs`, `agentcat-sdk/src/event/tests.rs`, `agentcat-sdk/src/config.rs` (rustdoc), `agentcat-sdk/src/client.rs` (rustdoc), `agentcat-sdk/src/logging.rs` (rustdoc)
- Create: `agentcat-sdk/tests/customer_redaction.rs` (`required-features = ["test-util"]` in `Cargo.toml`)

**Interfaces:**
- Produces: `impl Default for Event` (C49). `Queue::close(&self) -> usize` (crate-private: marks the queue closed and returns the number of pending events it abandons); `Delivery::drop` calls it and no longer reads the queue state.

- [ ] **Step 1: Write the failing tests.** In `delivery/tests.rs`: `flush_after_timed_out_shutdown_returns_true`, `cancelled_flush_leaves_waiting_list_when_worker_idles`, `cancelled_shutdown_leaves_waiting_list_when_worker_idles`, `shutdown_twice_second_returns_true`, `flush_before_first_event_returns_true_without_thread`, `shutdown_before_first_event_returns_true_without_thread`, `shutting_down_line_dropped_when_worker_never_started` (the C12 line; `queue.rs` `takes_lines`), `stop_awaits_aborted_tasks_before_answering` (deterministic: a task whose drop sets a flag; `stop` answers only after the flag is set). In `event/tests.rs`: `default_event_is_empty` (every option `None`, `timestamp == UNIX_EPOCH`). In `tests/customer_redaction.rs`: `customer_builds_event_for_redact_hook` (builds `Event::default()`, sets `parameters`, runs a closure of the `redact_event` shape, asserts the result) and `fake_records_redacted_event` (a `Fake` handle with a `redact_event` that replaces a secret; `fake.events()` holds the redacted value).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p agentcat-sdk --all-features -E 'test(/twice|before_first|cancelled_|timed_out|stop_awaits|never_started|default_event|customer_|fake_records/)'`. Expected: FAIL (compile errors for `Default` and `close`; behavior tests that already pass stay as pins).
- [ ] **Step 3: Implement** `impl Default for Event` and `Queue::close`; fix any behavior a test exposes. Write the rustdoc lines: C50 on `Options::redact_sensitive_information` and `Options::redact_event`; on `AgentCat`, that a hook closure holding an `AgentCat` clone makes a reference cycle, so the handle never drops and the last-drop drain never runs (call `shutdown` before the process ends, or let the hook capture only what it needs; a tool reaches the handle through `CallContext`); on `LogDestination::Tracing`, that the worker thread emits the events, so a subscriber set with `tracing::subscriber::with_default` on another thread does not see them (use a global subscriber).
- [ ] **Step 4: Run them to see them pass.** Same command. Expected: PASS.
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add agentcat-sdk
git commit -m "feat(event): Event implements Default; test(delivery): pin the lifecycle edge cases; refactor: Queue::close"
```

---

### Task 3: Code cleanup from the plan 4 reviews

Model: Opus 5.5.

**Files:**
- Create: `agentcat-sdk/src/config/env.rs`, `agentcat-sdk/src/test_support/json.rs`
- Modify: `agentcat-sdk/src/config.rs`, `agentcat-sdk/src/custom.rs`, `agentcat-sdk/src/call/run.rs`, `agentcat-sdk/src/pipeline/process.rs`, `agentcat-sdk/src/pipeline/normalize.rs`, `agentcat-sdk/src/js.rs`, `agentcat-sdk/src/event/assemble.rs`, `agentcat-sdk/src/testing.rs`, `agentcat-sdk/src/delivery.rs`, the six test files that define `fn object`

**Interfaces:**
- Produces: `config::env` (the environment lookup of `from_env` and `AGENTCAT_ENABLED`, moved unchanged); `test_support::json::object(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value>` replacing the six local copies.

- [ ] **Step 1: Refactor without behavior change.** Move the environment lookup to `config/env.rs`. Share one destructure and the empty-project check between `publish_string` and `publish_server` (`custom.rs`). Share the intercept branch of the untraced and degraded paths (`call/run.rs`). Make the collect pass borrow the untruncated event instead of copying it (`process.rs`); make `normalize_fields` clone once, not once per depth; make `own_entries` scan keys once on its slow path (`js.rs`). Rename the private `redact_event` of `event/assemble.rs` to `debug_redacted` (it prepares `Debug` output). In `testing.rs`, say that a flush that timed out leaves events for the drop, and name `AgentCat::from_env` beside `AgentCat::new` as calls that set the log destination. Wrap `delivery.rs:24` to 100 columns.
- [ ] **Step 2: Run the tests.** Run: `cargo nextest run --workspace --all-features && cargo run -q -p xtask -- parity`. Expected: PASS with the same test count as before the task; vectors and harness unchanged.
- [ ] **Step 3: Run `scripts/check.sh`, then commit.**

```bash
git add agentcat-sdk
git commit -m "refactor: split the environment lookup, share test and custom-event helpers, drop redundant event copies"
```

---

### Task 4: Pure-core test gaps and Rust test support

Model: Sonnet 5.5.

**Files:**
- Modify: `oracle/src/vectors/gen/*.mjs` (new cases), `agentcat-sdk/tests/vectors/*.json` and `MANIFEST.sha256` (generated), `agentcat-sdk/src/test_support/oracle.rs`, `agentcat-sdk/src/test_support/strategies.rs`, `agentcat-sdk/src/constants/tests.rs`, `agentcat-sdk/src/handles/ksuid/tests.rs`, `agentcat-sdk/src/handles/ksuid.rs`, `agentcat-sdk/src/handles/mod.rs`, `agentcat-sdk/src/tokens/tests.rs`, `agentcat-sdk/src/pipeline/{tags,normalize,truncate,sanitize,redact}/tests.rs`, `agentcat-sdk/src/pipeline/sanitize.rs`, `oracle/src/vectors/gen/handles.mjs`

- [ ] **Step 1: Add the vector cases** (generated by the real TS modules): a key above the `u32` range (`"4294967296"`); a real float for `stringify:float-one` (rename the integer case `stringify:integer-one`); an exhaustive `toLowerCase` table over the BMP sentinel set; truncation cases with an RD-24 `__proto__` key and an astral character at the last-resort cut. Replace raw invisible characters in `gen/handles.mjs` with `\uXXXX` escapes. Run: `node oracle/src/vectors/run.mjs && node oracle/src/vectors/run.mjs --check`. Expected: exit 0.
- [ ] **Step 2: Write the Rust tests and fixes:** `constants/tests.rs:288` reads the case input with an error naming the case (no `unwrap_or_default`) and loads the vector file once; the KSUID tests use TS's prefix list and check 10 distinct event IDs; the `from_parts` proptest weights the out-of-range margins; an infallible KSUID encoder removes the unreachable `Option` branch of `derive_session_id`; `tokens` asserts the divisor 3.5; the tags proptest weights the 50-tag cap and the 200-unit limit with astral characters; the normalize RD-24 check takes its expected value from the vector, not from `normalize`, and `test:same-object-twice` gets its comment; truncation's tests name `"[MaxProperties ~]"` once as a constant and both `expect` messages of `check_rd27` name the case; the sanitize proptest comments the inputs it leaves out, and `ts_event` asserts that it never makes a `toString` or `__proto__` key; rename `sanitize_parameters` to `sanitize_tool_values` (it also walks `structuredContent`); redaction gets a fixed case for a `""` key inside an array element and imports the hook-field and restored-field lists.
- [ ] **Step 3: Test support.** `OracleError`'s `Debug` prints the stack on separate lines; oracle error messages cap the reply line at 2 000 characters with a `… (N more)` suffix; `bmp_string` draws without modulo bias; the `node` child pipes its stderr through a relay thread into the test's output and is killed and awaited by a guard when the oracle is dropped (the likely source of nextest "leaky" reports). Test: `oracle_child_is_reaped` (the guard's drop waits for the child).
- [ ] **Step 4: Run.** Run: `cargo nextest run -p agentcat-sdk --all-features`. Expected: PASS, and no `LEAK` line in the nextest output.
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add oracle agentcat-sdk
git commit -m "test: close the plan 2 vector and test gaps; reap the oracle child and relay its stderr"
```

---

### Task 5: Delivery test gaps and the serde_json floor

Model: Opus 5.5.

**Files:**
- Modify: `agentcat-sdk/src/event/tests.rs`, `agentcat-sdk/src/logging/tests.rs`, `agentcat-sdk/src/pipeline/process.rs`, `agentcat-sdk/src/pipeline/process/tests.rs`, `agentcat-sdk/src/delivery/http/tests.rs`, `agentcat-sdk/src/delivery/tests.rs`, `agentcat-sdk/src/testing/tests.rs`, `agentcat-sdk/tests/fake.rs`, `parity/manifest.toml`, `.github/workflows/ci.yml` (`minimal-versions` job), `scripts/check.sh`

- [ ] **Step 1: Write the tests.** `from_ts_rejects_wrong_types` (table: a non-string tag value, a fractional and a negative duration, tokens above `u32::MAX`, `null`, an unparsable timestamp; each gives the documented field value); `logging/tests.rs`: `no_home_is_silent` asserts that no file appears under the temp root, `temp_home` removes its directory on drop, and `prefix_is_computed_once` checks the caching (the parity entry "caches the prefix" points to it); `event_id_failure_drops_with_line` through a crate-private seam on `process.rs` (an ID source the test can make fail); `real_transport_process_publishes` (the real `Transport::process` path against wiremock); `failed_build_publisher_drops_through_worker` (a publisher whose client build failed, run through the worker, logs C44's lines); `fake_events_with_no_handles_is_empty`, `failing_assert_tracked_panics_with_tool_name` (`#[should_panic]`), `telemetry_only_events_are_recorded_by_fake`; a proptest `byte_size_matches_stringify_len` over random events; `queue_drop_line_comes_before_published_line` waits on a notification, not a bounded sleep; the `Echo` tool of `tests/fake.rs` awaits a `tokio::time::sleep` so the tool phase overlaps too.
- [ ] **Step 2: The serde_json floor.** In a scratch copy of the workspace, run `cargo update -p serde_json --precise 1.0.127` (if a dependency forbids it, use the lowest version it allows and name it in the commit body). Run `json_features_empty_in_this_build`; then turn on `float_roundtrip`, then `arbitrary_precision` (a temporary feature line in the copy's `agentcat-sdk/Cargo.toml`) and check that the self-check reports each one. If a probe misfires, change the probe values (P20) with a comment that names the versions they were proven on. The `minimal-versions` job and `scripts/check.sh` then run `json_features_empty_in_this_build` with that serde_json pinned (on a copy of `Cargo.lock` they restore).
- [ ] **Step 3: Run.** Run: `cargo nextest run -p agentcat-sdk --all-features`. Expected: PASS.
- [ ] **Step 4: Run `scripts/check.sh`, then commit.**

```bash
git add agentcat-sdk parity .github scripts
git commit -m "test(delivery): close the plan 4 test gaps; prove the feature self-check at the serde_json floor"
```

---

### Task 6: Oracle tooling

Model: Sonnet 5.5.

**Files:**
- Modify: `oracle/src/vectors/write.mjs`, `oracle/src/strict-json.mjs`, `oracle/src/fetch-ts.mjs`, `oracle/src/logs.mjs`, `oracle/src/run.mjs`, `oracle/src/list-tests.mjs`, `oracle/src/driver.mjs`, `oracle/src/server.mjs`, `oracle/src/bundle.mjs`, `oracle/src/vectors/gen/queue.mjs`, `oracle/test/*`, `harness/src/contract/extract-schema.mjs`, `harness/test/*`

- [ ] **Step 1: Write the failing tests** (`node --test` style of `oracle/test/`): a hole in `cases` gives a named error with the set and index; a cyclic value gives an error with its path; extra array properties are rejected; `$repeat` outside `input` is rejected; the generator directory comes from `genDir`; `writeAll` checks every set before it writes any file; strict JSON rejects a `Set` and a class instance; the exact bits of `1e23` and `2**53+2`; a mulberry32 known answer; the `oracle-log-*` temp directory is removed on SIGINT and SIGTERM; an absent trace and the trace calls of `run.mjs`, `list-tests.mjs` and `driver.mjs`; `captureLogs` restores the earlier sink; the generator tests check family content (`queue.mjs`: every listed family prefix has at least one case with the expected fields); `extract-schema.mjs` output compiles under the validator's strict ajv settings.
- [ ] **Step 2: Run them to see them fail.** Run: `corepack pnpm -r --workspace-concurrency=1 test`. Expected: FAIL.
- [ ] **Step 3: Implement.** Also: `GIT_REDIRECTS` imports `DROPPED_ENV` from `fetch-ts.mjs` instead of copying it; `loadModule`'s doc says its bundles are not log-isolated; `server.test.mjs:63` uses a fresh request ID; `queue.mjs` builds its event error as `callWrap` does (with the protocol tags), advances the mock clock until the recorder settles, and `recorder.settled()` gets a watchdog with a named error; `bundle.mjs`'s header lists the modules it bundles now; a generator that sets a `deviation` other than RD-24 must not reuse the JSON round trip of `gen/normalize.mjs` (a test asserts it).
- [ ] **Step 4: Run.** Run: `corepack pnpm -r --workspace-concurrency=1 test && node oracle/src/vectors/run.mjs --check`. Expected: PASS; vector files unchanged unless a generator fix changes them (then regenerate and say why in the commit body).
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add oracle harness agentcat-sdk/tests/vectors
git commit -m "fix(oracle): named errors in vector tooling, temp-dir cleanup, generator watchdog; strict schema compile"
```

---

### Task 7: Harness fixes

Model: Sonnet 5.5.

**Files:**
- Modify: `harness/src/driver.mjs`, `harness/src/matrix.mjs`, `harness/src/rulings.mjs` (or wherever `loadRulings` lives), `harness/src/canary*.mjs`, `harness/rust-server/src/profile.rs`, `harness/rust-server/src/main.rs`, `harness/scenarios/hooks-fail.json`, `harness/test/*`, `harness/README.md`

- [ ] **Step 1: Write the failing tests:** the stdio transport detects a server that needed SIGKILL and fails the run (C46); `exitDeadline`'s message names the client close and the SIGTERM wait separately; an HTTP server that exits cleanly without the drained line fails; the drained-line and environment tests also run on the 2026-07-28 lane; `loadRulings` exits 2 on pin keys (`ts`, `rust`, `rustKeys`) in a delta rulings file; `callCanaryChange` compares the full text, not the truncated summary; `profile.rs` test `redact_event_hook_runs` runs the reference server's `redactEvent` hook on an event.
- [ ] **Step 2: Run them to see them fail.** Run: `corepack pnpm --filter harness test && cargo nextest run -p harness-ref-server`. Expected: FAIL.
- [ ] **Step 3: Implement.** A/A runs (`--servers rust,rust`) print no `EVENT UNUSED RULING` lines (rulings scoped to TS-vs-Rust pairs are skipped there). Add a `throws` hook to `hooks-fail.json` (a hook that throws, beside the ones that return an error), with its counts from a TS run. Fix the two README lines (the `diff.mjs` wrapping; "falls back to the `duration`").
- [ ] **Step 4: Run the matrix.** Run: `cargo build -p harness-ref-server && node harness/src/matrix.mjs`. Expected: exit 0.
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add harness
git commit -m "fix(harness): detect a killed stdio server, refuse pins in delta rulings, quiet A/A, add a throwing hook"
```

---

### Task 8: SSE lane in the harness

Model: Sonnet 5.5.

**Files:**
- Modify: `harness/src/servers/ts-v1.mjs`, `harness/src/servers/ts-v2.mjs`, `harness/rust-server/src/main.rs`, `harness/src/driver.mjs`, `harness/src/matrix.mjs`, `harness/scenarios/*.json` (transport lists), `harness/rulings*.json` (only new, justified rulings), `harness/test/*`, `harness/README.md`

**Interfaces:**
- Produces: transport name `http-sse` (C59), accepted by both servers (`--transport http-sse`) and by the driver; `matrix.mjs` includes it wherever a scenario lists `http`.

- [ ] **Step 1: Write the failing tests:** each server answers a `tools/call` over `http-sse` with `Content-Type: text/event-stream`; the driver reads it; `matrix.mjs --scenarios basic --transports http-sse` runs.
- [ ] **Step 2: Run them to see them fail.** Run: `corepack pnpm --filter harness test`. Expected: FAIL.
- [ ] **Step 3: Implement** with TS `enableJsonResponse: false` and rmcp `StreamableHttpServerConfig::default().with_json_response(false)` (stateful). Rule any new difference with its RD, or fix it, or add a pending RD.
- [ ] **Step 4: Run the matrix.** Run: `cargo build -p harness-ref-server && node harness/src/matrix.mjs`. Expected: exit 0 with the `http-sse` combinations listed; A/A and canaries as in `scripts/check.sh`.
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add harness
git commit -m "test(harness): http-sse lane with streamed responses for every http scenario"
```

---

### Task 9: Parity to zero

Model: Opus 5.5.

**Files:**
- Modify: `parity/manifest.toml`, `xtask/src/parity.rs`, `xtask/src/testdata/*`, Rust test files that gain tests, `harness/scenarios/*.json` (if a scenario must grow), `scripts/check.sh`, `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `harness:<scenario>` keys of Task 8 (`http-sse`).

- [ ] **Step 1: Improve the checker.** Tests in `xtask/src/parity.rs`: the `UnclaimedVectorCase` text says whether the entry is missing, `todo` or another status; a key repeated inside one case's `ts_test` is an error; the case names in a `covered-by-vector` reason must exist in the named vector file; a repository-level test runs the link checks on `xtask/src/testdata`. Run: `cargo nextest run -p xtask`. Expected: FAIL first, then PASS after the change.
- [ ] **Step 2: Fix the three known mappings:** "hanging resolveSessionId/eventTags/eventProperties do not delay the tool call" needs a Rust test that hangs all three hooks (add `hanging_session_tags_and_properties_hooks_do_not_delay_call`); the three `rebuild-on-demand` keys point to Rust tests of the rebuild (RD-12 rules the harness pagination), not to `harness:pagination`; the three identity "re-resolve per request" keys get `identify_re_resolves_per_call` (a hook that returns a new identity on its second call; the second event carries it).
- [ ] **Step 3: Map the 46 `todo` entries.** For each, read the TS test at the pinned SHA, then pick one: an existing Rust test or scenario that asserts the same behavior (name it), a new Rust test (write it; expected values from TS), a harness scenario (`http-sse` for `sse-smoke-v1` and `sse-smoke-v2`; the header sentinel for `header-capture` if `extra-projection` compares `requestInfo.headers`, otherwise a new step), or `n/a` with its category and RD (`edge-runtime-compatibility` export checks are `ts-host-only`, RD-19, unless a Rust test pins the same logging behavior). The `engine-callwrap` MRTR cases map to Rust tests of `call/run.rs` (B-025: `continuation` tag, `input_required` tag without mint-back). Write one line of reason per `n/a`.
- [ ] **Step 4: Make zero `todo` a gate.** `scripts/check.sh` and the CI job run `cargo run -q -p xtask -- parity --final`. Run it. Expected: exit 0; counts printed (`ported`, `n/a`, `todo` = 0).
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add parity xtask agentcat-sdk harness scripts .github
git commit -m "test(parity): map every TS test; parity --final in the gate"
```

---

### Task 10: Examples and their CI runs

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/examples/stdio.rs`, `agentcat-sdk/examples/streamable_http.rs`, `agentcat-sdk/examples/stateless_http.rs`, `agentcat-sdk/examples/hooks.rs`, `harness/src/examples-check.mjs`, `harness/test/examples-check.test.mjs`
- Modify: `agentcat-sdk/Cargo.toml` (`[dev-dependencies]`: rmcp features `transport-io`; tokio `signal`; `include` adds `examples/**`), `harness/README.md` (the examples check), `scripts/check.sh`, `.github/workflows/ci.yml` (`oracle-harness` runs the check; every `rmcp (<version>)` leg runs `cargo build -p agentcat-sdk --examples --all-features` and `cargo test -p agentcat-sdk --doc --all-features`)

**Interfaces:**
- Produces (C54): each example reads `AGENTCAT_PROJECT_ID` through `AgentCat::from_env` or `track(.., std::env::var(..))`, honors `AGENTCAT_API_URL`, and serves two tools (`add` with `a`, `b` numbers; `echo` with `text`), plus in `hooks.rs` a tool `fail` that returns an error recorded with `capture_error`. `streamable_http.rs` and `stateless_http.rs` read the port from `PORT` (default 8000), serve `/mcp`, and call `agentcat.shutdown(Duration::from_secs(5))` after the axum server stops on SIGTERM or Ctrl-C. `hooks.rs` serves stdio.
- Produces: `node harness/src/examples-check.mjs <example>` (`stdio`, `streamable_http`, `hooks`): starts the local mock, builds and runs the example with `AGENTCAT_API_URL` set to the mock and `AGENTCAT_PROJECT_ID=proj_examples`, drives `initialize`, `tools/list` and three calls with the TS client, closes the client right after the last call (for HTTP, then sends SIGTERM), and asserts: one publish per call (and the custom event for `hooks`), every stdout line parses as JSON-RPC, exit status 0 within 5 s of the stdin close or the SIGTERM, the last call's publish received before the exit.

- [ ] **Step 1: Write the check's tests** against a fake example (a Node script that prints a non-JSON line, or exits without publishing). Run: `corepack pnpm --filter harness test`. Expected: FAIL.
- [ ] **Step 2: Write the examples and the check.** Each example has a top comment: what it shows and the command that runs it. `streamable_http.rs` keeps rmcp's default SSE responses.
- [ ] **Step 3: Run.** Run: `cargo build -p agentcat-sdk --examples --all-features && for e in stdio streamable_http hooks; do node harness/src/examples-check.mjs $e; done`. Expected: exit 0 each.
- [ ] **Step 4: Run `scripts/check.sh`, then commit.**

```bash
git add agentcat-sdk harness scripts .github
git commit -m "docs(examples): stdio, streamable HTTP, stateless HTTP and hooks examples, checked against the mock API"
```

---

### Task 11: README, crate guide, CHANGELOG and metadata

Model: Opus 5.5.

**Files:**
- Create: `README.md` (repository root)
- Modify: `agentcat-sdk/README.md`, `agentcat-sdk/src/lib.rs`, rustdoc of every public item where a section below needs it, `agentcat-sdk/CHANGELOG.md`, `agentcat-sdk/Cargo.toml` (`homepage = "https://agentcat.com"`), `CONTRIBUTING.md` (new tools: cargo-llvm-cov, cargo-mutants, cargo-public-api; the SSE lane; the examples check), `.github/workflows/ci.yml` (`msrv` job adds `cargo test --workspace --doc --all-features` on 1.88), `scripts/check.sh` (same), `docs/superpowers/plans/README.md` (drop "billing block" wording for the current state)

- [ ] **Step 1: Write the crate guide (`lib.rs`, C55).** Sections: what the SDK records (tool calls with intent, sessions, actors, errors, token estimates; custom events); quickstart for stdio (from design §5) and for streamable HTTP; stateless HTTP; `Options` (a table of each setter and its TS default); environment variables (`AGENTCAT_PROJECT_ID`, `AGENTCAT_API_URL`, `MCPCAT_API_URL`, `AGENTCAT_ENABLED`); features (`rustls` default, `native-tls`, `test-util`); shutdown (`flush`, `shutdown`, blocking twins, the 5 s last-drop drain); logging (`~/agentcat.log` by default, `LogDestination::Tracing`, and for stdio servers: send your own `tracing` output to stderr); testing with `Fake` and `Event::default()`; differences from the TypeScript SDK (link to the deviation register in the repository); MSRV policy (research §9 text); known limits: a tokio runtime with the time driver is required (C32), rmcp's `local` feature is not supported (C27), `preserve_order` applies to the whole build (R07), feature unification can turn on serde_json's `arbitrary_precision` or `float_roundtrip` (number text and RD-23 change; the SDK logs a warning), axum's `nest_service` strips the path prefix from `requestInfo.url`, the tool registry is per handle and handler type, keyed by tool name and never pruned (C19: per-tenant tool names grow it; on a server without `get_tool`, one session's listing sets the strip set of another session's calls), a hook that holds an `AgentCat` clone (Task 2), redaction hooks without a budget (C50). Every code block compiles (`no_run` where it serves).
- [ ] **Step 2: Write the READMEs.** `agentcat-sdk/README.md`: badges (crates.io, docs.rs, CI, MSRV 1.88, license MIT), the TS README's links (Docs, Website), install (`cargo add agentcat-sdk`), the stdio quickstart, links to the guide and the examples; add `#[cfg(doctest)] #[doc = include_str!("../README.md")] pub struct ReadmeDoctests;` to `lib.rs`. Root `README.md`: what the repository holds (the crate, `xtask`, `harness`, `oracle`, `parity`), how the SDK proves parity with TypeScript (the four layers of design §11 in one paragraph each), links to `CONTRIBUTING.md`, `docs/DECISIONS.md` and the design.
- [ ] **Step 3: Write the CHANGELOG** (C56): `## [Unreleased]` with `### Added` items for the public API, the delivery policy, the features and the `Fake`, and one line that links the deviation register.
- [ ] **Step 4: Run.** Run: `cargo test --workspace --doc --all-features && cargo +1.88 test --workspace --doc --all-features && RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo +nightly doc -p agentcat-sdk --all-features --no-deps`. Expected: PASS.
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add README.md CONTRIBUTING.md agentcat-sdk docs/superpowers/plans/README.md scripts .github
git commit -m "docs: crate guide, READMEs, CHANGELOG and package metadata"
```

---

### Task 12: Public API snapshot and package checks

Model: Sonnet 5.5.

**Files:**
- Create: `xtask/src/public_api.rs`, `agentcat-sdk/public-api.txt` (generated)
- Modify: `xtask/src/main.rs`, `scripts/check.sh`, `.github/workflows/ci.yml` (new job `api`, in `all-green`'s `needs`), `CONTRIBUTING.md` (one line: how to bless the snapshot)

**Interfaces:**
- Produces (C51): `cargo run -p xtask -- public-api [--bless]`. Exit 0 when the generated text equals the snapshot and every external path is allowed; exit 1 with a unified diff, or with the disallowed paths listed.

- [ ] **Step 1: Write the failing tests** in `xtask/src/public_api.rs`: `allowlist_accepts_rmcp_and_std`, `allowlist_rejects_reqwest_tokio_chrono_agentcat_api` (on fixed text samples), `diff_reports_changed_line`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo nextest run -p xtask`. Expected: FAIL.
- [ ] **Step 3: Implement, bless, review.** Run: `cargo run -p xtask -- public-api --bless`. Read the snapshot: every item there is meant to be public (design §5, plus C49); anything else is a finding to fix in the SDK, not to bless.
- [ ] **Step 4: Package checks.** The `api` job and `scripts/check.sh` also run `cargo package -p agentcat-sdk --list` (the list holds only `src/**`, `examples/**`, `build.rs`, `README.md`, `CHANGELOG.md`, `LICENSE`, `Cargo.toml`, `Cargo.toml.orig`, `.cargo_vcs_info.json`, `Cargo.lock` if cargo adds it; a test in `xtask` or a shell check enforces the list) and `cargo publish -p agentcat-sdk --dry-run --allow-dirty`.
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add xtask agentcat-sdk/public-api.txt scripts .github CONTRIBUTING.md
git commit -m "ci: public API snapshot with an external-type allowlist; package and publish dry run"
```

---

### Task 13: Coverage floors

Model: Opus 5.5.

**Files:**
- Create: `xtask/src/coverage.rs`
- Modify: `xtask/src/main.rs`, test files of modules under their floor, `scripts/check.sh`, `.github/workflows/ci.yml` (new job `coverage`, in `all-green`'s `needs`), `.github/workflows/nightly.yml` (branch report)

**Interfaces:**
- Produces (C52): `cargo run -p xtask -- coverage <llvm-cov summary json>`: prints a table (file, lines covered, lines total, percent) for the crate and the pure set, and exits 1 below a floor, naming it.

- [ ] **Step 1: Write the failing tests** in `xtask/src/coverage.rs` on fixed JSON samples: `crate_floor_fails_at_89_9`, `pure_floor_fails_at_94_9`, `test_files_are_ignored`.
- [ ] **Step 2: Run them to see them fail, implement, run them to pass.** Run: `cargo nextest run -p xtask`.
- [ ] **Step 3: Measure.** Run: `cargo llvm-cov nextest -p agentcat-sdk --all-features --ignore-filename-regex '(/tests\.rs$|/test_support/|/tests/)' --json --summary-only --output-path target/coverage.json && cargo run -p xtask -- coverage target/coverage.json`.
- [ ] **Step 4: Reach the floors with tests,** each test asserting behavior (a test that only executes code is a finding). Uncovered code that no behavior can reach is a finding to remove, not to test.
- [ ] **Step 5: Gate it.** Add the two commands to `scripts/check.sh` and the `coverage` job; the nightly adds `cargo +nightly llvm-cov nextest --branch` and prints the branch percentage in the run summary (report only).
- [ ] **Step 6: Run `scripts/check.sh`, then commit.**

```bash
git add xtask agentcat-sdk scripts .github
git commit -m "ci: line coverage floors (90 % crate, 95 % pure modules); nightly branch report"
```

---

### Task 14: Mutation gates and nightly legs

Model: Sonnet 5.5.

**Files:**
- Create: `.cargo/mutants.toml`, `scripts/mutants-base.sh`
- Modify: `scripts/check.sh`, `.github/workflows/ci.yml` (job `mutants`, in `all-green`'s `needs`; `permissions: actions: read` for that job only), `.github/workflows/nightly.yml`, `CONTRIBUTING.md`

**Interfaces:**
- Produces (C53): `scripts/mutants-base.sh` prints the base commit: in CI, the head SHA of the newest successful `ci.yml` run on the same branch (`gh api`), else the parent commit (`HEAD^`); locally, `origin/rewrite`. The `mutants` job runs `git diff <base>...HEAD > target/mutants.diff && cargo mutants --in-diff target/mutants.diff` and fails on any missed mutant.
- Produces (nightly): job `mutants-full` with matrix `shard: [0..7]` (`cargo mutants --shard k/8`, the `node-oracle` action, `timeout-minutes: 350`, `mutants.out` uploaded as artifact `mutants-shard-<k>`), and job `mutants-score` that merges the shards' `outcomes.json` and fails below 90 % caught of viable mutants; job `rmcp-main` (the SDK tests and the `rmcp-canary` against rmcp's git `main` through a run-time `[patch.crates-io]`); the `rmcp-canary` on every 3.x minor; job `agentcat-api-latest` (builds against the newest `agentcat-api` from crates.io, `continue-on-error: true`, a warning in the summary).

- [ ] **Step 1: Write the config.** `.cargo/mutants.toml`: `test_tool = "nextest"`, `additional_cargo_args = ["--all-features"]`, `package = ["agentcat-sdk"]` (or the 27.x spelling), `exclude_globs` for `src/test_support/**` and `src/**/tests.rs`, a `timeout_multiplier`. Mutant copies must reuse `oracle/.cache` and `node_modules` without network (copy them, or point the copy at them; write which in a comment).
- [ ] **Step 2: Try the diff gate on a known diff.** Run: `git diff HEAD~3...HEAD > target/mutants.diff && cargo mutants --in-diff target/mutants.diff`. Expected: a result with no `missed` lines, or missed mutants that Task 15 kills (record them for Task 15).
- [ ] **Step 3: Write the jobs** above, `scripts/mutants-base.sh`, and the `scripts/check.sh` step (diff against `origin/rewrite`; an empty diff passes). The nightly's push trigger adds `.cargo/**` to its paths.
- [ ] **Step 4: Run `scripts/check.sh`, then commit, push, and watch the nightly push run.** The nightly's mutation shards may report survivors until Task 15; every other new nightly job must be green.

```bash
git add .cargo scripts .github CONTRIBUTING.md
git commit -m "ci: mutation diff gate and nightly full run; rmcp main leg, canary per minor, newest agentcat-api"
git push origin rewrite
```

---

### Task 15: Mutation baseline

Model: Opus 5.5.

**Files:**
- Modify: test files across `agentcat-sdk/src/**`, `.cargo/mutants.toml` (exclusions with reasons)

- [ ] **Step 1: Get the full results.** Download the 8 shard artifacts of the nightly push run of Task 14 (`gh run download <run id> -p 'mutants-shard-*'`). If that run did not finish, run the shards locally (`cargo mutants --shard k/4 -j 2`, one at a time in the background, logs under `target/mutants-shard-k/`; free disk first). Record caught, missed, timeout and unviable counts per file.
- [ ] **Step 2: Kill the missed mutants.** For each, add a test that fails on the mutant and asserts TS-derived or documented behavior. A mutant that cannot change behavior (an equivalent mutant) gets an `exclude_re` entry whose comment explains why. Never weaken an assertion to kill a mutant.
- [ ] **Step 3: Re-test the missed mutants** locally with `cargo mutants --iterate` (the downloaded outcomes in `mutants.out`). Expected: zero missed in `agentcat-sdk/src` outside the exclusions, so the score is at least 90 % caught of viable mutants.
- [ ] **Step 4: Record the counts** before and after in the header comment of `.cargo/mutants.toml` (this change also starts the nightly push run). Run `scripts/check.sh`, commit, push, and `gh run watch` the nightly push run: `mutants-score` green.

```bash
git add agentcat-sdk .cargo
git commit -m "test: kill surviving mutants; record equivalent-mutant exclusions"
git push origin rewrite
```

---

### Task 16: Ingest replay of every Rust capture

Model: Sonnet 5.5.

**Files:**
- Create: `harness/ingest/replay.sh`
- Modify: `harness/ingest/test_ingest_capture.py`, `harness/ingest/README.md`

**Interfaces:**
- Produces (C60): `harness/ingest/replay.sh <capture.raw.json>...` runs README steps 2 to 6 once (clone at `harness/contract/SOURCE`, `poetry install`, replay with `env -i` and a new empty HOME, cleanup), passing `CAPTURE_FILES` (colon-separated). The test prints `INGESTED <n> bodies OK from <m> files` and fails on any body that is not `ProcessResult.SUCCESS`.

- [ ] **Step 1: Extend the test** to read `CAPTURE_FILES` (keep `CAPTURE_FILE` working) and write the script. README: prerequisites list `corepack pnpm install` and the first oracle build; the HOME claim says that a new HOME hides only files found through HOME; the DNS guard sentence says it covers call-time lookups through Python's `socket` module; step 3 notes that `poetry install` runs with the real HOME.
- [ ] **Step 2: Run one full matrix, then the replay.** Run: `node harness/src/matrix.mjs && harness/ingest/replay.sh harness/out/*.rust*.raw.json` (the Rust tracked captures; adjust the glob to the matrix's file names). Expected: `1 passed`, every body `SUCCESS`. A failure is a bug in the SDK or a ruled difference; fix it or add a pending RD, and say which in the report.
- [ ] **Step 3: Write the result** (date, `harness/contract/SOURCE` commit, file and body counts, event types) to `harness/ingest/RESULTS.md`.
- [ ] **Step 4: Run `scripts/check.sh`, then commit.**

```bash
git add harness/ingest
git commit -m "test(ingest): replay every Rust capture through agentcat-server's process_event"
```

---

### Task 17: Dashboard demo tooling

Model: Sonnet 5.5.

**Files:**
- Create: `harness/demo/run.mjs`, `harness/demo/ts-server.mjs`, `harness/demo/scenario.json`, `harness/test/demo.test.mjs`
- Modify: `scripts/check.sh` and the `oracle-harness` CI job (`node harness/demo/run.mjs --mock`), `harness/README.md` (the demo)

**Interfaces:**
- Produces (C58): `node harness/demo/run.mjs --mock | --prod --project <id>`. The scenario: `initialize`; `tools/list`; `add` with intent text; `echo` twice in one session; `fail` (an error with `capture_error`); one custom event; a second session with one `echo`. Both servers use the same options: `identify` returns `{user_id: "demo-user", user_name: "Demo User"}`, `event_tags` returns `{"demo": "plan-5"}`, `redact_sensitive_information` replaces `secret-token` with `[REDACTED]` (the `echo` text of one call contains it). `--prod` prints the two `ses_` session IDs and writes them with the run time to `harness/out/demo/sessions.json`.

- [ ] **Step 1: Write the failing tests:** `--prod` without `--project proj_2yAR5QHJ3CE3icTpJlKJqu1ZiXG` exits 2; `--prod` with `CI=true` exits 2; `--mock` runs both servers and the mock receives the same event count from each.
- [ ] **Step 2: Run them to see them fail.** Run: `corepack pnpm --filter harness test`. Expected: FAIL.
- [ ] **Step 3: Implement.** `ts-server.mjs` loads the TS SDK from the oracle cache at the pinned SHA, like `harness/src/servers/ts-v2.mjs`. The Rust side runs `cargo run -p agentcat-sdk --example hooks`. In `--mock` mode, both servers run under the egress guard as in the harness; `--prod` sets no API URL (the SDK default) and loads no guard.
- [ ] **Step 4: Run.** Run: `node harness/demo/run.mjs --mock`. Expected: exit 0. Do not run `--prod` (Task 19 does).
- [ ] **Step 5: Run `scripts/check.sh`, then commit.**

```bash
git add harness scripts .github
git commit -m "test(demo): dashboard demo runner for the Testing project, with a mock mode in the gate"
```

---

### Task 18: docs.agentcat.com Rust pages (draft PR)

Model: Opus 5.5.

**Files (in a clone of `agentcathq/agentcat-ui` at `~/misc/agentcat-ui`, branch `docs/rust-sdk`):**
- Modify: `apps/docs/quickstart.mdx`, `apps/docs/sdk/custom-events.mdx`, `debug-mode.mdx`, `event-tags-properties.mdx`, `identifying-users.mdx`, `report-missing-tools.mdx`, `stateless-servers.mdx`, `tool-call-context.mdx`, `apps/docs/docs.json` (navigation for new pages)
- Create: `apps/docs/sdk/rust/runtimes.mdx` (tokio, the delivery thread, shutdown, logging to stderr) and `apps/docs/sdk/rust/limits.mdx` (the known limits of Task 11), after the pattern of `apps/docs/sdk/php/`

- [ ] **Step 1: Read the PHP docs PR** (`gh pr view 253 -R agentcathq/agentcat-ui`) and the C# draft (255) for the conventions: a ```` ```rust Rust ```` block after the Go block in each code group, notes for language-specific behavior.
- [ ] **Step 2: Write the pages.** Every Rust snippet comes from a compiled source: the README, the crate guide or an example of Task 10/11 (copy, do not invent). The install step shows `cargo add agentcat-sdk` and rmcp.
- [ ] **Step 3: Build the docs** with the repository's documented command (its `package.json`; for Mintlify, `mintlify broken-links` in `apps/docs`). Expected: no broken links.
- [ ] **Step 4: Commit, push the branch, open a draft PR** titled `docs: Rust SDK`. Body: what changed, C57 (draft until the crates.io release), the source of each snippet.

```bash
git -C ~/misc/agentcat-ui add apps/docs
git -C ~/misc/agentcat-ui commit -m "docs: Rust SDK"
git -C ~/misc/agentcat-ui push -u origin docs/rust-sdk
gh pr create -R agentcathq/agentcat-ui --draft --title "docs: Rust SDK" --body-file <body file>
```

---

### Task 19: Milestone close and the PR

Model: Opus 5.5 (the controller).

**Files:**
- Modify: `docs/DECISIONS.md` (C48 to C60; R29 if the owner batch has questions), `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (section 9: any new RD; section 11: C48 replaces the OS clause), `docs/superpowers/plans/carry-forward.md` (close the plan 5 section; what remains goes to a "Second pass" section), `docs/superpowers/plans/README.md` (plan 5 row)
- Create: `docs/demo/2026-10-dashboard-demo.md` (the demo report)

- [ ] **Step 1: Run the final gate.** Run: `scripts/check.sh`. Expected: exit 0, parity `--final` with zero `todo`. Push; `gh run watch` the CI run: every job green on all three systems. The newest nightly push run (Task 15) is green, with a mutation score of at least 90 %.
- [ ] **Step 2: Run the dashboard demo** (R12). Run: `node harness/demo/run.mjs --prod --project proj_2yAR5QHJ3CE3icTpJlKJqu1ZiXG`. Read both sessions back with the AgentCat MCP tools (`get_session`, `get_events`, `search_sessions`). Compare per event: type, tool name, intent, actor, tags, the redacted text, error type and message, token counts, duration presence. Write the report with the dashboard links; any difference is a known RD or a finding.
- [ ] **Step 3: Record the decisions and deviation rows** with the wording the code ended up with, and close the carry-forward section.
- [ ] **Step 4: Bring the pending rulings to the owner** as one batch of multiple-choice questions (R17), recommended option first; record the answers as R29. A change an answer requires lands before the PR.
- [ ] **Step 5: Update the plan index, commit, push, and open the PR** from `rewrite` into `main` (R15). Body: what the crate does, the verification evidence (gate commit, CI run links, parity counts, mutation score, coverage, ingest replay result, the demo report), the deviation register link, the docs-site draft PR link, and the owner's two post-merge steps: make `all-green` a required check (C17), and dispatch the nightly once so the issue-filing path runs (R21).

```bash
git add docs
git commit -m "docs: plan 5 close: decisions C48-C60, demo report, carry-forward, plan index"
git push origin rewrite
gh pr create --base main --head rewrite --title "agentcat-sdk 1.0.0: AgentCat analytics for rmcp servers" --body-file <body file>
```

---

## Carry-forward map

| Carry-forward item (plan 5 section) | Task |
|---|---|
| `homepage` in the publish metadata | 11 |
| `cancel-in-progress` only for PR runs | 1 (C48) |
| Windows: clone rename retry, DEP0190, `lock.mjs` break path, write-then-rename, release rename | 1 |
| Nightly Windows failure: parallel `bundle.mjs` rename race (found 2026-10-27, run 40495080931) | 1 |
| msrv job runs doctests | 11 |
| `extract-schema.mjs` strict ajv compile | 6 |
| Ingest replay README wording and prerequisites | 16 |
| Dispatch the nightly once after the merge (R21) | 19 (PR body, owner's post-merge step) |
| `lock.mjs` robustness (token, unreadable claim, sweep, interleavings, host name) | 1 |
| Vector tooling (`write.mjs`, `strict-json.mjs`, generator rule) | 6 |
| Oracle tooling (`GIT_REDIRECTS`, temp dir, traces, `captureLogs`, `loadModule` doc, request ID) | 6 |
| `xtask parity` (unclaimed text, repeated key, repository test, case names) | 9 |
| Rust test support (`OracleError` `Debug`, message cap, `bmp_string` bias) | 4 |
| Plan 2 vector and test coverage list | 4 |
| R26 docs.agentcat.com pages | 18 |
| Plan 3 docs: time driver, `local`, feature unification, `nest_service` | 11 |
| Egress guard and worker threads (watch item) | none: no TS change requires it; stays a watch item |
| `cfg(not(unix))` reference server built on macOS and Windows | 1 (C48 builds it on each system) |
| Registry growth and cross-session strip set (C19) | 11 (documented) |
| `rmcp-macros` pin of the rmcp legs | 1 |
| Lifecycle hardening (redaction budget, edge cases, `stop` test, `Queue::close`) | 2 (C50) |
| `Event` constructor | 2 (C49) |
| Oracle child stderr leak | 4 |
| Docs: hook-held `AgentCat` cycle, `with_default` on another thread | 2 |
| Plan 4 test gaps | 5 |
| Plan 4 code items (config split, `fn object`, duplicates, copies, clones) | 3 (speed and allocation benchmarks: second pass) |
| Plan 4 parity mapping (hanging hooks, rebuild-on-demand, re-resolve per request) | 9 |
| Plan 4 oracle generators (`queue.mjs`, `bundle.mjs` doc) | 6 |
| Plan 4 harness list | 7 |
| serde_json floor probes at 1.0.127 | 5 |
| CI with Windows and macOS before the PR | 1, 19 |
| Fix-wave residuals (`redact_event` name, `testing.rs` sentences, `delivery.rs:24`) | 3 |
