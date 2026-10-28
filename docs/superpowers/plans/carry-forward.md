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

Plan 5 closed every item carried to it, except the two that stay open by design: the nightly dispatch after the
merge (an owner step in the PR) and the egress-guard watch item (second pass). Each item and the commit that
closed it:

| Item | Closed by |
|---|---|
| `homepage` in the publish metadata | `122d0a5` |
| `cancel-in-progress` only for pull-request runs | `dbe456d` (C48), `037e731` (a concurrency group per pushed commit) |
| Windows: clone rename retry, DEP0190, the `lock.mjs` break path, write-then-rename, release rename | `dbe456d`, `2bff859`, `037e731` |
| Nightly Windows failure: the parallel `bundle.mjs` rename race (run 40495080931) | `dbe456d`, `037e731` |
| The msrv job runs the doctests | `122d0a5` |
| `extract-schema.mjs` strict ajv compile | `8fa455a` |
| Ingest replay README wording and prerequisites | `c2fa434` |
| Dispatch the nightly once after the merge (R21) | Open: an owner step after the merge, listed in the PR |
| `lock.mjs` robustness (token, unreadable claim, sweep, interleavings, host name) | `dbe456d`, `2bff859`, `037e731` |
| Vector tooling (`write.mjs`, `strict-json.mjs`, the generator rule) | `8fa455a` |
| Oracle tooling (`GIT_REDIRECTS`, temp directory, traces, `captureLogs`, `loadModule` doc, request ID) | `8fa455a` |
| `xtask parity` (unclaimed text, repeated key, repository test, case names) | `ec38289` |
| Rust test support (`OracleError` `Debug`, message cap, `bmp_string` bias) | `d71246c` |
| Plan 2 vector and test coverage list | `d71246c` |
| R26 docs.agentcat.com pages | agentcat-ui draft PR #265 (`7429ed0`; draft until the crates.io release, C57) |
| Plan 3 docs: time driver, `local`, feature unification, `nest_service` | `122d0a5` (the crate guide's known limits) |
| Egress guard and worker threads (watch item) | Open: no TS change requires it; moved to the second pass |
| `cfg(not(unix))` reference server built on macOS and Windows | `dbe456d` (C48 builds it on each system) |
| Registry growth and the cross-session strip set (C19) | `122d0a5` (documented) |
| `rmcp-macros` pin of the rmcp legs | `dbe456d` (the highest `rmcp-macros` of the same minor) |
| Lifecycle hardening (redaction budget, edge cases, `stop` test, `Queue::close`) | `3afd086`, `9086ec4` (C50: no budget, documented) |
| `Event` constructor | `3afd086` (C49) |
| Oracle child stderr leak | `d71246c` |
| Docs: a hook that holds an `AgentCat` clone, `with_default` on another thread | `3afd086` |
| Plan 4 test gaps | `e353183` |
| Plan 4 code items (config split, `fn object`, duplicates, copies, clones) | `00c504a` (speed and allocation benchmarks: second pass) |
| Plan 4 parity mapping (hanging hooks, rebuild-on-demand, re-resolve per request) | `ec38289` |
| Plan 4 oracle generators (`queue.mjs`, `bundle.mjs` doc) | `8fa455a` |
| Plan 4 harness list | `b029b63`, `8a05578` |
| serde_json floor probes at 1.0.127 | `e353183` |
| CI with Windows and macOS before the PR | `dbe456d` (every push runs on three systems) |
| Fix-wave residuals (`redact_event` name, `testing.rs` sentences, `delivery.rs:24`) | `00c504a` |

## Second pass

The first pass ends with plan 5. These items are left for the second pass (R04: the exporters, remote
diagnostics and the crates.io release). The plan 5 reviews and the final review found the minors; the final
review sorted them. Line numbers are as of `7cd430f` and can drift.

Scope of the second pass, from the decisions:

- The exporters (OTLP, Datadog, Sentry, PostHog) and remote SDK diagnostics (R04, C05, D07, D18, D45, D48-D50, P5).
- The crates.io release with trusted publishing and release-plz (R04, D67, design §8 row 43). The release names
  the version in `CHANGELOG.md` (C56) and takes the agentcat-ui draft PR #265 out of draft (C57).
- Speed and allocation measurements of the event pipeline (from plan 4).
- Watch item: the egress guard does not cover worker threads, native addons or `dns.resolve*`. Revisit if TS main
  moves test work to worker threads (from plan 3).

Harness, oracle and CI:

- `tryTake` can wait up to 6.35 s on Windows; `bundle.test.mjs` uses a wall-clock barrier (use a ready/go
  handshake); an `rmSync` failure leaves `.tmp` files in the cache; a failed release keeps the lock for
  the owner's lifetime.
- `refused_connection_is_retried` passes by its 300 ms timeout on Windows: rename it or split it by system.
- The oracle child's stderr relay stops at the first non-UTF-8 line, so the child can block on a full pipe (use a
  lossy `read_until` loop).
- Duplicated test helpers (`proto_renamed`, `dropKey`).
- The hard-coded count of 57 needs a comment on why it is pinned; `wireForm` turns non-plain objects into
  `{}` (guard on a plain-object check); a deviation rule found by a regex scan (a behavioral test is sturdier);
  `captureLogs` interleaves overlapping captures (document it); weak family-content checks in
  `queue.test`; the `settled()` loop spins after the watchdog rejects (add a hung-case test); the
  `writeAll` test reads the committed `queue.json`.
- The harness README names A/A as a user of the TS rulings; the close-deadline test can take 2 s;
  the sibling of the v2 auto-probe is not watched (name the limit in a comment); on the 2025 lane, `http` and
  `http-sse` share one path (say so in the README); the raw 2026 POST headers are not tied to the driver
 .
- A dying example child waits for the 60 s timeout; the session note is copied into four examples and
  nothing checks its text.
- The mutation gate: the diff of a merge commit, duplicate rmcp legs, shards on every push, the upload on a
  timeout (the base part was fixed before the merge).
- `ts-server.mjs` hard-codes `core::io::error::Error`: add a comment.
- `check.sh`'s preflight skips the stable toolchain.
- The `docs` and `minimal-versions` CI jobs use a floating nightly (final review, declined: pre-existing).
- `scripts/rmcp-versions.mjs:7` names `agentcat-ai/agentcat-rust-sdk` in its User-Agent (final review, declined:
  pre-existing).

Parity manifest and `xtask`:

- `manifest.toml` reason wording at line 1609, the inconsistency of lines 5742 and 1790, and a header
  note that harness keys name no transport.
- `parity.rs` doc and text nits, and a lenient reason check that lets a wrong reason pass.
- `xtask public_api` nits.

SDK docs and tests:

- No `Worker::Starting` row in the worker tests; "deterministic both ways" overstates; the phase-1 doc
  says the worker "stops once idle".
- An unrecorded doc line, and an unused `PartialEq`/`Eq` derive; the `config/env.rs` doc omits `home_dir`
 .
- `js.rs:97` is 102 columns; comment lines over 100 columns; `lib.rs:326` wrap, array-index keys, and
  `AgentCat` written as code.
- `bmp_string`'s inflation is not documented; the tags proptest reaches the 50-tag cap only by chance (add a
  fixed case at 50 tags).
- The floor-probe proof is not recorded.
- A test doc's analogy is wrong, and "and type" is not exercised.
- The network-audit instruction and the "m files" wording of the ingest README.
- The docs-site tag-key rule omits `$` (agentcat-ui).
