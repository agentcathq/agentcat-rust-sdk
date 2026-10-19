# Plan 1: Foundation (gates, parity harness, primitives, injection) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Put the design §11 build gates and CI in force on the existing crate, add the TS parity harness with its completeness tripwire, and bring every MCP-independent primitive (agent-facing text, KSUID and session IDs, UUIDv7, tag validation, token estimates, handles, tool-list injection, argument stripping) to byte parity with the TypeScript SDK, reverting the earlier "fixed" TS defects in these areas (D15).

**Architecture:** The existing crate is reworked in place, test first. Work runs on two stacked branches: `plan-1a-gates` (Tasks 1–3, pull request into `main`) and `plan-1b-parity-core` (Tasks 4–16, pull request into `plan-1a-gates`; GitHub retargets it to `main` when 1a merges). The parity suite compiles into the library's unit-test binary through `#[cfg(test)] #[path = "../tests/parity/mod.rs"] mod parity;`, so parity tests call `pub(crate)` items directly and the public API does not grow. A manifest generated from vitest's own test list maps each of the 1,024 TS test cases to a Rust test path; a tripwire test lists the binary's own tests and fails when a case of an enforced TS file has no Rust test.

**Tech Stack:** Rust stable and 1.88 (MSRV), edition 2024, rmcp 3.x, serde_json with `preserve_order`, cargo-deny 0.20.2, cargo-hack 0.6.45, cargo-public-api 0.52.0 (nightly toolchain), cargo-minimal-versions 0.1.37, cargo-llvm-cov 0.9.1, cargo-mutants 27.1.0, GitHub Actions, Python 3 (scripts), Node 22 with the TS repo's vitest (to list TS tests).

**Spec:** `docs/superpowers/specs/2026-10-19-rust-sdk-design.md` and `docs/DECISIONS.md` (the log wins on conflict). Behavioral source of truth: the TS repo `/home/ubuntu/agentcat-typescript-sdk` at `9b56e9e`, indexed row by row in `docs/research/06a`–`06c-ts-test-index.md`. Shared golden fixtures: `/home/ubuntu/agentcat-php-sdk/docs/audit/fixtures/`.

## Global Constraints

- rmcp `>= 3.0, < 4`. The crate and all tests build and pass against rmcp 3.0.0, 3.3.0 and the newest 3.x (3.5.1 today). MSRV 1.88, edition 2024.
- Runtime dependencies stay those of spec §6. This plan changes only `getrandom` (0.3 → 0.4). New dev-dependencies: `regex = "1"`.
- Behavior copies TS at `9b56e9e`, known defects included (D15). A difference is allowed only when `DECISIONS.md` (R1–R11) or spec §10 lists it.
- SDK log lines use the TS message texts verbatim, TS option names included (`resolveSessionId`, `track()`, `enableReportMissing`).
- String limits count UTF-16 code units (D09); a cut lands on a `char` boundary.
- JSON key order is part of the contract. Never call `serde_json::Map::remove` (with `preserve_order` it is `swap_remove` and moves the last key); use `shift_remove`, as JS `delete` keeps the order. `Map::insert` on an existing key keeps its position, as a JS assignment does.
- Outside tests: no `unwrap`, `expect`, `panic!`, or panicking index or slice; no `unsafe`; nothing on stdout or stderr.
- Internal items stay `pub(crate)`. `public-api.txt` changes only in a commit that changes the public API on purpose.
- An item that only tests use (a TS-library method the SDK itself never calls) is `#[cfg(test)]`. `scripts/check.sh` passes after every task, so no commit leaves dead code behind.
- Parity test paths are copied exactly from column `rust_test` of `tests/parity/manifest.tsv` (prefix `parity::`). Each `describe` level is a nested `mod` with `use super::*;`; the `it` title is the `fn`.
- Every local test run sets `AGENTCAT_DISABLE_DIAGNOSTICS=1`.
- Code comments: only for a constraint the code cannot show; never mention plans, tasks, reviews or tools. Commit messages: one line, Conventional Commits, no AI attribution, no `Co-Authored-By`.

## Review Focus

1. A tool whose `inputSchema.properties` (or `outputSchema.properties`) is a string, number, boolean or array must not break `tools/list`: a primitive takes the TS failure path (that tool listed unmodified, registry entry rolled back, `WARN: Handle injection failed for tool "x"; listing it unmodified - TypeError: Cannot create property 'session_id' on string 'abc'`), an array keeps its value while the registry records the parameter (JS writes a named key that JSON drops), and every other tool is still injected → Task 12 adds `malformed_schema_parts_take_the_ts_failure_path`.
2. Concurrent `tools/list` calls through one `AgentCat` (stateless HTTP builds one `Tracked` per request) must not panic, must log each `session_id` conflict exactly once, and must leave the declared set complete → Task 12 adds `concurrent_listings_report_each_conflict_once` (8 threads × 100 listings).
3. A `session_id` or `agent_id` argument that is a number, object, `null`, boolean or array resolves exactly like omission (mint, no agent) → Task 11 adds `non_string_handles_resolve_like_omission`.
4. A KSUID string with characters outside base62 (dashes, spaces, `é`, emoji) decodes exactly as the TS decoder does, so PostHog UUIDs agree across SDKs → Task 7 adds `ts_decoder_semantics_for_non_base62_input`, Task 8 adds `non_ksuid_input_matches_ts`.
5. Arguments or structured content nested 5,000 levels deep never overflow the stack; the token estimate is omitted instead → Task 10 adds `deeply_nested_values_omit_the_estimate`.

---

## File map

```
Cargo.toml                         getrandom 0.4; dev-dependency regex
clippy.toml                        disallowed stdout/stderr/exit and Map::remove (Task 1)
deny.toml                          licenses, bans, sources, advisories (Task 2)
public-api.txt                     cargo public-api snapshot (Task 2)
.cargo/mutants.toml                cargo-mutants settings (Task 15)
.github/workflows/ci.yml           pull-request and push gates (Tasks 3, 15)
.github/workflows/nightly.yml      newest rmcp, rmcp main, full mutation run (Task 15)
.github/dependabot.yml             security updates only (Task 3)
scripts/check.sh                   local gates (Tasks 2, 3, 15)
scripts/coverage-floor             line-coverage floor, one integer (Tasks 3, 15)
scripts/mutants-floor.env          nightly mutation floors (Task 15)
scripts/mutants-score.py           mutation score per group (Task 15)
scripts/parity/build-manifest.py   manifest and NOT_PORTED generator (Task 4)
scripts/parity/vitest/*.json       vitest's list of TS test names at 9b56e9e (Task 4)
scripts/parity/README.md           how to regenerate (Task 4)
src/lib.rs                         crate attributes (Task 1); `mod js` (Task 5); `mod parity` (Task 4)
src/js.rs                          JavaScript semantics the TS code relies on (Task 5)
src/logging.rs                     test-only log capture (Task 4)
src/ksuid.rs                       TS decoder semantics, Result API (Task 7)
src/exporters/posthog.rs           UUIDv7 on the new parse (Task 8)
src/validation.rs                  TS tag rules (Task 9)
src/tokens.rs                      depth cap (Task 10)
src/handles.rs                     unclamped protocol tag (Task 11)
src/inject.rs                      build_injected_list, TS failure paths, stripping (Tasks 12, 13)
src/tracked.rs                     listing and stripping on the new pipeline (Tasks 12, 13)
tests/parity/mod.rs                parity suite root
tests/parity/support.rs            fixtures, sid(), obj(), log capture re-export
tests/parity/completeness.rs       the tripwire
tests/parity/manifest.tsv          generated
tests/parity/NOT_PORTED.md         generated
tests/parity/golden.rs             PHP-fixture checks that are not TS test titles
tests/parity/vectors.rs            migrated tests/ts_parity.rs vectors for this plan's areas (Task 14)
tests/parity/<ts file module>.rs   constants, ksuid, posthog_uuidv7, validation, tokenestimate,
                                   handles, handle_injection, handle_conflict_report,
                                   context_parameters_edge_cases
tests/fixtures/php-audit/**        copied PHP audit fixtures + README.md (Task 4)
tests/ts_parity.rs, tests/hooks.rs, tests/composed.rs   updated (Tasks 9, 13, 14)
docs/PORTING.md                    deleted (Task 16)
docs/superpowers/plans/README.md   plan index (Task 16; created with this plan)
```

---

### Task 1: Lint gates

**Files:**
- Modify: `src/lib.rs` (crate attributes), every `src/**/*.rs` and `examples/**/*.rs` with a finding
- Create: `clippy.toml`

**Interfaces:**
- Produces: the lint configuration every later task builds under.

- [ ] **Step 0: Branch.** `git switch main && git pull --ff-only && git switch -c plan-1a-gates`.

- [ ] **Step 1: Write the gate configuration.** Replace the lint attributes at the top of `src/lib.rs` (keep the crate doc comment and `#![cfg_attr(docsrs, feature(doc_cfg))]`) with:

```rust
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(not(test), warn(clippy::pedantic))]
// AgentCat must never take the customer's server down.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::allow_attributes_without_reason
    )
)]
// Pedantic lints that do not fit this crate: options mirror the TS flags
// (many bools), public names repeat their module (`exporters::ExporterConfig`),
// and `#[must_use]` goes on public values that matter, not on every function.
#![cfg_attr(
    not(test),
    allow(
        clippy::must_use_candidate,
        clippy::module_name_repetitions,
        clippy::struct_excessive_bools,
        clippy::fn_params_excessive_bools
    )
)]
```

Keep the existing feature-dependent `allow(dead_code, …)` block, and give it a `reason = "…"`.

Create `clippy.toml`:

```toml
msrv = "1.88"
disallowed-macros = [
    { path = "std::println", reason = "stdout is the JSON-RPC channel of stdio servers" },
    { path = "std::print", reason = "stdout is the JSON-RPC channel of stdio servers" },
    { path = "std::eprintln", reason = "AgentCat never writes to stderr; use crate::logging::write_to_log" },
    { path = "std::eprint", reason = "AgentCat never writes to stderr; use crate::logging::write_to_log" },
    { path = "std::dbg", reason = "writes to stderr" },
]
disallowed-methods = [
    { path = "std::io::stdout", reason = "stdout is the JSON-RPC channel of stdio servers" },
    { path = "std::io::stderr", reason = "AgentCat never writes to stderr" },
    { path = "std::process::exit", reason = "AgentCat never ends the host process" },
    { path = "serde_json::map::Map::remove", reason = "with preserve_order this is swap_remove and reorders keys; JS delete keeps order: use shift_remove" },
    { path = "serde_json::map::OccupiedEntry::remove", reason = "use shift_remove; JS delete keeps key order" },
]
```

- [ ] **Step 2: Run the gates and see them fail.**

Run: `cargo clippy --lib --all-features -- -D warnings`
Expected: FAIL with about 190 findings (today: `doc_markdown` 53, `assigning_clones` 22, `indexing_slicing` 20, `return_self_not_must_use` 19, `cast_*` 38, `map_unwrap_or` 9, and smaller groups), plus `disallowed_methods` hits for `Map::remove`.

- [ ] **Step 3: Fix every finding.** Rules:
  - `doc_markdown`: put identifiers in backticks.
  - `return_self_not_must_use`: add `#[must_use]` to the builder methods.
  - `indexing_slicing`: use `get`, `get_mut`, `first_chunk`, `split_first`, iterators or `<[u8; N]>::try_from(slice)`; a fallback value replaces the panic.
  - `cast_*`: prefer `u32::try_from(x).unwrap_or(u32::MAX)` or `f64::from`. Where a cast is exact by construction (for example token math below 2^53), use `#[allow(clippy::cast_precision_loss, reason = "…")]` on the smallest item.
  - `Map::remove` → `shift_remove` (this also fixes key order in `strip_arguments` and in `additionalProperties` removal).
  - Every `#[allow]` carries `reason = "…"`.
  - `cargo clippy --fix --lib --allow-dirty -- -W clippy::pedantic` may apply the mechanical ones first; review its diff.
  - Test code (`#[cfg(test)]`, `tests/`, `examples/`) is not pedantic-checked, but `disallowed_macros` and `disallowed_methods` apply there too. Replace any `println!` in tests and examples.

