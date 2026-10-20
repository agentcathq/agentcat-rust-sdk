# Plan 1: Foundation (repository, gates, TS oracle, harness TS half) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build every piece of verification infrastructure before any SDK behavior exists: the Cargo workspace with its gates, an oracle that runs the real TypeScript SDK at the pinned commit, the vector and property-test pipelines, the TS test-by-test parity checker, and the TS half of the differential harness.

**Architecture:** One Cargo workspace (`agentcat-sdk`, `xtask`) and one pnpm workspace (`oracle`, `harness`). `oracle/` clones the public TS repository at the SHA in `oracle/oracle.lock`, builds it, and exposes its modules to generators (vector JSON files), to a JSON-lines process (property tests), and to the parity list. `harness/` drives MCP reference servers with one scripted TS client and records what they publish to a local mock API. Later plans add the Rust side to each of these.

**Tech Stack:** Rust 1.88+ (edition 2024), cargo-nextest, cargo-deny, cargo-hack, cargo-minimal-versions; Node 22, pnpm 10 workspaces, esbuild, `node:test`, ajv; `@modelcontextprotocol/sdk` 1.31.0, `@modelcontextprotocol/server` 2.0.0, `@modelcontextprotocol/client` 2.3.1, `@modelcontextprotocol/node` 2.0.0, zod 3.25.76; GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-20-rust-sdk-design.md` (decisions in `docs/DECISIONS.md`; research in `docs/research/`, especially `06-verification-strategy.md` and the spike under `docs/research/spikes/diff-spike/`).

## Global Constraints

- Branch `rewrite`; commits use conventional-commit subjects and carry no AI attribution lines.
- Rust: `edition = "2024"`, `rust-version = "1.88"`; the 1.88 toolchain is installed locally (`rustup toolchain list`).
- Dependencies (when first needed): `rmcp = { version = "3", default-features = false, features = ["server"] }`; `agentcat-api = { version = "=0.0.3", default-features = false }`; `serde_json` always with `features = ["preserve_order"]`; `http = "1"`. Every `[workspace.dependencies]` entry sets `default-features = false` unless a reason is written beside it.
- Never write to stdout or stderr from library code (clippy `print_stdout`/`print_stderr` denied).
- License MIT, `Copyright (c) 2026 AgentCat, Inc.`
- TS oracle source: `https://github.com/agentcathq/agentcat-typescript-sdk.git` at `9b56e9e920f35f7c37fc2a63ce6520a9ec97edea` (package version 2.1.1). Never build the oracle from the developer checkout in `~/agentcat-typescript-sdk` (dirty tree, stale `node_modules`).
- The TS clone's own pnpm version comes from its `packageManager` field (`pnpm@10.11.0`); run it through corepack.
- Harness and oracle never contact anything but `127.0.0.1`/`localhost`: the TS servers load `harness/src/egress-guard.mjs` and refuse a non-local `AGENTCAT_API_URL`; `DISABLE_DIAGNOSTICS=true` is always set for TS servers.
- Generated files (`agentcat-sdk/tests/vectors/*.json`, `parity/ts-tests.json`, `harness/contract/*`) are never edited by hand; each has a `--check` mode that regenerates into a temp dir and diffs.
- Local tools live in `~/.cargo/bin` (not on `PATH` by default): scripts prepend it.

## Review Focus

1. **Oracle built from the wrong source.** A cached clone that is dirty or at another SHA must fail the build, not produce vectors. Pinned by `oracle/test/fetch.test.mjs` (Task 3).
2. **Harness reaching production.** A TS server started with a non-local API URL must refuse to start, and a non-local `fetch` must throw. Pinned by `harness/test/egress.test.mjs` (Task 7).
3. **A normalizer that hides real drift.** A/A runs must diff to zero, and a one-field perturbation must make the diff fail. Pinned by `harness/test/aa.test.mjs` (Task 7).
4. **Hand-edited vectors.** A vector file whose hash or `ts_sha` disagrees with the manifest or the lock must fail `cargo test`. Pinned by `agentcat-sdk/tests/vector_integrity.rs` (Task 4).
5. **Harness timing.** A run that publishes fewer or more events than the scenario expects must exit non-zero instead of passing on a short wait. Pinned by `harness/test/driver.test.mjs` (Task 7).

---

### Task 1: Cargo workspace, crate skeleton and local gate

**Files:**
- Create: `Cargo.toml`, `agentcat-sdk/Cargo.toml`, `agentcat-sdk/src/lib.rs`, `agentcat-sdk/tests/json_key_order.rs`, `agentcat-sdk/README.md`, `agentcat-sdk/CHANGELOG.md`, `clippy.toml`, `deny.toml`, `.gitignore`, `LICENSE`, `scripts/check.sh`

**Interfaces:**
- Produces: workspace members `agentcat-sdk` (lib, `publish` allowed) and the `[workspace.lints]` table every later crate inherits with `[lints] workspace = true`; `scripts/check.sh` (exit 0 = local gate green), which later tasks extend.

