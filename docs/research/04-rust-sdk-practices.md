# 04 — Rust SDK practices: a style guide and architecture pattern set for `agentcat-sdk`

Date: 2026-10-20. Toolchain used for checks: rustc 1.99.1 stable, nightly 1.101.0 (2026-10-18), 1.88.

## 0. Scope, method, and citation format

This report compares 13 Rust telemetry and analytics SDKs plus `rmcp`, then derives rules for `agentcat-sdk`. All claims cite `repo/path:line`. The repos live in `~/misc/rust-obs-sdks/` (pulled today). `rmcp` lives in `~/misc/rust-mcp-survey/modelcontextprotocol_rust-sdk` and is cited as `rmcp/...`. Web sources have URLs.

| Repo (alias) | Commit | Role in this study |
|---|---|---|
| `sentry-rust` | 936e485 | Init guard, transport thread, rate limits, facade/core split |
| `opentelemetry-rust` | f61467c | Batch processor, retry policy, env precedence, external-type gate, MSRV policy |
| `tracing` | 9d0e557 | Lint block, MSRV + minimal-versions CI, `WorkerGuard`, sealed/newtype stability |
| `dd-trace-rs` | b41eecd | Config registry, env access lint, init that cannot fail |
| `libdatadog` | ce77b3f | No-panic lints, capability traits, byte-bounded buffer, semver PR gate |
| `posthog-rs` | 55277b2 | Closest analog (analytics events): queue, retry, public-API snapshot, trusted publishing |
| `logfire-rust` | 804eca4 | Most polished modern SDK: lints, stderr console, injectable env, insta, OIDC release |
| `axiom-rs`, `tracing-axiom` | ccb5edb, 7c4e15e | README doctests; several anti-patterns |
| `rust-server-sdk` (LaunchDarkly) | 40f5832 | Event processor, transport trait, contract tests, N-2 MSRV bot |
| `statsig-server-core` (statsig-rust) | 1a9b950 | Sharded queue, adaptive flush; poor library hygiene |
| `analytics-rust` (Segment), `rudder-sdk-rust` | 311dc39, 7af2c1e | Retry config with jitter (Rudder); otherwise weak references |
| `rmcp` | c960b4b | The host crate: edition 2024, MSRV 1.88, tokio, tracing, thiserror 2 |

Spot checks: I re-read the key lines for every "headline" claim (sentry transport thread and guard, posthog feature switch and Drop, OTel drop warning and external-types gate, LaunchDarkly retry, dd-trace-rs stdout fallback, tokio MSRV text). I also ran two scratch builds:

- `doc_auto_cfg` is gone: nightly rustdoc rejects `#![feature(doc_auto_cfg)]` with "removed in 1.92.0 ... merged into `doc_cfg`" (rust-lang/rust#138907). `#![cfg_attr(docsrs, feature(doc_cfg))]` alone renders "Available on crate feature X only" with no manual `#[doc(cfg)]`. `docsrs` is a well-known cfg, so it needs no `check-cfg` entry.
- The proposed `[lints]` table (section 18) passes `cargo clippy` on 1.99.1 and `cargo check` on 1.88, and the `const` Send/Sync assertion compiles on 1.88.

## 1. Executive summary

The best-in-class pattern for a background telemetry SDK in Rust, assembled from the references:

1. One published crate with private modules and a flat, curated `pub use` surface (posthog, LaunchDarkly, opentelemetry-otlp).
2. A cloneable `Client` handle (`Arc<Inner>`) built by a hand-written builder that returns `Result`, plus a `#[must_use]` shutdown guard (sentry, logfire, tracing-appender) and an explicit `async fn shutdown(timeout)` (OTel, posthog).
3. The hot path never blocks and never fails: a bounded queue with `try_send`, drop-newest on overflow, a drop counter, one warning on the first drop and a total at shutdown (OTel `span_processor.rs:862-873,930-939`).
4. The worker runs on a dedicated named OS thread with its own current-thread tokio runtime (sentry, logfire, LaunchDarkly, dd-trace-rs). It works inside or outside a host runtime, and a sync `Drop` can wait for it with a bounded timeout without deadlock.
5. reqwest 0.13 with `default-features = false`, explicit connect and request timeouts, jittered exponential retry that honors a clamped `Retry-After`, and an overall deadline (OTel `retry.rs`, posthog `retry.rs`, Rudder `retry.rs`).
6. A `Transport` trait that users and tests can implement, so no `reqwest` type appears in the public API (OTel `HttpClient`, LaunchDarkly `HttpTransport`, libdatadog capabilities).
7. `thiserror` 2 error types that are `#[non_exhaustive]`; delivery failures go to `tracing` and counters, never to the caller's hot path.
8. `tracing` for internal logs, with clippy `print_stdout`/`print_stderr` denied (logfire). This matters because stdout is the protocol channel of stdio MCP servers.
9. No panics in library code, enforced by clippy restriction lints (libdatadog, logfire), and `unsafe_code = "forbid"`.
10. A CI gate set: fmt, clippy (pedantic), cargo-hack feature matrix, MSRV, minimal-versions, cross-OS tests, docs with `-D warnings`, cargo-deny, semver checks, a public-API/external-types gate, and OIDC trusted publishing behind an approval environment.

## 2. Topic 1 — Crate and workspace layout, visibility, re-exports, prelude

What the references do:

- Multi-crate facade designs:
  - sentry: `sentry-types` (protocol, no I/O), `sentry-core` (Hub, Client, Transport traits; a no-op API without the `client` feature), `sentry` (facade with `init` and transports), and integration crates (`sentry-rust/Cargo.toml:3-19`; `sentry-rust/sentry/src/lib.rs:139-149`; `sentry-rust/sentry-core/src/scope/mod.rs:1-11`). The facade re-exports core with `#[doc(inline)] pub use sentry_core::*;` (`sentry-rust/sentry/src/lib.rs:144-145`).
  - OpenTelemetry: API crate, SDK crate, exporter crates, HTTP crate (`opentelemetry-rust/opentelemetry-sdk/src/lib.rs:169-191`).
  - libdatadog: about 60 `libdd-*` crates with per-crate versions (`libdatadog/Cargo.toml:4-68`).
- Single-crate designs: posthog-rs (one crate plus an unpublished compliance adapter, `posthog-rs/Cargo.toml:58-59`), LaunchDarkly (`rust-server-sdk/Cargo.toml:3-6`), axiom-rs, dd-trace-rs (one published crate), logfire (three crates, but only `logfire` is the user entry point).
- Visibility: the good SDKs keep modules private and re-export a flat list.
  - posthog: all modules private (`posthog-rs/src/lib.rs:89-102`), then a flat `pub use` list under the comment "Public interface - any change to this is breaking!" (`posthog-rs/src/lib.rs:104-167`).
  - LaunchDarkly: every module private except `data_sources`, flat re-exports (`rust-server-sdk/launchdarkly-server-sdk/src/lib.rs:20-79`).
  - opentelemetry-otlp: `mod exporter; mod span; mod retry;` all private (`opentelemetry-rust/opentelemetry-otlp/src/lib.rs:656-690`), flat re-exports (`:692-741`), foreign tonic types isolated in `pub mod tonic_types` (`:873`).
  - dd-trace-rs: `pub(crate) mod core;` plus selective re-exports; internals become `pub` only under a `test-utils` feature (`dd-trace-rs/datadog-opentelemetry/src/lib.rs:249-253,268-310`).
  - Counter-example: statsig-rust exposes about 28 `pub mod` including internals, and public struct fields (`statsig-server-core/statsig-rust/src/lib.rs:38-65`; `statsig-server-core/statsig-rust/src/statsig.rs:104-107`).
- `#[doc(hidden)]` is used only for macro plumbing: sentry macros (`sentry-rust/sentry-core/src/macros.rs:36-94`), OTel `_private` (`opentelemetry-rust/opentelemetry/src/lib.rs:305`), dd-trace-rs `dd_*` macros (`dd-trace-rs/datadog-opentelemetry/src/core/log.rs:312-340`).
- Prelude: none of the 13 SDKs has one (grep for `mod prelude` returns nothing).
- Workspace hygiene: `[workspace.package]` inheritance (sentry `Cargo.toml:21-26`, logfire `Cargo.toml:5-9`, libdatadog `Cargo.toml:108-113`), and centralized `[workspace.dependencies]` with `default-features = false`, enforced in CI by `cargo autoinherit` (sentry `lint.yml:42-50`) or a custom linter (`libdatadog/.github/workflows/lint.yml:198-222`).

Recommendation:

- Use a Cargo workspace (cheap to extend) with one published crate, `agentcat-sdk`, at 0.1. Put shared fields in `[workspace.package]`, dependencies in `[workspace.dependencies]`, lints in `[workspace.lints]`.
- Keep every module private (`mod`), re-export the public items at the crate root with one curated `pub use` block and the posthog "this list is the public API" comment. Use `pub(crate)` inside.
- Give public submodules only to groups that users name often and that grow: for example `agentcat_sdk::exporters` and `agentcat_sdk::testing`.
- Do not add a prelude. No reference uses one, and the API is small.
- Add a second published crate only with a concrete need, such as generated wire models shared with other tools (sentry-types precedent). Evidence favors one crate: the multi-crate splits in sentry and OTel exist because they serve many integrations and library authors, which `agentcat-sdk` does not.

Why: a flat surface over private modules lets you move code freely without semver breaks (Cargo SemVer guide: moving or renaming public paths is major). One crate means one version, one changelog, one docs.rs page.

## 3. Topic 2 — Initialization and API shape

What the references do:

- Init guard: `sentry::init(opts) -> ClientInitGuard` (`sentry-rust/sentry/src/init.rs:91-117`). The guard is `#[must_use]`, derefs to `Client`, and its `Drop` calls `close(None)`, which uses `shutdown_timeout` (default 2 s) (`init.rs:13-43`; `sentry-rust/sentry-core/src/clientoptions.rs:913`). tracing-appender returns a `#[must_use] WorkerGuard` whose `Drop` flushes with hard-coded timeouts (`tracing/tracing-appender/src/non_blocking.rs:100-102,282-301`). logfire returns a `#[must_use] ShutdownGuard` from `shutdown_guard()` (`logfire-rust/logfire/src/logfire.rs:564-596`).
- Builder that returns a handle: OTel `SdkTracerProvider::builder()` (`opentelemetry-rust/opentelemetry-sdk/src/trace/provider.rs:177`); logfire `configure()...finish() -> Result<Logfire, ConfigureError>` (`logfire-rust/logfire/src/lib.rs:237`; `config.rs:198`); LaunchDarkly `ConfigBuilder::new(key).build() -> Result<Config, BuildError>` (`rust-server-sdk/launchdarkly-server-sdk/src/config.rs:232,318`); axiom `Client::builder()...build()` (`axiom-rs/src/client.rs:141-149`).
- Options shape:
  - Public fields + `#[non_exhaustive]` + consuming setters + manual `Default`: sentry `ClientOptions` (`sentry-rust/sentry-core/src/clientoptions.rs:140-143,320,889-928`). `#[non_exhaustive]` blocks struct literals outside the crate, so the setters are required.
  - Private fields + builder: OTel `BatchConfig` with `pub(crate)` fields and `BatchConfigBuilder` (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:1029-1067`); logfire hand-written by-value builder with `#[must_use = "call .finish()…"]` (`logfire-rust/logfire/src/config.rs:25-26`); posthog `derive_builder` with a private generated `build_unchecked` and a hand-written validating `build()` (`posthog-rs/src/client/mod.rs:125-127,452-454`).
  - Anti-pattern: statsig `StatsigOptions` with public `Option<T>` fields and no `#[non_exhaustive]` (`statsig-server-core/statsig-rust/src/statsig_options.rs:25-30`): every new field is a breaking change.
  - Typestate builder: opentelemetry-otlp `SpanExporterBuilder<NoExporterBuilderSet>` → `with_http()` (`opentelemetry-rust/opentelemetry-otlp/src/span.rs:45-71`).
- Env vars and precedence:
  - OTel states the rule: code > signal-specific env > generic env > default (`opentelemetry-rust/CONTRIBUTING.md:300`; implemented in `opentelemetry-rust/opentelemetry-otlp/src/exporter/mod.rs:420-438`). Env var names are `pub const` (`exporter/mod.rs:38`).
  - dd-trace-rs: one `EnvSource` may call `std::env::var` (`dd-trace-rs/datadog-opentelemetry/src/core/configuration/sources.rs:254-264`); `clippy.toml` bans `std::env::var*`/`set_var`/`remove_var` elsewhere (`dd-trace-rs/clippy.toml:4-11`). Each value records its origin (`sources.rs:13-32`). A JSON registry of supported keys is code-generated and diffed in CI (`dd-trace-rs/supported-configurations.json`; `dd-trace-rs/.github/workflows/configuration.yaml:16-44`).
  - logfire: env lookup takes an injectable `Option<&HashMap>` so tests never mutate the process environment (`logfire-rust/logfire/src/internal/env.rs:8-23`).
  - sentry reads `SENTRY_DSN` (ignores invalid values), `SENTRY_RELEASE`, `SENTRY_ENVIRONMENT`, proxies (`sentry-rust/sentry/src/defaults.rs:80-114`).
- Disabled/no-op client instead of failure: sentry with no DSN (`sentry-rust/sentry-core/src/client/mod.rs:162-170`); posthog with a blank key (`posthog-rs/src/client/mod.rs:361-366`); logfire `SendToLogfire::IfTokenPresent` (`logfire-rust/logfire/src/config.rs:205-213`); dd-trace-rs wraps init in `catch_unwind` and falls back to a no-op provider (`dd-trace-rs/datadog-opentelemetry/src/lib.rs:557-620`).
- Global vs explicit:
  - Global: sentry Hub (`sentry-rust/sentry-core/src/hub_impl.rs:10-27`), posthog optional `OnceLock<Client>` whose Drop never runs (`posthog-rs/src/global.rs:10-11,40`), statsig `shared()` that silently builds a dummy instance when nothing is registered (`statsig-server-core/statsig-rust/src/statsig.rs:415-423`).
  - OTel documents that "Libraries should NOT call" `set_tracer_provider` (`opentelemetry-rust/opentelemetry/src/global/trace.rs:423-440`). dd-trace-rs offers `init_local()` that returns handles without touching globals (`dd-trace-rs/datadog-opentelemetry/src/lib.rs:421-424`).
- Handles: OTel providers are `#[derive(Clone)] struct { inner: Arc<Inner> }`; dropping the last clone shuts down (`opentelemetry-rust/opentelemetry-sdk/src/trace/provider.rs:16-22,137-148,157-160`). logfire `Logfire` is an `Arc` newtype (`logfire-rust/logfire/src/logfire.rs:64-65`). LaunchDarkly and posthog clients are not `Clone`; users wrap them in `Arc`.

Recommendation:

- Shape: `let client = agentcat_sdk::Client::builder(project_id).api_key(..).build()?;` returns a `Client` that is `Clone + Send + Sync + 'static` and wraps `Arc<Inner>`. Use the C-BUILDER guideline and a hand-written builder with private fields, consuming `#[must_use]` setters, and a `build() -> Result<Client, Error>` that validates. Do not use `derive_builder` (it leaks `derive_builder::UninitializedFieldError`, `posthog-rs/api/public-api.txt:17`).
- Return `Err` only for programmer errors (invalid URL, empty project id). For a missing API key or a kill-switch env var, return a disabled client that accepts and discards events, and log one `warn!` (posthog, sentry, logfire precedent).
- Shutdown: give `Client::shutdown(&self, timeout) -> impl Future<Output = Result<(), ShutdownError>>` and `Client::flush(&self, timeout)` as the documented path. Also give `client.shutdown_guard() -> ShutdownGuard` (`#[must_use]`), whose `Drop` runs a bounded blocking shutdown unless shutdown already ran (sentry, logfire, tracing-appender). As a final safety net, the last `Inner` drop runs the same bounded shutdown (OTel provider pattern). Make shutdown idempotent with `AtomicBool::compare_exchange` (`opentelemetry-rust/opentelemetry-sdk/src/trace/provider.rs:246-270`).
- Configuration precedence: code > `AGENTCAT_*` env > default (OTel rule). Read env vars in one module only and enforce it with clippy `disallowed-methods` (dd-trace-rs). Make that module take an injectable lookup so tests never call `std::env::set_var`. This is not optional: `set_var` is `unsafe` in edition 2024, and rule 9 forbids `unsafe`.
- Export the env var names as `pub const` and list them in a table in the crate docs (OTel).
- No global client in 0.1. Pass the handle explicitly to the rmcp integration. A `OnceLock` global can come later as an additive change.

Why: an explicit, cloneable handle is easy to test, has no hidden state, and fits rmcp, where the server handler owns its dependencies. The guard plus async shutdown covers both "user forgot" and "user is careful". Private fields plus a builder make every new option a minor change (Cargo SemVer guide `#struct-add-public-field-when-no-private`).

## 4. Topic 3 — Async runtime, background worker, Drop, flush and shutdown

What the references do (three models):

| Model | Who | Evidence |
|---|---|---|
| Dedicated OS thread with its own tokio runtime | sentry (reqwest transport), logfire, LaunchDarkly, dd-trace-rs exporter, libdd-telemetry `run()` | `sentry-rust/sentry/src/transports/tokio_thread.rs:130-137`; `logfire-rust/logfire/src/logfire.rs:761-794`; `rust-server-sdk/launchdarkly-server-sdk/src/events/dispatcher.rs:102-106`; `dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:161-185`; `libdatadog/libdd-telemetry/src/worker/mod.rs:1535-1564` |
| Dedicated OS thread with blocking I/O | posthog-rs, OTel default batch processor, sentry curl/ureq | `posthog-rs/src/client/transport.rs:1-17,136-158`; `opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:453,571-572,790` |
| Task on the host runtime | statsig (`Handle::try_current()` or a hidden global runtime), OTel `experimental_async_runtime`, tracing-axiom `install_batch(runtime::Tokio)`, libdd-telemetry `spawn()` | `statsig-server-core/statsig-rust/src/statsig_runtime.rs:41-56,211-218`; `opentelemetry-rust/opentelemetry-sdk/src/runtime.rs:23-41`; `tracing-axiom/src/builder.rs:207-211` |

- Why the owned thread wins in the references: sentry's design lets `init` work inside `#[tokio::test]` and with no runtime (`sentry-rust/sentry/tests/test_async.rs:4-7`). dd-trace-rs builds its runtime in a fresh thread because `block_on` panics inside a caller's runtime (`dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:161-185,298-305`). OTel's `TokioCurrentThread` runtime also spawns a separate thread to avoid deadlocks (`opentelemetry-rust/opentelemetry-sdk/src/runtime.rs:116-132`), and OTel documents that calling `shutdown()` on a current-thread tokio main can deadlock (`span_processor.rs:480-483`). The tokio docs confirm: `Runtime::block_on` panics inside an async context, and dropping a `Runtime` inside async context is not permitted (https://docs.rs/tokio/latest/tokio/runtime/struct.Runtime.html#method.block_on).
- Bounded queues and drop policy:
  - sentry: `sync_channel`, default capacity 30, `try_send`; full → drop and record `QueueOverflow` (`sentry-rust/sentry/src/transports/thread.rs:176-195`; `transports/mod.rs:61`).
  - OTel: data channel `sync_channel(max_queue_size)` (default 2048) and a separate control channel of 64 for flush/shutdown (`span_processor.rs:520-521,58-80`). First drop logs one warning; shutdown logs the total (`:862-873,930-939`).
  - posthog: CAS slot counter, default `max_queue_size` 10000, one warning per overflow episode (`posthog-rs/src/client/transport.rs:330-363`; `posthog-rs/src/client/mod.rs:258-288`).
  - LaunchDarkly: crossbeam `bounded(500)`, `try_send`, warning once per process via `Once` (`rust-server-sdk/launchdarkly-server-sdk/src/events/processor.rs:78-105`).
  - libdatadog: byte-bounded buffer (5 MB max, flush at 1.5 MB or 2 s), drop new chunk, count drops (`libdatadog/libdd-data-pipeline/src/trace_buffer/mod.rs:156-165,245-261`).
  - statsig: drops the oldest event; panics under `cfg!(test)` on lock timeout (`statsig-server-core/statsig-rust/src/event_logging/event_queue/queue.rs:107-118,145-148`).
- Batching: OTel exports early when the batch reaches `max_export_batch_size` and otherwise ticks on `scheduled_delay` with `recv_timeout` (`span_processor.rs:599,817-826`). posthog: `flush_at` 100, `flush_interval_ms` 5000 (`posthog-rs/src/client/mod.rs:258-288`). statsig adapts the interval: doubles on failure up to 60 s, halves on success (`statsig-server-core/statsig-rust/src/event_logging/flush_interval.rs:32,50`).
- Flush and shutdown APIs:
  - OTel: `force_flush()` and `shutdown_with_timeout(Duration)` send a oneshot over the control channel and wait with `recv_timeout`; results are `OTelSdkResult` with `Timeout(d)` and `AlreadyShutdown` (`span_processor.rs:892-905,929-950`).
  - dd-trace-rs spends one deadline budget across steps: flush, trigger shutdown, wait, stop telemetry (`dd-trace-rs/datadog-opentelemetry/src/span_processor.rs:692-739`).
  - LaunchDarkly `flush_blocking(timeout)` (`Duration::ZERO` = forever) and a `close()` that waits for in-flight sends by channel disconnect (`rust-server-sdk/launchdarkly-server-sdk/src/events/processor.rs:131-166`; `dispatcher.rs:152-166`).
  - posthog: `flush().await`, idempotent `shutdown().await` (`posthog-rs/src/client/async_client.rs:336-347,407-427`).
- Drop semantics and their flaws:
  - sentry: guard Drop is bounded by `shutdown_timeout`, but `TransportThread::drop` joins with `handle.join().unwrap()` and no timeout, reqwest has no request timeout, and the shutdown flag makes the worker drop queued items silently (`sentry-rust/sentry/src/transports/thread.rs:130,207-215`). Flush uses a blocking `send`, so a full queue can block past the timeout (`thread.rs:200-204`).
  - posthog: `Drop` does a blocking drain and the docs admit it blocks the executor thread (`posthog-rs/src/client/async_client.rs:1432-1445`).
  - LaunchDarkly: no `Drop`; buffered events are lost without `close()`, and `close()` has no timeout (`rust-server-sdk/launchdarkly-server-sdk/src/events/processor.rs:111-129`).
  - statsig: `Drop` calls `force_shutdown` and does not flush (`statsig-server-core/statsig-rust/src/statsig.rs:143-158`).
  - OTel: provider `Drop` shuts down if needed and logs errors only at debug, "non-actionable by the user" (`opentelemetry-rust/opentelemetry-sdk/src/trace/provider.rs:119-148`). But `.expect("Failed to spawn thread")` and `handle.join().unwrap()` remain (`span_processor.rs:687,950`).
- Test seams for time: posthog injects a `Clock` (`posthog-rs/src/client/transport.rs:83`); libdatadog injects `SleepCapability` and uses `tokio::time::pause` (`libdatadog/libdd-capabilities/src/sleep.rs:13-24`); libdatadog separates the worker logic from where it runs (`libdatadog/libdd-shared-runtime/src/worker.rs:19-43`).

Recommendation:

- Run delivery on one named OS thread, `agentcat-worker`, that owns a `tokio::runtime::Builder::new_current_thread().enable_all()` runtime and the `reqwest::Client`. Never use the host runtime, never call `block_on` on a host thread, never drop a `Runtime` on a host thread.
- Hot path: `capture()` is a sync, non-async method that calls `try_send` on a bounded `tokio::sync::mpsc` channel (its `try_send` needs no runtime). On `Full`, drop the new event, increment an `AtomicU64`, and log `warn!` on the first drop only; log the total at shutdown (OTel pattern). On `Closed`, count and drop.
- Use a separate small control channel for `Flush` and `Shutdown`, each carrying a `tokio::sync::oneshot` ack, so flush never waits behind a full data queue (fixes sentry's flaw; copies OTel).
- Batch by count, by serialized bytes, and by interval. Bound the queue by both events and bytes (libdatadog) so large tool payloads cannot exhaust memory.
- Write the worker as a runtime-agnostic `async fn run(rx, ctrl, transport, clock)` that tests drive on a `#[tokio::test(start_paused = true)]` runtime with an injected transport. Production only decides where it runs (libdatadog worker/runtime split; posthog `Clock`).
- Shutdown: one deadline budget across "drain queue, send final batches, stop thread" (dd-trace-rs). Wait for the thread with `recv_timeout` on the ack channel, never `join().unwrap()`. If the deadline passes, detach the thread and return `Err(ShutdownError::Timeout)`; the OS ends it at process exit.
- `Drop` of the guard blocks for at most the configured shutdown timeout (default 2 s, sentry; posthog's 30 s is too long for CLI-like stdio servers). Document that `shutdown().await` before the end of `main` makes `Drop` a no-op (C-DTOR-BLOCK).
- If the thread cannot spawn, return a disabled client and log `warn!` (sentry `thread.rs:163` uses `.ok()`), do not panic.

Why: rmcp servers run on tokio, but the SDK must also behave in `#[tokio::test]`, in current-thread runtimes, and when `main` returns. Only the owned-thread model lets a synchronous `Drop` wait for delivery with a bound and without deadlock. The cost is one OS thread and one small runtime, which every reference in this model accepts.

## 5. Topic 4 — HTTP transport

What the references do:

- Stack: reqwest in sentry, posthog, logfire, statsig, axiom, OTel (`opentelemetry-http`); hyper directly in libdatadog and LaunchDarkly (`libdatadog/libdd-common/Cargo.toml:45-58`).
- Default features off: sentry `reqwest = { version = "0.13.2", default-features = false }` (`sentry-rust/Cargo.toml:72`); posthog `default-features=false` + `rustls`, `blocking`, `json`, `gzip` (`posthog-rs/Cargo.toml:12-17`); logfire `rustls-no-provider` (`logfire-rust/logfire/Cargo.toml:24-25`); rmcp `default-features = false` (`rmcp/crates/rmcp/Cargo.toml:78`).
- reqwest 0.13 facts (`~/.cargo/registry/.../reqwest-0.13.5/Cargo.toml`; https://seanmonstar.com/blog/reqwest-v013-rustls-default/): default TLS is now rustls with the aws-lc-rs provider and rustls-platform-verifier; `rustls-tls` was renamed `rustls`; `rustls-no-provider` lets the app install a provider; `native-tls` now includes ALPN; MSRV 1.85. The `gzip` feature is response decompression only; request compression needs flate2 (posthog `src/compression.rs:12-15`, LaunchDarkly `sender.rs:115-124`).
- TLS feature names:
  - sentry: `native-tls`, `rustls`, `rustls-no-provider` (`sentry-rust/sentry/Cargo.toml:62-68`).
  - rmcp: `reqwest` (rustls), `reqwest-tls-no-provider`, `reqwest-native-tls` (`rmcp/crates/rmcp/Cargo.toml:143-147`).
  - logfire: provider-free by default plus `tls-aws-lc`; CI asserts with `cargo tree` that neither aws-lc-rs nor ring is compiled without that feature (`logfire-rust/.github/workflows/main.yml:79-91`). libdatadog has a similar crypto-provider check (`libdatadog/.github/workflows/check-crypto-providers.yml`).
  - aws-lc-sys carries an "OpenSSL" license item that OTel lists as a cargo-deny exception (`opentelemetry-rust/deny.toml:15`).
- Injection traits:
  - OTel `#[async_trait] pub trait HttpClient: Debug + Send + Sync { async fn send_bytes(&self, Request<Bytes>) -> Result<Response<Bytes>, HttpError>; }`; the client must enforce its own timeout, non-2xx returns `Ok` (`opentelemetry-rust/opentelemetry-http/src/lib.rs:27-31,127-147`). The OTLP exporter takes `with_http_client<T: HttpClient>` and exposes no reqwest type (`opentelemetry-rust/opentelemetry-otlp/src/exporter/http/mod.rs:836`).
  - LaunchDarkly `trait HttpTransport: Send + Sync + Clone + 'static { fn request(&self, Request<Option<Bytes>>) -> ResponseFuture; }` with a boxed future (launchdarkly-sdk-transport 0.1.4 `src/transport.rs:24,132,156`).
  - sentry `pub trait Transport: Send + Sync + 'static` with `send_envelope`, `flush`, `shutdown` defaults, plus `TransportFactory` (`sentry-rust/sentry-core/src/transport/mod.rs:15-34,51-82`). Bug: `impl Transport for Arc<T>` forgets to forward `flush` (`:104-112`).
  - libdatadog capability traits for HTTP and sleep, swappable for wasm and tests (`libdatadog/libdd-capabilities/src/lib.rs:6-25`).
  - posthog and sentry accept a user `reqwest::Client` (`posthog-rs/src/client/mod.rs:150-177`; `sentry-rust/sentry/src/transports/reqwest.rs:228`), which leaks the type into their public API.
- Timeouts:
  - posthog 30 s capture, 3 s flags (`posthog-rs/src/client/mod.rs:183,221`); statsig 10 s; axiom 10 s; OTel `OTEL_EXPORTER_OTLP_TIMEOUT` 10 s (`opentelemetry-rust/opentelemetry-otlp/src/exporter/mod.rs:38,49`).
  - None at all: sentry reqwest transport, LaunchDarkly default hyper transport ("There is no connect/read/write timeout by default", launchdarkly-sdk-transport `transport_hyper.rs:516`). Combined with untimed close, one hung POST blocks shutdown.
- Retry and backoff:
  - OTel `RetryPolicy::recommended()`: 3 retries, 100 ms → 1600 ms, 100 ms jitter (`opentelemetry-rust/opentelemetry-otlp/src/retry_policy.rs:36-42`). 429 and 503 honor `Retry-After` (seconds or HTTP-date); 502, 504 and network errors retry; others do not (`retry_classification.rs:33-82`). Total time bounded by the export timeout (`retry.rs:102-161`).
  - posthog: retries 408/5xx; 429 only with `Retry-After`, clamped to the configured maximum; exponential without jitter (`posthog-rs/src/client/retry.rs:33-46,103-115`).
  - Rudder: 3 retries, 100 ms base, 30 s cap, 0.2 jitter, `Retry-After` as a floor with no cap (`rudder-sdk-rust/src/retry.rs:27-28,48-70`).
  - LaunchDarkly: two attempts with a fixed 1 s sleep; 401/403/other 4xx disable events forever with no log line (`rust-server-sdk/launchdarkly-server-sdk/src/events/sender.rs:126-129,183-193`). One payload UUID is reused across retries as an idempotency key (`sender.rs:93,141`).
  - libdatadog: retries every 4xx and 5xx (`libdatadog/libdd-trace-utils/src/send_with_retry/mod.rs:196-219`; `libdatadog/libdd-http-client/src/retry.rs:81-83`). statsig treats 429 as non-retryable (`statsig-server-core/statsig-rust/src/networking/network_client.rs:21-28`). axiom's "do not retry 4xx" check is dead code (`axiom-rs/src/http.rs:82-108`).
- Rate limits: sentry `RateLimiter` parses its custom header, then `Retry-After`, then a bare 429 = 60 s, per category (`sentry-rust/sentry/src/transports/ratelimit.rs:29-80,184-190`), but uses `SystemTime` instead of `Instant` (`:13-19`).
- Compression: LaunchDarkly gzip on by default (`event-compression` feature) with a runtime toggle; posthog opt-in `capture_compression`; OTel `gzip-http`/`zstd-http` features; statsig gzip best or zstd.
- Other: OTel caps response bodies at 4 MiB (`opentelemetry-rust/opentelemetry-http/src/lib.rs:150`) and logs error bodies only at debug because they may echo secrets (`opentelemetry-otlp/src/exporter/http/mod.rs:542-552`). logfire sets a User-Agent (`logfire-rust/logfire/src/exporters.rs:41-60`). reqwest's blocking client panics inside tokio; OTel builds it on a throwaway thread to cope (`opentelemetry-otlp/src/exporter/http/mod.rs:245-264`).

Recommendation:

- Depend on `reqwest = { version = "0.13", default-features = false }` and enable only what you use. Serialize JSON yourself with `serde_json::to_vec` and send bytes, so you need no `json` feature.
- Define a public `Transport` trait that is dyn-compatible: `fn send(&self, request: TransportRequest) -> BoxFuture<'_, Result<TransportResponse, TransportError>>` with `Send + Sync + 'static` bounds (LaunchDarkly boxed future; OTel trait bounds). Use your own small request/response types or `http` 1.x types; do not take or return `reqwest` types. Ship the reqwest implementation as the default and an in-memory one under `test-util`.
- Set a connect timeout (for example 5 s) and a per-request timeout (for example 10 s) on every request. Never leave them unbounded.
- Retry policy: retry network errors, 408, 429, 500, 502, 503, 504 with exponential backoff plus full jitter (libdatadog http-client and Rudder use jitter; posthog and OTel recommend bounded totals). Honor `Retry-After` (seconds and HTTP-date) and clamp it to the policy maximum (posthog). Bound all retries by the flush or shutdown deadline (OTel). Do not retry other 4xx; drop the batch and log `warn!` with the status. For 401/403, disable delivery and log `error!` once (LaunchDarkly disables silently; do not copy the silence).
- Use `Instant` for every backoff and rate-limit window (sentry's `SystemTime` is wrong under clock jumps).
- Send `User-Agent: agentcat-sdk-rust/<version>` and an idempotency key per batch reused across retries, if the API supports one (LaunchDarkly).
- Offer request gzip with flate2 behind a `gzip` feature if the AgentCat API accepts `Content-Encoding: gzip`.
- Cap the response body that you read (OTel 4 MiB). Log response bodies only at `debug!` and never log the API key.
- TLS features: mirror reqwest and sentry names, `rustls` (default), `rustls-no-provider`, `native-tls`. If you offer `rustls-no-provider`, add the logfire-style `cargo tree` CI check.

Why: the references that fail in production fail here: no timeouts (sentry, LaunchDarkly), retry storms on 4xx (libdatadog), data loss on 429 (statsig), silent disable (LaunchDarkly). OTel's policy is the most complete public design and costs little code.

## 6. Topic 5 — Errors and panics

What the references do:

- thiserror: logfire `ConfigureError`/`ShutdownError`, both `#[non_exhaustive]`, with a `Box<dyn Error + Send + Sync>` catch-all (`logfire-rust/logfire/src/lib.rs:138-213`); opentelemetry-otlp `ExporterBuildError` (not non_exhaustive) (`opentelemetry-rust/opentelemetry-otlp/src/exporter/mod.rs:115-132`); LaunchDarkly (thiserror + `#[non_exhaustive]`, but flattened to `InvalidConfig(String)`, `rust-server-sdk/launchdarkly-server-sdk/src/config.rs:205-208`); sentry-types; rmcp (`thiserror = "2"`, `rmcp/crates/rmcp/Cargo.toml:53`).
- Hand-written: OTel `OTelSdkError { AlreadyShutdown, Timeout(Duration), InternalFailure(String) }` with `OTelSdkResult` (`opentelemetry-rust/opentelemetry-sdk/src/error.rs:11-61`); posthog (`#[non_exhaustive]`, `String` payloads lose `source()`, `posthog-rs/src/error.rs:3-61`); statsig `StatsigErr` with no `std::error::Error` impl at all (`statsig-server-core/statsig-rust/src/statsig_err.rs:5-59`).
- Private-field tricks instead of `#[non_exhaustive]`: OTel `ResponseBodyTooLarge { _private: () }` (`opentelemetry-rust/opentelemetry-http/src/lib.rs:157-159`), tracing `ParseLevelError { _p: () }` (`tracing/tracing-core/src/metadata.rs:565`).
- Feature-dependent variants: axiom `JoinError` exists only with the `tokio` feature (`axiom-rs/src/error.rs:86-89`) — enum shape depends on features.
- Leaking dependency errors: axiom `reqwest::Error`, `http::Method`, `serde_qs::Error` (`axiom-rs/src/error.rs:41,64,76`); Rudder `#[from] reqwest::Error` (`rudder-sdk-rust/src/errors.rs:7-17`); sentry `serde_json::Error` (`sentry-rust/sentry-types/src/protocol/envelope.rs:36-39`).
- Delivery errors are not returned to the caller: sentry returns `bool`/`()`; posthog reports through `on_error` hooks with a `#[non_exhaustive]` error and wraps user hooks in `catch_unwind` (`posthog-rs/src/client/mod.rs:432-440`; `posthog-rs/src/client/common.rs:66,88`); statsig through an optional observability client.
- No-panic policy:
  - libdatadog: `#![cfg_attr(not(test), deny(clippy::panic, clippy::unwrap_used, clippy::expect_used, clippy::todo, clippy::unimplemented))]` at crate roots (`libdatadog/libdd-data-pipeline/src/lib.rs:3-8`; `libdatadog/libdd-telemetry/src/lib.rs:7-11`), plus workspace lints (`libdatadog/Cargo.toml:384-390`) and `allow-*-in-tests` in `libdatadog/clippy.toml`.
  - logfire: `unwrap_used = "deny"` (`logfire-rust/logfire/Cargo.toml:137`).
  - Panics found in library paths elsewhere: sentry DSN conversion `.expect`, runtime build `.unwrap()`, `join().unwrap()` (`sentry-rust/sentry-core/src/clientoptions.rs:930-944`; `sentry-rust/sentry/src/transports/tokio_thread.rs:134-137`); OTel thread spawn `.expect` (`span_processor.rs:687`); LaunchDarkly `result_tx.send(..).unwrap()` (`sender.rs:168,191,203`); tracing-axiom `reqwest::Client::new()` (panics if TLS setup fails, `tracing-axiom/src/builder.rs:203`).

Recommendation:

- Use `thiserror` 2 (already in every rmcp build, so it adds no compile cost).
- One public `Error` enum for builder and lifecycle errors, `#[non_exhaustive]`, with lowercase `Display` text and no trailing punctuation (C-GOOD-ERR). Errors are `Send + Sync + 'static`. Wrap foreign errors as `#[source] Box<dyn std::error::Error + Send + Sync>` so no reqwest or serde type appears in signatures. Keep the same variants under every feature set.
- A separate small `ShutdownError { Timeout, AlreadyShutdown }`-style type is fine (OTel `OTelSdkError` shape), also `#[non_exhaustive]`.
- `capture()` returns `()`. Delivery failures go to `tracing`, to drop/failure counters, and to an optional `on_error`/`on_drop` callback (posthog). Wrap every user callback in `catch_unwind` (posthog).
- Deny `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` in library code via `[lints]`, and allow them in tests via `clippy.toml` (`allow-unwrap-in-tests` etc., libdatadog). Where a lock can poison, use `parking_lot` or recover with `into_inner()` instead of `expect`.
- Document `# Errors` sections on every fallible public fn (C-FAILURE).

Why: a telemetry SDK must never take down the host. The references show that panics creep in at exactly the edges (thread spawn, runtime build, TLS init, channel send), so a lint gate is the only reliable defense.

## 7. Topic 6 — Internal logging (and stdout safety)

What the references do:

- `tracing`: posthog (mostly `warn!`, `posthog-rs/Cargo.toml:29`), libdatadog (`libdatadog/libdd-trace-utils/src/send_with_retry/mod.rs:189-231`), OTel internal macros that wrap `tracing` with `target: env!("CARGO_PKG_NAME")` and a required event `name:` (`opentelemetry-rust/opentelemetry/src/global/internal_logging.rs:24-45`), rmcp (`rmcp/crates/rmcp/Cargo.toml:57`).
- `log`: LaunchDarkly (old `#[macro_use] extern crate log`, `rust-server-sdk/launchdarkly-server-sdk/src/lib.rs:12-13`), statsig, Rudder (which forces `env_logger` as a library dependency, `rudder-sdk-rust/Cargo.toml:21`).
- Debug flag with direct printing: sentry `debug: bool` → `eprintln!` via `sentry_debug!` (`sentry-rust/sentry-core/src/macros.rs:55-62`); messages are lost when no client is bound to the current hub.
- Dangerous fallbacks:
  - dd-trace-rs prints non-error levels to **stdout** with `println!` when no tracing subscriber is set (`dd-trace-rs/datadog-opentelemetry/src/core/log.rs:304-310`). In a stdio MCP server that corrupts the JSON-RPC stream.
  - libdatadog `NativeCapabilities::write_log_output` writes to stdout (`libdatadog/libdd-capabilities-impl/src/lib.rs:91-101`).
  - statsig installs `simple_logger` or overrides the app's `log::set_max_level` (`statsig-server-core/statsig-rust/src/output_logger.rs:128-133`).
- Good stdout hygiene: logfire denies `print_stdout` and `print_stderr` in clippy (`logfire-rust/logfire/Cargo.toml:134-135`), and its optional console exporter defaults to stderr (`logfire-rust/logfire/src/config.rs:364-365`).
- Self-telemetry loops: OTel worker threads enter `Context::enter_telemetry_suppressed_scope()` (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:574`); dd-trace-rs does the same around export-error logging (`dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:516-518`).
- Level discipline (OTel): error for export failures and shutdown timeouts; warn for the first drop, the drop summary, use after shutdown; info for lifecycle; debug for control-channel-full, errors in Drop, and HTTP bodies.
- Secrets: statsig truncates messages and scrubs secrets (`statsig-server-core/statsig-rust/src/output_logger.rs:155-167`); dd-trace-rs sends only the format template to telemetry (`dd-trace-rs/datadog-opentelemetry/src/core/log.rs:176-184`); LaunchDarkly pretty-prints whole payloads at debug (`sender.rs:95-99`).

Recommendation:

- Use `tracing` 0.1 for all internal diagnostics, with the default target (`agentcat_sdk::...`). Never install a subscriber or logger. rmcp servers already configure `tracing`, so users get SDK logs for free with `RUST_LOG=agentcat_sdk=debug`.
- Deny `clippy::print_stdout`, `clippy::print_stderr`, and `clippy::dbg_macro` in library code (logfire). Do not add a sentry-style `debug` flag that prints; document the `RUST_LOG` filter instead.
- Levels: `error!` for permanent delivery failure (auth), shutdown timeout; `warn!` for the first dropped event, the drop total at shutdown, ignored invalid env values, disabled client; `info!` at most once at init (enabled/disabled, endpoint host); `debug!` per batch (count, bytes, status, retry); `trace!` for payload sizes. Never log event payloads, tool arguments, or the API key at any level above `trace!`, and redact the key everywhere.
- Mark the worker thread so that any future SDK integration that captures `tracing` events ignores events produced by the SDK itself (OTel suppression-scope idea, implemented as a thread-local flag).
- Document in the README: for stdio servers, configure your `tracing` subscriber with `.with_writer(std::io::stderr)`.

Why: stdout belongs to the MCP protocol in stdio transports. Two references (dd-trace-rs, libdatadog) would corrupt it. A lint makes the rule mechanical.

## 8. Topic 7 — Cargo features and docs.rs

What the references do:

- Defaults: sentry turns on 8 features including the reqwest + native-tls transport (`sentry-rust/sentry/Cargo.toml:24-33,57`); opentelemetry-otlp defaults to `http-proto` + `reqwest-blocking-client` (`opentelemetry-rust/opentelemetry-otlp/Cargo.toml:67`); dd-trace-rs `default = []`; logfire `data-dir`, `export-http-protobuf`, `tls-aws-lc` (`logfire-rust/logfire/Cargo.toml:105-128`); LaunchDarkly `hyper-rustls-native-roots`, `crypto-aws-lc-rs`, `event-compression`, `float-roundtrip`.
- Naming: kebab-case crate-like names (sentry); `rt-tokio`, `experimental_*`, `spec_unstable_*`, `testing` (OTel `opentelemetry-sdk/Cargo.toml:49-67`); `test-utils` (libdatadog, dd-trace-rs); private features prefixed `_` (dd-trace-rs `_unstable_propagation`); rmcp `__reqwest` internal feature (`rmcp/crates/rmcp/Cargo.toml:141`).
- Non-additive features (anti-pattern): posthog `async-client` swaps the `Client` type (`posthog-rs/src/client/mod.rs:49-67`); statsig `custom_network_provider` removes reqwest; axiom makes TLS and runtime features mutually exclusive with `compile_error!`, which breaks `--all-features` (`axiom-rs/src/lib.rs:65-75`). libdd-http-client also uses `compile_error!` when no backend is on (`libdatadog/libdd-http-client/src/lib.rs:45-65`).
- Test-only features in the public set: posthog `test-harness`, `e2e-test` (`posthog-rs/Cargo.toml:50-56`); sentry `test` (`sentry-rust/sentry/Cargo.toml:52`); OTel `testing`.
- docs.rs: sentry uses a custom `--cfg doc_cfg` that needs a `check-cfg` entry, and has a typo'd `doc(cfg(feature="debug_images"))` (`sentry-rust/sentry/Cargo.toml:18-21`; `sentry-rust/Cargo.toml:35`; `sentry-rust/sentry/src/lib.rs:220`). OTel and tracing use `all-features = true` + `rustdoc-args = ["--cfg", "docsrs"]` and tag items by hand with `#[cfg_attr(docsrs, doc(cfg(..)))]` (`opentelemetry-rust/opentelemetry/Cargo.toml:20-22`; `tracing/tracing/Cargo.toml:106-109`). LaunchDarkly sets `features = []`.
- Current state (verified locally and at https://github.com/rust-lang/rust/pull/138907): `doc_auto_cfg` was merged into `doc_cfg` in 1.92; `#![cfg_attr(docsrs, feature(doc_cfg))]` now shows feature badges automatically. docs.rs passes `--cfg docsrs` itself since 2024 (https://github.com/rust-lang/docs.rs/pull/2390), so the `rustdoc-args` line is redundant but harmless and helps local builds.

Recommendation:

- Proposed feature set (all additive):
  - `default = ["rustls"]` (plus `gzip` if the API accepts it).
  - `rustls` → `reqwest/rustls`; `rustls-no-provider` → `reqwest/rustls-no-provider`; `native-tls` → `reqwest/native-tls`.
  - `gzip` → `dep:flate2` (request compression).
  - `test-util` → exposes `agentcat_sdk::testing` (tokio naming, familiar to rmcp users).
- Gate only code that adds dependencies. If the OTLP/Datadog/Sentry/PostHog exporters are hand-written JSON over the same reqwest client, compile them always and select them at runtime; add `exporter-*` features only for exporters that pull extra crates (for example protobuf).
- Never let a feature change a type, a signature, or an enum's variants. Never use `compile_error!` for feature combinations; return a builder error instead (for example "no TLS backend for an https endpoint").
- docs.rs: `[package.metadata.docs.rs] all-features = true`, `rustdoc-args = ["--cfg", "docsrs"]`, and `#![cfg_attr(docsrs, feature(doc_cfg))]` at the crate root. Add no manual `doc(cfg(..))`. Document every feature in a crate-level table.
- Feature names follow C-FEATURE: no `use-`/`with-` prefixes.

Why: feature unification makes non-additive features a correctness bug for every downstream user (Cargo reference, "Feature unification"). A small, flat feature set keeps the `cargo hack --feature-powerset` CI job cheap.

## 9. Topic 8 — MSRV, edition, `rust-version`

What the references do:

| Repo | Edition | `rust-version` | Policy / enforcement |
|---|---|---|---|
| sentry-rust | 2021 | 1.89 | CI matrix `[stable, 1.89.0]` on Linux/macOS/Windows (`sentry-rust/.github/workflows/check.yml:13-14`, `test.yml:13-14`); `clippy.toml` `msrv`; CHANGELOG has an MSRV section (`sentry-rust/CHANGELOG.md:5-7`) |
| opentelemetry-rust | 2021 | 1.75.0 | "current stable and the three most recent minor versions" (`opentelemetry-rust/README.md:144-155`); `cargo msrv verify` job (`.github/workflows/ci.yml:132-157`) |
| tracing | 2018 | 1.71.0 (appender 1.63.0) | "three most recent minor versions" (`tracing/README.md:260-272`); `cargo minimal-versions check --feature-powerset --no-dev-deps` on 1.71 and stable (`tracing/.github/workflows/CI.yml:146,166`) |
| dd-trace-rs / libdatadog | 2021 / 2024 | 1.87.0 | Floor set by Alpine and RHEL rustc (`libdatadog/Cargo.toml:102-107`); `rust-toolchain.toml` pins 1.87.0 |
| logfire-rust | 2024 | 1.91 | CI reads MSRV from Cargo.toml and adds it to the matrix (`logfire-rust/.github/workflows/main.yml:47-60`) |
| posthog-rs | 2018 | 1.78.0 | `cargo check --locked --lib` on MSRV (`posthog-rs/.github/workflows/ci.yaml:90-97`) |
| LaunchDarkly | 2021 | 1.97.0 | Daily bot sets MSRV to N-2 from endoflife.date and opens a "fix:" PR (`rust-server-sdk/.github/workflows/check-rust-versions.yml:4,19-30,47-75`); CI tests only the MSRV |
| axiom-rs | 2018 | 1.60 | False: reqwest 0.12 needs 1.64; never checked in CI |
| rmcp | 2024 | 1.88 | `resolver = "3"` (`rmcp/Cargo.toml:5,12-13`) |

- Dependency floors today: reqwest 0.13.5 → 1.85; tokio 1.54 → 1.85; thiserror 2.0.21 → 1.77; rmcp → 1.88.
- tokio policy, verbatim: "Tokio will keep a rolling MSRV ... policy of **at least** 6 months. When increasing the MSRV, the new Rust version must have been released at least six months ago. ... the MSRV is not increased automatically, and only as part of a minor release." (`~/.cargo/registry/.../tokio-1.54.0/README.md:190-195`).
- Cargo: raising the MSRV is a minor change (https://doc.rust-lang.org/cargo/reference/semver.html#env-new-rust); the MSRV-aware resolver is the default for edition 2024 (Rust 1.84 release notes).
- Current stable: 1.99.1 (2026-10-15).

Recommendation:

- Edition 2024 (same as rmcp; resolver 3 is implied for the package; set `resolver = "3"` explicitly in the virtual workspace root, as rmcp does).
- `rust-version` = rmcp's MSRV, today `1.88`. Policy text for the README: "The MSRV is never higher than the MSRV of the `rmcp` version we support, and never newer than a Rust release that is at least 6 months old. An MSRV increase is a minor release (0.x: the `y` in 0.y.z) and has its own CHANGELOG section."
- Enforce with a CI job that runs `cargo hack check --rust-version --workspace --all-targets` (Cargo CI guide) and the test suite on the MSRV toolchain, plus a `minimal-versions` job (tracing combines both). Set clippy `msrv` from `rust-version` (default behavior).
- Do not ship MSRV bumps as patches (LaunchDarkly does; tokio and Cargo treat them as minor).

Why: the SDK is useless without rmcp, so requiring a newer compiler than rmcp excludes real users for no gain, and requiring an older one buys nothing. The 6-month rule (tokio) is the ecosystem's most cited policy.

## 10. Topic 9 — Documentation

What the references do:

- Crate-level docs: OTel OTLP has a long `//!` guide with Quickstart, env var tables, feature flags, configuration reference (`opentelemetry-rust/opentelemetry-otlp/src/lib.rs:1-330`); posthog has a `no_run` example per feature variant (`posthog-rs/src/lib.rs:1-88`); logfire injects the current version into install snippets with `concat!(env!("CARGO_PKG_VERSION"))` (`logfire-rust/logfire/src/lib.rs:41,88`) and has a guide-only `usage` module under `#[cfg(any(docsrs, doctest))]` (`:120-121`).
- README handling:
  - `#![doc = include_str!("../README.md")]`: rmcp (`rmcp/crates/rmcp/src/lib.rs:3`), tracing-mock (`tracing/tracing-mock/src/lib.rs:1`).
  - README doctested without becoming the crate docs: `#[doc = include_str!("../README.md")] #[cfg(doctest)] pub struct ReadmeDoctests;` (`axiom-rs/src/lib.rs:61-63`; `tracing-axiom/src/lib.rs:49-51`).
  - README generated from lib.rs by cargo-readme and `README.tpl` (`sentry-rust/README.tpl`; release step `sentry-rust/.github/workflows/release.yml:34`).
- `missing_docs`: `#![deny(missing_docs)]` (LaunchDarkly `lib.rs:9-10`, dd-trace-rs `lib.rs:246`, axiom); `warn` via lints (sentry `Cargo.toml:34`, logfire `Cargo.toml:131`, OTel/tracing crate attrs). LaunchDarkly also denies `rustdoc::missing_crate_level_docs`.
- `deny(rustdoc::broken_intra_doc_links)` under docsrs: OTel (`opentelemetry-rust/opentelemetry-sdk/src/lib.rs:159`), tracing (`tracing/tracing/src/lib.rs:919`).
- Doc logos: sentry, OTel, tracing set `html_logo_url`/`html_favicon_url`.
- Examples: all good SDKs have `examples/`; LaunchDarkly and logfire use `required-features`; posthog does not (`posthog-rs/examples`, 6 examples).
- Doctests: OTel gates doctests with hidden `# #[cfg(...)]` lines (`opentelemetry-otlp/src/lib.rs:37`); sentry runs core doctests through the facade as a dev-dependency (`sentry-rust/sentry-core/Cargo.toml:43-46`). LaunchDarkly doctests need a turbofish because default type parameters do not drive inference (`processor_builders.rs:70-78`) — a sign that default generic parameters on public builders hurt ergonomics.
- Badges: sentry (build, codecov, crates.io, docs.rs), OTel (crates.io, docs, license, CI, codecov, Slack), tracing (crates.io, docs, license, build, Discord), axiom (docs.rs, build, crates.io, license).
- C-HTML-ROOT (`html_root_url`) is no longer in the API Guidelines checklist (https://rust-lang.github.io/api-guidelines/checklist.html).

Recommendation:

- Make `src/lib.rs` the canonical reference: what the SDK captures, a 10-line quickstart for a stdio rmcp server, a streamable-HTTP variant, the env var table, the feature table, shutdown semantics, the stdio/stderr warning, the MSRV policy, and links to examples. Use intra-doc links freely.
- Keep `README.md` short (badges, install, quickstart, links) and doctest it with the `#[cfg(doctest)] pub struct ReadmeDoctests;` trick (axiom). This avoids rustdoc rendering README HTML and broken relative links.
- Examples: `examples/stdio_server.rs`, `examples/streamable_http_server.rs`, `examples/custom_transport.rs`, `examples/exporters.rs`, each with `required-features` when needed. CI builds all examples.
- Every public item has docs; every public fn that can fail has `# Errors`; examples use `?`, not `unwrap()` (C-QUESTION-MARK, C-FAILURE). Gate with `missing_docs = "warn"` + `-D warnings` in CI, and `#![cfg_attr(docsrs, deny(rustdoc::broken_intra_doc_links))]`.
- README badges: crates.io version, docs.rs, CI status, MSRV (`rust-version`), license. Optional: codecov.
- Use the logfire `concat!(env!("CARGO_PKG_VERSION"))` trick for install snippets in crate docs.
- Avoid default type parameters on public builders (LaunchDarkly turbofish problem); use `Box<dyn Transport>`/`Arc<dyn Transport>` instead of a generic.

## 11. Topic 10 — Lints, safety, rustfmt

What the references do:

- Workspace lint tables:
  - sentry: clippy `arithmetic-side-effects`, `allow-attributes` (forces `#[expect(.., reason = ..)]`), `impl_trait_in_params`; rust `missing_docs` (`sentry-rust/Cargo.toml:28-35`).
  - OTel: `rust_2024_compatibility` and `clippy::all` (`opentelemetry-rust/Cargo.toml:92-100`).
  - libdatadog: clippy `all`, `pedantic`, `nursery`, `cargo` at warn, plus `panic`, `unwrap_used`, `expect_used`, `todo`, `unimplemented`, `unreachable`, `panic_in_result_fn`; rust `future_incompatible`, `nonstandard_style`, `rust_2018_idioms`, `unused` (`libdatadog/Cargo.toml:376-409`). Comment: "Using `warn` instead of `deny` ... Warnings fail CI anyway".
  - logfire: `print_stdout`, `print_stderr`, `dbg_macro`, `unwrap_used` deny; `pedantic` deny with `implicit_hasher` allowed; `missing_docs` warn (`logfire-rust/logfire/Cargo.toml:130-141`).
- Crate attribute blocks: tracing's famous list (`missing_debug_implementations, missing_docs, rust_2018_idioms, unreachable_pub, ... private_interfaces, private_bounds, ...`) at `tracing/tracing/src/lib.rs:925-945` and `tracing/tracing-subscriber/src/lib.rs:180-200`; OTel `future_incompatible, missing_debug_implementations, missing_docs, nonstandard_style, rust_2018_idioms, unreachable_pub, unused` plus `#![cfg_attr(test, deny(warnings))]` (`opentelemetry-rust/opentelemetry-sdk/src/lib.rs:149-163`).
- Anti-pattern: `#![deny(warnings)]` in source (axiom `axiom-rs/src/lib.rs:34`, tracing-axiom) breaks downstream builds when a new compiler adds a warning.
- `unsafe`: no main crate uses `#![forbid(unsafe_code)]`; sentry integrations use `#![deny(unsafe_code)]` (`sentry-rust/sentry-panic/src/lib.rs:19` and others). logfire has an `unsafe downcast_raw` in its tracing bridge (`logfire-rust/logfire/src/bridges/tracing.rs:169`). sentry's `release_name!` uses `static mut` + `transmute` (`sentry-rust/sentry-core/src/macros.rs:16-34`).
- clippy.toml: libdatadog `allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-panic-in-tests`, `max-struct-bools = 5`; dd-trace-rs `disallowed-methods` for env access; sentry `msrv`; tracing complexity thresholds.
- rustfmt: OTel `edition = "2021"` only; sentry none (nightly options commented out); dd-trace-rs/libdatadog use nightly-only options (`unstable_features = true`, `wrap_comments`, `format_code_in_doc_comments`; `dd-trace-rs/rustfmt.toml`), which forces nightly on contributors.

Recommendation:

- Use a `[workspace.lints]` table, not crate attributes (one place, applies to tests and examples, needs Rust 1.74+). Proposed table in section 18 (verified on 1.99.1 clippy and 1.88 check).
- `unsafe_code = "forbid"`. The SDK needs no unsafe, and forbidding it is cheap evidence of quality.
- clippy `pedantic` at warn with a short, commented allow list; the restriction lints `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`, `print_stdout`, `print_stderr` at deny; `allow_attributes` + `allow_attributes_without_reason` at warn so every exception is `#[expect(lint, reason = "...")]` (sentry).
- `clippy.toml`: `allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-panic-in-tests`, `allow-print-in-tests`, and `disallowed-methods` for `std::env::var*` outside the config module (dd-trace-rs).
- Fail CI on warnings with `RUSTFLAGS="-D warnings"` / `cargo clippy -- -D warnings`. Never put `deny(warnings)` in source.
- rustfmt: stable defaults only. With edition 2024 in `Cargo.toml`, no `rustfmt.toml` is needed; if you add one, use only stable keys.

## 12. Topic 11 — Testing

What the references do:

- Layout: unit tests inline (`#[cfg(test)]`), integration tests in `tests/` (all good SDKs). logfire shares test utilities with integration tests through `#[path = "../src/test_utils.rs"]` (`logfire-rust/logfire/tests/test_log_attributes.rs:6-7`).
- Mock HTTP:
  - wiremock: logfire (`logfire-rust/logfire/tests/test_http_sink.rs:4-5,20-28`), statsig, dd-trace-rs instrumentation.
  - httpmock: posthog (`posthog-rs/tests/test_transport.rs:59-73`), libdatadog (many crates), axiom.
  - mockito: LaunchDarkly (`.expect(2)` retry test, `sender.rs:307-312`), statsig.
  - Fake servers: OTel scripted fake OTLP server with retry tests for 503, 400, 429 + Retry-After (`opentelemetry-rust/opentelemetry-otlp/tests/integration_test/src/fake_otlp_http.rs:13-30`; `tests/http_retry.rs:181-219`); Rudder hand-rolled `TcpListener` (`rudder-sdk-rust/tests/retry.rs:54-63`).
  - sentry has no HTTP-level transport tests at all.
- In-process capture exposed to users: sentry `TestTransport` + `with_captured_events` behind `test` (`sentry-rust/sentry-core/src/test.rs:46-132`); OTel `InMemorySpanExporter` behind `testing` (`opentelemetry-rust/opentelemetry-sdk/src/trace/mod.rs:28-34`); LaunchDarkly `TestData`; libdatadog `MockHttpCapabilities` that records requests and works with paused time (`libdatadog/libdd-data-pipeline/tests/common/mock_http.rs:195-244`).
- Snapshots: logfire inline insta snapshots `assert_debug_snapshot!(x, @r#"..."#)` with deterministic id and timestamp generators (`logfire-rust/logfire/tests/test_http_sink.rs:72`; `logfire-rust/logfire/src/test_utils.rs:31-160`); libdatadog snapshot JSONs via the Datadog test agent in Docker (`libdatadog/libdd-trace-utils/src/test_utils/datadog_test_agent.rs:16,129-138`).
- Property and fuzz: proptest (dd-trace-rs, libdatadog, patched fork), bolero fuzzing in CI (`libdatadog/.github/workflows/fuzz.yml:76-85`).
- Cross-SDK contract harnesses: PostHog's `sdk-test-harness` drives an axum adapter (`posthog-rs/compliance/adapter/src/main.rs:464-471`; `.github/workflows/sdk-compliance-tests-v0.yml:28-35`); LaunchDarkly's contract-test service (`rust-server-sdk/contract-tests/src/main.rs:125-150`).
- Concurrency: no loom or shuttle anywhere; time is made deterministic with `tokio::time::pause` and injected clocks. sentry has `test_concurrent_init` and unwind tests (`sentry-rust/sentry/tests/test_client.rs:37-67`). OTel runs global-state tests one at a time with `--ignored --exact` (`opentelemetry-rust/scripts/test.sh:43-51`).
- Env-var pitfalls: tracing-axiom mutates env vars without a lock in parallel tests, and has `matches!(..)` statements without `assert!` that check nothing (`tracing-axiom/src/builder.rs:222-239,249-313`); logfire uses an `ENV_MUTEX` plus `unsafe set_var`.

Recommendation:

- Unit tests inline for pure logic (batching, retry classification, backoff math, `Retry-After` parsing, truncation, redaction).
- Worker tests: drive the runtime-agnostic worker `async fn` with an in-memory transport on `#[tokio::test(start_paused = true)]`, so batching intervals, backoff, and deadlines run in virtual time.
- HTTP tests: wiremock in `tests/` against the real reqwest transport, with `Mock::expect(n)` for retry counts, 429 + `Retry-After`, 5xx, 4xx-no-retry, 401 disable, timeouts (wiremock `set_delay`), gzip header and body. Pick wiremock: async-first on tokio and already used by logfire and statsig.
- Snapshot the exact wire JSON with insta (inline snapshots, redacted timestamps and ids, deterministic id generator behind a test hook). This is the cheapest guard against accidental wire-format changes.
- proptest for truncation and redaction invariants (UTF-8 boundaries, size limits) and for serialization round-trips.
- Expose `agentcat_sdk::testing` behind `test-util`: an in-memory transport that records batches, and a helper that builds a client wired to it. Users will want to assert that their MCP server emits events (sentry, OTel precedent).
- rmcp end-to-end tests: run an instrumented server and a client over `tokio::io::duplex` in one test, and assert captured events.
- Concurrency tests: many tasks capturing at once; shutdown racing capture; double shutdown; drop during shutdown; capture after shutdown (counted, no panic). Add compile-time Send/Sync assertions (section 17).
- Never mutate process env vars in tests; use the injectable env lookup. Always `assert!` on `matches!`.
- Owner decision: a cross-SDK compliance harness (PostHog, LaunchDarkly model) if AgentCat has or wants one for its TS, Python, PHP, and C# SDKs.

## 13. Topic 12 — CI and supply chain

What the references do (cited jobs):

| Check | Who | Evidence |
|---|---|---|
| fmt + clippy `-D warnings` | all serious repos | `sentry-rust/.github/workflows/lint.yml:21-25`; `posthog-rs/.github/workflows/ci.yaml:57` |
| cargo-hack feature matrix | tracing (`--feature-powerset`), OTel and dd-trace-rs (`--each-feature`) | `tracing/.github/workflows/CI.yml:95-101`; `opentelemetry-rust/scripts/lint.sh:15-18`; `dd-trace-rs/.github/workflows/lint.yaml:68` |
| Explicit feature combos | sentry, posthog (7 combos), LaunchDarkly (7) | `sentry-rust/.github/workflows/check.yml:31-50`; `posthog-rs/.github/workflows/ci.yaml:151-208` |
| MSRV job | sentry, OTel, tracing, logfire, posthog, dd | see section 9 |
| minimal-versions | tracing only | `tracing/.github/workflows/CI.yml:166` |
| Cross-OS | sentry (Linux/macOS/Windows), OTel, tracing, libdatadog, dd-trace-rs | `sentry-rust/.github/workflows/test.yml:13-14`; `opentelemetry-rust/.github/workflows/ci.yml:18-35` |
| beta/nightly | sentry weekly, OTel beta, tracing beta+nightly | `sentry-rust/.github/workflows/weekly.yml:15-31` |
| docs build | sentry, OTel, LaunchDarkly (cargo-docs-rs) | `sentry-rust/.github/workflows/doc.yml:24-25` |
| cargo-deny | OTel (advisories, licenses, bans, sources), dd-trace-rs, libdatadog bans | `opentelemetry-rust/.github/workflows/ci.yml:158-187`; `opentelemetry-rust/deny.toml` |
| advisories only | sentry weekly `rustsec/audit-check`, tracing nightly cargo-audit | `sentry-rust/.github/workflows/weekly.yml:41`; `tracing/.github/workflows/audit.yml` |
| cargo-semver-checks | OTel (label-gated), libdatadog (PR title must match semver level) | `opentelemetry-rust/.github/workflows/semver.yml:13,26-27`; `libdatadog/.github/workflows/pr-title-semver-check.yml:156-159,267-292` |
| Public API snapshot | posthog `cargo-public-api` vs committed `api/public-api.txt`; libdatadog | `posthog-rs/.github/workflows/ci.yaml:59-73`; `posthog-rs/scripts/check-public-api.sh` |
| External types allowlist | OTel `cargo check-external-types` | `opentelemetry-rust/.github/workflows/ci.yml:104-131`; `opentelemetry-rust/opentelemetry-otlp/allowed-external-types.toml` |
| Unused deps | OTel cargo-shear, libdatadog cargo-machete | `opentelemetry-rust/.github/workflows/ci.yml:268-291`; `libdatadog/.github/workflows/cargo-machete.yml` |
| Coverage | sentry, OTel, logfire, libdatadog (cargo-llvm-cov) | `sentry-rust/.github/workflows/ci.yml:43` |
| miri | OTel smoke, libdatadog partitions | `opentelemetry-rust/.github/workflows/ci.yml:61-78`; `libdatadog/.github/workflows/miri.yml:96` |
| Crypto provider check | logfire, libdatadog | `logfire-rust/.github/workflows/main.yml:79-91` |
| Publish dry run | posthog | `posthog-rs/.github/workflows/ci.yaml:125-149` |
| Workflow security | OTel zizmor + OSSF scorecard; sentry and Rudder pin actions by SHA; Rudder harden-runner | `opentelemetry-rust/.github/workflows/zizmor.yml`; `rudder-sdk-rust/.github/workflows/ci.yml:18` |
| Aggregate gate job | logfire alls-green, sentry "Check required jobs", libdatadog all-checks | `logfire-rust/.github/workflows/main.yml:107-115`; `sentry-rust/.github/workflows/ci.yml:55-70` |
| Workspace dependency inheritance | sentry cargo-autoinherit | `sentry-rust/.github/workflows/lint.yml:42-50` |
| cargo-mutants | none | — |

Recommendation: see the CI gate list in section 18. Skip miri (no unsafe). Add cargo-mutants as an optional weekly or `--in-diff` job; no reference uses it, so it is an extra, not a norm.

## 14. Topic 13 — Release and versioning

What the references do:

- Tooling: sentry uses getsentry Craft (`sentry-rust/.craft.yml`); posthog uses Sampo changesets with a Slack approval (`posthog-rs/.github/workflows/release.yml:44-115`); LaunchDarkly and Rudder use release-please (`rust-server-sdk/release-please-config.json:2-6`); dd-trace-rs uses cargo-release + git-cliff release-proposal PRs (`dd-trace-rs/.github/workflows/release-proposal-dispatch.yaml:74-100`; `dd-trace-rs/cliff.toml:21-38`); logfire uses `release.sh` that checks lockstep versions against the tag (`logfire-rust/release.sh:36-57`); OTel uses scripts with per-crate tags `<crate>-<X.Y.Z>` (`opentelemetry-rust/scripts/RELEASING.md:46-54`). No reference uses release-plz.
- Trusted publishing (OIDC, `rust-lang/crates-io-auth-action`): posthog (`release.yml:61,150-153`, environment "Release"), logfire (`main.yml:120-136`, environment `release`), Rudder (`publish.yml:22,44`). Long-lived tokens: dd-trace-rs (`publish.yaml`), LaunchDarkly (token from AWS SSM), statsig, axiom, tracing-axiom.
- Changelogs: sentry has `## Unreleased` with Features/Fixes/Deprecations and a dedicated "Minimum Supported Rust Version" subsection (`sentry-rust/CHANGELOG.md:3-7`); OTel per-crate `## vNext` with before/after code for breaking changes (`opentelemetry-rust/opentelemetry-otlp/CHANGELOG.md:1-30`); release-please default format (LaunchDarkly); git-cliff (dd-trace-rs).
- Conventional-commit PR titles: dd-trace-rs, libdatadog (`pr-name.yaml`), LaunchDarkly (`lint-pr-title.yml`). libdatadog checks that the PR title's semver level matches cargo-semver-checks output.
- Semver discipline: OTel `VERSIONING.md:8-33` (new signals behind experimental features, stabilized in a minor); LaunchDarkly ships MSRV bumps as patches (criticized above); tags: `v{version}` (logfire, Rudder with `validate-release-tag.sh`), bare `3.3.2` (LaunchDarkly), `{crate}-v{version}` (dd-trace-rs, OTel).
- Web facts: trusted publishing launched July 2025 (RFC 3691); since January 2026 crates.io supports a "Trusted Publishing Only" mode and blocks `pull_request_target`/`workflow_run` triggers (https://blog.rust-lang.org/2026/01/21/crates-io-development-update). The first publish of a new crate needs a token. release-plz supports trusted publishing natively, runs cargo-semver-checks, and writes Keep a Changelog via git-cliff (https://release-plz.dev/docs).

Recommendation:

- Use release-plz: a release PR on every push to `main`, version bumps from conventional commits, changelog via git-cliff in Keep a Changelog format, cargo-semver-checks before release, trusted publishing (`id-token: write`, no token secret), GitHub release and tag.
- Publish from a job that runs in a protected `release` environment with required reviewers (posthog, logfire). After the first manual publish, turn on "Trusted Publishing Only" on crates.io.
- Tags `v0.1.0` (single crate). If the workspace later publishes more crates, switch to `agentcat-sdk-v0.1.0`.
- CHANGELOG: Keep a Changelog 1.1.0 (`Unreleased`, Added/Changed/Deprecated/Removed/Fixed/Security), plus an "MSRV" note when it changes (sentry) and before/after snippets for breaking changes (OTel).
- Semver pre-1.0: treat `0.y` bumps as breaking and `0.y.z` as compatible (Cargo SemVer guide `#change-categories`). Breaking changes include adding a public field to a struct without private fields, adding a variant to an exhaustive enum, removing a feature, raising the MSRV only as a minor.
- PR titles in conventional-commit form, checked in CI (needed for release-plz).
- Owner decision: if AgentCat already standardizes on release-please across SDKs, use it instead (LaunchDarkly/Rudder precedent) and add the semver-checks job yourself.

## 15. Topic 14 — Metadata and packaging

What the references do:

- Licenses: MIT (sentry, tracing, posthog, logfire, Segment, Rudder); Apache-2.0 (OTel, dd-trace-rs, libdatadog, LaunchDarkly, rmcp); `MIT OR Apache-2.0` (axiom-rs, tracing-axiom); ISC (statsig). Datadog adds NOTICE files, SPDX headers checked in CI, and a `LICENSE-3rdparty.csv` regenerated and diffed in CI (`libdatadog/.github/workflows/lint.yml:331-384`).
- Manifest fields: the most complete are axiom-rs (license, description, homepage, repository, documentation, keywords, readme, `include = ["src/**/*.rs", "README.md", "LICENSE-APACHE", "LICENSE-MIT"]`, `axiom-rs/Cargo.toml:3-15`) and opentelemetry-otlp (categories `development-tools::debugging`, `development-tools::profiling`, `asynchronous`; keywords; `opentelemetry-rust/opentelemetry-otlp/Cargo.toml:8-13`). posthog lacks keywords, categories, documentation, homepage, readme, include (`posthog-rs/Cargo.toml:1-9`). libdatadog allows `clippy::cargo_common_metadata` with a TODO (`libdatadog/Cargo.toml:393-394`). LaunchDarkly uses `exclude = ["coverage.sh"]`.
- rmcp: `keywords = ["mcp", "sdk", "tokio", "modelcontextprotocol"]`, `categories = ["network-programming", "asynchronous"]` (`rmcp/Cargo.toml:19-24`).
- C-METADATA: set description, license, repository, keywords, categories; set `documentation` only if docs are not on docs.rs (crates.io links docs.rs automatically); set `homepage` only for a dedicated site.

Recommendation (draft):

```toml
[package]
name = "agentcat-sdk"
version = "0.1.0"
edition = "2024"
rust-version = "1.88"
description = "Usage analytics for MCP servers built on rmcp: captures tool calls, sessions, and intent, and sends them to AgentCat in the background."
license = "MIT OR Apache-2.0"          # owner decision
repository = "https://github.com/<org>/agentcat-rust-sdk"
homepage = "https://<agentcat site>"
readme = "README.md"
keywords = ["mcp", "rmcp", "analytics", "telemetry", "observability"]   # max 5
categories = ["development-tools::debugging", "development-tools::profiling", "asynchronous", "network-programming"]
include = ["src/**/*.rs", "examples/**/*.rs", "README.md", "CHANGELOG.md", "LICENSE*"]
```

- Omit `documentation` (docs.rs is automatic). Keep tests and CI files out of the package with `include` (axiom). Run `cargo package --list` in CI to review contents.
- License: match AgentCat's other SDKs. If there is no constraint, `MIT OR Apache-2.0` is the Rust ecosystem norm and satisfies C-PERMISSIVE.
- Add SPDX headers only if the license policy requires it (Datadog does).

## 16. Topic 15 — Public API stability

What the references do:

- Sealed traits: OTel `WithExportConfig: sealed::WithExportConfig` (`opentelemetry-rust/opentelemetry-otlp/src/exporter/mod.rs:62-70,371`); tracing `Value: sealed::Sealed` (`tracing/tracing-core/src/lib.rs:306-308`; `field.rs:350`); sentry `IntoProtocolMetric`, `IndexedEnum`, `LossSource`; libdatadog `ResultInfallibleExt`. LaunchDarkly seals by accident: re-exported traits take types from private modules, so users cannot implement them even though docs say they can (`rust-server-sdk/launchdarkly-server-sdk/src/lib.rs:40,68`; `config.rs:284`).
- Private-inner newtypes instead of `#[non_exhaustive]`: tracing `Level(LevelInner)`, `Interest(InterestKind)` (`tracing/tracing-core/src/metadata.rs:221,587`; `subscriber.rs:589-592`). `#[non_exhaustive]` widely: sentry `ClientOptions`, `TransportOptions`, enums; OTel 32 uses on data enums and structs.
- Evolution with deprecation shims: sentry `#[deprecated]` methods kept for a release (`sentry-rust/sentry-core/src/transport/mod.rs:72`).
- `impl Trait` returns: OTel `fn export(..) -> impl Future<Output = OTelSdkResult> + Send` (`opentelemetry-rust/opentelemetry-sdk/src/trace/export.rs:30-33`); logfire `impl SpanExporter + use<>` (`logfire-rust/logfire/src/exporters.rs:96-99`).
- Dependency type leakage:
  - Gated: OTel allowlists external types and checks them in CI (`opentelemetry-rust/opentelemetry-otlp/allowed-external-types.toml`); the OTLP exporter takes a generic `HttpClient` instead of reqwest. posthog snapshots the whole public API.
  - Leaky: posthog `reqwest::Client`, `reqwest::blocking::Client`, `uuid::Uuid`, `chrono::DateTime`, `serde_json::Value` (`posthog-rs/api/public-api.txt:156,167,114`; `posthog-rs/src/event.rs:154,190`); sentry `pub use debugid::*`, `uuid`, `url`, serde_json re-exports, `reqwest::Client` in options (`sentry-rust/sentry-types/src/lib.rs:55-56`); statsig tokio `Handle`, `task::Id`, `Notify` (`statsig-server-core/statsig-rust/src/statsig_runtime.rs:41,88-91`); dd-trace-rs pins `opentelemetry = "=0.33.0"` and returns `SdkTracerProvider` (`dd-trace-rs/Cargo.toml:46-60`).
- Re-exporting dependencies: sentry `pub use sentry_types as types` (`sentry-rust/sentry-core/src/lib.rs:170`); OTel `pub mod tonic_types` (`opentelemetry-otlp/src/lib.rs:873`); opentelemetry-http re-exports `bytes::Bytes` and `http::{Request, Response}` (`opentelemetry-rust/opentelemetry-http/src/lib.rs:68,70`); axiom re-exports `http::HeaderMap` (`axiom-rs/src/lib.rs:55-58`).
- C-STABLE: a 1.0 crate must only expose 1.0 public dependencies (https://rust-lang.github.io/api-guidelines/necessities.html).

Recommendation:

- Allowed external types in the public API: `rmcp` types (unavoidable for the integration), `http` 1.x types (only if the `Transport` trait uses them), `serde::Serialize` as a bound, `std` types. Forbidden: `reqwest`, `tokio`, `hyper`, `chrono`, `time`, `uuid`, `url`, `serde_json::Value` in signatures (use `impl Serialize` inputs and your own newtypes such as `EventId`, `SessionId`, and `std::time::SystemTime`).
- Enforce with `cargo check-external-types` and a committed allowlist (OTel), plus a committed `cargo public-api` snapshot (posthog) so every API change appears in PR diffs.
- `#[non_exhaustive]` on every public enum and on any struct with public fields; decide before the first release, because adding it later is breaking (Cargo SemVer guide `#attr-adding-non-exhaustive`). Prefer private fields with accessors (C-STRUCT-PRIVATE).
- Seal traits that users consume but must not implement (for example an `IntoEventProperties` conversion trait). Keep `Transport` open and add new methods only with default bodies.
- Use `#[doc(hidden)]` only for macro support items; never as a substitute for `pub(crate)`.
- Return `impl Future`/`impl Iterator` where the concrete type is an implementation detail.
- Use `#[deprecated(since = "...", note = "use X")]` for one minor release before removal (sentry).
- Do not re-export `rmcp`. Users depend on rmcp directly and Cargo unifies the semver-compatible version. Document the supported rmcp major in the README and treat an rmcp major bump as a `0.y` bump of `agentcat-sdk`.
- Implement `Debug` for every public type (C-DEBUG); write a manual `Debug` for anything that holds the API key and redact it.
- Avoid default type parameters on public types (LaunchDarkly turbofish problem); prefer trait objects for the transport.

## 17. Topic 16 — Send, Sync, 'static, and compile-time assertions

What the references do:

- Trait bounds: sentry `Transport: Send + Sync + 'static`, `TransportFactory: Send + Sync`, `Integration: Sync + Send + Any` (`sentry-rust/sentry-core/src/transport/mod.rs:15`; `integration.rs:55`); OTel `SpanExporter: Send + Sync + Debug`, `PushMetricExporter: Send + Sync + 'static`, `HttpClient: Debug + Send + Sync` (`opentelemetry-rust/opentelemetry-sdk/src/trace/export.rs:17`; `metrics/exporter.rs:13`); LaunchDarkly `HttpTransport: Send + Sync + Clone + 'static`; OTel `Runtime: Clone + Send + Sync + 'static` with `fn delay(..) -> impl Future + Send` (`opentelemetry-rust/opentelemetry-sdk/src/runtime.rs:23-41`).
- Callbacks: sentry `Arc<dyn Fn(T) -> Option<T> + Send + Sync>` (`sentry-rust/sentry-core/src/clientoptions.rs:14`); posthog hooks `Fn + Send + Sync`; libdatadog `Box<dyn Fn + Send + Sync>` (`libdatadog/libdd-data-pipeline/src/trace_buffer/mod.rs:321`).
- Assertions: sentry `compile_fail` doctest proving `SwitchGuard: !Send` (`sentry-rust/sentry-core/src/hub_impl.rs:37-50`) and `fn assert_sync<T: Sync>()` in sentry-tower (`sentry-rust/sentry-tower/src/lib.rs:342-351`); LaunchDarkly test asserts `Client: Send + Sync` (`rust-server-sdk/launchdarkly-server-sdk/src/client.rs:953-957`); libdd-telemetry test-time `is_send`/`is_sync` (`libdatadog/libdd-telemetry/src/worker/mod.rs:1582-1591`); `static_assertions` only in libdd-common. sentry manually implements `RefUnwindSafe for Client` (`sentry-rust/sentry-core/src/client/mod.rs:621`).
- C-SEND-SYNC: types are Send and Sync where possible (https://rust-lang.github.io/api-guidelines/interoperability.html).

Recommendation:

- `Client`, `ShutdownGuard`, the builder, errors, and all event types: `Send + Sync + 'static`. `Client: Clone + Debug`.
- `Transport: Send + Sync + 'static` and returned futures are `Send`. Callbacks are `Arc<dyn Fn(..) + Send + Sync + 'static>`.
- Assert at compile time with no dependency (verified on 1.88 and 1.99.1):

```rust
const _: () = {
    const fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_send_sync::<Client>();
    assert_send_sync::<ShutdownGuard>();
    assert_send_sync::<Error>();
};
```

- Also assert `UnwindSafe`/`RefUnwindSafe` for `Client` if the inner design allows it, so users can capture events across `catch_unwind`.
- Use `compile_fail` doctests for any intentional `!Send` type (sentry).

## 18. Where the references disagree, and the pick

| Question | Options seen | Pick for agentcat-sdk | Reason |
|---|---|---|---|
| Crate topology | Facade/core/types (sentry, OTel, libdatadog) vs one crate (posthog, LaunchDarkly, axiom, dd-trace-rs) | One published crate in a workspace | No ecosystem of integrations or library authors to serve; one version and one docs page |
| Init shape | `init() -> Guard` (sentry) vs builder → handle (OTel, logfire, LaunchDarkly, axiom) vs free fn (posthog) | Builder → cloneable `Client` + `shutdown_guard()` + async `shutdown()` | Explicit, testable, no globals; guard covers forgetful users |
| Options | Pub fields + non_exhaustive + setters (sentry) vs private + builder (OTel, logfire, posthog) | Private fields + hand-written builder | Every new option is a minor change; no derive_builder leak |
| Global state | Hub/OnceLock (sentry, posthog, statsig) vs explicit (OTel guidance, dd `init_local`) | Explicit only in 0.1 | OTel: libraries must not set globals; additive later |
| Runtime model | Own thread + own runtime (sentry, logfire, LaunchDarkly, dd) vs own thread + blocking HTTP (posthog, OTel BSP) vs host runtime (statsig, tracing-axiom) | Own named thread + current-thread tokio runtime | Works in and out of tokio, bounded sync Drop, no `block_on` panics |
| Queue overflow | Drop newest (sentry, OTel, posthog, LaunchDarkly, libdatadog) vs drop oldest (statsig) | Drop newest, count, warn once + total | Majority; no lock contention on the hot path |
| Retry 4xx | Retry all 4xx (libdatadog) vs none except 408/429 (OTel, posthog, Rudder) vs disable on 401/403 silently (LaunchDarkly) | Retry 408/429/5xx/network only; disable on 401/403 with `error!` | Avoid retry storms and silent data loss |
| 429 | Retry with Retry-After (OTel, posthog, Rudder) vs terminal (statsig) | Retry, honor Retry-After clamped to max | Rate limits are temporary |
| HTTP stack | reqwest (most) vs hyper direct (libdatadog, LaunchDarkly) | reqwest 0.13, default features off | Same stack as rmcp; less code |
| Errors | thiserror (logfire, otlp, LaunchDarkly, rmcp) vs hand-written (posthog, OTel SDK, libdatadog) | thiserror 2 | Already in every rmcp build |
| Internal logs | tracing (posthog, libdatadog, OTel, rmcp) vs log (LaunchDarkly, statsig) vs eprintln flag (sentry) vs stdout fallback (dd) | tracing; deny print lints | rmcp users already run a tracing subscriber; stdout safety |
| Mock HTTP | wiremock (logfire, statsig) vs httpmock (posthog, libdatadog, axiom) vs mockito (LaunchDarkly) | wiremock | Async-first on tokio |
| MSRV policy | N-3 (OTel, tracing), N-2 bot (LaunchDarkly), 6 months (tokio), distro floor (libdatadog), recent (logfire 1.91) | = rmcp MSRV (1.88), never < 6 months old, bump in minor | Never exclude a working rmcp user |
| Lint strictness | clippy::all (sentry, OTel) vs pedantic+nursery+cargo warn (libdatadog) vs pedantic deny (logfire) vs `deny(warnings)` in source (axiom) | pedantic warn + no-panic/no-print deny + `-D warnings` in CI | Strict but does not break downstream builds |
| rustfmt | Nightly options (Datadog) vs stable defaults (OTel, sentry) | Stable defaults | No nightly for contributors |
| README | include_str as crate docs (rmcp, tracing-mock) vs cfg(doctest) only (axiom) vs generated (sentry) | lib.rs canonical + README doctested via cfg(doctest) | Intra-doc links in rustdoc, clean GitHub README, README still tested |
| Test features | `test` (sentry), `testing` (OTel), `test-utils` (libdatadog), `test-harness` (posthog) | `test-util` | Same name as tokio |
| Release tool | Craft, Sampo, release-please, cargo-release+git-cliff, scripts | release-plz (or release-please if AgentCat standardizes on it) | semver-checks + git-cliff + trusted publishing built in |
| Publishing auth | OIDC (posthog, logfire, Rudder) vs long-lived tokens (dd, LaunchDarkly, statsig, axiom) | OIDC trusted publishing + protected environment | No long-lived secret |
| License | MIT, Apache-2.0, dual, ISC | Owner decision; default suggestion dual MIT/Apache-2.0 | Ecosystem norm |

Anti-patterns to avoid (each seen in at least one reference): non-additive features (posthog, statsig, axiom); `compile_error!` for feature combos (axiom); unbounded HTTP and shutdown (sentry, LaunchDarkly); `join().unwrap()` in Drop (sentry, OTel); silent permanent disable (LaunchDarkly); blocking drain in Drop inside async code without a bound (posthog); printing to stdout (dd-trace-rs, libdatadog); installing a global logger (statsig); public fields without `#[non_exhaustive]` (statsig); error types without `std::error::Error` (statsig); dependency types in signatures (posthog, sentry, axiom, statsig); `deny(warnings)` in source (axiom); false MSRV never checked (axiom, tracing-axiom); `matches!` without `assert!` (tracing-axiom); long-lived publish tokens (several).

## 19. Choices the owner must make

1. Runtime model: confirm the dedicated worker thread with its own current-thread runtime (recommended) versus a task on the host runtime.
2. Shutdown UX: default shutdown timeout (sentry 2 s, OTel 5 s, posthog 30 s; recommend 2–5 s) and whether the guard's `Drop` may block for that long.
3. Global client: explicit handle only (recommended) or an optional `OnceLock` global.
4. MSRV: 1.88 tied to rmcp (recommended), 1.95 (6-month rule), or 1.97 (N-2).
5. License: match other AgentCat SDKs, or dual MIT/Apache-2.0.
6. Default TLS: rustls with aws-lc-rs (reqwest default; C build and an "OpenSSL"-licensed component in aws-lc-sys), `rustls-no-provider` (app installs ring or aws-lc), or native-tls.
7. API facts the SDK needs: does the AgentCat API accept gzip (or zstd)? an idempotency key header? `Retry-After` on 429/503? maximum batch size and 413 behavior? which statuses are permanent?
8. Release tooling: release-plz (recommended) or release-please for cross-SDK consistency; conventional-commit PR titles required.
9. Crate topology: one crate (recommended) or a separate models crate.
10. Exporters: always compiled and selected at runtime (recommended if hand-written) or feature-gated; hand-written OTLP JSON versus the `opentelemetry` crates (0.x, breaking every few months, and their types would leak).
11. rmcp version policy: support one rmcp major at a time and bump `0.y` on an rmcp major (recommended).
12. Delivery observability: expose a stats snapshot and/or `on_error`/`on_drop` callbacks.
13. Env var names and kill switch semantics (`AGENTCAT_*`), consistent with other AgentCat SDKs.
14. Test surface: ship `test-util`; build a cross-SDK compliance harness or not.
15. Crate naming: `agentcat-sdk` (import `agentcat_sdk`); also reserve `agentcat` on crates.io.

## 20. Proposed agentcat-sdk style guide

### 20.1 Rules

Layout and API

1. Publish one crate, `agentcat-sdk`, from a Cargo workspace with `[workspace.package]`, `[workspace.dependencies]` (all with `default-features = false`), and `[workspace.lints]`. — sentry `Cargo.toml:21-35`; libdatadog `lint.yml:198-222`.
2. Keep every module private; expose the API through one curated `pub use` block at the crate root, marked as the public interface. — posthog `src/lib.rs:104-167`; LaunchDarkly `lib.rs:20-79`; otlp `lib.rs:656-741`.
3. Do not add a prelude. — none of the 13 references has one.
4. Build the client with a hand-written builder: private fields, consuming `#[must_use]` setters, `build() -> Result<Client, Error>`. — logfire `config.rs:25-26`; OTel `span_processor.rs:1029-1067`; C-BUILDER.
5. Make `Client` a cheap `Clone` over `Arc<Inner>`. — OTel `provider.rs:157-160`; logfire `logfire.rs:64-65`.
6. Return a disabled client (and log one `warn!`) when the key is missing or the kill switch is set; return `Err` only for programmer errors. — posthog `client/mod.rs:361-366`; sentry `client/mod.rs:162-170`; dd-trace-rs `lib.rs:557-620`.
7. Do not create a global client in 0.1. — OTel `global/trace.rs:423-440`.
8. Resolve configuration as code > `AGENTCAT_*` env > default; read env vars in one module only and ban `std::env::var*` elsewhere with clippy `disallowed-methods`; make the env lookup injectable for tests. — OTel `CONTRIBUTING.md:300`; dd-trace-rs `clippy.toml:4-11`; logfire `internal/env.rs:8-23`.
9. Export env var names as `pub const` and document them in a crate-level table. — OTel `exporter/mod.rs:38`.

Runtime and delivery

10. Run delivery on one named OS thread that owns a current-thread tokio runtime and the HTTP client; never use the host runtime, never `block_on` on a host thread, never drop a `Runtime` on a host thread. — sentry `tokio_thread.rs:130-137`; logfire `logfire.rs:761-794`; dd-trace-rs `span_exporter.rs:161-185`; tokio `Runtime::block_on` docs.
11. Make `capture()` synchronous and non-blocking: `try_send` into a bounded queue (bounded by events and bytes); on overflow drop the new event, count it, warn on the first drop, and log the total at shutdown. — OTel `span_processor.rs:862-873,930-939`; libdatadog `trace_buffer/mod.rs:156-165,245-261`.
12. Send `Flush` and `Shutdown` over a separate control channel with oneshot acks. — OTel `span_processor.rs:520-521,892-905`.
13. Write the worker as a runtime-agnostic `async fn` with injected transport and clock; test it on a paused tokio clock. — libdatadog `libdd-shared-runtime/src/worker.rs:19-43`; posthog `transport.rs:83`.
14. Provide `async fn flush(timeout)` and `async fn shutdown(timeout)`; make shutdown idempotent with `compare_exchange`; spend one deadline across drain, send, and stop. — OTel `provider.rs:246-270`; dd-trace-rs `span_processor.rs:692-739`.
15. Provide a `#[must_use] ShutdownGuard` whose `Drop` runs a bounded blocking shutdown (default 2–5 s) unless shutdown already ran; the last `Inner` drop does the same as a safety net. — sentry `init.rs:13-43`; tracing-appender `non_blocking.rs:282-301`; OTel `provider.rs:137-148`.
16. Never `join().unwrap()`; wait with `recv_timeout` on an ack and detach on timeout. — counter-examples sentry `thread.rs:207-215`, OTel `span_processor.rs:950`.

HTTP

17. Use `reqwest` 0.13 with `default-features = false`; serialize bodies yourself. — sentry `Cargo.toml:72`; rmcp `crates/rmcp/Cargo.toml:78`.
18. Put a connect timeout and a request timeout on every request. — counter-examples sentry reqwest transport; LaunchDarkly `transport_hyper.rs:516`.
19. Retry network errors, 408, 429, and 5xx with exponential backoff and full jitter; honor `Retry-After` clamped to the policy maximum; bound all retries by the active deadline; do not retry other 4xx; on 401/403 disable delivery and log `error!`. — OTel `retry_policy.rs:36-42`, `retry_classification.rs:33-82`, `retry.rs:102-161`; posthog `retry.rs:103-115`; Rudder `retry.rs:27-70`.
20. Use `Instant`, never `SystemTime`, for backoff and rate-limit windows. — counter-example sentry `ratelimit.rs:13-19`.
21. Define a dyn-compatible `Transport: Send + Sync + 'static` trait with a boxed `Send` future and SDK-owned (or `http` 1.x) types; never accept or return `reqwest` types. — OTel `opentelemetry-http/src/lib.rs:137-147`; LaunchDarkly transport `transport.rs:24,132`.
22. Send a `User-Agent` with the SDK version, reuse one idempotency key per batch across retries if the API supports it, cap response body reads, and log response bodies only at `debug!`. — logfire `exporters.rs:41-60`; LaunchDarkly `sender.rs:93,141`; OTel `opentelemetry-http/src/lib.rs:150`, `exporter/http/mod.rs:542-552`.

Errors, panics, logging

23. Use `thiserror` 2; mark every public error enum `#[non_exhaustive]`; keep variants identical across features; wrap foreign errors as `Box<dyn Error + Send + Sync>`; write lowercase `Display` text without trailing punctuation. — logfire `lib.rs:138-213`; C-GOOD-ERR; counter-example axiom `error.rs:86-89`.
24. Never surface delivery failures through `capture()`; report them via `tracing`, counters, and optional callbacks; wrap user callbacks in `catch_unwind`. — posthog `client/mod.rs:432-440`, `common.rs:66,88`.
25. Deny `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` in library code; allow them in tests via `clippy.toml`. — libdatadog `Cargo.toml:384-390`, `clippy.toml`; logfire `Cargo.toml:137`.
26. Log with `tracing` only; never install a subscriber or logger; deny `print_stdout`, `print_stderr`, `dbg_macro`. — logfire `Cargo.toml:134-135`; counter-examples dd-trace-rs `core/log.rs:304-310`, statsig `output_logger.rs:128-133`.
27. Follow the level map: `error!` permanent failures and shutdown timeouts; `warn!` first drop, drop totals, ignored config, disabled client; `info!` once at init; `debug!` per batch; `trace!` sizes. Never log payloads or keys above `trace!`. — OTel internal logging levels.

Features and docs

28. Keep features additive and few: `rustls` (default), `rustls-no-provider`, `native-tls`, `gzip`, `test-util`; gate only code that adds dependencies; never use `compile_error!` for combinations. — sentry `Cargo.toml:62-68`; Cargo features reference; counter-examples posthog `client/mod.rs:49-67`, axiom `lib.rs:65-75`.
29. Configure docs.rs with `all-features = true` and `--cfg docsrs`, and use `#![cfg_attr(docsrs, feature(doc_cfg))]` without manual `doc(cfg)`. — OTel `opentelemetry/Cargo.toml:20-22`; rust-lang/rust#138907 (verified locally).
30. Make `src/lib.rs` the canonical guide (quickstart, env table, feature table, shutdown, stdio note, MSRV policy); doctest the README with `#[cfg(doctest)]`. — otlp `lib.rs:1-330`; axiom `lib.rs:61-63`.
31. Document every public item, add `# Errors` sections, and use `?` in examples. — LaunchDarkly `lib.rs:9-10`; C-FAILURE; C-QUESTION-MARK.
32. Ship runnable examples for stdio and streamable-HTTP rmcp servers and a custom transport, with `required-features` where needed. — LaunchDarkly `examples/custom_transport.rs`.

Lints and stability

33. Forbid `unsafe_code`; warn on `missing_docs`, `missing_debug_implementations`, `unreachable_pub`, `rust_2018_idioms`; enable clippy `pedantic` with a commented allow list; require `#[expect(.., reason = ..)]` for exceptions. — tracing `lib.rs:925-945`; sentry `Cargo.toml:28-35`; logfire `Cargo.toml:133-141`.
34. Never write `#![deny(warnings)]` in source; deny warnings in CI only. — counter-example axiom `lib.rs:34`.
35. Use stable rustfmt defaults. — OTel `rustfmt.toml`; counter-example Datadog nightly options.
36. Keep `reqwest`, `tokio`, `hyper`, `chrono`, `uuid`, `url`, and `serde_json::Value` out of public signatures; allow only `rmcp`, `http` 1.x, `serde` bounds, and `std`. Enforce with `cargo check-external-types` and a committed `cargo public-api` snapshot. — OTel `ci.yml:104-131`, `allowed-external-types.toml`; posthog `ci.yaml:59-73`; C-STABLE.
37. Put `#[non_exhaustive]` on every public enum and every struct with public fields before the first release; seal traits users must not implement; use `#[doc(hidden)]` only for macro support. — Cargo SemVer guide; OTel `exporter/mod.rs:62-70`; tracing `tracing-core/src/lib.rs:306-308`.
38. Do not re-export `rmcp`; document the supported rmcp major and bump `0.y` when it changes. — C-STABLE; rmcp is at 3.6.
39. Implement `Debug` for all public types and redact the API key. — C-DEBUG.
40. Assert `Send + Sync + 'static` for `Client`, `ShutdownGuard`, and `Error` at compile time with a `const` block. — LaunchDarkly `client.rs:953-957`; sentry `hub_impl.rs:37-50`; verified on 1.88.

Versioning and release

41. Use edition 2024 and `rust-version` equal to rmcp's MSRV (1.88 today); never require a Rust release younger than 6 months; raise the MSRV only in a `0.y` release with a CHANGELOG note. — rmcp `Cargo.toml:12-13`; tokio README; sentry `CHANGELOG.md:5-7`; Cargo SemVer `#env-new-rust`.
42. Keep a Changelog 1.1.0 with an `Unreleased` section; show before/after code for breaking changes. — keepachangelog.com; OTel otlp `CHANGELOG.md:1-30`.
43. Release with release-plz from conventional commits, with cargo-semver-checks, publish via crates.io trusted publishing from a protected `release` environment, tag `v{version}`, then enable "Trusted Publishing Only". — release-plz docs; posthog `release.yml:61,150-153`; logfire `main.yml:120-136`.
44. Fill `description`, `license`, `repository`, `homepage`, `readme`, `keywords` (5), `categories`, `rust-version`, and an `include` list; omit `documentation`. — axiom `Cargo.toml:3-15`; otlp `Cargo.toml:8-13`; C-METADATA.

Testing

45. Test HTTP behavior with wiremock against the real transport (retries, 429 + Retry-After, 4xx, 401, timeouts, gzip). — logfire `tests/test_http_sink.rs`; OTel `tests/http_retry.rs:181-219`.
46. Snapshot the wire JSON with inline insta snapshots and deterministic ids/timestamps. — logfire `test_http_sink.rs:72`, `test_utils.rs:31-160`.
47. Use proptest for truncation, redaction, and serialization invariants. — dd-trace-rs, libdatadog.
48. Ship `agentcat_sdk::testing` (in-memory transport) behind `test-util`. — sentry `test.rs:46-132`; OTel `trace/mod.rs:28-34`.
49. Cover concurrency and lifecycle: parallel capture, shutdown races, double shutdown, capture after shutdown. Never mutate process env in tests; always `assert!` results. — sentry `tests/test_client.rs:37-67`; counter-example tracing-axiom `builder.rs:222-313`.

### 20.2 Proposed `[workspace.lints]` (checked with clippy 1.99.1 and rustc 1.88)

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"
missing_debug_implementations = "warn"
unreachable_pub = "warn"
unused_qualifications = "warn"
rust_2018_idioms = { level = "warn", priority = -1 }

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
# no panics in library code
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
# stdout is the MCP stdio protocol channel
print_stdout = "deny"
print_stderr = "deny"
dbg_macro = "deny"
# every exception needs #[expect(lint, reason = "...")]
allow_attributes = "warn"
allow_attributes_without_reason = "warn"
# pedantic lints we do not want
must_use_candidate = "allow"
```

`clippy.toml`:

```toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests = true
allow-print-in-tests = true
disallowed-methods = [
  { path = "std::env::var", reason = "read configuration through the config module" },
  { path = "std::env::var_os", reason = "read configuration through the config module" },
  { path = "std::env::vars", reason = "read configuration through the config module" },
]
```

### 20.3 Proposed CI gates

Required on every PR (one aggregate "all green" job is the only required check — logfire `main.yml:107-115`, sentry `ci.yml:55-70`):

1. `cargo fmt --all --check`.
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings` on a pinned stable toolchain.
3. `cargo hack clippy --each-feature --no-dev-deps -- -D warnings` (OTel `scripts/lint.sh:15-18`, dd-trace-rs `lint.yaml:68`).
4. `cargo hack check --feature-powerset --no-dev-deps` (tracing `CI.yml:95-101`; the feature set is small enough for the full powerset).
5. `cargo test --workspace --all-features` and `cargo test --doc` on stable × {ubuntu, macos, windows} (sentry `test.yml:13-14`).
6. MSRV: `cargo hack check --rust-version --workspace --all-targets` and the test suite on the `rust-version` toolchain, with the version read from `Cargo.toml` (logfire `main.yml:47-60`; Cargo CI guide).
7. `cargo minimal-versions check --direct --workspace` on nightly (tracing `CI.yml:166`).
8. Docs: `RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo +nightly doc --workspace --all-features --no-deps` (sentry `doc.yml:24-25`).
9. `cargo deny check advisories bans licenses sources` with a committed `deny.toml` (OTel `ci.yml:158-187`).
10. `cargo semver-checks` against the last release (OTel `semver.yml`; release-plz runs it too).
11. Public API gates: `cargo check-external-types` with an allowlist (OTel `ci.yml:104-131`) and a committed `cargo public-api` snapshot diff (posthog `ci.yaml:59-73`).
12. `cargo package --list` / `cargo publish --dry-run` (posthog `ci.yaml:125-149`).
13. Build all examples.
14. If `rustls-no-provider` exists: assert with `cargo tree` that neither `aws-lc-rs` nor `ring` is compiled under that feature set (logfire `main.yml:79-91`).
15. PR title is a conventional commit (dd-trace-rs `pr-name.yaml`; LaunchDarkly `lint-pr-title.yml`).
16. Workflow hygiene: actions pinned by SHA (sentry, Rudder), zizmor on workflows (OTel `zizmor.yml`), least-privilege `permissions:`.

Informational or scheduled:

17. Coverage with `cargo llvm-cov` uploaded to Codecov (sentry `ci.yml:43`; logfire; OTel).
18. Weekly: clippy and tests on beta and nightly (sentry `weekly.yml:15-31`); `cargo deny check advisories` (sentry `weekly.yml:41`); a latest-dependencies job with `cargo update` and `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=allow` (Cargo CI guide).
19. Optional: `cargo mutants --in-diff` on PRs or weekly (https://mutants.rs/pr-diff.html); unused-dependency check with cargo-shear or cargo-machete (OTel, libdatadog).
20. Not needed: miri (no unsafe code).

Release workflow: release-plz on push to `main` (release PR), and on merge a `release` job in a protected environment with `permissions: id-token: write` that publishes through crates.io trusted publishing.