- [ ] **Step 4: Run the gates and see them pass.**

Run: `cargo clippy --lib --all-features -- -D warnings && cargo clippy --lib -- -D warnings && cargo clippy --all-targets --all-features -- -D warnings`
Expected: no warnings.
Run: `cargo fmt --check && AGENTCAT_DISABLE_DIAGNOSTICS=1 cargo test --all-targets && cargo test --doc && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`
Expected: all pass (97 tests today plus doc tests).

- [ ] **Step 5: Prove the bans bite.** Add `println!("x");` inside any function in `src/util.rs`, run `cargo clippy --lib -- -D warnings` → error `use of a disallowed macro std::println`. Replace it with `serde_json::Map::new().remove("k");` → error `use of a disallowed method`. Remove the line.

- [ ] **Step 6: Commit.**

```bash
git add -A && git commit -m "build: strict lint gates (pedantic, no panics, no stdout, ordered key removal)"
```

### Task 2: Dependency, MSRV, feature and public-API gates

**Files:**
- Modify: `Cargo.toml` (`getrandom = "0.4"`), `src/ksuid.rs` (getrandom call if its API moved), `Cargo.lock`
- Create: `deny.toml`, `public-api.txt`, `scripts/check.sh`

**Interfaces:**
- Produces: `scripts/check.sh [--full] [--bless-public-api]` (Task 3 and Task 15 extend it); `public-api.txt` (the snapshot later plans update on purpose).

- [ ] **Step 1: Install the tools.**

```bash
cargo install --locked cargo-deny@0.20.2 cargo-hack@0.6.45 cargo-public-api@0.52.0 cargo-minimal-versions@0.1.37 cargo-llvm-cov@0.9.1
rustup toolchain install nightly --profile minimal
rustup toolchain install 1.88 --profile minimal
```

- [ ] **Step 2: Write `deny.toml`.**

```toml
[graph]
all-features = true
exclude-dev = true

[advisories]
version = 2
yanked = "deny"

[licenses]
version = 2
allow = ["MIT", "Apache-2.0", "Unicode-3.0", "ISC", "BSD-3-Clause", "Zlib"]
confidence-threshold = 0.93

[bans]
multiple-versions = "deny"
wildcards = "deny"
skip = []

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

- [ ] **Step 3: Run it and see it fail.** `cargo deny check` → FAIL on duplicates: `getrandom` 0.3.4 (this crate) and 0.4.3 (`uuid` via rmcp), `r-efi` 5/6 (through the two getrandom versions), `syn` 2.0.x and 3.0.x (`agentcat-api` → `serde_with` → `darling` 0.24 uses syn 3).

- [ ] **Step 4: Fix.** Set `getrandom = "0.4"` (its MSRV is 1.85) and run `cargo update -p getrandom`; `getrandom::fill(&mut buf)` keeps its name in 0.4 (follow the compiler if not). For each duplicate that remains, add a skip with the reason, for example `{ crate = "syn@2.0.119", reason = "agentcat-api's serde_with uses darling 0.24 on syn 3; everything else is on syn 2" }` (use the exact older version `cargo deny check bans` prints). Run `cargo deny check` → `advisories ok, bans ok, licenses ok, sources ok`.

- [ ] **Step 5: MSRV, minimal versions, feature powerset.**

Run: `cargo +1.88 check --lib --all-features` → pass.
Run: `cargo +nightly minimal-versions check --direct --lib --all-features` → pass. If a direct dependency fails at its declared floor, raise that floor in `Cargo.toml` to the lowest version that builds (`serde_json` must allow `Map::shift_remove`).
Run: `cargo hack check --lib --feature-powerset --depth 2 --exclude-features default,exporters` → pass.

- [ ] **Step 6: Public API snapshot.** `cargo +nightly public-api --simplified > public-api.txt`. If cargo-public-api rejects the nightly's rustdoc JSON format, install the nightly date its error names (for example `rustup toolchain install nightly-2026-10-01`) and use that toolchain name everywhere this plan says `nightly` for public-api (here, `scripts/check.sh`, `ci.yml`).

- [ ] **Step 7: Write `scripts/check.sh`** (`chmod +x`):

```bash
#!/usr/bin/env bash
# Runs the pull-request gates locally.
# Usage: scripts/check.sh [--full] [--bless-public-api]
set -euo pipefail
cd "$(dirname "$0")/.."
export AGENTCAT_DISABLE_DIAGNOSTICS=1 DISABLE_DIAGNOSTICS=1
full=0
bless=0
for arg in "$@"; do
  case "$arg" in
    --full) full=1 ;;
    --bless-public-api) bless=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done
step() { printf '\n== %s\n' "$*"; }

step fmt
cargo fmt --check
step clippy
cargo clippy --lib --all-features -- -D warnings
cargo clippy --lib -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
step test
cargo test --all-targets
cargo test --doc
step docs
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
step deny
cargo deny check
step public-api
if [ "$bless" = 1 ]; then cargo +nightly public-api --simplified > public-api.txt; fi
cargo +nightly public-api --simplified | diff -u public-api.txt -
if [ "$full" = 1 ]; then
  step msrv
  cargo +1.88 check --lib --all-features
  step minimal-versions
  cargo +nightly minimal-versions check --direct --lib --all-features
  step features
  cargo hack check --lib --feature-powerset --depth 2 --exclude-features default,exporters
fi
```

- [ ] **Step 8: Run** `scripts/check.sh --full` → every step passes.

- [ ] **Step 9: Commit.**

```bash
git add -A && git commit -m "build: cargo-deny, MSRV, minimal-versions, feature powerset and public API gates"
```

### Task 3: CI workflow and pull request 1a

**Files:**
- Rewrite: `.github/workflows/ci.yml`
- Create: `.github/dependabot.yml`, `scripts/coverage-floor`
- Modify: `scripts/check.sh` (coverage step in `--full`)

**Interfaces:**
- Consumes: `scripts/check.sh` commands (Task 2).
- Produces: required check `all-green`; `scripts/coverage-floor` (Task 15 raises it).

- [ ] **Step 1: Write `scripts/coverage-floor`** with the single line `81` (measured on `main` at `e3c8c25`: 81.5 % lines). Add to the `--full` block of `scripts/check.sh`:

```bash
  step coverage
  cargo llvm-cov --all-targets --summary-only --fail-under-lines "$(cat scripts/coverage-floor)"
```

- [ ] **Step 2: Write `.github/workflows/ci.yml`.** Triggers: `push` to `main`, `pull_request`, `workflow_dispatch` (drop the old weekly schedule; the nightly workflow of Task 15 replaces it). `permissions: contents: read`. `concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }`. `env: { CARGO_TERM_COLOR: always, AGENTCAT_DISABLE_DIAGNOSTICS: "1", DISABLE_DIAGNOSTICS: "1", RUST_BACKTRACE: "1" }`. Pin every action by SHA:
  - `actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1`
  - `dtolnay/rust-toolchain@683b588c487a69d01f6c6ddf40f036ff682cc2c8 # master 2026-10-16` (always with `toolchain:`)
  - `Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2`
  - `taiki-e/install-action@8cca0918f8471c33515202b73f56ab27f2bed283 # v2.87.29`
  - `EmbarkStudios/cargo-deny-action@3c6349835b2b7b196a839186cb8b78e02f7b5f25 # v2.1.1`
  - `actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2`

  Jobs (each: checkout, toolchain, rust-cache, then the commands):

  | Job `name` | Runner | Toolchain / tools | Commands |
  |---|---|---|---|
  | `lint` | ubuntu-latest | stable + clippy, rustfmt | `cargo fmt --check`; the three clippy commands of `check.sh`; both `cargo doc` commands with `RUSTDOCFLAGS: -D warnings` |
  | `test (${{ matrix.os }})` | matrix `os: [ubuntu-latest, macos-latest, windows-latest]`, `fail-fast: false` | stable | `cargo test --all-targets`; `cargo test --doc` |
  | `rmcp ${{ matrix.rmcp }}` | ubuntu-latest, matrix `rmcp: ['3.0.0', '3.3.0', 'newest']` | stable | `newest`: `cargo update -p rmcp -p rmcp-macros`; otherwise `cargo update -p rmcp --precise ${{ matrix.rmcp }}`; then `cargo test --all-targets` |
  | `msrv` | ubuntu-latest | `1.88` | `cargo check --lib --all-features` |
  | `minimal-versions` | ubuntu-latest | nightly; install-action `cargo-minimal-versions@0.1.37,cargo-hack@0.6.45` | `cargo minimal-versions check --direct --lib --all-features` |
  | `features` | ubuntu-latest | stable; install-action `cargo-hack@0.6.45` | `cargo hack check --lib --feature-powerset --depth 2 --exclude-features default,exporters` |
  | `deny` | ubuntu-latest | cargo-deny-action, `command: check` | — |
  | `public-api` | ubuntu-latest | nightly (the name chosen in Task 2 Step 6); install-action `cargo-public-api@0.52.0` | `cargo public-api --simplified \| diff -u public-api.txt -` |
  | `coverage` | ubuntu-latest | stable + `llvm-tools-preview`; install-action `cargo-llvm-cov@0.9.1` | `cargo llvm-cov --all-targets --lcov --output-path lcov.info --fail-under-lines "$(cat scripts/coverage-floor)"`; upload `lcov.info` |
  | `all-green` | ubuntu-latest | — | `needs:` every job above; `if: always()`; one step: `run: exit 1` when `contains(needs.*.result, 'failure') \|\| contains(needs.*.result, 'cancelled')` |

- [ ] **Step 3: Write `.github/dependabot.yml`:** `version: 2`; two `updates` entries (`package-ecosystem: cargo` and `github-actions`, `directory: /`, `schedule: { interval: weekly }`, `open-pull-requests-limit: 0`). Version updates stay off; security updates open pull requests once the repository setting is on (Task 16).

- [ ] **Step 4: Lint the workflows.**

```bash
gh release download v1.7.12 -R rhysd/actionlint -p "actionlint_1.7.12_linux_$(dpkg --print-architecture).tar.gz" -O - | tar -xz actionlint
./actionlint .github/workflows/*.yml && rm actionlint
```
Expected: no output.

- [ ] **Step 5: Run** `scripts/check.sh --full` → every step passes.

- [ ] **Step 6: Commit and open pull request 1a.**

```bash
git add -A && git commit -m "ci: lint, OS and rmcp matrices, MSRV, minimal versions, features, deny, public API, coverage"
git push -u origin plan-1a-gates
gh pr create --base main --title "Build gates and CI" --body "Lint, dependency, MSRV, minimal-versions, feature, public-API and coverage gates, with CI on Ubuntu, macOS and Windows and the rmcp 3.0.0 / 3.3.0 / newest matrix. One required check: all-green."
gh pr checks --watch
```
Expected: every check green. Fix failures with new commits on this branch (Windows path or line-ending assumptions in tests are the likely ones; a test that is Unix-only by nature gets `#[cfg(unix)]` and a one-line reason). Do not merge: the owner merges at the end of the plan.

### Task 4: Parity harness