- [ ] **Step 0: Install the missing local tool**

`cargo-nextest` is not installed (the other cargo tools are, in `~/.cargo/bin`).
Run: `cargo install cargo-nextest --locked && ~/.cargo/bin/cargo-nextest nextest --version`
Expected: a version line.

- [ ] **Step 1: Write the failing test** `agentcat-sdk/tests/json_key_order.rs`

```rust
//! R07: `mcp_session` must stay the first key, so `serde_json/preserve_order` must be on in the SDK's own dependency.
#[test]
fn serde_json_keeps_insertion_order() {
    let mut map = serde_json::Map::new();
    map.insert("mcp_session".into(), serde_json::json!({}));
    map.insert("echoed".into(), serde_json::json!("x"));
    map.insert("a".into(), serde_json::json!(1));
    let text = serde_json::to_string(&map).unwrap();
    assert_eq!(text, r#"{"mcp_session":{},"echoed":"x","a":1}"#);
}
```

- [ ] **Step 2: Write the manifests so the test compiles and fails**

Root `Cargo.toml`: `[workspace] members = ["agentcat-sdk"]`, `resolver = "3"`; `[workspace.package]` with `edition = "2024"`, `rust-version = "1.88"`, `license = "MIT"`, `repository = "https://github.com/agentcathq/agentcat-rust-sdk"`; `[workspace.lints]` copied verbatim from `docs/research/04-rust-sdk-practices.md` §20.2. `agentcat-sdk/Cargo.toml`: `version = "1.0.0"`, `description`, `keywords = ["mcp", "rmcp", "analytics", "telemetry", "agentcat"]`, `categories = ["development-tools::debugging", "api-bindings"]`, `include = ["src/**", "README.md", "CHANGELOG.md", "LICENSE"]`, `[package.metadata.docs.rs] all-features = true, rustdoc-args = ["--cfg", "docsrs"]`, `[lints] workspace = true`. Root `[workspace.dependencies]` gets `serde_json = { version = "1", default-features = false, features = ["std"] }` (no `preserve_order` yet), and the crate uses `serde_json = { workspace = true }`.

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p agentcat-sdk --test json_key_order`
Expected: FAIL (keys come out sorted: `{"a":1,"echoed":"x","mcp_session":{}}`).

- [ ] **Step 3: Add `preserve_order` to the `serde_json` dependency**

- [ ] **Step 4: Run the test**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p agentcat-sdk --test json_key_order`
Expected: PASS.

- [ ] **Step 5: Write `src/lib.rs`, the config files and the gate**

`lib.rs`: crate docs (one paragraph: what AgentCat is, "work in progress"), `#![forbid(unsafe_code)]`, `#![cfg_attr(docsrs, feature(doc_cfg))]`; no items yet. `clippy.toml`: verbatim from practices §20.2. `deny.toml`: `[licenses] allow` = MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Unicode-3.0, Zlib (add others only with a comment naming the crate); `[bans] multiple-versions = "warn"`; `[sources] unknown-registry = "deny", unknown-git = "deny"`. `.gitignore`: `/target`, `node_modules/`, `oracle/.cache/`, `harness/out/`. `LICENSE`: MIT text. `README.md`: one-paragraph placeholder. `CHANGELOG.md`: Keep a Changelog 1.1.0 header with an empty `## [Unreleased]`.

`scripts/check.sh` (bash, `set -euo pipefail`, prepends `$HOME/.cargo/bin` to `PATH`, runs from the repo root):
1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo nextest run --workspace --all-features` (when `cargo-nextest` is absent, the script prints `install with: cargo install cargo-nextest --locked` and exits 1)
4. `cargo test --workspace --doc`
5. `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`
6. `cargo deny check`
7. `cargo +1.88 check --workspace --all-targets`

- [ ] **Step 6: Run the gate**

Run: `scripts/check.sh`
Expected: exit 0.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml agentcat-sdk clippy.toml deny.toml .gitignore LICENSE scripts/check.sh
git commit -m "build: workspace, crate skeleton, lints and local gate"
```

### Task 2: CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`, `.github/dependabot.yml`

**Interfaces:**
- Consumes: the commands of `scripts/check.sh` (Task 1).
- Produces: the `all-green` job that later tasks add needs to; job ids `fmt`, `clippy`, `hack`, `test`, `msrv`, `docs`, `deny`, `minimal-versions`.

- [ ] **Step 1: Write `ci.yml`**

Triggers: `push` to `rewrite` and `main`, `pull_request`, `workflow_dispatch`. `permissions: contents: read`. `concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }`. Every action pinned by full commit SHA with the tag in a trailing comment; resolve SHAs with `gh api repos/<owner>/<repo>/git/ref/tags/<tag> --jq .object.sha` (dereference annotated tags). Use `dtolnay/rust-toolchain`, `Swatinem/rust-cache`, `taiki-e/install-action` (for nextest, cargo-hack, cargo-minimal-versions), `EmbarkStudios/cargo-deny-action`.

