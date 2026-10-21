# Plan 2: Pure core (JS semantics, constants, handles, tokens, event pipeline) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build every pure function of the SDK (milestone 3 of the design) and prove each one equal to the TypeScript SDK at `9b56e9e` with TS-generated vectors (L1) and property tests against the live TS oracle (L2).

**Architecture:** Crate-private modules in `agentcat-sdk`: `js` (JavaScript semantics), `constants`, `handles` (KSUID, session handles), `tokens`, and `pipeline` (tag validation, normalize, truncation, sanitization, redaction). The pipeline functions work on the event as TS holds it in memory: a `serde_json::Map` with TS's camelCase field names (type alias `TsEvent`); plan 4 converts the Rust event to and from that shape. Tests live in the crate (`#[cfg(test)]`), share one test-support module (the oracle client, the vector loader, proptest strategies), and compare Rust output with TS output through `js::stringify`, so number format and key order count. Nothing becomes public in this plan.

**Tech Stack:** Rust 1.88+ (edition 2024), `serde_json` (`preserve_order`, no `float_roundtrip`), `ryu-js`, `sha2`, `getrandom` 0.4, proptest, cargo-nextest; Node 22, the oracle of plan 1 (esbuild bundles of the pinned TS source).

**Spec:** `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (sections 6.1, 6.5, 6.6, 6.7, 7, 9, 11). Behavior contract: `docs/research/01-ts-sdk-behavior.md` B-052 to B-076 and B-088 to B-093, K-04, K-11, Appendix C. Decisions: `docs/DECISIONS.md` (D09, D15, R07, R19, R22, R23). Deferred items: `docs/superpowers/plans/carry-forward.md`, section "Plan 2".

## Global Constraints

- Branch `rewrite`; conventional-commit subjects; no AI attribution lines; push only to `origin rewrite`.
- `edition = "2024"`, `rust-version = "1.88"`. Library code: no `unwrap`, `expect`, `panic!`, `todo!`, printing; every lint exception is `#[expect(<lint>, reason = "...")]`.
- Every new module is private and every item in it is `pub(crate)` at most. `lib.rs` exports nothing in this plan.
- `serde_json` keeps `features = ["preserve_order"]` (R07). Nothing in the workspace turns on `float_roundtrip` (R22).
- New dependencies go into `[workspace.dependencies]` with `default-features = false`; each floor is the lowest version that passes `cargo +nightly minimal-versions check --direct --workspace` (R-1).
- Expected values come from TS only: a vector file written by `node oracle/src/vectors/run.mjs`, or a live oracle call. Never type a TS output into a Rust test. A Rust-only test may assert a shape, a Rust-only path, or a deviation, and it names the deviation id.
- Vector files are never edited by hand. After a generator change, run `node oracle/src/vectors/run.mjs` and commit the output with the generator.
- Read TS code and tests only in the oracle's clone, `oracle/.cache/ts-9b56e9e920f35f7c37fc2a63ce6520a9ec97edea` (`node oracle/src/fetch-ts.mjs` creates it). Never read `~/agentcat-typescript-sdk`.
- Oracle and harness processes never contact anything but loopback; never disable the egress guard.
- Lengths are UTF-16 code units (D09). A cut that falls inside a surrogate pair drops the whole character (RD-22). Count with `js::utf16_len`, cut with `js::cut_utf16`; never use `str::len()` or `chars().count()` for a TS length.
- Iterate JSON objects in JavaScript order wherever TS iterates them (`js::own_entries`: array-index keys first, ascending, then insertion order).
- Deviations: RD-20 (values Rust cannot hold; shared references), RD-22 (surrogate cut), RD-23 (float parsing, R22), RD-24 (`__proto__` keys kept, R23), RD-25 (array `response` kept, R23). A vector case whose TS output differs because of RD-24 or RD-25 carries `"deviation": "RD-24"` (or `RD-25`), and its Rust test asserts the Rust behavior for that id explicitly. RD-22 needs no flag: `test_support::assert_js_eq` applies it to every expected value.
- Parity rule, for each TS test file a task covers: a pure input-to-output test becomes a vector case with `"ts_test": ["<key>"]` and the manifest entry `status = "n/a"`, `category = "covered-by-vector:<file>.json"`, `reason = "<file>.json case <name>; checked by agentcat-sdk::<module>::tests::<test>"`. A behavior pinned by a named Rust test is `ported` with that test name. A test of JS-only mechanics (object identity, mutation of the input, async scheduling, `Buffer`, `inspect`, `Date` objects) is `n/a`, `ts-host-only`, with the reason. A test whose input Rust cannot hold is `n/a`, `ruled-divergence:RD-20` (or the matching RD id). A test of a later plan's code (rmcp results, hooks, the queue, servers) stays `todo`. Entries stay in `parity/ts-tests.json` order.
- Before each commit, run the full local gate: `scripts/check.sh` must exit 0.
- Model per task (owner rule: Opus 5.5 for the hardest half, Sonnet 5.5 for the rest; reviewers use the implementer's model): stated at the top of each task.

## Review Focus

1. **Numbers and key order in `JSON.stringify` output.** `{"n":1.0}`, `1e21`, `-0`, `0.1+0.2`, integers above 2^53, and objects with keys like `"10"`, `"2"`, `"4294967295"`, `"01"`. A person expects token estimates and the 100 KB check to count the bytes TS counts. Pinned by `js.json` cases `stringify:*` and `stringify-text:*` (Task 5) and `tokens.json` case `input:floats` (Task 9).
2. **Emoji at every UTF-16 cut point.** An astral character straddling the 200-unit `agent_id` tag limit, the 2048 `userIntent` limit, the 32 768 string limit and a last-resort cut. A person expects TS's cut position minus the half character, never a panic on a byte boundary. Pinned by `handles.json` case `tags:agent-id-emoji-at-200` (Task 8), `normalize.json` case `string:emoji-at-limit` (Task 11), `truncate.json` cases `field:user-intent-emoji-at-2048` and `content:emoji-at-32768` (Task 12).
3. **Events at the 100 KB boundary.** Events of exactly 102 400 and 102 401 bytes, events that fit only at depth 9, 5 or 1, and events where the last-resort cut gives up after its rounds. A person expects the same size path as TS. Pinned by `truncate.json` cases `size:*` (Task 12).
4. **Handle values with JavaScript whitespace and sentinel spellings.** `" START\t"`, `"﻿start"`, `"start\u0085"`, `"ſtart"`, `"ses_<27 chars>\n"`. A person expects the TS resolution: BOM is trimmed, NEL is not, only ASCII case folds. Pinned by `handles.json` cases `resolve:*` and `valid:*` (Task 8).
5. **Customer JSON that TS treats specially.** A non-string `context` (`userIntent` 5, `true`, `{}`, a 2049-item array), a `"..."` key at the breadth limit, a `__proto__` key (RD-24), an array `response` (RD-25). A person expects TS's result, or the recorded deviation. Pinned by `truncate.json` cases `intent:*` (Task 12), `normalize.json` case `breadth:dots-key` (Task 11), and the RD-24 and RD-25 cases of Tasks 11 to 14.

---

### Task 1: Strict vector encoding and vector-set rules

Model: Opus 5.5.

**Files:**
- Create: `oracle/src/strict-json.mjs`, `oracle/src/vectors/write.mjs`, `oracle/src/vectors/support.mjs`, `oracle/test/strict-json.test.mjs`, `oracle/test/vector-support.test.mjs`
- Modify: `oracle/src/vectors/run.mjs` (CLI only; logic moves to `write.mjs`), `oracle/src/vectors/gen/constants.mjs` (drop its own duplicate-name check), `oracle/test/vectors.test.mjs`, `oracle/test/drift.test.mjs`, `agentcat-sdk/tests/vector_integrity.rs`

**Interfaces:**
- Produces (`strict-json.mjs`), the marker contract every vector file and every oracle reply follows:
  - `{"$utf16": [<UTF-16 code units>]}` stands for a string that is not well-formed UTF-16 (TS cut a surrogate pair).
  - `{"$f64": "<16 lowercase hex digits>"}` stands for a number that is `-0` or not plain: its IEEE-754 bits, big-endian.
  - `{"$repeat": ["<unit>", <count>]}` appears in case inputs only: `unit` repeated `count` times.
  - `export function isPlainNumber(n: number): boolean`: true for a safe integer (other than `-0`), or a finite number whose shortest JS text has a digit string (dot and exponent removed) of at most 2^53 and a decimal exponent `e` (value = digits × 10^e) with `|e| <= 22`. serde_json without `float_roundtrip` parses exactly these (one correctly rounded operation).
  - `export function encodeStrict(value: unknown): unknown`: a JSON-safe copy with the markers applied. Throws `Error("strict-json: <path>: <reason>")` (path like `$.cases[3].input.a[2]`) for `undefined` (also as a property or array hole), functions, symbols, bigints, NaN, ±Infinity, objects whose prototype is not `Object.prototype` or `null` (Date, RegExp, Map, Set, Buffer, class instances), and objects that carry a key `$utf16`, `$f64` or `$repeat` unless the object is exactly a well-formed `$repeat` macro (one key; `[string, safe non-negative integer]`).
  - `export function expand(value: unknown): unknown`: a copy with every `$repeat` macro replaced by its string.
- Produces (`vectors/support.mjs`): `export function tsKey(file: string, title: string): string` = `"v1|src/tests/<file>.test.ts|<title>"` (or `"v2|src/tests/v2/<file>.test.ts|<title>"` when `file` starts with `v2/`); `export function seeded(seed: number): () => number` (mulberry32, values in [0, 1)); `export function pick(rand, items)`, `export function int(rand, lo, hi)` (inclusive), `export function text(rand, alphabet, lo, hi)`.
- Produces (`vectors/write.mjs`): `export async function writeAll({ outDir, genDir }): Promise<Record<string, number>>` and `export function checkSet(name: string, cases: unknown[]): void`. A set must have at least one case; every case is an object with a non-empty string `name`, unique in the set; `ts_test`, when present, is a non-empty array of strings. The encoded file must be at most 2 MiB (2 097 152 bytes). Each failure throws an error that names the generator and the case. Files are `encodeStrict({ts_sha, generator, cases})`, 2-space JSON, trailing newline, as before.
- Case fields every later generator uses: `name`, `input` (may hold `$repeat` macros), `output` (TS result), and as needed `input_text` (JSON text, for key-order cases), `logs` (TS log messages), `error` (TS error text), `ts_test`, `deviation`.

- [ ] **Step 1: Write the failing tests**

`oracle/test/strict-json.test.mjs` (`node:test`):
- `encodeStrict({ s: "a\ud83d" })` deep-equals `{ s: { $utf16: [97, 0xd83d] } }`; `"😀"` and `"é"` come back unchanged.
- `encodeStrict(-0)` is `{ $f64: "8000000000000000" }`; `encodeStrict(0.1 + 0.2)` is `{ $f64: "3fd3333333333334" }`; `encodeStrict(0.1)`, `encodeStrict(1e21)` and `encodeStrict(2 ** 53)` are numbers; `encodeStrict(1e23)` and `encodeStrict(2 ** 53 + 2)` are `$f64` objects.
- Throws with the path for: `{ a: [1, undefined] }` (`$.a[1]: undefined`), `[ , 1]`, `{ f() {} }`, `Symbol()`, `1n`, `NaN`, `Infinity`, `new Date(0)`, `/x/`, `new Map()`, `{ $utf16: [1] }`, `{ $repeat: ["ab", -1] }`; `{ $repeat: ["ab", 3] }` passes unchanged.
- `expand({ a: [{ $repeat: ["ab", 3] }] })` deep-equals `{ a: ["ababab"] }`.
- `isPlainNumber`: true for `0`, `12`, `0.1`, `1.5e-7`, `1e21`, `123456789012345` (15 digits); false for `-0`, `0.1 + 0.2`, `1e23`, `5e-324`, `2 ** 53 + 2`.

`oracle/test/vector-support.test.mjs`: `tsKey("constants", "a > b")` is `"v1|src/tests/constants.test.ts|a > b"`; `tsKey("v2/hooks", "x")` is `"v2|src/tests/v2/hooks.test.ts|x"`; two `seeded(7)` streams give the same first 5 values.

`oracle/test/vectors.test.mjs` (new tests): `checkSet` throws on an empty set, a case without `name`, a duplicate `name` (message names it), `ts_test: "x"` (not an array), and a set whose encoded size exceeds 2 MiB.

`oracle/test/drift.test.mjs` (new tests, carry-forward): a vector file present only in the committed dir yields the line `exists only in the committed vectors`; one present only in the fresh dir yields `exists only in the fresh vectors`; a case missing from the fresh file is listed under `cases only in the committed file (1)` with its name.

`agentcat-sdk/tests/vector_integrity.rs`: `at_least_one_vector_set_exists` asserts that `json_files()` and `manifest()` are both non-empty.

- [ ] **Step 2: Run them to see them fail**

Run: `corepack pnpm --filter oracle test`
Expected: FAIL (modules missing). `vector_integrity` passes already (it needs no new code); keep it.

- [ ] **Step 3: Implement `strict-json.mjs`, `support.mjs`, `write.mjs`; slim `run.mjs` to the CLI**

`run.mjs` keeps its exact arguments, usage line and exit codes. Use `String.prototype.isWellFormed` for the string check.

- [ ] **Step 4: Regenerate and run**

Run: `node oracle/src/vectors/run.mjs && git diff --stat agentcat-sdk/tests/vectors && corepack pnpm --filter oracle test && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk --test vector_integrity`
Expected: `constants.json` and `MANIFEST.sha256` are unchanged (no diff); all tests pass.

- [ ] **Step 5: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add oracle agentcat-sdk/tests/vector_integrity.rs
git commit -m "test(oracle): strict vector encoding with markers; vector-set rules and size cap"
```

### Task 2: Oracle process — one core bundle, log capture, error stacks, SHA trace

Model: Opus 5.5.

**Files:**
- Create: `oracle/src/logs.mjs`, `oracle/test/core.test.mjs`
- Modify: `oracle/src/bundle.mjs`, `oracle/src/server.mjs`, `oracle/src/functions.mjs`, `oracle/src/paths.mjs`, `oracle/src/vectors/run.mjs`, `oracle/src/parity/list-tests.mjs`, `oracle/src/vectors/gen/constants.mjs` (use `loadCore`), `oracle/test/server.test.mjs`, `oracle/test/fetch.test.mjs`, `oracle/src/egress-guard.mjs` (header), `harness/src/driver.mjs`, `harness/src/diff.mjs`, `harness/test/aa.test.mjs`, `harness/test/driver.test.mjs`, `CONTRIBUTING.md`, `scripts/check.sh`

**Interfaces:**
- Produces (`bundle.mjs`): `export async function loadCore(): Promise<Core>`, where `Core` has the namespaces `constants`, `handles`, `tools`, `tokenEstimate`, `validation`, `truncation`, `sanitization`, `redaction`, `logging` (each `src/modules/<name>.ts`) and `KSUID` (the default export of `src/thirdparty/ksuid/index.js`). One esbuild bundle (`<bundleDir>/core.mjs`, entry given as esbuild `stdin` that re-exports each namespace), so every module shares one `logging` instance. `BUNDLE_FORMAT` becomes 3. `loadModule(name)` keeps working. The import-meta plugin resolves `srcDir` and each file path with `realpathSync` before it compares them (carry-forward: a symlinked checkout).
- Produces (`logs.mjs`): `export async function captureLogs(core, fn): Promise<{ value, logs: string[] }>` runs `fn()` with `core.logging.setDiagnosticsSink` collecting entries, strips the `[<timestamp>] [<version prefix>] ` head from each entry (an entry without that head throws), and clears the sink in `finally`. `export function isolateLogFile(core): void`: on first call, points TS's log file into a fresh temp directory (set `HOME` and `USERPROFILE` to it, call `core.logging.writeToLog("oracle: log file isolated")`, restore both variables), and removes the directory on process exit. `loadCore` calls it, so no oracle run writes to the developer's `~/agentcat.log`.
- Produces (`server.mjs` reply contract, replacing plan 1's): `{"id": n, "ok": encodeStrict(result)}`; `{"id": n, "undefined": true}`; `{"id": n, "err": <message>, "stack": <stack or null>}`, where `message` is `error.message` for an `Error` and `String(error)` otherwise; a result that `encodeStrict` rejects is `{"id": n, "err": "result is not strict JSON: <reason>", "stack": null}`.
- Produces (`functions.mjs`), built on `loadCore()`: `"oracle.sha"` (returns `lock().sha`), `"oracle.echo"` (returns its argument), `"oracle.throw"` (throws `new TypeError(<argument>)`), `"validation.validateTags"` (returns `captureLogs` of `validateTags(tags)`, so `{value, logs}`). `"JSON.stringify"` and `"tokenEstimate.estimateInputTokens"` stay.
- Produces (`paths.mjs`): `export function traceOverride(): void` writes `oracle: ORACLE_SHA=<sha> overrides oracle/oracle.lock (<locked sha>)` to stderr once per process when `ORACLE_SHA` is set. `server.mjs`, `vectors/run.mjs`, `parity/list-tests.mjs` and `harness/src/driver.mjs` call it at start.
- Produces (`harness/src/driver.mjs`): `report.json` gets `ts_sha` (the effective `lock().sha`).
- Produces (`harness/src/diff.mjs`): a ruling may carry `"pending": true` (boolean only; any other value is rejected as before), so a provisional ruling can enter `rulings.json` before the owner answers (R17).

- [ ] **Step 1: Write the failing tests**

`oracle/test/core.test.mjs`:
- `loadCore()` returns all ten names; `core.KSUID.withPrefix("ses").fromParts(14e11, Buffer.alloc(16))` is `"ses_000000000000000000000000000"`.
- `captureLogs(core, () => core.validation.validateTags({ "a/b": "x" }))` deep-equals `{ value: null, logs: ['Dropping invalid tag: "a/b" — key contains invalid characters or is empty'] }`.
- With `HOME` set to an empty temp dir for the test process before `loadCore()`, a call that logs leaves `<that dir>/agentcat.log` absent.
- The import-meta plugin rewrites `import.meta.url` when the clone is reached through a symlink (create a temp symlink to `tsDir()`, bundle one module from it, and find the source file URL in the output).

`oracle/test/server.test.mjs` (new cases): `oracle.throw` with `"boom"` replies `err: "boom"` and a `stack` that contains `TypeError: boom`; `oracle.sha` replies the lock SHA; `oracle.echo` with the request text `{"id":5,"fn":"oracle.echo","args":[-0]}` replies `{"id":5,"ok":{"$f64":"8000000000000000"}}`; a server started with `ORACLE_SHA` equal to the locked SHA writes the trace line to stderr.

`oracle/test/fetch.test.mjs`: the test `git()` helper scrubs `GIT_DIR`, `GIT_INDEX_FILE` and `GIT_WORK_TREE` as `run()` does; a test sets `GIT_DIR=/nonexistent` in `process.env` around one call and still reads the clone's `HEAD`.

`harness/test/aa.test.mjs`: a ruling `{"path": "...", "reason": "...", "deviation": "RD-01", "pending": true}` that matches a difference makes `diff.mjs` exit 0; `"pending": "yes"` makes it exit 2 (or the current bad-rulings code) naming the key; in `harness/test/driver.test.mjs`, the `basic.json` stdio run's `report.json` has `ts_sha` equal to the lock SHA.

- [ ] **Step 2: Run them to see them fail**

Run: `corepack pnpm -r --workspace-concurrency=1 test`
Expected: FAIL on the new tests only.

- [ ] **Step 3: Implement**

Also make these two text changes (carry-forward N1, N2):
- `oracle/src/egress-guard.mjs` header, at the end of the "Not covered" paragraph: "A connection whose custom `lookup` receives the host name `localhost` passes: the guard checks the name, not the address that lookup returns."
- `CONTRIBUTING.md` (the Node prerequisite) and the `check.sh` hint: "Node.js 22 or later. Node 22 to 24 include corepack; on Node 25 and later, run `npm install -g corepack`."

- [ ] **Step 4: Run all JS tests and the vectors check**

Run: `corepack pnpm -r --workspace-concurrency=1 test && node oracle/src/vectors/run.mjs --check`
Expected: PASS; vectors up to date.

- [ ] **Step 5: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add oracle harness CONTRIBUTING.md scripts/check.sh
git commit -m "test(oracle): one core bundle with log capture and an isolated log file; error stacks; ORACLE_SHA trace"
```

### Task 3: Parity checker — vector links, order, missing entries

Model: Sonnet 5.5.

**Files:**
- Modify: `xtask/src/parity.rs`, `xtask/src/main.rs`, `parity/manifest.toml` (only if the order check finds entries out of order)

**Interfaces:**
- Consumes: the case field `ts_test` (Task 1).
- Produces (`parity.rs`), new `Problem` variants, each with a `Display` line that says what to do:
  - `EmptyPorted(String)`: a `ported` entry with an empty `rust` list (today it is reported as `Unmapped`, which misleads). Text: `ported without tests: <key>: list the Rust tests or harness scenarios in rust`.
  - `OutOfOrder { key: String, previous: String }`: manifest entries must follow the order of `parity/ts-tests.json` (compare by index in that list, not by string collation).
  - `MissingVectorCase { key: String, file: String }`: a `covered-by-vector:<file>` entry whose file has no case with `key` in its `ts_test`.
  - `UnclaimedVectorCase { file: String, case: String, key: String }`: a case whose `ts_test` names a key whose manifest entry is not `covered-by-vector:<that file>`.
- Produces: `pub fn vector_ts_tests(dir: &Path) -> Result<BTreeMap<String, BTreeMap<String, Vec<String>>>, String>` (file stem → case name → `ts_test` keys); `check` and `check_repo` take this map in place of the stem set (the stems are its keys).
- Produces: `cargo run -q -p xtask -- parity --print-missing` writes to stdout one `[[test]]` block with `status = "todo"` for each TS key that has no manifest entry, in `ts-tests.json` order, and exits 0. Usage line: `usage: cargo run -p xtask -- parity [--final | --print-missing]`.

- [ ] **Step 1: Write the failing unit tests** in `parity.rs`: one per new variant (`ported_with_empty_rust_is_empty_ported`, `entries_out_of_ts_order_are_reported`, `covered_by_vector_needs_a_case_with_the_key`, `a_case_must_be_claimed_by_its_entry`), plus `print_missing_lists_unmapped_keys_in_order` (two TS keys, a manifest with the second only, output equals one `todo` block for the first).

- [ ] **Step 2: Run them to see them fail**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p xtask`
Expected: FAIL (variants missing).

- [ ] **Step 3: Implement; update the existing tests that built a stem set**

- [ ] **Step 4: Run**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p xtask && cargo run -q -p xtask -- parity && cargo run -q -p xtask -- parity --print-missing`
Expected: tests pass; `parity` exits 0; `--print-missing` prints nothing (every key is mapped).

- [ ] **Step 5: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add xtask parity
git commit -m "feat(xtask): parity links vector cases to TS tests; manifest order; --print-missing"
```

### Task 4: In-crate test support

Model: Opus 5.5.

**Files:**
- Move: `agentcat-sdk/tests/support/mod.rs` → `agentcat-sdk/src/test_support/mod.rs`; `agentcat-sdk/tests/support/oracle.rs` → `agentcat-sdk/src/test_support/oracle.rs`; the tests of `agentcat-sdk/tests/oracle_smoke.rs` → `agentcat-sdk/src/test_support/smoke.rs` (delete the old file)
- Create: `agentcat-sdk/src/test_support/markers.rs`, `agentcat-sdk/src/test_support/vectors.rs`, `agentcat-sdk/src/test_support/strategies.rs`
- Modify: `agentcat-sdk/src/lib.rs` (`#[cfg(test)] mod test_support;`), `.github/workflows/nightly.yml` (the comment that names `oracle_smoke`)

**Interfaces:**
- Consumes: the reply and marker contracts (Tasks 1 and 2).
- Produces (`test_support::oracle`): `pub(crate) fn call(func: &str, args: &[Value]) -> Result<Option<Value>, OracleError>`, with every `$f64` marker of an `ok` value decoded and `$utf16` markers left in place; `pub(crate) fn lock()` and `pub(crate) fn is_poisoned()` as in plan 1; `pub(crate) struct OracleError { pub(crate) message: String, pub(crate) stack: Option<String> }` with `Display` = the message, then the stack on the next line when present. At spawn, the client calls `oracle.sha` and fails the spawn when the reply differs from the `sha` of `oracle/oracle.lock`, with the text `the oracle runs TS <x> but oracle/oracle.lock pins <y>; unset ORACLE_SHA` (carry-forward).
- Produces (`test_support::markers`): `pub(crate) fn decode_f64(value: Value) -> Value`; `pub(crate) fn d09(value: Value) -> Value` (each `$utf16` marker becomes the string with every unpaired surrogate dropped: RD-22); `pub(crate) fn expand(value: Value) -> Value` (`$repeat`).
- Produces (`test_support::vectors`): `pub(crate) fn cases(set: &str) -> Vec<Map<String, Value>>`, which reads `agentcat-sdk/tests/vectors/<set>.json`, decodes `$f64`, expands `$repeat` in `input`, and panics with the file name on any error; `pub(crate) fn name(case: &Map<String, Value>) -> &str`.
- Produces (`test_support`): `pub(crate) fn config() -> proptest::test_runner::Config` (256 cases unless `PROPTEST_CASES` says otherwise) and `pub(crate) fn config_capped(max: u32) -> Config` (the smaller of `PROPTEST_CASES` and `max`).
- Produces (`test_support::strategies`), the shared input generators every property test uses:
  - `pub(crate) fn js_char() -> impl Strategy<Value = char>`: weighted mix of ASCII letters and digits, `"` and `\`, U+0000–U+001F, U+007F, the JavaScript whitespace set and its neighbors (U+0009–U+000D, U+0020, U+0085, U+00A0, U+1680, U+180E, U+2000–U+200B, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF), BMP letters (`é`, `世`), U+E000–U+F8FF, astral characters (U+1F600, U+1D11E), and any `char`.
  - `pub(crate) fn js_string(max_len: usize) -> impl Strategy<Value = String>`; `pub(crate) fn bmp_string(len: Range<usize>) -> impl Strategy<Value = String>` (no astral characters; for the size-guard tests).
  - `pub(crate) fn object_key() -> impl Strategy<Value = String>`: array-index-like keys (`"0"`–`"20"`, `"4294967294"`, `"4294967295"`, `"01"`, `"-1"`, `"1.5"`), short words, `"..."`, `"context"`, `"type"`, `"text"`, and `js_string(12)`; never `__proto__` (RD-24 has its own vector cases), `$utf16`, `$f64` or `$repeat`.
  - `pub(crate) fn number() -> impl Strategy<Value = serde_json::Number>`: any `i64`, any `u64`, any finite `f64`, and the specials `0.0`, `-0.0`, `1e21`, `1e-7`, `1e-6`, `0.1 + 0.2`, `2^53 ± 1`.
  - `pub(crate) fn json_value(depth: u32, width: usize) -> impl Strategy<Value = Value>`.

- [ ] **Step 1: Move the files and write the failing tests**

`markers.rs` tests: `decode_f64` of `{"$f64":"8000000000000000"}` is a number whose `as_f64()` is negative zero; `{"$f64":"3fd3333333333334"}` becomes `0.1 + 0.2`; `d09` of `{"$utf16":[97,55357]}` is `"a"` and of `{"$utf16":[55357,56832,98]}` is `"😀b"`; `expand` of `{"a":[{"$repeat":["ab",3]}]}` is `{"a":["ababab"]}`; all three recurse through arrays and objects.

`vectors.rs` test: `cases("constants")` is non-empty and every case has a name.

`smoke.rs`: the plan-1 smoke tests, adapted to `OracleError`, plus `oracle_runs_the_locked_sha` and `an_error_reply_carries_message_and_stack` (`oracle.throw` with `"boom"`: `message == "boom"`, the stack contains `TypeError: boom`).

`strategies.rs` test: a proptest `object_keys_are_never_reserved`.

- [ ] **Step 2: Run them to see them fail**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk`
Expected: FAIL (modules missing).

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk`
Expected: PASS (the moved smoke tests now run as `agentcat-sdk::test_support::smoke::…`).

- [ ] **Step 5: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add -A agentcat-sdk .github/workflows/nightly.yml
git commit -m "test: in-crate test support (oracle client, markers, vector loader, strategies)"
```

### Task 5: `js` — JavaScript semantics

Model: Opus 5.5.

**Files:**
- Create: `agentcat-sdk/src/js.rs`, `agentcat-sdk/src/js/tests.rs`, `oracle/src/vectors/gen/js.mjs`, `agentcat-sdk/tests/vectors/js.json` (generated)
- Modify: `Cargo.toml` and `agentcat-sdk/Cargo.toml` (`ryu-js`), `agentcat-sdk/src/lib.rs`, `agentcat-sdk/src/test_support/mod.rs` (`assert_js_eq`), `oracle/src/functions.mjs`

**Interfaces:**
- Produces (`js`):
  - `pub(crate) fn stringify(value: &Value) -> String`: `JSON.stringify(value)`. Objects in `own_entries` order; numbers through `number_to_string`; strings escaped as JSON.stringify does (serde_json's string escaping matches it; the tests prove it).
  - `pub(crate) fn number_to_string(number: &serde_json::Number) -> String`: JavaScript `Number.prototype.toString()` of the number as an `f64` (`ryu_js`). `-0` gives `"0"`.
  - `pub(crate) fn own_entries(object: &Map<String, Value>) -> Vec<(&String, &Value)>`: JavaScript own-property order: keys that are array indices (the canonical decimal text of an integer from 0 to 4 294 967 294) first, in ascending numeric order, then the other keys in insertion order.
  - `pub(crate) fn utf16_len(text: &str) -> usize`.
  - `pub(crate) fn cut_utf16(text: &str, units: usize) -> &str`: the longest prefix of at most `units` UTF-16 units that ends on a `char` boundary (RD-22).
  - `pub(crate) fn trim(text: &str) -> &str`: `String.prototype.trim`. JavaScript whitespace is U+0009, U+000A, U+000B, U+000C, U+000D, U+0020, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000 and U+FEFF. It is not `char::is_whitespace` (that includes U+0085 and excludes U+FEFF).
  - `pub(crate) fn to_js_string(value: Option<&Value>) -> String`: `String(value)`, with `None` = `undefined`. Arrays join their elements with `,`, where `null` elements are empty; objects give `[object Object]`.
  - `pub(crate) fn type_of(value: Option<&Value>) -> &'static str`: `typeof` (`null` and arrays give `"object"`).
- Produces (`test_support`): `pub(crate) fn assert_js_eq(actual: &Value, expected: &Value, context: &str)`: compares `js::stringify(actual)` with `js::stringify(&markers::d09(expected.clone()))` and, on a difference, panics with `context`, the first differing byte offset and 40 bytes of each side around it.
- Produces (`functions.mjs`): `"String.prototype.trim"` (`s => s.trim()`), `"String.prototype.slice"` (`(s, n) => s.slice(0, n)`), `"String.length"` (`s => s.length`), `"oracle.String"` and `"oracle.typeof"` (each takes `{value?}`; an absent `value` is `undefined`), `"oracle.reformat"` (`text => JSON.stringify(JSON.parse(text))`).
- Produces (`js.json`), case name prefixes: `stringify:` (`input` → `output` = JSON text), `stringify-text:` (`input_text` → `output` = `JSON.stringify(JSON.parse(input_text))`; numbers in `input_text` must be plain, so R22 does not apply), `cut:` (`input: [text, units]`), `trim:`, `utf16-len:`, `string:`, `typeof:`. `string:` and `typeof:` cases carry `{value?}` inputs like the oracle functions.

`lib.rs` gets `mod js;` and the crate-level attribute
`#![cfg_attr(not(test), expect(dead_code, reason = "the pure core is wired into the public API from plan 3 on"))]`.

- [ ] **Step 1: Write the generator and the failing tests**

`gen/js.mjs` cases (with `loadCore` not needed: these are JS built-ins):
- `stringify:` `{"n": 1.0}`, `1e21`, `1e-7`, `1e-6`, `-0`, `0.1 + 0.2`, `5e-324`, `1.7976931348623157e308`, `2 ** 53 + 1` as an integer, `18446744073709551615`, `-9223372036854775808`, strings with every control character, `"\\"`, `"\""`, U+007F, U+2028, U+2029, `<>&`, astral characters; nested arrays and objects; plus 40 values from `seeded(1)`.
- `stringify-text:` `{"b":1,"1":2,"a":3,"0":4}`, `{"10":1,"9":2,"x":3}`, `{"4294967295":1,"4294967294":2,"01":3,"-1":4,"1.5":5}`, duplicate keys `{"a":1,"b":2,"a":3}`, nested objects with index keys; plus 20 texts from `seeded(2)`.
- `cut:` `"a😀b"` at 0 to 4 units, `"😀😀😀"` at 1, 3 and 5, `""` at 0 and 3.
- `trim:` each JavaScript whitespace character on both sides of `"x"`, and U+0085, U+180E, U+200B, U+FEFF.
- `utf16-len:` `""`, `"é"`, `"😀"`, `"a😀b"`.
- `string:` and `typeof:` `undefined`, `null`, `true`, `0`, `-0`, `1e21`, `1.5`, `"s"`, `[]`, `[1, [2, 3], null, "a", {}]`, `{}`.

`js/tests.rs`:
- `vectors_match_ts`: every `js.json` case, dispatched on the name prefix, equals the TS output (strings compared exactly; `cut:` through `assert_js_eq`).
- Property tests against the oracle, each with `test_support::config()`: `stringify_matches_js` (`json_value(6, 8)` → `"JSON.stringify"`), `reformat_matches_js` (a JSON text built from `json_value` with plain numbers only, shuffled insertion order and repeated keys; Rust parses it with serde_json and stringifies; the oracle runs `"oracle.reformat"`), `trim_matches_js`, `cut_matches_js` (`(js_string(30), 0..40usize)`), `utf16_len_matches_js`, `to_js_string_matches_js`, `type_of_matches_js`.

- [ ] **Step 2: Generate and run to see the tests fail**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk js::`
Expected: FAIL (`js` functions missing).

- [ ] **Step 3: Implement `js.rs` and `assert_js_eq`; add `ryu-js`**

- [ ] **Step 4: Run**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk js:: && node oracle/src/vectors/run.mjs --check`
Expected: PASS.

- [ ] **Step 5: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add Cargo.toml Cargo.lock agentcat-sdk oracle
git commit -m "feat(js): JSON.stringify, key order, UTF-16 cuts and trim as JavaScript does them"
```

### Task 6: `constants`

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/constants.rs`, `agentcat-sdk/src/constants/tests.rs`
- Modify: `agentcat-sdk/src/lib.rs`, `oracle/src/vectors/gen/constants.mjs` (`ts_test` links), `agentcat-sdk/tests/vectors/constants.json` (generated), `parity/manifest.toml` (`src/tests/constants.test.ts`)

**Interfaces:**
- Produces: one `pub(crate) const <NAME>: &str` per ported TS export, with the TS export name: `DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`, `AGENTCAT_CUSTOM_EVENT_TYPE`, `SESSION_ID_PARAM_DESCRIPTION`, `SESSION_ID_PARAM_PATTERN`, `SESSION_START_SENTINEL`, `AGENT_ID_PARAM_DESCRIPTION`, `MINT_BACK_HEADER_ISSUED`, `MINT_BACK_ISSUED_BODY`, `MINT_BACK_HEADER_UNRECOGNIZED`, `MINT_BACK_UNRECOGNIZED_BODY`, `MCP_SESSION_KEY`, `MCP_SESSION_FIELD_DESCRIPTION`, `MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE`, `MCP_SESSION_SESSION_ID_DESCRIPTION`, `MCP_SESSION_AGENT_ID_DESCRIPTION`, `MCP_SESSION_STATUS_DESCRIPTION`, `META_CLIENT_INFO_KEY`, `META_PROTOCOL_VERSION_KEY`, `AGENTCAT_TAG_AGENT_ID`, `AGENTCAT_TAG_SESSION_SOURCE`, `AGENTCAT_TAG_AGENT_SOURCE`, `AGENTCAT_TAG_PROTOCOL_VERSION`, `AGENTCAT_TAG_MRTR`, `GET_MORE_TOOLS_NAME`, `SESSION_ID_PARAM`, `AGENT_ID_PARAM`.
- Produces: `pub(crate) fn mint_back_session_line(session_id: &str) -> String`; `pub(crate) fn get_more_tools_descriptor() -> Value` (the object `getReportMissingToolDescriptor()` returns, keys in TS order); `pub(crate) fn get_more_tools_result() -> Value` (the object `handleReportMissing` returns).
- Not ported, listed in the test with the reason "exporters and diagnostics: second pass (R04, C05, RD-16)": `AGENTCAT_SOURCE`, `DIAGNOSTICS_SCOPE_NAME`, `DEFAULT_DIAGNOSTICS_ENDPOINT`, `DEFAULT_DIAGNOSTICS_TOKEN`.

- [ ] **Step 1: Add `ts_test` links to the generator and regenerate**

Each case that a test of `src/tests/constants.test.ts` pins gets that test's key (`tsKey("constants", "<title>")`). The `mintBack:<kind>` cases stay until Task 8 moves them.

- [ ] **Step 2: Write the failing tests** in `constants/tests.rs`

`every_ported_constant_equals_ts` (a table of `(name, value)` pairs, each equal to the case of that name), `every_ts_constant_is_ported_or_listed` (every plain-value case name is in the table or in the not-ported list, and the reverse), `mint_back_session_line_matches_ts`, `get_more_tools_descriptor_matches_ts` and `get_more_tools_result_matches_ts` (through `assert_js_eq`, so key order counts).

- [ ] **Step 3: Run them to see them fail; implement; run them to see them pass**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk constants::`
Expected: FAIL, then PASS.

- [ ] **Step 4: Map the parity entries of `constants.test.ts`** (Global Constraints, parity rule; the exporter and diagnostics tests are `second-pass`), then run `cargo run -q -p xtask -- parity`.
Expected: exit 0.

- [ ] **Step 5: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk oracle parity
git commit -m "feat(constants): agent-facing strings and tag names, equal to TS"
```

### Task 7: KSUID

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/handles/mod.rs` (declares `pub(crate) mod ksuid;` for now), `agentcat-sdk/src/handles/ksuid.rs`, `agentcat-sdk/src/handles/ksuid/tests.rs`, `oracle/src/vectors/gen/ksuid.mjs`, `agentcat-sdk/tests/vectors/ksuid.json` (generated)
- Modify: `Cargo.toml` and `agentcat-sdk/Cargo.toml` (`getrandom` 0.4), `agentcat-sdk/src/lib.rs`, `oracle/src/functions.mjs`, `parity/manifest.toml` (`src/tests/ksuid.test.ts`)

**Interfaces:**
- Produces: `pub(crate) const EPOCH_MS: u64 = 1_400_000_000_000;` `pub(crate) const MAX_TIME_MS: u64 = 1_000 * 4_294_967_295 + EPOCH_MS;` `pub(crate) fn from_parts(prefix: &str, time_ms: u64, payload: &[u8; 16]) -> Option<String>`: `<prefix>_` + 27 base62 characters (alphabet `0-9A-Za-z`, left-padded with `0`) of the 20 bytes `u32 BE floor((time_ms - EPOCH_MS) / 1000)` + payload; `None` when `time_ms` is outside `EPOCH_MS..=MAX_TIME_MS` (TS throws there). `pub(crate) fn random(prefix: &str) -> Option<String>`: the current time in ms since the Unix epoch and 16 bytes from `getrandom::fill`; `None` when the clock is outside the range or the random source fails (TS throws in both cases; plans 3 and 4 map `None` to TS's failure path). Implement `random` through a private `random_at(prefix, now_ms: Option<u64>, fill: impl FnOnce(&mut [u8; 16]) -> Result<(), getrandom::Error>)` so the `None` paths are testable.
- Produces (`functions.mjs`): `"ksuid.fromParts"`: `(prefix, timeMs, payloadHex) => KSUID.withPrefix(prefix).fromParts(timeMs, Buffer.from(payloadHex, "hex"))`.
- Produces (`ksuid.json`): cases `from-parts:*` with `input: {prefix, time_ms, payload_hex}` and `output` (the ID) or `error` (the TS error text); `min-max:*` with the `MIN_STRING_ENCODED` and `MAX_STRING_ENCODED` values.

- [ ] **Step 1: Write the generator and the failing tests**

Generator cases: `EPOCH_MS` with 16 zero bytes (Appendix C: `ses_000000000000000000000000000`); `Date.parse("2024-01-01T00:00:00Z")` with bytes `00..0f` and prefix `evt` (Appendix C); `MAX_TIME_MS` with 16 `ff` bytes; `EPOCH_MS - 1` and `MAX_TIME_MS + 1` (errors); 40 cases from `seeded(3)` (times across the range, any payload); the inputs of the `fromParts` tests of `ksuid.test.ts`, each with its `ts_test`.

`ksuid/tests.rs`: `vectors_match_ts` (an `error` case expects `None`); `min_and_max_match_ts` (`from_parts` at `EPOCH_MS` with zeros, and at `MAX_TIME_MS` with `ff` bytes, give the two encoded constants after the prefix); proptest `from_parts_matches_ts` (`time_ms` in `EPOCH_MS - 2_000..=MAX_TIME_MS + 2_000`, any payload; an oracle `err` means `None`); `random_has_the_shape_and_differs` (prefix, 27 base62 characters, two calls differ); `random_at_is_none_outside_the_range_or_without_randomness` (`now_ms` of `None`, `EPOCH_MS - 1`, and a failing `fill`).

- [ ] **Step 2: Generate and run the tests to see them fail**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk ksuid::`
Expected: FAIL.

- [ ] **Step 3: Implement; run to see them pass**

- [ ] **Step 4: Map the parity entries of `ksuid.test.ts`**

`fromParts` and epoch tests: covered by the vector; the uniqueness and `randomSync` tests: `ported` to `random_has_the_shape_and_differs`; `parse`, `compare`, `equals`, `toString`, `toJSON`, `inspect`, buffer and `date` property tests: `ts-host-only` ("the SDK only encodes KSUIDs; this is the TS KSUID library's own API"); the invalid-payload test: `ts-host-only` ("the payload is a `[u8; 16]`"). Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 5: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add Cargo.toml Cargo.lock agentcat-sdk oracle parity
git commit -m "feat(handles): KSUID encoding and random IDs, equal to TS"
```

### Task 8: Session handles

Model: Opus 5.5.

**Files:**
- Modify: `agentcat-sdk/src/handles/mod.rs`, `Cargo.toml` and `agentcat-sdk/Cargo.toml` (`sha2` moves from dev-dependencies to dependencies), `oracle/src/vectors/gen/constants.mjs` (the `mintBack:*` cases move out), `oracle/src/functions.mjs`, `parity/manifest.toml` (`src/tests/handles.test.ts`)
- Create: `agentcat-sdk/src/handles/tests.rs`, `oracle/src/vectors/gen/handles.mjs`, `agentcat-sdk/tests/vectors/handles.json` (generated)

**Interfaces:**
- Consumes: `ksuid::{from_parts, random}`, `js::{trim, cut_utf16}`, `constants::*`.
- Produces (`handles`):
  - `pub(crate) fn new_session_id() -> Option<String>` (`random("ses")`), `pub(crate) fn new_event_id() -> Option<String>` (`random("evt")`).
  - `pub(crate) fn derive_session_id(id: &str, project_id: Option<&str>) -> String`: sha256 of `"<id>:<project_id>"` (or of `id` when the project ID is `None` or empty); time = `Date.UTC(2024, 0, 1)` + the first 4 hash bytes as a big-endian `u32` (in ms); payload = hash bytes 4..20; `from_parts("ses", ..)` (always in range).
  - `pub(crate) fn is_valid_session_id(value: &str) -> bool`: exactly `ses_` + 27 ASCII base62 characters (JavaScript `$` does not match before a trailing newline).
  - `pub(crate) fn extract_handle(args: Option<&Map<String, Value>>, name: &str) -> Option<String>`: a string value, `js::trim`med, non-empty.
  - `pub(crate) enum SessionSource { Hook, Supplied, Minted, Invalid, Foreign }` with `pub(crate) fn as_str(self) -> &'static str` (`"hook"`, …).
  - `pub(crate) struct HandleResolution { pub(crate) session_id: String, pub(crate) session_source: SessionSource, pub(crate) hook_mode: bool, pub(crate) agent_id: Option<String> }` (an `agent_id` is always `"supplied"`).
  - `pub(crate) fn resolve_handles(args: Option<&Map<String, Value>>, hook_mode: bool, agent_tracking: bool, session_param_is_ours: bool) -> Option<HandleResolution>`: B-055, B-060, B-064 in TS's order; the start sentinel check is `trimmed.eq_ignore_ascii_case("start")` (equal to JS `toLowerCase() === "start"`; the property test proves it). `None` only when minting fails.
  - `pub(crate) fn build_mint_back_text(res: &HandleResolution) -> Option<String>`.
  - `pub(crate) fn build_structured_mint_back(res: &HandleResolution) -> Option<Map<String, Value>>` (keys in the order `session_id`, `agent_id`, `status`).
  - `pub(crate) fn build_handle_tags(res: &HandleResolution, protocol_version: Option<&str>) -> Map<String, Value>`: `agentcat_session_id_source`; then, with an agent ID, `agentcat_agent_id` (CR and LF each become a space, then `js::cut_utf16(.., 200)`: RD-22) and `agentcat_agent_id_source: "supplied"`; then `agentcat_protocol_version` when the version is non-empty.
  - `pub(crate) fn session_from_hook_value(hook_value: Option<&str>, project_id: Option<&str>) -> Option<(String, SessionSource)>`: a value that is non-empty after `js::trim` gives `(derive_session_id(trimmed, project_id), Hook)`; otherwise `(new_session_id()?, Minted)`.
- Produces (`functions.mjs`): `"handles.deriveSessionId"`, `"handles.isValidSessionId"`, `"handles.extractHandle"`, `"handles.buildMintBackText"`, `"handles.buildStructuredMintBack"`, `"handles.buildHandleTags"` (resolution objects in TS's camelCase form), `"oracle.resolveHandles"` (`{args, hookMode, agentTracking, sessionParamIsOurs}` → TS's resolution) and `"oracle.sessionFromHookValue"` (`{value, projectId}`). Both `oracle.*` wrappers replace a minted ID with `"<minted>"` after checking it with `isValidSessionId`; the Rust tests mask the same way.
- Produces (`handles.json`): case prefixes `derive:`, `valid:`, `extract:`, `resolve:`, `mint-back:` (the five cases from `constants.json`, renamed), `structured:`, `tags:`, `hook-value:`.

- [ ] **Step 1: Write the generator and the failing tests**

Generator cases, each TS test input of `handles.test.ts` that the module covers with its `ts_test`, plus:
- `derive:` the six Appendix C calls; IDs with whitespace, Unicode and astral characters; project `""`, `"p"`, `proj_…`; 40 from `seeded(4)`.
- `valid:` 26, 27 and 28 body characters; a non-base62 body character; an Arabic-Indic digit; a trailing `"\n"`; inner whitespace; `"ses_"` alone; wrong prefixes.
- `extract:` string, number, `null`, array and object values; `"  x  "`, `"﻿x"`, `"x\u0085"`, `""`, `"   "`; missing key; `args` `null`.
- `resolve:` every combination of hook mode, agent tracking and `session_param_is_ours`, with session values omitted, `"start"`, `" START\t"`, `"sTaRt"`, `"﻿start"`, `"start\u0085"`, `"ſtart"`, `"restart"`, `"ses_bogus"`, a valid ID, a valid ID with surrounding spaces, and agent values omitted, `""`, `"  a  "`.
- `structured:` and `mint-back:` every source in and out of hook mode, with and without an agent ID.
- `tags:` agent IDs plain, with `\r\n`, of 199, 200, 201 and 250 units, `"a" × 199 + "😀"` (`tags:agent-id-emoji-at-200`), and protocol versions absent, `""`, `"2025-11-25"`.
- `hook-value:` `"customer-abc"`, `"  x  "`, `""`, `"   "`, `null`, with project IDs absent, `""` and `"proj_1"`.

`handles/tests.rs`: `vectors_match_ts` (minted IDs masked); property tests `derive_matches_ts` (`js_string(40)`, an optional `js_string(10)` project), `is_valid_matches_ts` (strings built around the `ses_` shape), `extract_and_resolve_match_ts` (argument maps whose `session_id` and `agent_id` values come from a mix of `js_string(8)`, `"start"` spellings with whitespace, and valid IDs), `handle_tags_match_ts` (agent IDs from `js_string(260)`); Rust-only `new_ids_have_the_shape` (`is_valid_session_id(&new_session_id().unwrap())`; `new_event_id()` starts with `evt_`).

- [ ] **Step 2: Generate and run the tests to see them fail**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk handles::`
Expected: FAIL.

- [ ] **Step 3: Implement; run to see them pass**

- [ ] **Step 4: Map the parity entries of `handles.test.ts`**

The pure tests: covered by the vector or `ported`. `appendMintBack` and `mirrorStructuredMintBack` (rmcp results, plan 3) and `invokeSessionHook` (hooks, plan 4): stay `todo`. Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 5: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add Cargo.toml Cargo.lock agentcat-sdk oracle parity
git commit -m "feat(handles): session IDs, derivation, resolution, mint-back text and handle tags"
```

### Task 9: Token estimates

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/tokens.rs`, `agentcat-sdk/src/tokens/tests.rs`, `oracle/src/vectors/gen/tokens.mjs`, `agentcat-sdk/tests/vectors/tokens.json` (generated)
- Modify: `agentcat-sdk/src/lib.rs`, `oracle/src/functions.mjs`, `parity/manifest.toml` (`src/tests/tokenEstimate.test.ts`)

**Interfaces:**
- Consumes: `js::stringify`.
- Produces: `pub(crate) fn estimate_tokens(bytes: u64) -> i32` (`ceil(bytes / 3.5)` in integer arithmetic, `(2 × bytes + 6) / 7` without overflow; `0` for `0`; at most `i32::MAX`); `pub(crate) fn estimate_input_tokens(arguments: Option<&Value>) -> Option<i32>` (`None` for `None` and `null`; else the estimate of `js::stringify(arguments).len()`); `pub(crate) fn estimate_output_tokens(response: Option<&Value>) -> Option<i32>` (B-072: `None` for `None` and `null`; a response without a `content` array counts the whole response's JSON; an empty `content` counts `structuredContent`'s JSON unless it is absent or `null` (then `0`); otherwise the sum of `text` blocks' text bytes and `resource` blocks' `resource.text` bytes, with one rounding).
- Produces (`functions.mjs`): `"tokenEstimate.estimateOutputTokens"`, `"tokenEstimate.estimateTokens"`.
- Produces (`tokens.json`): prefixes `bytes:`, `input:`, `output:`, `utf8:` (`utf8ByteLength` of a well-formed string; the Rust test checks `str::len()` against it).

- [ ] **Step 1: Write the generator and the failing tests**

Generator cases: each input of `tokenEstimate.test.ts` with its `ts_test`; Appendix C token rows; `input:floats` (`{x: 1.0, y: 1e21, z: 0.1}`); `bytes:` `0`, `1`, `7`, `13`, `4096`, `7516192765`, `2 ** 53`; `output:` blocks of every type, `text` that is not a string, `resource` as a string, an array and an object without `text`, a `structuredContent` of `null`, `{}` and an array, responses that are a string, a number and an array; 30 from `seeded(5)`.

`tokens/tests.rs`: `vectors_match_ts`; property tests `input_tokens_match_ts` (`json_value(5, 6)` objects) and `output_tokens_match_ts` (a `tool_result()` strategy, local to this test file: `content` arrays of mixed blocks, an optional `structuredContent`, extra keys).

- [ ] **Step 2: Generate; run to see them fail; implement; run to see them pass**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk tokens::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries of `tokenEstimate.test.ts`**

Pure cases: covered by the vector. "a toJSON that throws", "cannot be serialized", "a content getter that throws" and the lone-surrogate `utf8ByteLength` case: `ruled-divergence:RD-20`. `token-estimates.test.ts` (servers): stays `todo`. Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk oracle parity
git commit -m "feat(tokens): input and output token estimates, equal to TS"
```

### Task 10: Tag validation

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/pipeline/mod.rs`, `agentcat-sdk/src/pipeline/tags.rs`, `agentcat-sdk/src/pipeline/tags/tests.rs`, `oracle/src/vectors/gen/tags.mjs`, `agentcat-sdk/tests/vectors/tags.json` (generated)
- Modify: `agentcat-sdk/src/lib.rs`, `parity/manifest.toml` (`src/tests/validation.test.ts`)

**Interfaces:**
- Produces (`pipeline/mod.rs`): module docs that define the TS event shape (the event as TS holds it in memory when the queue runs: a JSON object with TS's camelCase field names, `timestamp` as an ISO-8601 string with milliseconds, which the redaction walk treats as TS treats a `Date`), and `pub(crate) type TsEvent = serde_json::Map<String, Value>;`.
- Produces (`pipeline::tags`): `pub(crate) fn validate_tags(tags: &Map<String, Value>, log: &mut Vec<String>) -> Option<Map<String, Value>>`: B-093 in `js::own_entries` order; key pattern `^[a-zA-Z0-9$_.:\- ]+$`, key at most 32 units, value a string of at most 200 units without `\n`; each dropped entry and the cap push the TS log message (the text after the log head) to `log`; `type_of` gives the `got <type>` word; `None` for an empty input or no survivors; at most the first 50 survivors.
- Consumes: `"validation.validateTags"` (Task 2).
- Produces (`tags.json`): cases with `input` (or `input_text` for order), `output` and `logs`.

- [ ] **Step 1: Write the generator and the failing tests**

Generator cases: each input of `validation.test.ts` with its `ts_test`; the Appendix C example; 60 valid entries (cap); 50 valid entries mixed with invalid ones; index-like keys after string keys in `input_text`, so the first 50 differ by order; each allowed special character; `é` in a key; keys of 32 and 33 characters; values of 200 and 201 units, and of 100 and 101 emoji; values with `\r`, `\n`, U+2028; values of type number, `null`, boolean, array, object; `{}`; 30 from `seeded(6)`.

`tags/tests.rs`: `vectors_match_ts` (output and logs); proptest `validate_tags_matches_ts` (maps of `object_key()` or allowed-character keys to strings, numbers and `null`).

- [ ] **Step 2: Generate; run to see them fail; implement; run to see them pass**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk pipeline::tags::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries of `validation.test.ts`; run `cargo run -q -p xtask -- parity`** (exit 0)

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk oracle parity
git commit -m "feat(pipeline): tag validation with the TS log lines"
```

### Task 11: `normalize`

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/pipeline/normalize.rs`, `agentcat-sdk/src/pipeline/normalize/tests.rs`, `oracle/src/vectors/gen/normalize.mjs`, `agentcat-sdk/tests/vectors/normalize.json` (generated)
- Modify: `agentcat-sdk/src/pipeline/mod.rs`, `oracle/src/functions.mjs`, `parity/manifest.toml` (the `normalize` tests of `src/tests/truncation.test.ts`)

**Interfaces:**
- Produces: `pub(crate) const MAX_DEPTH: usize = 10;` `pub(crate) const MAX_BREADTH: usize = 100;` `pub(crate) const MAX_STRING_LENGTH: usize = 32_768;` `pub(crate) fn normalize(value: &Value, depth: usize, max_breadth: usize, max_string_length: usize) -> Value`: B-092 for JSON values: an array or object with no depth left becomes `"[Array]"` or `"[Object]"`; arrays keep `max_breadth` items, then `"[MaxProperties ~]"`; objects keep `max_breadth` keys in `js::own_entries` order, then the key `"..."` = `"[MaxProperties ~]"` (an existing `"..."` key keeps its position and gets this value, as a JS assignment does); a string longer than `max_string_length` units becomes `cut_utf16(.., max_string_length) + "..."`; `null`, booleans and numbers are unchanged. A `__proto__` key is kept (RD-24).
- Produces (`functions.mjs`): `"truncation.normalize"`.
- Produces (`normalize.json`): cases with `input` (or `input_text`), `limits` (`[depth, breadth, max_string]`, absent = defaults) and `output`.

- [ ] **Step 1: Write the generator and the failing tests**

Generator cases: each `normalize` test input of `truncation.test.ts` that JSON can carry, with its `ts_test`; the Appendix C 12-level object and 102-element array; depth 0, 1 and 2 on arrays and objects; arrays and objects of 99, 100, 101 and 102 entries; `breadth:dots-key` (an object whose 5th key is `"..."`, with 120 keys); index-like keys beyond the breadth limit in `input_text`; strings of 32 768 and 32 769 units and `string:emoji-at-limit` (an emoji at units 32 767–32 768); small limits (`[3, 2, 5]`) on mixed values; `proto:kept` (a `__proto__` key with an object value, `"deviation": "RD-24"`); 30 from `seeded(7)` with small limits.

`normalize/tests.rs`: `vectors_match_ts` (for RD-24 cases: the Rust output keeps the `__proto__` key with a normalized value, and the Rust output without that key equals the TS output); property tests `normalize_matches_ts_small_limits` (`json_value(6, 6)`, depth `0..6`, breadth `0..6`, max string `0..20`) and `normalize_matches_ts_default_limits` (`json_value(12, 4)`).

- [ ] **Step 2: Generate; run to see them fail; implement; run to see them pass**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk pipeline::normalize::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries of the `normalize` tests**

Covered by the vector, except: `undefined`, `NaN`, `Infinity`, `BigInt`, symbols, functions, circular references, "omit undefined object properties": `ruled-divergence:RD-20`; `Date` and Invalid Date: `ts-host-only` ("no `Date` values reach the Rust pipeline"). Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk oracle parity
git commit -m "feat(pipeline): normalize with TS depth, breadth and string limits"
```

### Task 12: Truncation

Model: Opus 5.5.

**Files:**
- Create: `agentcat-sdk/src/pipeline/truncate.rs`, `agentcat-sdk/src/pipeline/truncate/tests.rs`, `oracle/src/vectors/gen/truncate.mjs`, `agentcat-sdk/tests/vectors/truncate.json` (generated)
- Modify: `agentcat-sdk/src/pipeline/mod.rs`, `agentcat-sdk/src/test_support/strategies.rs` (`ts_event`), `oracle/src/functions.mjs`, `parity/manifest.toml` (the `truncateEvent` tests of `src/tests/truncation.test.ts`), `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (row RD-26)

**Interfaces:**
- Consumes: `normalize`, `js::{stringify, utf16_len, cut_utf16, own_entries, to_js_string}`.
- Produces: `pub(crate) const MAX_EVENT_BYTES: usize = 102_400;` `pub(crate) struct TruncateError { message: String }` (`Debug`, and `Display` = `message`, which is TS's `String(error)` text, for example `TypeError: str.slice is not a function`); `pub(crate) fn truncate_event(event: &TsEvent) -> Result<TsEvent, TruncateError>`.
- Produces (`test_support::strategies`): `pub(crate) fn ts_event(text: impl Strategy<Value = String> + Clone) -> impl Strategy<Value = TsEvent>`: a tool-call-shaped event in TS key order (`id: ""`, `sessionId`, `projectId`, `eventType`, `timestamp`, `duration`, `inputTokens`, `resourceName`, `parameters: {request, extra}`, `response: {content, structuredContent?, isError?}`, where `content` mixes blocks of type `text`, `image`, `audio`, `resource` (with `text` or `blob`), `resource_link` and an unknown type, `userIntent`, `isError`, `error: {message, type, platform, frames?}`, the identify fields, `sdkLanguage`, `agentcatVersion`, the server and client fields, `tags`, `properties`), each optional field present or absent, and strings drawn from `text`. Tasks 13 and 14 reuse it.
- Produces (`functions.mjs`): `"oracle.truncateEvent"`: `event => { try { return { value: truncateEvent(event) }; } catch (e) { return { error: String(e) }; } }`.

Rules that the TS source fixes and a reader can miss:
- The field limits (`userIntent` 2048; `resourceName`, `serverName`, `serverVersion`, `clientName`, `clientVersion` 256) use TS's `truncateString` on any JSON value (K-11): absent or `null` stays; a string longer than the limit becomes `cut_utf16(.., limit) + "..."`; an array with more elements than the limit becomes `to_js_string` of its first `limit` elements + `"..."` (a string), and a shorter array stays; a number, a boolean or an object is an error with the text `TypeError: str.slice is not a function`. Exception, provisional ruling RD-26 (pending): TS keeps an object whose `length` property compares at most the limit under JavaScript's loose comparison (for example `{"length": 10}`); Rust treats every object as an error. Add the RD-26 row to design section 9 with "Provisional ruling (pending, milestone 3 batch)" in the Why column.
- `error.message` gets the 2048 limit; `error.frames` with more than 50 entries keeps the first 25 and the last 25 (the SDK always builds `frames` as an array).
- A `text` block of `response.content` longer than 32 768 units is cut, the block's other keys and their order unchanged.
- An array `response` stays an array (RD-25); TS turns it into an object with index keys.
- Then `normalize` with the default limits on `parameters`, `response`, `identifyActorData` and `error` when each is not `null`.
- The size guard measures `js::stringify` bytes of the whole event. Over 102 400 bytes: for depth 9 down to 1, normalize those four fields again at that depth and return the first result that fits; else normalize them at depth 1 and run the last-resort loop: at most 10 rounds; each round measures the size, returns when it fits, collects every string longer than 100 units anywhere in the event (top-level fields, `tags` and `properties` included) in `own_entries` order with array indices as path segments, stable-sorts them by length (longest first), sets `remaining = excess + 200`, and for each string while `remaining > 0`: `reduction = min(remaining, floor(len / 2))`, skips it when `reduction < 10`, else replaces it with `cut_utf16(s, len - reduction) + "..."` and subtracts `reduction`; a round with no cut stops the loop. The result can still exceed the limit.

- [ ] **Step 1: Write the generator and the failing tests**

Generator cases, built with `$repeat` for long strings and kept to the 2 MiB cap (size-guard cases use ASCII and BMP text only, because RD-22 can change a byte count after a cut):
- Each `truncateEvent` test input of `truncation.test.ts` that JSON can carry, with its `ts_test`; the Appendix C case (lengths 2051, 259, 32 771, 32 771; 68 054 bytes).
- `field:*` each field at its limit and one unit over; `field:user-intent-emoji-at-2048`; `content:emoji-at-32768`; frames of 50 and 51 entries.
- `intent:*` `userIntent` of `5`, `true`, `{"a":1}`, `{"length":10}` (`"deviation": "RD-26"`), a 2048-item array, and a 2049-item array of strings, numbers, `null`, nested arrays and objects.
- `size:*` events of exactly 102 400 and 102 401 bytes (the generator tunes a filler string); events that fit at depth 9, at depth 5 and only at depth 1; last-resort events with one huge `properties` string, two equal-length strings (stable order), strings of 101 to 120 units (reductions under 10), and 3 000 strings of 100 units in `properties` (no candidate: the oversized event comes back).
- `response:array` (`"deviation": "RD-25"`).
- 20 from `seeded(8)`.

`truncate/tests.rs`: `vectors_match_ts` (an `error` case expects `Err` with that exact text; RD-25: the Rust `response` is the input array normalized, and TS's is the object with index keys; RD-26: Rust returns `Err`); property tests `truncate_matches_ts_field_limits` (`ts_event(js_string(40))` with some fields lengthened near their limits; under 100 KB) and `truncate_matches_ts_size_guard` (`config_capped(1_000)`; `ts_event(bmp_string(..))` of 90 to 400 KB).

- [ ] **Step 2: Generate; run to see them fail; implement; run to see them pass**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk pipeline::truncate::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries of the `truncateEvent` tests**

Covered by the vector; "does not mutate the original": `ts-host-only` ("Rust borrows the input"); "preserve timestamp as Date": `ts-host-only` ("the Rust pipeline holds the timestamp as its ISO string, which no cut reaches"). `e2e-truncation.test.ts`: stays `todo`. Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk oracle parity docs/superpowers/specs
git commit -m "feat(pipeline): event truncation and the 100 KB size guard, equal to TS"
```

### Task 13: Sanitization

Model: Sonnet 5.5.

**Files:**
- Create: `agentcat-sdk/src/pipeline/sanitize.rs`, `agentcat-sdk/src/pipeline/sanitize/tests.rs`, `oracle/src/vectors/gen/sanitize.mjs`, `agentcat-sdk/tests/vectors/sanitize.json` (generated)
- Modify: `agentcat-sdk/src/pipeline/mod.rs`, `oracle/src/functions.mjs`, `parity/manifest.toml` (`src/tests/sanitization.test.ts`)

**Interfaces:**
- Consumes: `js::{utf16_len, own_entries, to_js_string}`, `strategies::ts_event`.
- Produces: `pub(crate) fn sanitize_event(event: &TsEvent) -> TsEvent`: B-090. `response` (when not `null`): an object gets each `content` block mapped (`text` and `resource_link` kept; `image` and `audio` replaced by their text blocks; `resource` whose `resource` is an object with a `blob` key, of any value, replaced; other `resource` kept; any other `type` replaced by `[unsupported content type "<to_js_string(type)>" redacted - not supported by AgentCat]`, where an absent `type` gives `undefined`; a block that is not an object kept), and a `structuredContent` that is an object or array gets the parameter walk; an array `response` stays an array (RD-25); a string or number `response` stays. `parameters` (when not `null`) gets the parameter walk: any string of at least 10 240 units matching `^[A-Za-z0-9+/\n\r]+=*$` becomes `[binary data redacted - not supported by AgentCat]`; arrays and objects recurse (objects in `own_entries` order); a `__proto__` key is kept (RD-24).
- Produces (`functions.mjs`): `"sanitization.sanitizeEvent"`.

- [ ] **Step 1: Write the generator and the failing tests**

Generator cases: each input of `sanitization.test.ts` with its `ts_test`; base64 strings of 10 239 and 10 240 units; with `=` and `==` padding, with `\n` and `\r\n`, with URL-safe `-_` characters, with text after the padding, and all `=`; non-base64 long strings; every block type including `type` as a number, `null`, an object and absent; `resource` variants (`blob: null`, `blob` string, `text`, both, `resource` a string, an array, absent); a top-level string `parameters`; `proto:kept` (RD-24) and `response:array` (RD-25); 20 from `seeded(9)`.

`sanitize/tests.rs`: `vectors_match_ts` (RD-24 and RD-25 handled as in Tasks 11 and 12); proptest `sanitize_matches_ts` (`ts_event` whose `content` mixes all block types and whose strings sometimes are long base64 runs).

- [ ] **Step 2: Generate; run to see them fail; implement; run to see them pass**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk pipeline::sanitize::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries of `sanitization.test.ts`**

"should not mutate the original event": `ts-host-only`. `e2e-sanitization.test.ts`: stays `todo`. Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk oracle parity
git commit -m "feat(pipeline): sanitization of binary content, equal to TS"
```

### Task 14: Redaction

Model: Opus 5.5.

**Files:**
- Create: `agentcat-sdk/src/pipeline/redact.rs`, `agentcat-sdk/src/pipeline/redact/tests.rs`, `oracle/src/vectors/gen/redact.mjs`, `agentcat-sdk/tests/vectors/redact.json` (generated)
- Modify: `agentcat-sdk/src/pipeline/mod.rs`, `oracle/src/functions.mjs`, `parity/manifest.toml` (`src/tests/redaction.test.ts`)

**Interfaces:**
- Consumes: `js::own_entries`, `strategies::ts_event`.
- Produces: `pub(crate) fn redact_strings<E>(event: &TsEvent, redact: impl FnMut(&str) -> Result<String, E>) -> Result<TsEvent, E>`: B-089. Walks the event in `own_entries` order (arrays by index) and passes every string to `redact`, except strings under the top-level keys `sessionId`, `id`, `projectId`, `server`, `serverName`, `serverVersion`, `clientName`, `clientVersion`, `identifyActorGivenId`, `identifyActorName`, `identifyData`, `resourceName`, `eventType`, `actorId`, `tags`, `properties`, and except the top-level `timestamp` (a `Date` in TS). Protection applies only at the top level. `identifyActorData` is redacted (K-04). The first `Err` stops the walk and is returned. A `__proto__` key is kept (RD-24). Plan 4 runs async hooks with it in two passes (collect the strings, await the hooks in order, replay the results).
- Produces: `pub(crate) fn apply_event_redaction(original: &TsEvent, hook_result: Option<TsEvent>) -> Option<TsEvent>`: B-088. `None` (the hook dropped the event) gives `None`. Otherwise the hook's map, with `id`, `sessionId`, `projectId`, `eventType`, `timestamp` restored from `original` in that order (a key the hook kept keeps its position; a key the hook removed is appended; a key absent from `original` is removed).
- Produces (`functions.mjs`): `"oracle.redactEvent"`: `{event, throwAt?}` → runs `redactEvent` with a recording redactor `s => "[R:" + s + "]"` that throws `new Error("redactor failed")` on call number `throwAt` (1-based); returns `{value, calls}` or `{error, calls}`. `"oracle.applyEventRedaction"`: `{original, replacement}` → runs `applyEventRedaction` on a copy of `original` with a hook that returns `replacement` (`null` drops); returns the event or `null`.

- [ ] **Step 1: Write the generator and the failing tests**

Generator cases: each `redactEvent` and `applyEventRedaction` input of `redaction.test.ts` that JSON can carry, with its `ts_test`; every protected key at the top level and the same names nested; `identifyActorData` strings; a top-level `timestamp` and a nested `timestamp`; index-like keys in `input_text` (they change the call order); `throwAt` 1 and 3; restore cases (the hook changes, removes or reorders each restored key; `original` without `projectId`; a `null` replacement; a replacement with new keys); `proto:kept` (RD-24); 20 from `seeded(10)`.

`redact/tests.rs`: `vectors_match_ts` (output and the exact call list; a `throwAt` case expects `Err` after exactly the recorded calls); property tests `redact_strings_matches_ts` (`ts_event(js_string(20))`; output and calls) and `apply_event_redaction_matches_ts` (random `original` and `replacement` maps over the event keys).

- [ ] **Step 2: Generate; run to see them fail; implement; run to see them pass**

Run: `node oracle/src/vectors/run.mjs && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk pipeline::redact::`
Expected: FAIL, then PASS.

- [ ] **Step 3: Map the parity entries of `redaction.test.ts`**

Pure walk and restore tests: covered by the vector. Functions, `undefined`, circular structures and the shared-reference test: `ruled-divergence:RD-20`. "preserve Date objects" and "keep the same object identity": `ts-host-only`. "propagate hook errors" and every integration test: stay `todo` (plan 4). Run `cargo run -q -p xtask -- parity` (exit 0).

- [ ] **Step 4: Gate and commit**

Run: `scripts/check.sh` (exit 0), then:

```bash
git add agentcat-sdk oracle parity
git commit -m "feat(pipeline): string redaction walk and event-hook restore, equal to TS"
```

### Task 15: Milestone close

The controller runs this task (as R-15 did for plan 1).

- [ ] **Step 1: Full gate and CI**

Run: `scripts/check.sh`, push, and watch the CI run on `rewrite`; dispatch the nightly once (`gh workflow run nightly.yml --ref rewrite`) and watch it.
Expected: both green. The output of `cargo hack --no-dev-deps` and of the minimal-versions step has no unused-dependency warning (carry-forward); `cargo run -q -p xtask -- parity` reports its counts.

- [ ] **Step 2: Update the carry-forward list**

Remove the "Plan 2" section items that this plan closed. An item that stays open moves to the plan that owns it, with the reason. Add any item that a task review deferred to a later plan.

- [ ] **Step 3: Rulings batch (R17)**

Bring every pending row of design section 9 (RD-26 at least) to the owner as one batch of multiple-choice questions, explained in plain words. Record the answers in `docs/DECISIONS.md` and design section 9.

- [ ] **Step 4: Plan index**

Mark plan 2 done in `docs/superpowers/plans/README.md` with the final commit SHA and the CI run IDs, and commit.