**Files:**
- Create: `scripts/parity/build-manifest.py`, `scripts/parity/README.md`, `scripts/parity/vitest/v1.json`, `scripts/parity/vitest/v2.json`, `scripts/parity/vitest/e2e-http.json`
- Create: `tests/parity/mod.rs`, `tests/parity/support.rs`, `tests/parity/completeness.rs`, `tests/parity/golden.rs`, `tests/parity/manifest.tsv`, `tests/parity/NOT_PORTED.md`
- Create: `tests/fixtures/php-audit/README.md`; copy `handles/{strings.json,golden.txt,derive-vectors.json,listing-fixtures.json}` and `exporters/golden-output.txt` from `/home/ubuntu/agentcat-php-sdk/docs/audit/fixtures/`
- Modify: `src/lib.rs`, `src/logging.rs`, `Cargo.toml` (`[dev-dependencies] regex = "1"`)

**Interfaces:**
- Produces:
  - `crate::logging::capture_logs<T>(f: impl FnOnce() -> T) -> (T, Vec<String>)` under `#[cfg(test)]`: runs `f` with this thread's SDK log lines captured (the message only, no timestamp or version prefix); while capturing, lines go nowhere else (no file, no `tracing`, no diagnostics).
  - `tests/parity/support.rs`: `pub(crate) fn fixture_text(rel: &str) -> String` and `pub(crate) fn fixture_json(rel: &str) -> serde_json::Value` (paths under `tests/fixtures/`), `pub(crate) fn sid(label: &str) -> String`, `pub(crate) fn obj(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value>`, `pub(crate) use crate::logging::capture_logs;`.
  - `tests/parity/completeness.rs`: `const ENFORCED: &[&str]` (TS files) and `const DEFERRED: &[(&str, &str, u8)]` (TS file, TS test name, owning plan). Later tasks only append to these two lists.

- [ ] **Step 0: Branch.** `git switch -c plan-1b-parity-core` (from `plan-1a-gates`).

- [ ] **Step 1: List the TS tests with vitest.** In `/home/ubuntu/agentcat-typescript-sdk` (at `9b56e9e`, dependencies installed):

```bash
R=/home/ubuntu/agentcat-rust-sdk/scripts/parity/vitest; mkdir -p "$R"
npx vitest list --json="$R/v1.json" --project v1 >/dev/null
npx vitest list --json="$R/v2.json" --project v2 >/dev/null
npx vitest list --json="$R/e2e-http.json" --project e2e-http >/dev/null
python3 -c "import json,sys; print([len(json.load(open(f'$R/{p}.json'))) for p in ('v1','v2','e2e-http')])"
```
Expected: `[806, 105, 109]`. (vitest leaves out the one `it.skip` test and the three Workers tests; the script takes those from the index.)

- [ ] **Step 2: Write `scripts/parity/build-manifest.py`:**

```python
#!/usr/bin/env python3
"""Builds tests/parity/manifest.tsv and tests/parity/NOT_PORTED.md.

Inputs:
  1. `vitest list --json` output for the v1, v2 and e2e-http projects of
     agentcat-typescript-sdk at 9b56e9e (the authoritative test names).
  2. docs/research/06a-06c-ts-test-index.md (the port / adapt / N/A call per row).

Usage: build-manifest.py <out_dir> <index_dir> <vitest_json>...
"""
import hashlib
import json
import re
import sys
from collections import OrderedDict

KEYWORDS = {
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod",
    "move", "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait",
    "true", "type", "unsafe", "use", "where", "while", "abstract", "become", "box", "do",
    "final", "macro", "override", "priv", "try", "typeof", "unsized", "virtual", "yield",
}

# Files that run outside the three vitest projects; their rows come from the index alone.
INDEX_ONLY = {"workers/event-delivery.test.ts"}


def ident(segment: str) -> str:
    s = re.sub(r"[^a-z0-9]+", "_", segment.lower()).strip("_") or "t"
    if s[0].isdigit():
        s = "t_" + s
    if len(s) > 80:
        s = s[:70].rstrip("_") + "_" + hashlib.sha256(segment.encode()).hexdigest()[:8]
    if s in KEYWORDS:
        s += "_"
    return s


def file_module(ts_file: str) -> str:
    return ident(ts_file.removesuffix(".test.ts").replace("/", "_"))


def parse_index(index_dir):
    """file -> list of (count, title, disposition, verdict text)."""
    rows = OrderedDict()
    current = None
    for part in ("06a", "06b", "06c"):
        with open(f"{index_dir}/{part}-ts-test-index.md", encoding="utf-8") as fh:
            for line in fh:
                m = re.match(r"^### src/tests/(\S+\.test\.ts)", line)
                if m:
                    current = m.group(1)
                    rows.setdefault(current, [])
                    continue
                if line.startswith("### "):
                    current = None
                    continue
                if current is None:
                    continue
                m = re.match(r"^\| (\d+)(?: ×(\d+))? \| (.*) \|$", line.rstrip("\n"))
                if not m:
                    continue
                cells = [c.strip().replace("\\|", "|") for c in re.split(r"(?<!\\) \| ", m.group(3))]
                verdict = cells[-1]
                low = verdict.lower()
                if low.startswith("n/a"):
                    disp = "na"
                elif low.startswith("adapt"):
                    disp = "adapt"
                elif low.startswith("port"):
                    disp = "port"
                else:
                    raise SystemExit(f"{current} row {m.group(1)}: unknown verdict {verdict!r}")
                rows[current].append((int(m.group(2) or 1), cells[0], disp, verdict))
    return rows


def expand_ellipses(titles):
    """Index titles abbreviate a repeated describe path as "... > it"."""
    out = []
    for t in titles:
        for mark in ("... > ", "… > "):
            if t.startswith(mark) and out:
                t = out[-1].rsplit(" > ", 1)[0] + " > " + t[len(mark):]
        out.append(t)
    return out


def last_segment(title: str) -> str:
    return title.split(" > ")[-1].strip().strip("'\"`")


def main():
    out_dir, index_dir, *vitest_files = sys.argv[1:]
    index = parse_index(index_dir)
    cases = OrderedDict()
    for path in vitest_files:
        with open(path, encoding="utf-8") as fh:
            for case in json.load(fh):
                f = case["file"].split("/src/tests/", 1)[1]
                cases.setdefault(f, []).append(case["name"])

    manifest = []
    problems = []
    for f, rows in index.items():
        expanded = [r for r in rows for _ in range(r[0])]
        names = cases.get(f)
        if names is None:
            if f not in INDEX_ONLY and rows:
                problems.append(f"{f}: no vitest cases")
                continue
            names = expand_ellipses([r[1] for r in expanded])
        if len(expanded) != len(names):
            if len(expanded) - len(names) == 1 and f == "error-capture.test.ts":
                # it.skip: vitest does not list it; take its describe path from its neighbour.
                idx = next(i for i, r in enumerate(expanded) if "in_app frames correctly" in r[1])
                prefix = names[idx - 1].rsplit(" > ", 1)[0]
                skipped_it = last_segment(expanded[idx][1]).removesuffix(" (skipped)")
                names = names[:idx] + [f"{prefix} > {skipped_it}"] + names[idx:]
            else:
                problems.append(f"{f}: index has {len(expanded)} cases, vitest has {len(names)}")
                continue
        # describe.each matrices list lanes outer, scenarios inner, while the index
        # lists scenarios with a ×N count: pair those by title. Every other file
        # lists cases in vitest order.
        paired = []
        if not f.startswith("e2e-http/"):
            paired = list(zip(names, expanded))
        else:
            by_last = {}
            for r in expanded:
                by_last.setdefault(last_segment(r[1]), []).append(r)
            for n in names:
                pool = by_last.get(last_segment(n))
                if not pool:
                    problems.append(f"{f}: no index row for {n!r}")
                    break
                paired.append((n, pool[0]))
        used = {}
        for n, r in paired:
            path = "::".join([file_module(f)] + [ident(s) for s in n.split(" > ")])
            k = used.get(path, 0) + 1
            used[path] = k
            if k > 1:
                path = f"{path}_{k}"
            manifest.append((f, n, r[2], path, r[3]))

    if problems:
        print("\n".join(problems), file=sys.stderr)
        raise SystemExit(1)

    with open(f"{out_dir}/manifest.tsv", "w", encoding="utf-8") as fh:
        fh.write("ts_file\tts_name\tdisposition\trust_test\n")
        for f, n, d, p, _ in manifest:
            fh.write(f"{f}\t{n}\t{d}\t{p}\n")
    with open(f"{out_dir}/NOT_PORTED.md", "w", encoding="utf-8") as fh:
        fh.write("# TS tests with no Rust port\n\n")
        fh.write("Generated by `scripts/parity/build-manifest.py`. Each row is an `N/A` row of\n")
        fh.write("`docs/research/06a`–`06c`. Edit the index, then regenerate.\n\n")
        fh.write("| TS file | TS test | Reason |\n|---|---|---|\n")
        previous = ""
        for f, n, d, _, note in manifest:
            if d == "na":
                reason = note.split(":", 1)[1].strip() if ":" in note else note
                if reason.rstrip(".").lower() == "same":
                    reason = previous
                previous = reason
                name = n.replace("|", "\\|")
                fh.write(f"| `{f}` | {name} | {reason} |\n")
    counts = {}
    for _, _, d, _, _ in manifest:
        counts[d] = counts.get(d, 0) + 1
    print(f"{len(manifest)} cases: {counts}")


if __name__ == "__main__":
    main()
```

Write `scripts/parity/README.md`: what the two inputs are, the Step 1 commands, the Step 3 command, and the rule that a changed index or a new TS commit means regenerating both files in the same commit.

- [ ] **Step 3: Write the failing tripwire.** `tests/parity/mod.rs`:

```rust
//! The parity suite: one test per TS test case, at the path that
//! `manifest.tsv` names for it.
#![allow(clippy::module_inception, reason = "test modules mirror TS describe blocks, which repeat the file name")]

mod completeness;
mod golden;
mod support;
```

`src/lib.rs` (after the module list):

```rust
#[cfg(all(
    test,
    feature = "rmcp",
    feature = "preserve-order",
    feature = "otlp",
    feature = "datadog",
    feature = "sentry",
    feature = "posthog"
))]
#[path = "../tests/parity/mod.rs"]
mod parity;
```

`tests/parity/completeness.rs`:

```rust
//! The tripwire: every TS test case of an enforced file has a Rust test at
//! the path the manifest names, and every `N/A` case is in `NOT_PORTED.md`.

use std::collections::BTreeSet;
use std::process::Command;

const MANIFEST: &str = include_str!("manifest.tsv");
const NOT_PORTED: &str = include_str!("NOT_PORTED.md");

/// TS test files whose cases must all have Rust tests.
const ENFORCED: &[&str] = &[];

/// Cases of enforced files that a later plan ports: (TS file, TS test, plan).
const DEFERRED: &[(&str, &str, u8)] = &[];

struct Case {
    ts_file: &'static str,
    ts_name: &'static str,
    disposition: &'static str,
    rust_test: &'static str,
}

fn cases() -> Vec<Case> {
    MANIFEST
        .lines()
        .skip(1)
        .map(|line| {
            let mut cols = line.split('\t');
            let mut next = || cols.next().expect("four tab-separated columns");
            Case { ts_file: next(), ts_name: next(), disposition: next(), rust_test: next() }
        })
        .collect()
}

fn listed_tests() -> BTreeSet<String> {
    let exe = std::env::current_exe().expect("path of the running test binary");
    let output = Command::new(exe)
        .args(["--list", "--format", "terse"])
        .output()
        .expect("list the tests of this binary");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|l| l.strip_suffix(": test"))
        .map(str::to_owned)
        .collect()
}

fn deferred(case: &Case) -> bool {
    DEFERRED.iter().any(|(f, n, _)| *f == case.ts_file && *n == case.ts_name)
}

#[test]
fn manifest_covers_every_ts_case() {
    let all = cases();
    assert_eq!(all.len(), 1024);
    let count = |d: &str| all.iter().filter(|c| c.disposition == d).count();
    assert_eq!((count("port"), count("adapt"), count("na")), (672, 201, 151));
    let paths: BTreeSet<_> = all.iter().map(|c| c.rust_test).collect();
    assert_eq!(paths.len(), all.len(), "duplicate rust_test paths");
}