Jobs (Linux unless stated, C17):
- `fmt`: `cargo fmt --all --check`.
- `clippy`: stable, `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- `hack`: `cargo hack clippy --workspace --each-feature --no-dev-deps -- -D warnings`.
- `test`: matrix `os: [ubuntu-latest]` on push, plus `macos-latest` and `windows-latest` when `github.event_name != 'push'`; `cargo nextest run --workspace --all-features` and `cargo test --workspace --doc`.
- `msrv`: toolchain `1.88`, `cargo nextest run --workspace --all-features`.
- `docs`: nightly, `RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo doc --workspace --all-features --no-deps`.
- `deny`: `cargo-deny-action` with `command: check`.
- `minimal-versions`: nightly, `cargo minimal-versions check --direct --workspace`.
- `all-green`: `needs` every job above, `if: always()`, fails unless every `needs.*.result` is `success`.

`dependabot.yml`: `github-actions` and `cargo` ecosystems, weekly.

- [ ] **Step 2: Push and watch**

Run: `git add .github && git commit -m "ci: lint, test, MSRV, docs, deny and minimal-versions gates" && git push origin rewrite && gh run watch --exit-status $(gh run list --branch rewrite --limit 1 --json databaseId --jq '.[0].databaseId')`
Expected: the run ends with `all-green` successful. If a job fails, fix it in a follow-up commit before moving on.

### Task 3: Oracle — the TS SDK at the pinned SHA

**Files:**
- Create: `package.json` (root, private), `pnpm-workspace.yaml`, `oracle/package.json`, `oracle/oracle.lock`, `oracle/src/paths.mjs`, `oracle/src/fetch-ts.mjs`, `oracle/src/bundle.mjs`, `oracle/test/fetch.test.mjs`

**Interfaces:**
- Produces (`oracle/src/paths.mjs`): `export function lock(): { repo: string, sha: string }`, `export function tsDir(): string` (the clone, `oracle/.cache/ts-<sha>`), `export function tsDist(): string` (`<tsDir>/dist/index.mjs`), `export function bundleDir(): string` (`oracle/.cache/bundle-<sha>`).
- Produces (`oracle/src/fetch-ts.mjs`): `export async function ensureTs(opts?: { force?: boolean }): Promise<void>` (`force` ignores the marker and re-verifies) and a CLI (`node oracle/src/fetch-ts.mjs`) that clones or fetches, checks out the detached SHA, verifies a clean tree, runs `corepack pnpm install --frozen-lockfile` and `corepack pnpm build` in the clone, and is a no-op when a marker file `<tsDir>/.oracle-ok` holds the SHA.
- Produces (`oracle/src/bundle.mjs`): `export async function loadModule(name: string): Promise<object>` that bundles `src/modules/<name>.ts` (or `src/thirdparty/ksuid/index.js` for `"ksuid"`) from the clone with esbuild (`format: "esm"`, `platform: "node"`, `bundle: true`) into `<bundleDir>/<name>.mjs` on first use and imports it.

- [ ] **Step 1: Write the failing test** `oracle/test/fetch.test.mjs` (`node:test`)

```js
test("the clone is clean and at the locked SHA", async () => {
  await ensureTs();
  assert.equal(git(["rev-parse", "HEAD"]), lock().sha);
  assert.equal(git(["status", "--porcelain"]), "");
  assert.ok(existsSync(tsDist()));
});
test("a dirty clone is rejected", async () => {
  writeFileSync(join(tsDir(), "src", "dirty.txt"), "x");
  try { await assert.rejects(ensureTs({ force: true }), /not clean/); }
  finally { rmSync(join(tsDir(), "src", "dirty.txt")); }
});
test("modules load from the pinned source", async () => {
  const tokens = await loadModule("tokenEstimate");
  assert.equal(typeof tokens.estimateInputTokens, "function");
  assert.equal(tokens.estimateInputTokens({ n: 1.0 }), 2);
});
```

(`git(args)` runs `git -C tsDir() …` and trims.)

- [ ] **Step 2: Run it to see it fail**

Run: `pnpm install && pnpm --filter oracle test`
Expected: FAIL (modules not found).

- [ ] **Step 3: Implement `paths.mjs`, `fetch-ts.mjs`, `bundle.mjs`**

`oracle.lock` is JSON: `{"repo": "https://github.com/agentcathq/agentcat-typescript-sdk.git", "sha": "9b56e9e920f35f7c37fc2a63ce6520a9ec97edea"}`. Root `package.json`: `"private": true`, `"packageManager": "pnpm@10.11.0"`, `"engines": {"node": ">=22"}`. `pnpm-workspace.yaml`: `packages: [oracle, harness]`. `oracle/package.json`: `"type": "module"`, `"test": "node --test test/"`, dependency `esbuild` (exact version).

- [ ] **Step 4: Run the test**

Run: `pnpm --filter oracle test`
Expected: PASS (first run clones and builds; later runs reuse the marker).

- [ ] **Step 5: Commit**

```bash
git add package.json pnpm-workspace.yaml pnpm-lock.yaml oracle
git commit -m "test: oracle that builds the TS SDK at the pinned commit"
```

### Task 4: Vector pipeline with the constants vectors

**Files:**
- Create: `oracle/src/vectors/run.mjs`, `oracle/src/vectors/gen/constants.mjs`, `agentcat-sdk/tests/vectors/constants.json` (generated), `agentcat-sdk/tests/vectors/MANIFEST.sha256` (generated), `agentcat-sdk/tests/vector_integrity.rs`
- Modify: `agentcat-sdk/Cargo.toml` (dev-dependencies `sha2`, `serde_json`, `hex`)

**Interfaces:**
- Consumes: `loadModule`, `lock` (Task 3).
- Produces: the generator contract every later vector set follows — a module in `oracle/src/vectors/gen/<name>.mjs` exporting `export async function generate(): Promise<{ cases: unknown[] }>`; `run.mjs` writes `agentcat-sdk/tests/vectors/<name>.json` as `{"ts_sha": <sha>, "generator": "oracle/src/vectors/gen/<name>.mjs", "cases": [...]}` (2-space JSON, trailing newline) and rewrites `MANIFEST.sha256` (`<hex sha256>  <file name>` per line, sorted). `node oracle/src/vectors/run.mjs --check` regenerates into a temp dir and exits 1 on any byte difference. `--out <dir>` writes the files to `<dir>` instead (the nightly sweep uses it).
- Produces (`constants.json`): `cases` = one object per exported string constant of `src/modules/constants.ts`, the `get_more_tools` descriptor and canned result of `src/modules/tools.ts`, and the mint-back texts of `src/modules/handles.ts`: `{"name": "<export name>", "value": <exact value>}`. Plan 2 asserts the Rust constants against it.

- [ ] **Step 1: Write the failing test** `agentcat-sdk/tests/vector_integrity.rs`

```rust
#[test]
fn every_vector_file_matches_the_manifest() { /* parse MANIFEST.sha256; for each line, sha256(file) == listed hash */ }
#[test]
fn every_vector_file_is_listed() { /* every tests/vectors/*.json appears in MANIFEST.sha256 */ }
#[test]
fn every_vector_file_comes_from_the_locked_sha() {
    // ts_sha of each file == "sha" in ../oracle/oracle.lock (read via CARGO_MANIFEST_DIR)
}
```

The bodies are exactly the comments; failures name the offending file.

- [ ] **Step 2: Run it to see it fail**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk --test vector_integrity`
Expected: FAIL (no manifest).