#[test]
fn enforced_and_deferred_entries_name_real_cases() {
    let all = cases();
    for file in ENFORCED {
        assert!(all.iter().any(|c| c.ts_file == *file), "{file} is not in the manifest");
    }
    for (file, name, plan) in DEFERRED {
        assert!(ENFORCED.contains(file), "deferred case of a file that is not enforced: {file}");
        assert!(
            all.iter().any(|c| c.ts_file == *file && c.ts_name == *name && c.disposition != "na"),
            "no portable case {file} > {name}"
        );
        assert!((2..=7).contains(plan), "plan {plan} for {name}");
    }
}

#[test]
fn every_enforced_case_has_its_rust_test() {
    let listed = listed_tests();
    let missing: Vec<String> = cases()
        .iter()
        .filter(|c| ENFORCED.contains(&c.ts_file) && c.disposition != "na" && !deferred(c))
        .map(|c| format!("parity::{}", c.rust_test))
        .filter(|path| !listed.contains(path))
        .collect();
    assert!(missing.is_empty(), "{} TS cases have no Rust test:\n{}", missing.len(), missing.join("\n"));
}

#[test]
fn every_enforced_na_case_is_listed_in_not_ported() {
    let missing: Vec<&str> = cases()
        .iter()
        .filter(|c| ENFORCED.contains(&c.ts_file) && c.disposition == "na")
        .filter(|c| !NOT_PORTED.contains(&format!("| `{}` | {} |", c.ts_file, c.ts_name.replace('|', "\\|"))))
        .map(|c| c.ts_name)
        .collect();
    assert!(missing.is_empty(), "N/A cases missing from NOT_PORTED.md: {missing:?}");
}
```

Create `tests/parity/golden.rs` and `tests/parity/support.rs` as empty modules with a `//!` line each.

- [ ] **Step 4: Run** `AGENTCAT_DISABLE_DIAGNOSTICS=1 cargo test --lib parity` → FAIL to compile: `couldn't read tests/parity/manifest.tsv`.

- [ ] **Step 5: Generate the manifest.**

```bash
python3 scripts/parity/build-manifest.py tests/parity docs/research scripts/parity/vitest/v1.json scripts/parity/vitest/v2.json scripts/parity/vitest/e2e-http.json
```
Expected: `1024 cases: {'port': 672, 'adapt': 201, 'na': 151}`.

- [ ] **Step 6: Fixtures, support and log capture.**
  - Copy the five fixture files to `tests/fixtures/php-audit/` keeping the `handles/` and `exporters/` folders. `tests/fixtures/php-audit/README.md`: generated by the PHP audit from `agentcat-typescript-sdk@9b56e9e` (see `agentcat-php-sdk/docs/audit/fixtures/README.md`); shared oracle with PHP and C# (R10); never edit by hand.
  - `src/logging.rs`: a `thread_local!` `RefCell<Option<Vec<String>>>`; `write_to_log` pushes `message` into it and returns early when it is `Some` (all under `#[cfg(test)]`); `capture_logs` sets it to `Some(Vec::new())`, runs `f`, takes the lines, and restores the previous value (also on unwind, with a drop guard). Unit test `capture_logs_collects_only_this_threads_lines`: a line written in the closure is captured; a line written on a spawned thread inside the closure is not.
  - `tests/parity/support.rs`:

```rust
//! Shared helpers for the parity suite.
#![allow(dead_code, unused_imports, reason = "each helper serves some parity files, not all")]

use serde_json::{Map, Value};

pub(crate) use crate::logging::capture_logs;

pub(crate) fn fixture_text(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub(crate) fn fixture_json(rel: &str) -> Value {
    serde_json::from_str(&fixture_text(rel)).expect("fixture is JSON")
}

/// The TS test helper `sid(label)`: a well-formed `ses_` ID built from a label.
pub(crate) fn sid(label: &str) -> String {
    let body: String = label.chars().filter(char::is_ascii_alphanumeric).chain(std::iter::repeat('0')).take(27).collect();
    format!("ses_{body}")
}

pub(crate) fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(map) => map,
        other => panic!("expected a JSON object, got {other}"),
    }
}
```

- [ ] **Step 7: Run** `AGENTCAT_DISABLE_DIAGNOSTICS=1 cargo test --lib parity` → 4 completeness tests pass (nothing enforced yet). Run `scripts/check.sh` → pass.

- [ ] **Step 8: Commit.**

```bash
git add -A && git commit -m "test: TS parity harness with manifest, tripwire and shared PHP fixtures"
```

### Task 5: JavaScript semantics helpers

**Files:**
- Create: `src/js.rs`
- Modify: `src/lib.rs` (`mod js;`), `src/util.rs` (move `js_trim` out), `src/inject.rs` (use `js::truthy`), `src/tokens.rs` (move `js_number_string` out), their call sites

**Interfaces:**
- Produces (all `pub(crate)` in `crate::js`):
  - `fn trim(s: &str) -> &str` — `String.prototype.trim`: ECMAScript WhiteSpace and LineTerminator (`char::is_whitespace` minus U+0085, plus U+FEFF).
  - `fn truthy(v: Option<&Value>) -> bool` — `undefined`, `null`, `false`, `0`, `-0`, `""` are falsy; arrays and objects are truthy.
  - `fn number_to_string(f: f64) -> String` — `Number.prototype.toString` (moved from `tokens::js_number_string` with its tests).
  - Later tasks add `type_of` (Task 9) and `template_string` (Task 12) here, each with the first code that uses it.

- [ ] **Step 1: Write the failing unit tests** in `src/js.rs` `#[cfg(test)] mod tests`:
  - `trim`: `"\u{feff} x \u{a0}\n"` → `"x"`; `"\u{2028}x\u{3000}"` → `"x"`; `"\u{85}x"` unchanged; `"\u{180e}x"` unchanged.
  - `truthy`: `None`, `null`, `false`, `0`, `-0.0`, `""` → false; `"0"`, `[]`, `{}`, `1`, `true` → true.
  - `number_to_string`: the existing `numbers_format_like_javascript` cases plus `1e21` → `"1e+21"`, `1e-7` → `"1e-7"`, `0.1 + 0.2` → `"0.30000000000000004"`, `-0.0` → `"0"`, `5e-324` → `"5e-324"`.
- [ ] **Step 2: Run** `cargo test --lib js::` → FAIL (module missing).
- [ ] **Step 3: Implement** and switch every caller of the moved helpers.
- [ ] **Step 4: Run** `cargo test --lib js::` → PASS; `scripts/check.sh` → pass.
- [ ] **Step 5: Commit** — `git commit -m "refactor: one module for the JavaScript semantics the TS SDK relies on"`

### Task 6: Agent-facing text

**Files:**
- Create: `tests/parity/constants.rs`
- Modify: `tests/parity/mod.rs` (`mod constants;`), `tests/parity/golden.rs`, `tests/parity/completeness.rs` (`ENFORCED` += `"constants.test.ts"`)

**Interfaces:**
- Consumes: `crate::constants::*` (unchanged), `support::{fixture_json, fixture_text}`, `crate::util::{sha256, hex, utf16_len}`.

- [ ] **Step 1: Write the failing tests.**
  - Add `"constants.test.ts"` to `ENFORCED`.
  - `tests/parity/constants.rs`: the 11 cases of `docs/research/06a` § `constants.test.ts`, at their manifest paths, with the exact literals of each row. Row 11 compiles `SESSION_ID_PARAM_PATTERN` with `regex::Regex` and checks: matches `"start"` and `"ses_"` + 27×`a`; rejects `"ses_"` + 25×`a`, `"restart"`, `"START"`. Example of the pattern every parity file follows:

```rust
//! Parity: `src/tests/constants.test.ts`.

use crate::constants::*;

mod brand_constants_wire_literals {
    use super::*;

    #[test]
    fn emits_the_agentcat_custom_event_type_on_the_wire() {
        assert_eq!(AGENTCAT_CUSTOM_EVENT_TYPE, "agentcat:custom");
    }
}
```
  - `tests/parity/golden.rs`:
    - `agent_facing_strings_match_the_ts_fixture`: for each of the 13 keys of `php-audit/handles/strings.json` that names a constant (`DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`, `SESSION_ID_PARAM_DESCRIPTION`, `SESSION_ID_PARAM_PATTERN`, `AGENT_ID_PARAM_DESCRIPTION`, `MINT_BACK_HEADER_ISSUED`, `MINT_BACK_ISSUED_BODY`, `MINT_BACK_HEADER_UNRECOGNIZED`, `MINT_BACK_UNRECOGNIZED_BODY`, `MCP_SESSION_FIELD_DESCRIPTION`, `MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE`, `MCP_SESSION_SESSION_ID_DESCRIPTION`, `MCP_SESSION_AGENT_ID_DESCRIPTION`, `MCP_SESSION_STATUS_DESCRIPTION`) assert equality with the Rust constant of the same name.
    - `golden_txt_lengths_and_hashes_match`: for each line of `php-audit/handles/golden.txt` before `---JSON---` (format `NAME\tlen_utf16=N\tbytes=M\tsha256=H\temdash=E`) whose NAME is a constant above or `GET_MORE_TOOLS_DESCRIPTION`, `GET_MORE_TOOLS_CONTEXT_DESCRIPTION`, `GET_MORE_TOOLS_RESPONSE_TEXT` (→ `GET_MORE_TOOLS_RESPONSE`): assert `utf16_len == N`, UTF-8 length `== M`, `hex(&sha256(..)) == H`, and the count of `—` `== E`. (`MINTED_BLOCK_EXAMPLE` and `UNRECOGNIZED_BLOCK` are checked in Task 11.)
- [ ] **Step 2: Run** `AGENTCAT_DISABLE_DIAGNOSTICS=1 cargo test --lib parity` → FAIL: `every_enforced_case_has_its_rust_test` lists 11 missing tests (then, once written, any literal that differs).
- [ ] **Step 3: Fix** any constant that differs from the fixture (expected: none).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "test: constants parity and golden agent-facing strings"`

### Task 7: KSUID and session IDs

**Files:**
- Modify: `src/ksuid.rs` (and its callers: `src/exporters/posthog.rs`, `derive_session_id`)
- Create: `tests/parity/ksuid.rs`
- Modify: `tests/parity/mod.rs`, `tests/parity/golden.rs`, `tests/parity/completeness.rs` (`ENFORCED` += `"ksuid.test.ts"`)

**Interfaces:**
- Produces (`crate::ksuid`):
  - `pub(crate) enum KsuidError { EncodedLength, Decode { expected_at_least: usize }, Time, Payload, Buffer }` with `Display` = the TS messages: `Valid encoded KSUIDs are 27 characters`; `Fixed length of 20 is too small, expected at least {n}`; `Valid KSUID timestamps must be in milliseconds since 1970-01-01T00:00:00Z, no earlier than 2014-05-13T16:53:20Z and no later than 2150-06-19T23:21:35Z`; `Valid KSUID payloads are 16 bytes`; `Valid KSUID buffers are 20 bytes`.
  - `impl Ksuid`: `pub(crate) const MIN_STRING: &str = "000000000000000000000000000"`, `pub(crate) const MAX_STRING: &str = "aWgEPTl1tmebfsQzFP4bxwgy80V"`, `fn from_parts(time_ms: u64, payload: &[u8]) -> Result<Self, KsuidError>`, `fn from_bytes(bytes: &[u8]) -> Result<Self, KsuidError>`, `fn random() -> Self`, `fn random_at(time_ms: u64) -> Result<Self, KsuidError>`, `fn parse(encoded: &str) -> Result<Self, KsuidError>`, `fn timestamp(&self) -> u32`, `fn time_ms(&self) -> u64`, `fn payload(&self) -> [u8; 16]`, `fn as_bytes(&self) -> &[u8; 20]`, `fn encode(&self) -> String`.
  - `Ksuid: Ord` (byte order), `Display` (27 characters), `Debug` (`KSUID { <27 characters> }`), `serde::Serialize` (the 27-character string).
  - Unchanged: `new_prefixed(prefix) -> String`, `new_session_id()`, `new_event_id()`, `pub fn derive_session_id(id: &str, project_id: Option<&str>) -> String`, `is_valid_session_id(&str) -> bool`.
  - Only the parity tests use `from_bytes`, `random_at`, `MIN_STRING`, `MAX_STRING`, `as_bytes`, the `Serialize` impl and `KsuidError::Buffer`: they are `#[cfg(test)]`.

- [ ] **Step 1: Write the failing tests.**
  - `ENFORCED` += `"ksuid.test.ts"`.
  - `tests/parity/ksuid.rs`: the 24 portable cases of `docs/research/06b` § `ksuid.test.ts` (row 2 is N/A). Adapt rows: 1 → `as_bytes().len() == 20`, `encode().len() == 27`; 6 → `from_parts(t, &[42; 15])` and `&[42; 17]` → `Err(KsuidError::Payload)`; 12 → `from_bytes` of 19 and 21 bytes → `Err(Buffer)`, 20 bytes → `Ok`; 13 → the `Buffer` error's message; 14 → `serde_json::to_string(&k) == format!("\"{k}\"")`; 15 → `format!("{k:?}") == format!("KSUID {{ {k} }}")`; 22 → `new_prefixed("user")` matches `^user_[0-9A-Za-z]{27}$`.
  - Review Focus 4, `ts_decoder_semantics_for_non_base62_input` (values from the TS decoder at `9b56e9e`):

    | Input to `Ksuid::parse` | Result |
    |---|---|
    | 27 × `-` | `Ok`, `timestamp() == 0`, bytes `000000000000000000000000000000000000003f` |
    | 27 × space | `Ok`, `timestamp() == 0`, bytes `0000000000000000000000000000000000000050` |
    | 13 × `😀` then `a` (27 UTF-16 units) | `Ok`, bytes `000000000000000024533fc4f1c940234e857024` |
    | 27 × `é` | `Err`, message `Fixed length of 20 is too small, expected at least 21` |
    | `aWgEPTl1tmebfsQzFP4bxwgy80W` (one above max) | `Err`, same message |
    | 14 × `😀` (28 UTF-16 units) | `Err(EncodedLength)` |
    | `MAX_STRING` | `Ok`, bytes 20 × `ff`, `time_ms() == 5_694_967_295_000` |
  - `golden.rs` `derive_vectors_match`: every row of `php-audit/handles/derive-vectors.json` (`projectId: null` → `None`) gives its `sessionId`; and `derive_session_id("x", Some("")) == derive_session_id("x", None)`.
- [ ] **Step 2: Run** `AGENTCAT_DISABLE_DIAGNOSTICS=1 cargo test --lib parity` → FAIL (missing tests, then the new API and the decoder rows).
- [ ] **Step 3: Implement.** `from_parts` and `random_at`: `Err(Time)` outside `[14e11, 1e3 × (2^32 − 1) + 14e11]` ms; seconds = `(time_ms − 14e11) / 1000` (floor). `parse` is a line-for-line port of `src/thirdparty/ksuid/base62.js` and `base-convert-int-array.js`:

```rust
pub(crate) fn parse(encoded: &str) -> Result<Self, KsuidError> {
    if encoded.encode_utf16().count() != STRING_LENGTH {
        return Err(KsuidError::EncodedLength);
    }
    // `Array.from` walks code points; `charCodeAt(0)` reads the first UTF-16 unit.
    // There is no alphabet check: other characters give out-of-range digits.
    let digits: Vec<i64> = encoded
        .chars()
        .map(|c| {
            let mut units = [0u16; 2];
            let code = c.encode_utf16(&mut units).first().map_or(0, |&u| i64::from(u));
            if code < 58 { code - 48 } else if code < 91 { code - 55 } else { code - 61 }
        })
        .collect();
    let mut out = [0i64; BYTE_LENGTH];
    let mut offset = BYTE_LENGTH;
    let mut input = digits.clone();
    while !input.is_empty() {
        let Some(next) = offset.checked_sub(1) else {
            return Err(KsuidError::Decode { expected_at_least: max_length(digits.len()) });
        };
        offset = next;
        let mut quotients = Vec::with_capacity(input.len());
        let mut remainder: i64 = 0;
        for digit in &input {
            let acc = digit + remainder * 62;
            let q = acc.div_euclid(256); // Math.floor(acc / 256)
            remainder = acc % 256; // JS `%` keeps the dividend's sign
            if !quotients.is_empty() || q > 0 {
                quotients.push(q);
            }
        }
        if let Some(slot) = out.get_mut(offset) {
            *slot = remainder;
        }
        input = quotients;
    }
    // Buffer.from(array) stores each value modulo 256.
    let mut bytes = [0u8; BYTE_LENGTH];
    for (b, v) in bytes.iter_mut().zip(out) {
        *b = u8::try_from(v.rem_euclid(256)).unwrap_or(0);
    }
    Ok(Self(bytes))
}
```
  `max_length(n)` is `ceil(n × log2(62) / 8)` in `f64` (27 → 21). Update `to_uuid_v7` (`Ksuid::parse(bare).ok()`) and `derive_session_id` to the `Result` API.
- [ ] **Step 4: Run** → PASS; `scripts/check.sh` → pass.
- [ ] **Step 5: Commit** — `git commit -m "fix: KSUID decoding with the TS decoder's semantics (D15)"`

### Task 8: UUIDv7 and trace IDs

**Files:**
- Create: `tests/parity/posthog_uuidv7.rs`
- Modify: `tests/parity/mod.rs`, `tests/parity/golden.rs`, `tests/parity/completeness.rs` (`ENFORCED` += `"posthog-uuidv7.test.ts"`); `src/exporters/posthog.rs` only if a test fails

**Interfaces:**
- Consumes: `crate::exporters::posthog_uuid_v7(&str) -> String` (the crate-visible re-export of `posthog::to_uuid_v7`), `crate::exporters::trace_context::{trace_id, span_id, datadog_trace_id, datadog_span_id}`, `Ksuid` (Task 7).

- [ ] **Step 1: Write the failing tests.**
  - `ENFORCED` += `"posthog-uuidv7.test.ts"`; the 12 cases of `docs/research/06b` § `posthog-uuidv7.test.ts`, plus every row of that section's golden-vector table (`ses_2yXlSmKnrVgSWKGxWkkGlLaGu7v` → `01977375-f600-7355-b83a-0dea9024f86c`, …, `""` → `f4c8996f-b924-73b0-8442-98fc1c149afb`).
  - `golden.rs` `exporter_id_vectors_match` (header of `php-audit/exporters/golden-output.txt`): `trace_id("ses_2yXlShd4ODDcHLYliGW9VgH3m1R") == "b17987c6b00767670c45519276a327c3"`, `span_id("evt_2yXlTL6bElM5Whm0HjT0Iz8LL0M") == "38fbea6b36e16f37"`, Datadog `884202591345649603` and `4106133231481483063`; `to_uuid_v7` of the session → `01977375-f600-7179-87c6-b00767670c45`, of the event → `01977376-0988-78fb-aa6b-36e16f376637`, `my-customer-task-id` → `634902a7-d0c7-7220-91c6-0fa56fad8828`, `""` → `f4c8996f-b924-73b0-8442-98fc1c149afb`, `ses_session45600000000000000000` → `6c350afa-fa75-7081-b0c6-8ad3272daa6c`, `abc_def_ghi` → `ae0466b7-1fe3-7984-a344-541280a247bb`, 27 × `-` → `0145f680-b000-7d7a-8884-df06b0d17ac0`. Empty IDs: `trace_id("")` is 32 hex characters and two calls differ; `span_id("")` likewise with 16.
  - Review Focus 4, `non_ksuid_input_matches_ts` (TS at `9b56e9e`):

    | `to_uuid_v7` input | Output |
    |---|---|
    | `ses_` + 27 × `-` | `0145f680-b000-786d-98cc-d5d01e28dec2` |
    | `ses_` + 27 × `é` | `88417185-4ad5-75bc-9149-acad1fa28640` |
    | `ses_` + 13 × `😀` + `a` | `0145f680-b000-7343-b240-b7c50fc2c863` |
    | `ses_` + 27 × `z` | `824c5521-0b85-7994-816a-e1cc13ecbe70` |
    | `ses_aWgEPTl1tmebfsQzFP4bxwgy80V` | `052df680-ac18-70fb-a3ad-b18287c0f14b` |
    | `ses_aWgEPTl1tmebfsQzFP4bxwgy80W` | `69af7543-b7cc-7b64-a50a-27922b46ef70` |
    | `ses_` + 27 × space | `0145f680-b000-7647-ba2b-7a4d7eca1c91` |
    | `Ses_2yXlSmKnrVgSWKGxWkkGlLaGu7v` | `4d232166-b827-7896-8f84-4a5e15317ae1` |
- [ ] **Step 2: Run** → FAIL until every test exists; with Task 7 in place the vectors should already pass.
- [ ] **Step 3: Fix** `to_uuid_v7` only if a vector fails (prefix rule: strip `^[a-z]+_`, ASCII lower-case letters only).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "test: UUIDv7 and trace ID parity, including non-KSUID input"`

### Task 9: Tag validation

**Files:**
- Modify: `src/validation.rs`, `src/agentcat.rs` (the `event_tags` adapter), `src/tracked.rs` (call site), `tests/hooks.rs`
- Create: `tests/parity/validation.rs`
- Modify: `tests/parity/mod.rs`, `tests/parity/completeness.rs` (`ENFORCED` += `"validation.test.ts"`)

**Interfaces:**
- Consumes: `crate::util::utf16_len`.
- Produces: `pub(crate) fn validate_tags(tags: impl IntoIterator<Item = (String, serde_json::Value)>) -> Option<Vec<(String, String)>>`. The public `event_tags` hook keeps its signature; its adapter maps each value to `Value::String`. `pub(crate) fn js::type_of(v: &Value) -> &'static str`: `"object"` for `null`, arrays and objects; `"boolean"`, `"number"`, `"string"`.

- [ ] **Step 1: Write the failing tests.**
  - `js::tests::type_of_matches_js`: `null` → object, `[1]` → object, `{}` → object, `true` → boolean, `1.5` → number, `"a"` → string.
  - `ENFORCED` += `"validation.test.ts"`; the 11 cases of `docs/research/06c` § `validation.test.ts` with their exact log lines (via `capture_logs`). Row 7 (adapt) passes `json!(123)` and expects `Dropping invalid tag: "key1" — non-string value (got number)`.
  - Extra cases in the same file under `mod rust_extras`: `"key\n"` is dropped as invalid characters; value `"a\rb"` is kept; key `"é"` is dropped; a value of 100 × `😀` (200 UTF-16 units) is kept and 101 × `😀` is dropped (D09); input `[("a","1"),("b","2"),("a","3")]` gives `[("a","3"),("b","2")]` (`Object.fromEntries`: first position, last value); `null` logs `non-string value (got object)`.
  - `tests/hooks.rs` `tags_are_validated_and_sdk_tags_win`: now expects `tags["bad key"] == "x"` and `tags["agentcat:reserved"] == "x"` (TS accepts both), `multi` still dropped.