- [ ] **Step 3: Implement `run.mjs` and `gen/constants.mjs`; generate**

Run: `node oracle/src/vectors/run.mjs`
Expected: writes `constants.json` and `MANIFEST.sha256`; prints the case count.

- [ ] **Step 4: Run the tests and the check mode**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk --test vector_integrity && node oracle/src/vectors/run.mjs --check`
Expected: PASS; `--check` exits 0. Then change one byte in `constants.json`, confirm both fail, and restore it.

- [ ] **Step 5: Commit**

```bash
git add oracle/src/vectors agentcat-sdk/tests/vectors agentcat-sdk/tests/vector_integrity.rs agentcat-sdk/Cargo.toml
git commit -m "test: TS-generated vector pipeline with integrity manifest"
```

### Task 5: JS oracle process for property tests

**Files:**
- Create: `oracle/src/functions.mjs`, `oracle/src/server.mjs`, `oracle/test/server.test.mjs`, `agentcat-sdk/tests/support/mod.rs`, `agentcat-sdk/tests/support/oracle.rs`, `agentcat-sdk/tests/oracle_smoke.rs`
- Modify: `agentcat-sdk/Cargo.toml` (dev-dependency `proptest`)

**Interfaces:**
- Consumes: `loadModule` (Task 3).
- Produces (`oracle/src/functions.mjs`): `export const functions: Record<string, (...args: unknown[]) => unknown>`, starting with `"JSON.stringify"` and `"tokenEstimate.estimateInputTokens"`; Plan 2 adds entries.
- Produces (`oracle/src/server.mjs`): calls `ensureTs()` first (a no-op when the marker matches), then reads JSON lines `{"id": n, "fn": "<name>", "args": [...]}` on stdin and writes `{"id": n, "ok": <result>}` or `{"id": n, "err": "<message>"}` on stdout, one line each; `undefined` results are sent as `{"id": n, "undefined": true}`.
- Produces (`tests/support/oracle.rs`): `pub struct Oracle`; `pub fn oracle() -> &'static std::sync::Mutex<Oracle>` (one process per test binary, spawned on first use with `node <repo>/oracle/src/server.mjs`); `impl Oracle { pub fn call(&mut self, func: &str, args: &[serde_json::Value]) -> Result<Option<serde_json::Value>, String> }` (`Ok(None)` = JS `undefined`). A missing `node` panics with a message naming the requirement.
- Produces: proptest case count from `PROPTEST_CASES` (default 256). Proptest writes failing seeds to `agentcat-sdk/proptest-regressions/`; that directory is committed whenever it appears.