- [ ] **Step 2: Run** `AGENTCAT_DISABLE_DIAGNOSTICS=1 cargo test --lib parity && cargo test --test hooks` → FAIL.
- [ ] **Step 3: Implement** the TS order of checks: key matches `[A-Za-z0-9$_.:\- ]+` in full (else `Dropping invalid tag: "<key>" — key contains invalid characters or is empty`); key over 32 UTF-16 units (`— key exceeds max length of 32`); non-string value (`— non-string value (got <type_of>)`); value over 200 UTF-16 units (`— value exceeds max length of 200`); value containing `\n` (`— value contains newline character`). No valid entry → `None`. More than 50 valid entries (counted before duplicate keys merge) → log `Dropping <n> tag(s) — exceeds maximum of 50 entries per event` and keep the first 50. The dash is U+2014. Remove the reserved-prefix rule.
- [ ] **Step 4: Run** → PASS; `scripts/check.sh` → pass.
- [ ] **Step 5: Commit** — `git commit -m "fix: tag validation with the TS rules (D15) and UTF-16 lengths (D09)"`

### Task 10: Token estimates

**Files:**
- Modify: `src/tokens.rs`
- Create: `tests/parity/tokenestimate.rs`
- Modify: `tests/parity/mod.rs`, `tests/parity/completeness.rs` (`ENFORCED` += `"tokenEstimate.test.ts"`), `docs/superpowers/specs/2026-10-19-rust-sdk-design.md` (§10 row 10)

**Interfaces:**
- Produces (unchanged names): `TOKEN_ESTIMATE_BYTES_PER_TOKEN: f64 = 3.5`, `estimate_tokens(bytes: f64) -> u32`, `estimate_input_tokens(args: Option<&Value>) -> Option<u32>`, `estimate_output_tokens(response: Option<&Value>) -> Option<u32>`. JSON walks stop at depth 1,000 and return `None`.

- [ ] **Step 1: Write the failing tests.**
  - `ENFORCED` += `"tokenEstimate.test.ts"`; the 35 portable cases of `docs/research/06c` § `tokenEstimate.test.ts` (rows 13, 21, 22, 39 are N/A). Inputs are the row's JSON parsed with `serde_json::from_str`; `utf8ByteLength` rows assert `str::len`.
  - `mod rust_extras`: `{"e":"😀"}` → 4; `{"c":"\u0001"}` → 4 (14 bytes, escaped as JS does); `{"q":"\""}` → 3; and Review Focus 5, `deeply_nested_values_omit_the_estimate`: build `[[[…]]]` 5,000 levels deep without recursion (wrap in a loop), `estimate_input_tokens(Some(&v)) == None`, and `estimate_output_tokens` with that value as `structuredContent` and empty `content` → `None`; 999 levels → `Some(_)`. Run the body on a thread with a 64 MiB stack (`std::thread::Builder::new().stack_size(64 << 20)`): dropping a 5,000-deep `Value` recurses inside serde_json itself.
- [ ] **Step 2: Run** → FAIL (missing tests; the depth test overflows or returns `Some`).
- [ ] **Step 3: Implement** a depth counter in the compact-JSON byte walk: more than 1,000 nested arrays or objects → `None`. Add row 10 to spec §10: `| 10 | JSON walks (token estimates) stop at depth 1,000 and omit the value | Rust has no catchable stack overflow; TS stops at V8's stack limit |`.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `git commit -m "test: token estimate parity; JSON walks stop at depth 1,000"`

### Task 11: Handles

**Files:**
- Modify: `src/handles.rs`
- Create: `tests/parity/handles.rs`
- Modify: `tests/parity/mod.rs`, `tests/parity/golden.rs`, `tests/parity/completeness.rs`

**Interfaces:**
- Consumes: `crate::js::trim`, `crate::ksuid::{new_session_id, derive_session_id, is_valid_session_id}`.
- Produces (signatures unchanged): `SessionSource { Hook, Supplied, Minted, Invalid, Foreign }` with `as_str()`; `HandleResolution { session_id: String, session_source: SessionSource, hook_mode: bool, agent_id: Option<String> }`; `extract_handle(Option<&Map<String, Value>>, &str) -> Option<String>`; `resolve_handles(hook_mode: bool, enable_agent_tracking: bool, args: Option<&Map<String, Value>>, session_param_is_ours: bool) -> HandleResolution`; `build_mint_back_text(&HandleResolution) -> Option<String>`; `build_structured_mint_back(&HandleResolution) -> Option<Map<String, Value>>`; `mirror_structured_mint_back(&mut Option<Value>, Map<String, Value>) -> bool`; `build_handle_tags(&HandleResolution, Option<&str>) -> BTreeMap<String, String>`; `session_from_hook_value(Option<&str>, Option<&str>) -> (String, SessionSource)`.

- [ ] **Step 1: Write the failing tests.**
  - `ENFORCED` += `"handles.test.ts"`; `DEFERRED` += these 8 cases, plan 4 (they need the engine's `CallToolResult` decoration and the hook runner): `appendMintBack > prepends a text block as the first content element`, `appendMintBack > prepends on isError results too`, `appendMintBack > leaves non-array content untouched`, `appendMintBack > does not mutate the original result`, `invokeSessionHook > resolves the hook value and forwards request/extra verbatim`, `invokeSessionHook > resolves null when the hook returns nullish`, `invokeSessionHook > contains a synchronous throw`, `invokeSessionHook > contains a rejection`.
  - `tests/parity/handles.rs`: the other 70 cases of `docs/research/06b` § `handles.test.ts`, with the row's constants (`T`, `A`, `MINTED_BLOCK(id)`). TS `resolveHandles(options, projectId, req(args), extra, ours)` maps to `resolve_handles(options.resolveSessionId is set, options.enableAgentTracking === true, Some(&args), ours)`; row 26 also asserts `!crate::agentcat::Options::default().enable_agent_tracking`. Rows 57–60 call `mirror_structured_mint_back` on `Some(json!(…))` and check the key order of the result. Row 42 gets a strict twin `hook_mode_ignores_a_valid_supplied_session_id` with `sid("agent_sent")`.
  - `mod rust_extras`: Review Focus 3, `non_string_handles_resolve_like_omission`: `session_id` = `123`, `{}`, `null`, `true`, `["ses_…"]` → `Minted`; the same values for `agent_id` with tracking on → `agent_id == None`. D09: `agent_id` = 150 × `😀` → tag of 100 × `😀`; `a` + 100 × `😀` → tag `a` + 99 × `😀` (no split pair). D15: `build_handle_tags(res, Some("2026-07-28\nX"))` keeps the value verbatim.
  - `golden.rs` `mint_back_blocks_match`: `build_mint_back_text` for `Minted` with `ses_2cOHEO0LYGADMzRvWTXXVbbgxgm` equals `strings.json` `MINTED_BLOCK_FOR_ses_2cOHEO0LYGADMzRvWTXXVbbgxgm` and the `MINTED_BLOCK_EXAMPLE` line of `golden.txt`; for `Invalid` it equals `UNRECOGNIZED_BLOCK`; `session_from_hook_value(Some(" corr-1 "), Some("proj_1")) == ("ses_2afjLT0irA6g3tOA5CklOPgcSqQ", Hook)`.
- [ ] **Step 2: Run** → FAIL (missing tests; the protocol tag is clamped).
- [ ] **Step 3: Implement:** `build_handle_tags` inserts `agentcat_protocol_version` verbatim when non-empty (D15; delete the clamp for it). The agent tag keeps its `\r`/`\n` → space and its 200-unit cut.
- [ ] **Step 4: Run** → PASS; `scripts/check.sh` → pass.
- [ ] **Step 5: Commit** — `git commit -m "fix: protocol version tag as TS sends it (D15); handles parity"`

### Task 12: Tool-list injection

**Files:**
- Modify: `src/inject.rs`, `src/tracked.rs` (listing path)
- Create: `tests/parity/handle_conflict_report.rs`, `tests/parity/context_parameters_edge_cases.rs`, `tests/parity/handle_injection.rs` (listing cases only)
- Modify: `tests/parity/mod.rs`, `tests/parity/golden.rs`, `tests/parity/completeness.rs` (`ENFORCED` += `"context-parameters-edge-cases.test.ts"`, `"handle-conflict-report.test.ts"`)

**Interfaces:**
- Consumes: `crate::js::{truthy, type_of}`, constants.
- Produces (`crate::js`): `pub(crate) fn template_string(v: &Value) -> String`, what `${v}` prints: `null` → `"null"`, booleans, numbers through `number_to_string`, strings as they are, arrays as their elements joined by `","` with `null` elements as `""`, objects as `"[object Object]"`. Unit test `template_string_matches_js`: `null` → `"null"`; `[1,[2,null],"a"]` → `"1,2,,a"`; `{"a":1}` → `"[object Object]"`; `[{}]` → `"[object Object]"`; `2.5` → `"2.5"`; `"x"` → `"x"`.
- Produces (`crate::inject`):
  - `InjectConfig` (unchanged fields and `new`).
  - `InjectMemory` (per `AgentCat`, thread-safe): `declares_session_id(&self, tool: &str) -> bool`, `record_customer_report_missing(&self)`, and, `#[cfg(test)]`, `reported_conflicts(&self) -> Vec<String>` and `declared_session_params(&self) -> Vec<String>` (both sorted).
  - `Injected { session_id: bool, agent_id: bool, context: bool }` (unchanged) with `#[cfg(test)] names(self) -> Vec<&'static str>` in injection order.
  - `#[derive(Debug, Default)] pub(crate) struct InjectedList { pub(crate) tools: Vec<Value>, pub(crate) registry: HashMap<String, Injected>, pub(crate) output_registry: HashSet<String> }`.
  - `pub(crate) fn build_injected_list(cfg: &InjectConfig, memory: &InjectMemory, tools: &[Value]) -> InjectedList` — never mutates `tools`.
  - `report_missing_descriptor() -> Value` (unchanged); `report_missing_response(context: Option<&Value>) -> Value` logs `Missing tool reported (context length: <len>)` where `<len>` is what TS prints for `args?.context?.length ?? 0`: a string's UTF-16 length, an array's element count, `template_string` of an object's `length` member when that member is present and not `null`, else `0`.
  - Removed: `inject_tool`, `prepare_properties`, `declared_anywhere`, the composed-schema unit tests (they test the reverted fix).

- [ ] **Step 1: Write the failing tests.**
  - `ENFORCED` += `"context-parameters-edge-cases.test.ts"`, `"handle-conflict-report.test.ts"`.
  - `tests/parity/context_parameters_edge_cases.rs`: the 12 cases of `docs/research/06a` § `context-parameters-edge-cases.test.ts`, each through `build_injected_list` with handles off (`InjectConfig::new(false, false, false, true, None, false)`) and the row's tool JSON; row 12 counts exactly 2 captured log lines; row 9 uses a tool with no `inputSchema` key (the JSON-level port handles it as TS does).
  - `tests/parity/handle_conflict_report.rs`: the 8 portable cases of `docs/research/06b` § `handle-conflict-report.test.ts` (row 3 is N/A). "Per server" and "per `AgentCatData`" map to one `InjectMemory` per `AgentCat` (spec §9.2): a second server is a second `InjectMemory`.
  - `tests/parity/handle_injection.rs`: rows 1–12 and 20–27 of `docs/research/06b` § `handle-injection.test.ts` (rows 13–19 and 28 come in Task 13). Key order is asserted with `serde_json::to_string` of the schema or with a collected `Vec` of keys.
  - `golden.rs`:
    - `listing_fixtures_match`: for each of the 7 configurations of `php-audit/handles/listing-fixtures.json`, build from `input_tools` with a fresh `InjectMemory` and these settings, and assert the tools serialize to the same string as the fixture's (key order included), the registry equals the fixture's (per tool, as a set of names), and the output registry equals it:

      | Fixture key | `InjectConfig::new(tracing, hook_mode, agent_tracking, tool_call_context, custom_description, report_missing)` |
      |---|---|
      | `defaults` | `(true, false, false, true, None, true)` |
      | `agent_tracking` | `(true, false, true, true, None, true)` |
      | `hook_mode_agent_tracking` | `(true, true, true, true, None, true)` |
      | `hook_mode_no_agent` | `(true, true, false, true, None, true)` |
      | `tracing_off` | `(false, false, false, true, None, true)` |
      | `context_off_report_missing_off` | `(true, false, false, false, None, false)` |
      | `custom_context_description` | `(true, false, false, true, Some("Why?"), true)` |
    - `get_more_tools_descriptor_and_reply_match`: `report_missing_descriptor()` serializes like `strings.json` `GET_MORE_TOOLS_DESCRIPTOR`; `report_missing_response(Some(&json!("abc")))` equals `GET_MORE_TOOLS_RESULT`; the logged length is `3` for `"abc"`, `2` for `"😀"`, `3` for `[1,2,3]`, `5` for `{"length":5}`, `abc` for `{"length":"abc"}`, `0` for `{"length":null}`, `42`, `null` and an absent context.
  - `mod rust_extras` in `handle_injection.rs`:
    - Review Focus 1, `malformed_schema_parts_take_the_ts_failure_path`: with `InjectConfig::new(true, false, true, true, None, false)`, tools `a` (`inputSchema: {"type":"object","properties":"abc"}`), `b` (`properties: 5`), `c` (`properties: true`), `d` (`properties: []`), `e` (plain `inputSchema` `{"type":"object","properties":{"q":{"type":"string"}}}` and `outputSchema: {"type":"object","properties":"x"}`), `ok` (plain). Expect:
      - `a`, `b`, `c` listed exactly as given, with empty registry entries. Logs: `WARN: Handle injection failed for tool "a"; listing it unmodified - TypeError: Cannot create property 'session_id' on string 'abc'` (then `… on number '5'` for `b`, `… on boolean 'true'` for `c`), and from the context step `WARN: Context injection failed for tool "a"; listing it unmodified - TypeError: Cannot create property 'context' on string 'abc'` (and likewise for `b`, `c`).
      - `e`: the handle step fails with `… - TypeError: Cannot create property 'mcp_session' on string 'x'` and is rolled back, then the context step succeeds on the original: its input schema gains only `context`, its output schema is unchanged, its registry entry is `{context}`, and it is not in the output registry.
      - `d` keeps `properties: []`, gains `required: ["session_id","agent_id","context"]`, and its registry records all three (JS writes named keys onto the array; JSON drops them).
      - `ok` is fully injected.
    - `removing_additional_properties_keeps_key_order`: `{"type":"object","additionalProperties":false,"properties":{"q":{}},"required":["q"]}` → keys `type, properties, required` in that order.
    - Review Focus 2, `concurrent_listings_report_each_conflict_once`: one `Arc<InjectMemory>`, 8 threads × 100 calls of `build_injected_list` over `[own_session tool, plain tool]`, each thread inside `capture_logs`; the sum of captured `ERROR:` lines is 1 and `declared_session_params() == ["own_session"]`.
- [ ] **Step 2: Run** → FAIL (missing tests; then `{}` gains `type`, composed tools get stripping entries, `additionalProperties` handling, TS log texts, failure paths).
- [ ] **Step 3: Implement `build_injected_list`** as a port of `src/engine/listWrap.ts` `buildInjectedList`, `src/modules/handle-injection.ts` and `src/modules/context-parameters.ts`, operating on `serde_json::Value`:
  1. `result = tools.to_vec()`. When `report_missing`: if a tool's `name` is the string `get_more_tools`, call `memory.record_customer_report_missing()`; otherwise push `report_missing_descriptor()`.
  2. When `inject_session_id || inject_agent_id`, map every tool through the handle step; on `Err(e)`: remove the tool's registry entry and output-registry entry, log `WARN: Handle injection failed for tool "<name>"; listing it unmodified - <e>`, and keep the original tool.
  3. When `inject_context`, map every tool except one named `get_more_tools` through the context step; on `Err(e)`: clear `context` in its registry entry, log `WARN: Context injection failed for tool "<name>"; listing it unmodified - <e>`, keep the tool.
  4. Every tool with a truthy `name` gets a registry entry (empty if none), keyed by `template_string(name)`.

  Tool name for logs and keys: `template_string(name)` when `truthy(name)`, else `"unknown"`. JS property writes go through one helper, so failure paths match TS:

```rust
/// `target[key] = value` in strict-mode JS, on JSON.
fn js_set(target: &mut Value, key: &str, value: Value) -> Result<(), String> {
    match target {
        // An existing key keeps its position, as in JS.
        Value::Object(map) => {
            map.insert(key.to_owned(), value);
            Ok(())
        }
        // JS stores a named property on the array object; JSON drops it.
        Value::Array(_) => Ok(()),
        Value::Null => Err(format!("TypeError: Cannot set properties of null (setting '{key}')")),
        other => Err(format!(
            "TypeError: Cannot create property '{key}' on {} '{}'",
            js::type_of(other),
            js::template_string(other)
        )),
    }
}
```
  Reads use `target.as_object()?.get(key)` (a named key on an array or primitive reads `undefined`).

  Handle step (TS `addHandleParametersToTool`): if `truthy(inputSchema.oneOf || allOf || anyOf)`: when `inject_session_id` and `truthy(inputSchema.properties.session_id)`, record the tool as declaring `session_id`; log `WARN: Tool "<name>" has complex schema (oneOf/allOf/anyOf). Skipping handle injection.`; return the tool unchanged (no registry entry here, so step 4 gives it an empty one and nothing is stripped). Else: a falsy `inputSchema` becomes `{"type":"object","properties":{},"required":[]}`; a falsy `properties` becomes `{}`; `additionalProperties == false` is removed with `shift_remove`. `session_id`: declared (truthy) → record declared, and on the first conflict for this tool in `memory` log `ERROR: Tool "<name>" already declares a 'session_id' parameter. AgentCat will not inject its own, and calls to this tool are published without a session, so they cannot be correlated. Your parameter is untouched and still reaches your handler. If you already manage sessions, pass a resolveSessionId hook to track() — AgentCat will derive its session from your identifier and stop injecting session_id entirely.`; otherwise `js_set` the property `{"type":"string","description":SESSION_ID_PARAM_DESCRIPTION,"pattern":SESSION_ID_PARAM_PATTERN}`, record it, add to `required`. `agent_id`: declared → `WARN: Tool "<name>" already has 'agent_id' parameter. Skipping agent_id injection.`; otherwise `{"type":"string","description":AGENT_ID_PARAM_DESCRIPTION}`, record, `required`. `required`: an array gets the name appended when absent; anything else is replaced by `[name]` through `js_set`. Then the output step: skip a falsy `outputSchema`; composed → `WARN: Tool "<name>" has complex outputSchema (oneOf/allOf/anyOf). Skipping mcp_session injection; mint-back stays content-only for this tool.`; copy it, give a falsy `properties` `{}`, declared `mcp_session` → `WARN: Tool "<name>" already declares 'mcp_session' in outputSchema. Skipping injection.`; otherwise `js_set` `mcp_session` = `{"type":"object","description":…,"properties":{session_id?, agent_id?, status?}}` exactly as today's `inject_output_schema`, replace `outputSchema` with the copy, and add the tool to the output registry.

  Context step (TS `addContextParameterToTool`), in this order: `truthy(inputSchema.properties.context)` → `WARN: Tool "<name>" already has 'context' parameter. Skipping context injection.`; composed → `WARN: Tool "<name>" has complex schema (oneOf/allOf/anyOf). Skipping context injection.`; else the same `inputSchema` / `properties` / `additionalProperties` preparation, `js_set` `context` = `{"type":"string","description":cfg.context_description}`, record `context`, then `required`.

  The customer-owned `get_more_tools` warning is the TS text: `WARN: This server registers its own "get_more_tools" tool, which is shadowed by AgentCat's report-missing tool: calls to it are answered by AgentCat and never reach your handler. Rename your tool, or set enableReportMissing: false to keep yours reachable.`

- [ ] **Step 4: Switch the engine's listing path** (`src/tracked.rs`):
  - `inject_list(&self, tools: &[Tool]) -> Result<(Vec<Tool>, InjectedList), String>`: serialize each tool with `serde_json::to_value`, call `build_injected_list(&self.shared.inject, &self.shared.memory, &values)`, convert each advertised value back with `tool_from_json`; any conversion failure is an `Err` with its message.
  - `list_tools`: on `Ok`, merge one `ToolInjection { strip: entry, declares_session_id: memory.declares_session_id(name), output_injected: output_registry.contains(name) }` per registry entry (keep merging; plan 4 ports the TS replace-per-listing), set `listed` on the last page, serve the advertised tools. On `Err(e)`: log `Warning: AgentCat tool-list injection failed; serving the original tool list unmodified - <e>` and serve the original page. `get_more_tools` is now appended on every page (TS), so drop the last-page logic and the `customer_report_missing` / `note_customer_tool` bookkeeping.
  - `inject_one(&self, tool: &Tool)`: `build_injected_list` with a copy of the config whose `report_missing` is `false`, over `[tool]`; return the advertised tool and its `ToolInjection`. `ToolInjection` moves to `tracked.rs`.
  - Update or delete engine tests that assert the reverted behaviors (`get_more_tools` only on the last page; composed tools stripping echoed handles).
- [ ] **Step 5: Run** `AGENTCAT_DISABLE_DIAGNOSTICS=1 cargo test --all-targets` → PASS; `scripts/check.sh` → pass.
- [ ] **Step 6: Commit** — `git commit -m "fix: tool-list injection as TS builds it, failure paths and log texts included (D15)"`

### Task 13: Argument stripping

**Files:**
- Modify: `src/inject.rs`, `src/tracked.rs` (strip call sites)
- Modify: `tests/parity/handle_injection.rs` (rows 13–19, 28), `tests/parity/completeness.rs`, `tests/composed.rs`

**Interfaces:**
- Produces: `heuristic_strip(name: &str) -> Injected` (unchanged: all three, but not `context` for `get_more_tools`); `strip_arguments(args: &mut Map<String, Value>, strip: Injected)` removes with `shift_remove`, so the remaining keys keep their order.

- [ ] **Step 1: Write the failing tests.**
  - `ENFORCED` += `"handle-injection.test.ts"`; `DEFERRED` += `registry storage + request cloning > stores per server object` and `outputSchema injection > output registry accessors are keyed per server object` (plan 4: per-instance registry semantics).
  - Rows 13–17 and 19 of `docs/research/06b` § `handle-injection.test.ts`. A TS registry entry `Set` maps to `Injected`; "no registry entry" maps to `heuristic_strip`; row 17 passes `None` through; row 19 strips a clone and asserts the original still holds `session_id` and `context`.
  - `mod rust_extras` `stripping_keeps_argument_order`: `{"a":1,"session_id":"x","b":2,"context":"y","c":3}` with all three stripped → keys `a, b, c`.
  - `tests/composed.rs`: keep `a_branch_declared_session_id_reaches_the_handler`; add `echoed_handles_reach_a_composed_tool` (TS: a composed tool has an empty registry entry, so echoed handles reach the handler). Use a hand-written `ServerHandler` in that file: `list_tools` returns one tool `echo_args` with input schema `{"type":"object","oneOf":[{"properties":{"a":{"type":"string"}}},{"properties":{"b":{"type":"string"}}}]}`, and `call_tool` returns the received arguments, serialized, as one text block. Track it with agent tracking on, list, then call with `{"a":"x","session_id":sid("composed"),"agent_id":"opus|cc|k3n9x"}`: the echoed text contains both `session_id` and `agent_id`.
- [ ] **Step 2: Run** → FAIL (missing tests; key order).
- [ ] **Step 3: Implement** the `shift_remove` change; the engine already strips through `strip_arguments`.
- [ ] **Step 4: Run** `AGENTCAT_DISABLE_DIAGNOSTICS=1 cargo test --all-targets` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "fix: registry-scoped stripping that keeps argument order"`

### Task 14: Migrate the old vector tests and remove dead helpers

**Files:**
- Create: `tests/parity/vectors.rs`
- Modify: `tests/ts_parity.rs`, `src/lib.rs` (`__private`), `tests/hooks.rs`, `tests/parity/mod.rs`

**Interfaces:**
- Consumes: everything above.
- Produces: `__private` keeps only `sanitize`, `truncate`, `normalize`, `capture_tool_result_error` and `publish_request_json` (plans 2 and 3 move those).

- [ ] **Step 1: Move** these tests from `tests/ts_parity.rs` into `tests/parity/vectors.rs`, calling crate items directly: `derived_session_ids_match`, `ksuid_encoding_matches`, `posthog_uuid_v7_matches`, `trace_context_matches`, `session_id_validation_matches`, `handle_extraction_matches`, `resolution_mint_back_and_tags_match`, `structured_mirror_matches`, `argument_stripping_matches`, `schema_injection_matches`, `report_missing_descriptor_matches`, `token_estimates_match`, `constants_match_ts_source`. Delete every "Fixed TS gap" adjustment: the raw TS vectors of `tests/fixtures/ts-vectors.json` must match as they are (`schema_injection_matches` runs `build_injected_list`).
- [ ] **Step 2: Run** `AGENTCAT_DISABLE_DIAGNOSTICS=1 cargo test --lib parity::vectors` → PASS (a failure here is a parity bug in Tasks 7–13: fix the code, not the vector).
- [ ] **Step 3: Remove** the `__private` wrappers no longer used; `tests/hooks.rs` checks the session ID shape with `regex::Regex::new("^ses_[0-9A-Za-z]{27}$")` instead of `__private::is_valid_session_id`.
- [ ] **Step 4: Run** `scripts/check.sh` → pass (re-bless `public-api.txt` only if `__private` appeared in it).
- [ ] **Step 5: Commit** — `git commit -m "test: move TS vector checks into the parity suite"`

### Task 15: Mutation and coverage gates, nightly workflow

**Files:**
- Create: `.cargo/mutants.toml`, `scripts/mutants-score.py`, `scripts/mutants-floor.env`, `.github/workflows/nightly.yml`
- Modify: `.github/workflows/ci.yml`, `scripts/check.sh`, `scripts/coverage-floor`

**Interfaces:**
- Produces: `scripts/mutants-score.py --min-core N --min-adapter N DIR...` (exit 1 below a floor); CI jobs `mutants-diff` and `mutants-diff-score`.

- [ ] **Step 1: Write `scripts/mutants-score.py`:**

```python
#!/usr/bin/env python3
"""Mutation score per group from cargo-mutants output directories.

Usage: mutants-score.py --min-core N --min-adapter N DIR...
Each DIR holds caught.txt, missed.txt, timeout.txt (cargo-mutants' mutants.out).
Score = (caught + timeout) / (caught + timeout + missed); unviable mutants are left out.
Groups: adapter = src/tracked.rs and src/tool_call.rs; core = every other src/ file.
"""
import argparse
import pathlib
import sys

ADAPTER = {"src/tracked.rs", "src/tool_call.rs"}


def lines(directory, name):
    path = pathlib.Path(directory, name)
    return path.read_text(encoding="utf-8").splitlines() if path.exists() else []


def group(mutant):
    return "adapter" if mutant.split(":", 1)[0] in ADAPTER else "core"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--min-core", type=float, required=True)
    parser.add_argument("--min-adapter", type=float, required=True)
    parser.add_argument("dirs", nargs="+")
    args = parser.parse_args()
    stats = {"core": [0, 0], "adapter": [0, 0]}
    missed = []
    for d in args.dirs:
        for m in lines(d, "caught.txt") + lines(d, "timeout.txt"):
            stats[group(m)][0] += 1
        for m in lines(d, "missed.txt"):
            stats[group(m)][1] += 1
            missed.append(m)
    ok = True
    for name, minimum in (("core", args.min_core), ("adapter", args.min_adapter)):
        caught, miss = stats[name]
        total = caught + miss
        score = 100.0 if total == 0 else 100.0 * caught / total
        print(f"{name}: {score:.1f}% ({caught} caught, {miss} missed), minimum {minimum}%")
        ok = ok and score >= minimum
    if missed:
        print("\nMissed mutants:\n" + "\n".join(sorted(missed)))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Write `.cargo/mutants.toml`** with `timeout_multiplier = 3.0` and `exclude_globs = ["src/lib.rs"]` (crate attributes and test helpers). It sets no environment: the CI jobs and `check.sh` export `AGENTCAT_DISABLE_DIAGNOSTICS=1`. Write `scripts/mutants-floor.env`:

```bash
# Nightly whole-crate mutation floors. Targets (spec §11.2): CORE=90, ADAPTER=80.
# Raise these to the measured scores as plans 2-7 add tests; never lower them.
CORE=0
ADAPTER=0
```

- [ ] **Step 3: Add the pull-request mutation jobs to `ci.yml`.** Install `cargo-mutants@27.1.0` with install-action; pin `actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2`.

```yaml
  mutants-diff:
    name: mutants (shard ${{ matrix.shard }})
    if: github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    timeout-minutes: 180
    strategy:
      fail-fast: false
      matrix:
        shard: [0, 1, 2, 3]
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          fetch-depth: 0
      - uses: dtolnay/rust-toolchain@683b588c487a69d01f6c6ddf40f036ff682cc2c8 # master 2026-10-16
        with:
          toolchain: stable
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
      - uses: taiki-e/install-action@8cca0918f8471c33515202b73f56ab27f2bed283 # v2.87.29
        with:
          tool: cargo-mutants@27.1.0
      - name: Mutate the changed lines
        run: |
          git diff "origin/${{ github.base_ref }}...HEAD" -- src > pr.diff
          mkdir -p out
          if [ -s pr.diff ]; then
            set +e
            cargo mutants --in-diff pr.diff --shard ${{ matrix.shard }}/4 --output out
            code=$?
            set -e
            # 0 all caught, 2 some missed, 3 some timed out: the score job decides.
            case $code in 0|2|3) ;; *) exit $code ;; esac
          fi
      - uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
        with:
          name: mutants-diff-${{ matrix.shard }}
          path: out/
          if-no-files-found: ignore

  mutants-diff-score:
    name: mutants score (changed code)
    if: github.event_name == 'pull_request'
    needs: mutants-diff
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - uses: actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2
        with:
          pattern: mutants-diff-*
          path: shards
      - run: python3 scripts/mutants-score.py --min-core 95 --min-adapter 95 shards/*/mutants.out
```
  Add both jobs to `all-green.needs` (a `skipped` result on push is allowed by the `all-green` rule).

- [ ] **Step 4: Write `.github/workflows/nightly.yml`.** `on: { schedule: [{ cron: "0 5 * * *" }], workflow_dispatch: {} }`, same `permissions`, `env` and pinned actions. Jobs:
  - `newest-rmcp`: `cargo update -p rmcp -p rmcp-macros && cargo test --all-targets` (includes `tests/trait_drift.rs`).
  - `rmcp-main`: append to `Cargo.toml`

```toml
[patch.crates-io]
rmcp = { git = "https://github.com/modelcontextprotocol/rust-sdk", branch = "main" }
rmcp-macros = { git = "https://github.com/modelcontextprotocol/rust-sdk", branch = "main" }
```
    then `cargo update -p rmcp -p rmcp-macros`; if `cargo tree -i rmcp --depth 0` shows the git source, run `cargo test --all-targets`; otherwise print `::notice::rmcp main is outside the 3.x range` and succeed (today main is 3.5.1).
  - `mutants-full`: matrix `shard: [0..7]` (8 shards), `timeout-minutes: 300`, `cargo mutants --shard ${{ matrix.shard }}/8 --output out` with the same exit-code rule, upload `mutants-full-<shard>`.
  - `mutants-full-score`: download `mutants-full-*`, `source scripts/mutants-floor.env`, `python3 scripts/mutants-score.py --min-core "$CORE" --min-adapter "$ADAPTER" shards/*/mutants.out`.

- [ ] **Step 5: Extend `scripts/check.sh`** with option `--mutants`: `git diff origin/main...HEAD -- src > target/pr.diff` and, when not empty, `cargo mutants --in-diff target/pr.diff --output target/mutants` (accepting exit codes 0, 2, 3) then `python3 scripts/mutants-score.py --min-core 95 --min-adapter 95 target/mutants/mutants.out`.

- [ ] **Step 6: Ratchet coverage.** Run `cargo llvm-cov --all-targets --summary-only`; write the line percentage, rounded down, to `scripts/coverage-floor` (it must not be lower than 81).

- [ ] **Step 7: Lint and run.** actionlint as in Task 3 Step 4 → clean. `scripts/check.sh --full --mutants` → pass. If the in-diff score is below 95 %, add tests that kill the listed missed mutants (in the matching parity file's `rust_extras`, or the module's unit tests), then rerun.

- [ ] **Step 8: Commit** — `git commit -m "ci: mutation testing on changed code, nightly upstream and full mutation runs, coverage ratchet"`

### Task 16: Documentation, repository settings, pull request 1b

**Files:**
- Delete: `docs/PORTING.md` (and fix links to it in `README.md` and `docs/`)
- Modify: `CHANGELOG.md`, `docs/superpowers/plans/README.md`, `docs/DECISIONS.md` (only if a task found a new difference; none expected beyond spec §10 row 10)

- [ ] **Step 1: Docs.** Delete `docs/PORTING.md`; `grep -rn "PORTING" README.md docs src tests` → no hits after fixing links. `CHANGELOG.md` `Unreleased`: "Behavior now follows the TS SDK in tag validation, tool-schema injection (composed schemas, empty schemas, failure paths, log texts), the protocol-version tag, KSUID decoding and argument key order." In `docs/superpowers/plans/README.md`, mark plan 1 done with both pull request links.
- [ ] **Step 2: Repository settings** (report each result to the owner; do not retry a refusal):

```bash
gh api -X PUT repos/agentcathq/agentcat-rust-sdk/vulnerability-alerts
gh api -X PUT repos/agentcathq/agentcat-rust-sdk/automated-security-fixes
gh api -X PUT repos/agentcathq/agentcat-rust-sdk/branches/main/protection --input - <<'EOF'
{"required_status_checks":{"strict":true,"contexts":["all-green"]},"enforce_admins":false,"required_pull_request_reviews":null,"restrictions":null}
EOF
```
  A `403` on branch protection means the plan of the organization does not offer it for private repositories: say so in the report.
- [ ] **Step 3: Exit check.** `scripts/check.sh --full --mutants` → pass. `AGENTCAT_DISABLE_DIAGNOSTICS=1 cargo test --lib parity -- --list | grep -c ': test'` ≥ 219 + the completeness and golden tests. `ENFORCED` holds the 9 files of this plan; `DEFERRED` holds the 10 plan-4 cases.
- [ ] **Step 4: Commit and open pull request 1b.**

```bash
git add -A && git commit -m "docs: retire porting notes; changelog and plan index"
git push -u origin plan-1b-parity-core
gh pr create --base plan-1a-gates --title "TS parity harness, primitives and injection" --body "Parity manifest and tripwire for all 1,024 TS test cases; 219 cases ported for constants, KSUID, UUIDv7, tags, tokens, handles, injection and stripping; D15 reverts in these areas; mutation testing on changed code; nightly upstream and mutation runs."
gh pr checks --watch
```
  Expected: every check green, including `mutants score (changed code)`. The owner merges 1a, then 1b (GitHub retargets 1b to `main` when 1a's branch is deleted).