- [ ] **Step 1: Write the failing tests**

`oracle/test/server.test.mjs`: spawn the server, send `{"id":1,"fn":"JSON.stringify","args":[{"n":1.0}]}`, expect `{"id":1,"ok":"{\"n\":1}"}`; send an unknown `fn`, expect an `err` line; send `estimateInputTokens` with `null`, expect `{"id":3,"undefined":true}`.

`agentcat-sdk/tests/oracle_smoke.rs`:

```rust
mod support;
#[test]
fn js_drops_the_fraction_of_whole_floats() {
    let out = support::oracle::oracle().lock().unwrap()
        .call("JSON.stringify", &[serde_json::json!({"n": 1.0})]).unwrap();
    assert_eq!(out, Some(serde_json::json!("{\"n\":1}")));
}
proptest::proptest! {
    #[test]
    fn js_stringify_of_strings_round_trips(s in "\\PC*") {
        let out = support::oracle::oracle().lock().unwrap()
            .call("JSON.stringify", &[serde_json::json!(s)]).unwrap().unwrap();
        let back: String = serde_json::from_str(out.as_str().unwrap()).unwrap();
        proptest::prop_assert_eq!(back, s);
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm --filter oracle test; PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p agentcat-sdk --test oracle_smoke`
Expected: FAIL (no server).

- [ ] **Step 3: Implement `functions.mjs`, `server.mjs`, `oracle.rs`**

- [ ] **Step 4: Run them**

Run: same as Step 2.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add oracle agentcat-sdk/tests/support agentcat-sdk/tests/oracle_smoke.rs agentcat-sdk/Cargo.toml
git commit -m "test: JS oracle process for property tests"
```

### Task 6: Parity list and checker

**Files:**
- Create: `oracle/src/parity/list-tests.mjs`, `parity/ts-tests.json` (generated), `parity/manifest.toml` (generated once, then hand-maintained), `xtask/Cargo.toml`, `xtask/src/main.rs`, `xtask/src/parity.rs`
- Modify: root `Cargo.toml` (`members += "xtask"`), `scripts/check.sh` (add `cargo run -q -p xtask -- parity`)

**Interfaces:**
- Consumes: `ensureTs`, `tsDir`, `lock` (Task 3).
- Produces (`list-tests.mjs`): runs, inside the clone, `corepack pnpm exec vitest list --json` for projects `v1`, `v2`, `e2e-http` and for `vitest.workers.config.ts`, and writes `parity/ts-tests.json` = `{"ts_sha", "count", "tests": [{"key", "project", "file", "title"}]}` sorted by `key`, where `key = "<project>|<file relative to the clone>|<full title joined by ' > '>"`. Duplicate keys fail the run. `--check` and `--out <dir>` as in Task 4. `--init-manifest` writes `parity/manifest.toml` with one `todo` entry per key and refuses to overwrite an existing file.
- Produces (`parity/manifest.toml` schema), one table per TS test:

```toml
[[test]]
key = "v1|src/tests/handles.test.ts|resolveHandles > mints a handle on start"
status = "ported"            # "todo" | "ported" | "n/a"
rust = ["agentcat-sdk::handles::resolve_mints_on_start", "harness:basic"]   # ported only
category = "ts-host-only"    # n/a only: ts-host-only | second-pass | ruled-divergence:RD-xx | covered-by-vector:<file>.json
reason = "…"                 # n/a only
```

- Produces (`xtask/src/parity.rs`): `pub struct TsTests`, `pub struct Manifest` (serde), `pub enum Problem { Unmapped(String), Stale(String), DanglingRust { key: String, test: String }, DanglingScenario { key: String, scenario: String }, Todo(String), MissingReason(String), BadCategory { key: String, category: String } }`, and `pub fn check(ts: &TsTests, manifest: &Manifest, rust_tests: &BTreeSet<String>, scenarios: &BTreeSet<String>, final_gate: bool) -> Vec<Problem>`. `main.rs` subcommand `parity [--final]` loads `parity/ts-tests.json` and `parity/manifest.toml`, collects Rust test names from `cargo nextest list --workspace --all-features --message-format json` as `<binary-id>::<test name>` (for example `agentcat-sdk::handles::resolve_mints_on_start` for `tests/handles.rs`, `agentcat-sdk::tokens::tests::rounds_up` for a unit test), and scenario names from `harness/scenarios/*.json` file stems. A `rust` entry that starts with `harness:` names a scenario; every other entry names a Rust test. It prints each problem and exits 1 if any. `todo` is a problem only with `--final`.

- [ ] **Step 1: Write the failing unit tests** in `xtask/src/parity.rs`

One `#[test]` per `Problem` variant, each building a two-key `TsTests` and a small `Manifest` in code and asserting `check(..)` returns exactly that problem; plus `clean_manifest_has_no_problems` and `todo_is_allowed_without_final`.

- [ ] **Step 2: Run them to see them fail**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p xtask`
Expected: FAIL (`check` not implemented).

- [ ] **Step 3: Implement `parity.rs` and `main.rs`** (dependencies: `serde`, `serde_json`, `toml`; `publish = false`; `[lints] workspace = true`)

- [ ] **Step 4: Generate the list and the manifest, run everything**

Run: `node oracle/src/parity/list-tests.mjs && node oracle/src/parity/list-tests.mjs --init-manifest && PATH=$HOME/.cargo/bin:$PATH cargo nextest run -p xtask && cargo run -q -p xtask -- parity`
Expected: the list has about 1,020 keys (record the exact count in the commit message); unit tests pass; `parity` exits 0; `parity --final` exits 1 listing `todo` keys.

- [ ] **Step 5: Commit**

```bash
git add oracle/src/parity parity xtask Cargo.toml scripts/check.sh
git commit -m "test: generated TS test list and parity checker (N keys)"
```

### Task 7: Harness TS half (v1 reference server, mock API, driver, normalizer, rulings)

Port the spike (`docs/research/spikes/diff-spike/`) into a maintained package, renaming `SPIKE_*` environment variables to `HARNESS_*`.

**Files:**
- Create: `harness/package.json`, `harness/README.md`, `harness/src/egress-guard.mjs`, `harness/src/mock-api.mjs`, `harness/src/driver.mjs`, `harness/src/normalize.mjs`, `harness/src/diff.mjs`, `harness/src/servers/ts-v1.mjs`, `harness/src/dump-contract.mjs`, `harness/contract/tool-contract.json` (generated), `harness/scenarios/basic.json`, `harness/rulings.json`, `harness/test/egress.test.mjs`, `harness/test/driver.test.mjs`, `harness/test/aa.test.mjs`

**Interfaces:**
- Consumes: `ensureTs`, `tsDist` (Task 3); the TS server imports the SDK with `await import(pathToFileURL(tsDist()))`.
- Produces — the server protocol every reference server (TS now, Rust in Plan 3) follows: argv `stdio`, `http <port>` (stateful streamable HTTP, one tracked server per MCP session, as in the spike) or `http-stateless <port>` (no session ID; one tracked server per request, as the TS e2e stateless lane does); for HTTP, one JSON ready line `{"ready": true, "url": "http://127.0.0.1:<port>/mcp"}` on stdout; env `AGENTCAT_API_URL` (must be local), `HARNESS_PROJECT_ID`, `HARNESS_OPTIONS` (JSON options profile), `HARNESS_NO_TRACK=1` (serve untracked), `HOME` (a harness temp dir).
- Produces — the options profile (documented in `harness/README.md`, every key needs a Rust twin later). Flags copied as given: `enableTracing`, `enableReportMissing`, `enableToolCallContext`, `enableAgentTracking`, `customContextDescription`. Hooks, each a canned behavior: `resolveSessionId`, `identify`, `eventTags`, `eventProperties`, `redactSensitiveInformation`. A behavior is `{"returns": <value or null>}`, `{"throws": "<message>"}`, or either of these plus `"sleepMs": <n>`; `redactSensitiveInformation` also accepts `{"replace": "<secret>"}` (every occurrence becomes `[REDACTED]`). The TS server turns each behavior into a function with the hook's signature from `src/types.ts` at the pinned SHA.
- Produces — the scenario format of the spike's `basic.json` with the new options profile; `expectPublishes` is a number, or an object keyed by transport (`stdio`, `http`, `http-stateless`) when the counts differ.
- Produces — `node harness/src/driver.mjs <scenario.json> <label> <stdio|http|http-stateless> [--lane 2025-11-25|2026-07-28] -- <server cmd…>`: writes `harness/out/<scenario>.<label>.<transport>.{raw,transcript,normalized,report}.json`, exits 1 when the publish count differs from `expectPublishes` after the settle window (`settleTimeoutMs` default 10 000, `quietMs` default 500).
- Produces — `node harness/src/diff.mjs <a.normalized.json> <b.normalized.json> [--rulings harness/rulings.json]`: exits 1 on any difference not matched by a ruling, and on any ruling that matched nothing. `rulings.json` = `[{"path": "<regex>", "reason": "…", "deviation": "RD-xx"}]`, empty at first.
- Produces — the normalizer rules of the spike (camelCase alias drop, timestamp class, duration class, language fields, extra headers only, error stack elided, ID placeholders) plus three new rules, each listed in `report.rules`:
  - `mcp-session-first`: every `structuredContent` that has `mcp_session` must have it as its first key; a violation becomes a diff entry.
  - `property-order`: for each tool in a `tools/list` result, the key order of `inputSchema.properties` is added as an explicit array, so an order difference shows as a diff (object comparison ignores order).
  - `literal-derived-ids`: when the profile's `resolveSessionId` returns a string, the normalizer computes `deriveSessionId(value.trim(), projectId)` with `loadModule("handles")` and leaves that ID literal instead of mapping it to a placeholder.

- [ ] **Step 1: Write the failing tests**

`egress.test.mjs`: (a) spawning `ts-v1.mjs stdio` with `AGENTCAT_API_URL=https://api.agentcat.com` exits with code 2 and stderr `refusing to start`; (b) with the guard preloaded, `fetch("https://api.agentcat.com/")` rejects with `/egress-guard: blocked/`.

`driver.test.mjs`: running `basic.json` with `expectPublishes` changed to 12 (a temp copy) against the TS server exits 1; with 11 it exits 0 and `report.json` has `publishes: 11`.

`aa.test.mjs`: (a) two stdio runs of `basic.json` diff to zero; (b) one stateful HTTP run publishes 11; (c) one stateless HTTP run exits 0 against the `http-stateless` count in `basic.json` (take the count from the first TS run; if it is not 11, the commit message says why); (d) a copy of a normalized file with one event's `resource_name` changed makes `diff.mjs` exit 1; (e) a rulings file with a ruling that matches nothing makes `diff.mjs` exit 1; (f) a `structuredContent` whose first key is not `mcp_session` produces a diff entry; (g) two tool schemas that differ only in property order produce a diff entry; (h) a run with `resolveSessionId: {"returns": "customer-session-1"}` keeps the derived `ses_` ID literal in the normalized file.

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm --filter harness test`
Expected: FAIL.

- [ ] **Step 3: Implement the harness files** (dependencies pinned exactly: `@modelcontextprotocol/sdk` 1.31.0, `zod` 3.25.76); generate `tool-contract.json` with `node harness/src/dump-contract.mjs` (runs `ts-v1.mjs` untracked and writes its `tools/list` result).

- [ ] **Step 4: Run the tests**

Run: `pnpm --filter harness test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add harness pnpm-lock.yaml
git commit -m "test: differential harness, TS half (v1 server, driver, normalizer, rulings)"
```

### Task 8: TS v2 reference server and the 2026-07-28 lane (C18)

**Files:**
- Create: `harness/src/servers/ts-v2.mjs`, `harness/test/lane-2026.test.mjs`
- Modify: `harness/src/driver.mjs` (the `--lane` option), `harness/package.json` (`@modelcontextprotocol/server` 2.0.0, `@modelcontextprotocol/node` 2.0.0, `@modelcontextprotocol/client` 2.3.1, exact)

**Interfaces:**
- Consumes: the server protocol, options profile and driver of Task 7; the tool contract (`harness/contract/tool-contract.json`) — `ts-v2.mjs` must serve the same six tools with the same schemas.
- Produces: `--lane 2026-07-28` connects with the v2 `Client` and `versionNegotiation: { mode: "auto" }` (see the TS repo's `src/tests/e2e-http/lanes.ts`, `connectV2Client`); `--lane 2025-11-25` (default) keeps the v1 client.

- [ ] **Step 1: Write the failing test** `lane-2026.test.mjs`: `basic.json` against `ts-v2.mjs` over HTTP with `--lane 2026-07-28` publishes 11 events, two runs diff to zero, and the transcript's `tools/list` tools equal the tool contract's names and input schemas (before AgentCat's injections, checked with `HARNESS_NO_TRACK=1`).

- [ ] **Step 2: Run it to see it fail**

Run: `pnpm --filter harness test -- --test-name-pattern 2026`
Expected: FAIL.

- [ ] **Step 3: Implement `ts-v2.mjs` and the lane option** (HTTP via `createMcpHandler`/`toNodeHandler` as the TS e2e lanes do; stdio via the v2 stdio transport).

- [ ] **Step 4: Run the harness tests**

Run: `pnpm --filter harness test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add harness pnpm-lock.yaml
git commit -m "test: TS v2 reference server and the 2026-07-28 client lane"
```

### Task 9: Wire contract check (L4) and the local ingest replay

**Files:**
- Create: `harness/src/contract/extract-schema.mjs`, `harness/contract/publish-event.schema.json` (generated), `harness/contract/SOURCE` (generated), `harness/src/contract/validate.mjs`, `harness/test/contract.test.mjs`, `harness/ingest/test_ingest_capture.py`, `harness/ingest/README.md`

**Interfaces:**
- Produces (`extract-schema.mjs`): `node harness/src/contract/extract-schema.mjs <path to agentcat-server checkout> <server sha>` reads `schema.yml` at that SHA (`git show <sha>:schema.yml`), takes the request-body schema of `POST /v1/public/events/`, converts OpenAPI 3.0 `nullable: true` to JSON Schema `type: [<t>, "null"]`, resolves `$ref`s inline, and writes `publish-event.schema.json` plus `SOURCE` (`agentcat-server <sha> schema.yml`). Use the current `origin/main` of `~/agentcat-server` (fetch first) and record its SHA. `--check` mode as in Task 4 (needs the checkout, so it runs locally, not in CI).
- Produces (`validate.mjs`): `node harness/src/contract/validate.mjs <raw.json>` validates every `POST /v1/public/events/` body with ajv and exits 1 listing each invalid body and error. The driver calls it after every run and records `contract: "ok" | <errors>` in `report.json`.
- Produces (`harness/ingest/`): the spike's ingest test, adapted to read `CAPTURE_FILE`, plus a README with the exact local steps (agentcat-server checkout, Docker, the test command) (R14). Not run in CI.

- [ ] **Step 1: Write the failing test** `contract.test.mjs`: every body from a fresh `basic.json` TS run validates; the same body with `project_id` removed fails; with `session_id: 5` fails.

- [ ] **Step 2: Run it to see it fail**

Run: `pnpm --filter harness test -- --test-name-pattern contract`
Expected: FAIL.

- [ ] **Step 3: Implement, extract the schema, wire `validate` into the driver**

- [ ] **Step 4: Run the harness tests; run the ingest replay once locally**

Run: `pnpm --filter harness test`, then follow `harness/ingest/README.md` against `harness/out/basic.ts.stdio.raw.json`.
Expected: PASS; the replay prints `INGESTED 11 bodies OK`. Record the replay result in the commit message.

- [ ] **Step 5: Commit**

```bash
git add harness
git commit -m "test: publish-body contract check and local ingest replay"
```

### Task 10: CI for the oracle and harness, and the nightly drift sweep

**Files:**
- Modify: `.github/workflows/ci.yml` (job `oracle-harness`, added to `all-green`'s `needs`), `scripts/check.sh` (add the same Node steps)
- Create: `.github/workflows/nightly.yml`

**Interfaces:**
- Consumes: every CLI above.
- Produces: job `oracle-harness` (Linux): Node 22 via `actions/setup-node`, `corepack enable`, `pnpm install --frozen-lockfile`, cache `oracle/.cache` keyed on `hashFiles('oracle/oracle.lock')`, then `pnpm -r test`, `node oracle/src/vectors/run.mjs --check`, `node oracle/src/parity/list-tests.mjs --check`, `cargo run -q -p xtask -- parity`. The Rust `test` and `msrv` jobs also run the Node 22, corepack, `pnpm install --frozen-lockfile` and `oracle/.cache` steps, because the oracle smoke test spawns `node oracle/src/server.mjs`.
- Produces (`nightly.yml`): `schedule` (daily 06:00 UTC) and `workflow_dispatch`; `permissions: contents: read, issues: write`. Steps: resolve the TS repository's current `main` with `git ls-remote <repo> refs/heads/main`, build the oracle at that SHA (override the lock with an env var `ORACLE_SHA` read by `paths.mjs`), regenerate vectors and the test list with `--out <temp dir>`, diff them against the committed files ignoring the `ts_sha` field, run `basic.json` A/A on both TS servers, and run the Rust property tests with `PROPTEST_CASES=10000` (at the pinned SHA); on any difference or failure, create or update one open issue titled `TS drift: <date>` with the diff summary (`gh issue list --label ts-drift` / `gh issue create|comment`). Later plans add the rmcp matrix, the canary and full mutation runs to this file.

- [ ] **Step 1: Add `ORACLE_SHA` support to `paths.mjs`** with a test in `fetch.test.mjs`: with `ORACLE_SHA` set, `lock().sha` returns it.

- [ ] **Step 2: Write the job and the workflow; run the local gate**

Run: `scripts/check.sh`
Expected: exit 0.

- [ ] **Step 3: Push and watch CI; dispatch the nightly once**

Run: `git add .github scripts oracle && git commit -m "ci: oracle and harness gates; nightly TS drift sweep" && git push origin rewrite`, then `gh run watch --exit-status <ci run id>` and `gh workflow run nightly.yml --ref rewrite && gh run watch --exit-status <nightly run id>`
Expected: CI `all-green` succeeds; the nightly run succeeds (TS `main` equals the pinned SHA today, so no issue is opened).

### Task 11: Milestone close

- [ ] **Step 1: Full gate and CI**

Run: `scripts/check.sh` and confirm the latest CI run on `rewrite` is green.
Expected: both green.

- [ ] **Step 2: Rulings batch (R17)**

List every difference found during this plan that is not forced (for example, host-SDK baseline differences seen while building the harness). If there are any, bring them to the owner as one batch of multiple-choice questions and record the answers in `docs/DECISIONS.md` and the design's section 9. If there are none, say so in the plan index.

- [ ] **Step 3: Update the plan index**

Mark plan 1 done in `docs/superpowers/plans/README.md` with the final commit SHA, and commit.
