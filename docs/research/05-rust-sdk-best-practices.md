# 05: Rust SDK best practices from observability and analytics vendors

Research date: 2026-10-19. This report tests the proposed `agentcat-sdk` publisher design
(`docs/ts-sdk-audit/02` §3.5 and `03` §0, §5.2) against the Rust SDKs of 13 vendors. It ends
with an opinionated checklist for `agentcat-sdk`.

The proposed design, in one line: `publish()` does a `try_send` into a bounded queue; one
dedicated OS thread runs its own current-thread tokio runtime and the async `reqwest` 0.13
client from `agentcat-api` 0.0.3; the customer calls `shutdown(timeout).await` to flush; the
`Drop` of the last handle does a bounded blocking flush as a fallback.

## How to read this report

- Every vendor repo is a shallow clone under `/home/ubuntu/misc/rust-obs-sdks/`. A citation
  `sentry-rust/sentry/src/transports/tokio_thread.rs:130` means that file and line in that
  clone. Line ranges use `a-b`.
- Code that comes from crates.io and not from a clone is cited with its docs.rs source URL,
  or as `<crate>-<version>/<path>:<line>` for the copy in `~/.cargo/registry/src/`.
- "Inference" marks a conclusion that the code implies but that no doc or issue states.
- "Not found" means a search of the clone found nothing. It does not prove absence.
- Measurements ran on this machine: aarch64 Linux, 4 cores, 15 GB RAM, rustc and cargo
  1.99.1, no `sccache`, no `libssl-dev`, no `cmake`.

### Repos and commits

| Vendor | Repo (verified with `gh repo view`) | Commit | Crate studied | Status |
|---|---|---|---|---|
| Sentry | getsentry/sentry-rust | d1e05c0 | `sentry` 0.49.3 | Official, active |
| PostHog | PostHog/posthog-rs | 55277b2 | `posthog-rs` 0.27.0 | Official, active |
| Datadog | DataDog/dd-trace-rs | 2349288 | `datadog-opentelemetry` 0.5.2 | Official, active |
| Datadog | DataDog/libdatadog | c3b0ee0 | `libdd-*` crates | Official, active |
| OpenTelemetry | open-telemetry/opentelemetry-rust | 7a6b7f9 | `opentelemetry_sdk`, `opentelemetry-otlp` 0.33.1 | CNCF, active |
| Tokio | tokio-rs/tracing | 9d0e557 | `tracing-appender` 0.2.5 | Active |
| LaunchDarkly | launchdarkly/rust-server-sdk | b437f19 | `launchdarkly-server-sdk` 3.3.1 | Official, active |
| Statsig | statsig-io/statsig-server-core | 1a9b950 | `statsig-rust` 0.23.2 | Official, active |
| Statsig (legacy) | statsig-io/rust-sdk | af07359 | `statsig` 1.4.0 | Official, superseded |
| Pydantic | pydantic/logfire-rust | 804eca4 | `logfire` 0.13.1 | Official, active |
| Axiom | axiomhq/axiom-rs (+ axiomhq/tracing-axiom 7c4e15e) | ccb5edb | `axiom-rs` 0.11.6 | Official; `tracing-axiom` stale |
| Honeycomb | nlopes/libhoney-rust | f64def9 | `libhoney-rust` 0.1.6 | Community; last release 2021 |
| Segment | segmentio/analytics-rust | 311dc39 | `analytics` 0.2.0 | Archived 2024 |
| RudderStack | rudderlabs/rudder-sdk-rust | 7af2c1e | `rudderanalytics` 2.0.1 | Official, active |

`honeycombio/libhoney-rust` does not exist. The crate lives at `nlopes/libhoney-rust`, its
only crates.io owner is `nlopes`, and Honeycomb now tells Rust users to use
`opentelemetry-otlp` (https://docs.honeycomb.io/getting-data-in/libhoney). Segment's README
says the library is "community supported. Segment does not manage or update this library"
(`analytics-rust/README.md:3-4`). Both are included as cautionary examples, not as models.

---

## 0. Verdict and summary

**The core of the proposed design is right.** Eight active vendors deliver from a background
worker. Seven of them own the thread that sends; only Statsig runs on the customer's runtime
when one exists. Four use exactly our shape: a `std::thread` that builds a `current_thread`
tokio runtime and drives an async HTTP client.

| Who uses "OS thread + own current-thread runtime + async client" | Evidence |
|---|---|
| Sentry, `reqwest` transport (the default) | `sentry-rust/sentry/src/transports/tokio_thread.rs:130-142` |
| OpenTelemetry, `TokioCurrentThread` runtime adapter | `opentelemetry-rust/opentelemetry-sdk/src/runtime.rs:115-132` |
| Pydantic Logfire, all export | `logfire-rust/logfire/src/logfire.rs:761-795` |
| Datadog, instrumentation telemetry worker | `libdatadog/libdd-telemetry/src/worker/mod.rs:1549-1564` |

Two more use the same idea with a different client. PostHog and OpenTelemetry's default
processors run a plain thread with `reqwest::blocking` (`posthog-rs/src/client/transport.rs:1-7`;
`opentelemetry-rust/opentelemetry-sdk/CHANGELOG.md:529-532`). `reqwest::blocking` itself hides
a thread named `reqwest-internal-sync-runtime` that runs a `new_current_thread()` runtime
(https://docs.rs/crate/reqwest/0.13.5/source/src/blocking/client.rs, lines 1424-1428). So
their stack is our design split over two threads. LaunchDarkly and Datadog's trace exporter
own a multi-thread runtime instead (`rust-server-sdk/launchdarkly-server-sdk/src/events/dispatcher.rs:102-106`;
`dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:298-305`).

The vendors that did not own their thread show why they should have:

- Statsig uses the ambient runtime when one exists (`statsig-server-core/statsig-rust/src/statsig_runtime.rs:41-61`).
  Its tasks die with `#[tokio::main]`, and its fork hook silently stopped event flushes in
  production (https://github.com/statsig-io/statsig-server-core/issues/52).
- Logfire's metrics reader spawned lazily onto the application runtime. Most exported records
  became its own HTTP logs, and `shutdown()` deadlocked (https://github.com/pydantic/logfire-rust/issues/141).
- OpenTelemetry moved all batch processors to dedicated threads in 0.28 because "many
  shutdown and deadlock issues reported stem from incorrect use or combinations of async
  runtimes" (https://github.com/open-telemetry/opentelemetry-rust/issues/2386).

**Where the proposal is wrong or incomplete.** The details below decide whether the design
works in production. The section "Recommendations for agentcat-sdk" gives the fix for each one.

1. **`agentcat-api` cannot see `Retry-After`.** The generated error keeps only `status`,
   `content` and `entity`, not the headers (`agentcat-api-0.0.3/src/apis/mod.rs:5-8`).
   Every vendor that handles 429 well reads `Retry-After`.
2. **`agentcat-api`'s `Configuration::default()` calls `reqwest::Client::new()`**
   (`agentcat-api-0.0.3/src/apis/configuration.rs:70`), which "panics if a TLS backend cannot
   be initialized" (`reqwest-0.13.5/src/async_impl/client.rs:2516-2522`). PostHog shipped
   this panic and fixed it in 0.25.5 (`posthog-rs/CHANGELOG.md:21`). It also sets the user
   agent `OpenAPI-Generator/v1/rust` (`agentcat-api-0.0.3/src/apis/configuration.rs:69`).
3. **No HTTP timeouts are planned for connect.** Async reqwest has no default timeout
   (`reqwest-0.13.5/src/async_impl/client.rs:1449,1461`). Sentry sets none, so its 2 s
   shutdown can hang on one request (`sentry-rust/sentry/src/transports/reqwest.rs:93-121`).
4. **The `Drop` fallback needs four guards that `03` §5.2 does not list:** never call
   `block_on` on the caller's thread, never join the worker without a deadline, never wait
   when `Drop` runs on the worker thread itself, and never panic. Sentry, OpenTelemetry,
   PostHog and libhoney each broke one of these rules (sections B.2 and B.3).
5. **`shutdown(timeout).await` must not use `tokio::time` on the customer's runtime.** A
   runtime built without `enable_time` panics with "timers are disabled". Datadog catches
   exactly this panic (`libdatadog/libdd-shared-runtime/src/shared_runtime/mod.rs:104-125,147-171`).
   Put the deadline on the worker side.
6. **Flush and shutdown signals need their own channel.** With one shared channel, a full
   data queue swallows the shutdown signal (OTel #2478, fixed by a separate control channel
   at `opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:521`). Sentry's flush
   uses a blocking `send` on the shared channel and can exceed its own timeout
   (`sentry-rust/sentry/src/transports/thread.rs:200-204`).
7. **Drop policy.** The TS SDK drops the oldest event (`02` §3.2). Every Rust vendor with a
   queue, except Statsig, drops the newest with `try_send`. Use drop-newest and count
   drops by reason. A count cap alone also lets 10,000 events of up to 100 KB each hold
   about 1 GB; add a byte cap.
8. **`02` §3.5 says "spawn the worker on the current tokio runtime (`Handle::try_current()`)".**
   That is Statsig's design, and it fails (see above). `03` §5.2 already supersedes it.
   Always use the dedicated thread.
9. **Default features are too heavy.** `03` §5.4 turns on all exporters and diagnostics by
   default. Exporters add no dependencies, so they cost little, but opt-in matches Datadog
   (`default = []`) and keeps the review surface small. TLS defaults are fine.

### Ten findings that matter most

1. Own the thread and the runtime (7 of the 8 active vendors with background delivery).
2. Build, use and drop the HTTP client and the runtime only on the worker thread. Creating or
   dropping either inside an async context panics (OTel #2400, #2739; reqwest
   `reqwest-0.13.5/src/blocking/client.rs:1275`; Datadog moves runtime drop to a `datadog-trace-drop` thread at
   `dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:110-135`).
3. The hot path is one `try_send` plus one atomic. All serialization, redaction and
   compression run on the worker (PostHog 0.14.0, `posthog-rs/CHANGELOG.md:223-227`; axiom PR #9).
4. Drop the newest event when full. Count drops by reason in atomics. Warn once per episode.
   Report totals at shutdown (OTel `opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:862-873,930-939`; PostHog
   `posthog-rs/src/client/transport.rs:350-364`; Sentry client reports `sentry-rust/sentry-core/src/client/client_reports/inner.rs:15-31`).
5. Retry only network errors, 408, 429, 5xx. Honor `Retry-After` with a cap. Add jitter.
   Bound the whole retry loop by a deadline (OTLP `opentelemetry-rust/opentelemetry-otlp/src/retry_classification.rs:31-47`,
   `opentelemetry-rust/opentelemetry-otlp/src/retry.rs:42-45`; PostHog `posthog-rs/src/client/retry.rs:35-47`).
6. Set connect and request timeouts. Cap each request at the time left before the shutdown
   deadline (PostHog `posthog-rs/src/client/transport.rs:393-406`).
7. Make `shutdown` idempotent, bounded, and honest: return a typed result that says what was
   lost (OTel `OTelSdkError::{AlreadyShutdown, Timeout}` at `opentelemetry-rust/opentelemetry-sdk/src/error.rs:13-37`).
8. Never panic. Deny `unwrap_used`, `expect_used` and `panic` in non-test code (libdatadog
   `libdatadog/libdd-data-pipeline/src/lib.rs:3-8`; logfire `logfire-rust/logfire/Cargo.toml:133-140`). Degrade to a
   disabled client when a thread, runtime or TLS setup fails.
9. Use builders and `#[non_exhaustive]` options from day one. Sentry had to break every user in
   0.49.0 to get there (`sentry-rust/CHANGELOG.md:38-55`). RudderStack needed a 2.0.0 to add
   one field (`rudder-sdk-rust/CHANGELOG.md:10-15`).
10. Enforce MSRV, feature combinations and semver in CI. Only OpenTelemetry and libdatadog run
    most of the tools; tracing-appender declares an MSRV that CI never tests
    (`tracing/tracing-appender/Cargo.toml:21` vs `tracing/.github/workflows/CI.yml:135`).

---

## A. Delivery architecture

### A.1 Thread or task

| Vendor | Sender | Runtime it uses | Spawned at |
|---|---|---|---|
| Sentry (reqwest) | 1 OS thread `sentry-transport` | Own `current_thread`, `block_on` | `sentry-rust/sentry/src/transports/tokio_thread.rs:130-142` |
| Sentry (ureq, curl) | 1 OS thread `sentry-transport` | None (blocking client) | `sentry-rust/sentry/src/transports/thread.rs:124-126` |
| Sentry (logs, metrics, sessions) | 3 more threads: `sentry-logs-batcher`, `sentry-metrics-batcher`, `sentry-session-flusher` | None | `sentry-rust/sentry-core/src/client/batcher.rs:73-75`; `sentry-rust/sentry-core/src/session.rs:209-211` |
| OTel BatchSpanProcessor | 1 OS thread `OpenTelemetry.Traces.BatchProcessor` | None; `futures_executor::block_on(export)` | `opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:571-573,790` |
| OTel BatchLogProcessor | 1 OS thread `OpenTelemetry.Logs.BatchProcessor` | Same | `opentelemetry-rust/opentelemetry-sdk/src/logs/batch_log_processor.rs:469-471,681` |
| OTel PeriodicReader | 1 OS thread `OpenTelemetry.Metrics.PeriodicReader` | Same | `opentelemetry-rust/opentelemetry-sdk/src/metrics/periodic_reader.rs:179-180,432` |
| tracing-appender | 1 OS thread `tracing-appender` | None, sync `recv()` + `write_all` | `tracing/tracing-appender/src/worker.rs:53-73` |
| Datadog traces | Own multi-thread runtime, 2 workers, built on thread `datadog-trace-init` | Own | `dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:175-184,298-305` |
| Datadog telemetry | Unnamed OS thread | Own `current_thread` | `libdatadog/libdd-telemetry/src/worker/mod.rs:1549-1564` |
| PostHog | 1 OS thread `posthog-transport` | None; `reqwest::blocking` | `posthog-rs/src/client/transport.rs:1-7,142-145` |
| LaunchDarkly events | 1 unnamed OS thread with crossbeam `select!` | Own multi-thread, 5 workers | `rust-server-sdk/launchdarkly-server-sdk/src/events/processor.rs:84`; `rust-server-sdk/launchdarkly-server-sdk/src/events/dispatcher.rs:102-106` |
| Statsig | Tokio tasks | Caller's runtime if any, else a global 5-worker runtime | `statsig-server-core/statsig-rust/src/statsig_runtime.rs:41-61,211-218` |
| Logfire | 1 OS thread `logfire-export-runtime` | Own `current_thread` | `logfire-rust/logfire/src/logfire.rs:761-795` |
| axiom-rs | None; each `ingest` call is an async call on the caller's runtime | Caller's | `axiom-rs/src/client.rs:259-288` |
| libhoney-rust | Own multi-thread runtime that builds a second runtime inside a task | Own (twice) | `libhoney-rust/src/transmission.rs:175-189,213` |
| Segment, RudderStack | None; synchronous `reqwest::blocking` per call | None | `analytics-rust/src/http.rs:13-16`; `rudder-sdk-rust/src/client.rs:47-118` |

Why each vendor chose its own thread:

- Sentry: "Cannot start a runtime from within a runtime" (https://github.com/getsentry/sentry-rust/issues/184),
  fixed by moving reqwest setup off the main thread (`sentry-rust/CHANGELOG.md:1164`, PR #186).
- OpenTelemetry: processing must be "independent of shared thread pools (e.g., Tokio),
  ensuring it continues even when pools are saturated" (https://github.com/open-telemetry/opentelemetry-rust/issues/2386).
  Exports also "interfere[s] with the business logic" (https://github.com/open-telemetry/opentelemetry-rust/issues/2066).
- Datadog: the OTel processor APIs "lead to crashes/deadlocks if used improperly within an
  async context" (https://github.com/DataDog/dd-trace-rs/pull/8).
- PostHog: "Being a plain thread with a blocking client (never a tokio task) it works for the
  async client, the blocking client, and ... a `std::panic` hook with no runtime present"
  (`posthog-rs/src/client/transport.rs:3-7`).

One OTel comment describes the exact reason a current-thread runtime cannot host its own
exporter task: "there is only one thread to run task, so it's a deadlock. Thus, we spawn the
background task in a separate thread" (`opentelemetry-rust/opentelemetry-sdk/src/runtime.rs:120-126`).

Thread count matters for leanness. Sentry with default features starts 4 threads per
`Client`, and `Client::clone` starts new batcher threads (`sentry-rust/sentry-core/src/client/mod.rs:106-121`).
Datadog merged several runtimes into one teardown path (https://github.com/DataDog/dd-trace-rs/pull/275).
**One thread for AgentCat and all its exporters is the right target.**

### A.2 No runtime, a different runtime, a runtime that shuts down

| Vendor | No runtime | Customer uses `current_thread` | Customer runtime shuts down |
|---|---|---|---|
| Sentry | Works | Works | Works; transport owns its runtime |
| OTel default processors | Works | `shutdown()`/`force_flush()` block the caller; docs say use `spawn_blocking` (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:480-483`); #2583 closed with docs only | Works |
| OTel async-runtime processors | Panics (needs `tokio::spawn`) | Deadlock (`opentelemetry-rust/opentelemetry-sdk/src/runtime.rs:120-126`) | Tasks die |
| OTel + async `reqwest-client` | "there is no reactor running" panic on the worker (https://github.com/open-telemetry/opentelemetry-rust/issues/2739) | Same | Same |
| OTel + `grpc-tonic` | Must be built inside a runtime; tonic "captures/stores the tokio runtime at creation time" (https://github.com/open-telemetry/opentelemetry-rust/issues/2402) | Works | Breaks |
| Datadog traces | Works | Works | Works |
| PostHog transport | Works | Works | Works; `AsyncFlagPoller` dies with the runtime (`posthog-rs/src/local_evaluation.rs:634`) |
| LaunchDarkly events | Works | Works | Works; data sources are `tokio::spawn` tasks and die |
| Statsig | Creates a global runtime | Uses it | Tasks die; no detection |
| Logfire | Works | Works, except #141 | Works, except #141 |
| axiom-rs | Needs tokio or async-std | Works | Calls fail |

Two more traps from the vendors:

- **Creating or dropping a runtime or a blocking client inside async code panics.**
  `reqwest::blocking::Client::new` "panics if called from within an async runtime"
  (`reqwest-0.13.5/src/blocking/client.rs:1275`). OTel hit it at exporter build time (#2400) and now
  builds the client on a helper thread (`opentelemetry-rust/opentelemetry-otlp/src/exporter/http/mod.rs:241-266`).
  Datadog builds its runtime on `datadog-trace-init` and drops it on `datadog-trace-drop`
  (`dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:110-135,175-184`). PostHog moves a
  user-supplied blocking client into `spawn_blocking` (`posthog-rs/src/client/async_client.rs:145-167`).
- **Spawn only through your own `Handle`.** Logfire's #141 came from `runtime::Tokio::spawn`,
  which calls the ambient `tokio::spawn` (`opentelemetry-rust/opentelemetry-sdk/src/runtime.rs:74-82`).

A subtle point on Sentry's worker: it does a blocking `std` `receiver.into_iter()` inside
`rt.block_on` (`sentry-rust/sentry/src/transports/tokio_thread.rs:142-143`). Between envelopes,
nothing polls the runtime, so hyper's connection tasks stall (inference). Use an async
`recv().await` on a `tokio::sync::mpsc` channel instead.

### A.3 Channel and capacity

| Vendor | Data channel | Capacity (default) | Control channel |
|---|---|---|---|
| Sentry | `std::sync::mpsc::sync_channel` | **30 envelopes** (`sentry-rust/sentry/src/transports/mod.rs:59-61`); configurable since #1311 because "the hardcoded capacity of 30 can saturate" | Same channel |
| OTel spans / logs | `std::sync::mpsc::sync_channel` | **2,048** (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:61-63`; `opentelemetry-rust/opentelemetry-sdk/src/logs/batch_log_processor.rs:60-62`) | Separate `sync_channel(64)` (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:521`) |
| tracing-appender | `crossbeam_channel::bounded` | **128,000 lines** (`tracing/tracing-appender/src/non_blocking.rs:66-67`) | `bounded(0)` rendezvous (`tracing/tracing-appender/src/non_blocking.rs:157`) |
| Datadog traces | `Mutex<SharedState>` + `Condvar` + `tokio::sync::Notify` | **5,000,000 estimated bytes** (`dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:319-324`) | Generation counter |
| Datadog telemetry | `tokio::sync::mpsc::channel` | 5,000 (`libdatadog/libdd-telemetry/src/worker/mod.rs:1455`) | Same |
| PostHog | Unbounded `std::sync::mpsc` gated by an `AtomicUsize` | **10,000** across channel + batch + retry queue (`posthog-rs/src/client/mod.rs:272-275`; `posthog-rs/src/client/transport.rs:104-107,332-348`) | Same channel |
| LaunchDarkly | `crossbeam_channel::bounded` | **500** (`rust-server-sdk/launchdarkly-server-sdk/src/events/processor_builders.rs:22`) | Same channel |
| Statsig | 16 sharded `parking_lot::Mutex<VecDeque>` | 2,000 × 100 batches = 200,000 (`statsig-server-core/statsig-rust/src/event_logging/event_logger.rs:32-40`) | Tasks |
| Logfire | `tokio::sync::mpsc::channel` (OTel async variant) | 2,048 | Oneshot |
| libhoney-rust | `crossbeam_channel::bounded` | 40,000 (`libhoney-rust/src/transmission.rs:191-192`) | Magic stop event in the data channel |

Two design points stand out:

- **PostHog bounds everything in flight with one counter**, not only the channel. A bounded
  channel alone leaves the retry queue and the current batch unbounded.
- **OTel separates control from data.** A full data queue cannot block or lose a flush or
  shutdown request (#2478, PR #2526).

PostHog also boxes events in the channel to keep each slot small (`posthog-rs/src/client/transport.rs:35-38`).

### A.4 Full-queue policy and drop reporting

| Vendor | Policy | Counted | Reported |
|---|---|---|---|
| Sentry | Drop newest (`try_send`) | Lock-free `[AtomicU64; reasons×categories]` (`sentry-rust/sentry-core/src/client/client_reports/inner.rs:15-31`) | Client report piggy-backed on the next envelope (`sentry-rust/sentry-core/src/client/envelope_sender.rs:54-66`) |
| OTel | Drop newest | `AtomicUsize` (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:862-873`) | Warn on first drop, total at shutdown (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:930-939`) |
| tracing-appender | Drop newest (lossy default) or block | `Arc<AtomicUsize>`, `dropped_lines()` (`tracing/tracing-appender/src/non_blocking.rs:135,313-338`) | Never logged |
| Datadog | Drop newest chunk (byte check before add) | Read-and-reset counters (`libdatadog/libdd-data-pipeline/src/trace_buffer/mod.rs:434-447`) | Telemetry every 10 s (`dd-trace-rs/datadog-opentelemetry/src/spans_metrics.rs:72-79`) |
| PostHog | Drop newest | Internal only | Warn once per full episode, re-armed after drain (`posthog-rs/src/client/transport.rs:350-364`) |
| LaunchDarkly | Drop newest | None | One `Once` warning for the process lifetime (`rust-server-sdk/launchdarkly-server-sdk/src/events/processor.rs:95-105`) |
| Statsig | Drop **oldest** per shard | `AtomicU64` | Sent to Statsig as `statsig::log_event_dropped_event_count` (`statsig-server-core/statsig-rust/src/event_logging/event_logger_ops_stats.rs:39-69`) |
| libhoney-rust | `is_full()` check then send: a race | None | A "queue overflow" response in a second bounded channel that freezes when unread (https://github.com/nlopes/libhoney-rust/issues/74) |

Drop-newest is the consensus. It is also the only policy a lock-free bounded channel gives
for free. Two details from OTel matter: count the event before you `try_send` so a concurrent
flush cannot miss it (#3453, `opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:812-818`), and a 429 is not a client-side loss
(Sentry: "transports **must never** record a loss when receiving an HTTP `429`",
`sentry-rust/sentry-core/src/client/client_reports/recorder.rs:56-58`).

### A.5 Batching

| Vendor | Max per request | Interval | Byte cap |
|---|---|---|---|
| Sentry errors | 1 event per envelope | Immediate | None; 413 is logged (`sentry-rust/sentry/src/transports/reqwest.rs:164-175`) |
| Sentry logs, metrics | 100 items | 5 s (`sentry-rust/sentry-core/src/client/batcher.rs:16-19`) | None |
| OTel spans / logs | 512 | 5 s spans, 1 s logs (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:56-69`; `opentelemetry-rust/opentelemetry-sdk/src/logs/batch_log_processor.rs:44-46`) | 64 MiB request cap (`opentelemetry-rust/opentelemetry-otlp/src/exporter/http/mod.rs:34`) |
| Datadog traces | Size-driven | 1 s in dd-trace-rs (`dd-trace-rs/datadog-opentelemetry/src/core/configuration/configuration.rs:2431`) | Flush at 2.5 MB |
| PostHog | 100 | 5 s (`posthog-rs/src/client/mod.rs:258-270`) | None |
| LaunchDarkly | 500 + summary | 5 s (`rust-server-sdk/launchdarkly-server-sdk/src/events/processor_builders.rs:21`) | None |
| Statsig | 2,000 | Tick 1 s, flush ≤ 60 s | None; 413 not retried |
| Logfire | 512 | 500 ms "matches Python" (`logfire-rust/logfire/src/logfire.rs:813-817`) | None |
| axiom-rs `ingest_stream` | 1,000 | 1 s (`axiom-rs/src/client.rs:385`) | None |
| libhoney-rust | 50 | 100 ms (`libhoney-rust/src/transmission.rs:28,32`) | None |
| Segment | — | Caller-driven | 32 KiB per message, 512 KiB per batch (`analytics-rust/src/batcher.rs:8-9`) |

OTel wakes the worker early only when a batch fills, using one `AtomicBool` to avoid a storm of
wake-up messages (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:826-860`). PR #2494 reports a 25% throughput gain from it.

**AgentCat cannot batch today.** The API has one endpoint, `POST /v1/public/events/`, with one
event per request (`agentcat-api-0.0.3/src/apis/events_api.rs:37-53`). The TS SDK sends one
event per request with 5 in flight (`02` §3.1). Sentry also sends one envelope per error
event, so per-event delivery is acceptable for low-rate events like tool calls.

### A.6 Retries, backoff, 429 and Retry-After

| Vendor | Retries | Backoff | Jitter | Retried statuses | Retry-After |
|---|---|---|---|---|---|
| Sentry | **None** | — | — | — | Parsed for rate limiting: `X-Sentry-Rate-Limits` > `Retry-After` > 429 default 60 s (`sentry-rust/sentry/src/transports/reqwest.rs:148-160`; `sentry-rust/sentry/src/transports/ratelimit.rs:29-39`) |
| OTLP 0.33 | 3 (4 attempts) | 100 ms ×2, cap 1.6 s (`opentelemetry-rust/opentelemetry-otlp/src/retry_policy.rs:34-43`) | ≤100 ms | 429, 502, 503, 504, network (`opentelemetry-rust/opentelemetry-otlp/src/retry_classification.rs:31-47`) | Seconds or HTTP date, parse cap 600 s, sleep cap 30 s (`opentelemetry-rust/opentelemetry-otlp/src/retry.rs:42-45`) |
| Datadog | 5 (6 attempts) | 100 ms ×2 (`libdatadog/libdd-trace-utils/src/send_with_retry/retry_strategy.rs:40-48`) | None | **All 4xx and 5xx**, network | None on the agent path |
| PostHog | 3 attempts | 200 ms ×2, cap 30 s (`posthog-rs/src/client/retry.rs:103-115`) | None | 408, 500, 502, 503, 504, network; 429 only with `Retry-After` (`posthog-rs/src/client/retry.rs:35-47`) | Minimum delay, clamped |
| LaunchDarkly | 2 attempts | Fixed 1 s (`rust-server-sdk/launchdarkly-server-sdk/src/events/sender.rs:126-129`) | None | Every status outside 400-499, plus 400, 408, 429 (`rust-server-sdk/launchdarkly-server-sdk/src/reqwest.rs:1-10`) | None; 401/403 disable events for the process (`rust-server-sdk/launchdarkly-server-sdk/src/events/sender.rs:183-193`) |
| Statsig | Re-queue up to 5 | Flush interval doubles 1 s → 60 s | None | Not 400, 403, 405, 413, 429, 501 | None |
| RudderStack | 3 | 100 ms, cap 30 s (`rudder-sdk-rust/src/retry.rs:23-33`) | 20% | 429, 5xx, connect, timeout | Yes, **uncapped**, `thread::sleep` on the caller |
| axiom-rs | `backoff` crate, 500 ms ×2, 30 s total (`axiom-rs/src/http.rs:82-86`) | — | crate default | Only transport errors (inference: `error_for_status` never called) | None |
| libhoney, Segment | None | — | — | — | — |

Best-of-breed is OTLP plus PostHog: a short exponential backoff with jitter, a fixed retryable
set, `Retry-After` honored but capped, the body serialized once outside the loop
(`opentelemetry-rust/opentelemetry-otlp/src/exporter/http/mod.rs:441-446`), and the whole loop inside one
deadline (`opentelemetry-rust/opentelemetry-otlp/src/retry.rs:156-165`). PostHog adds one more good idea: retries
wait in a queue keyed by `next_at` so they do not block other sends
(`posthog-rs/src/client/transport.rs:901-912`). Datadog's "retry every 4xx" and RudderStack's
uncapped `Retry-After` are anti-patterns.

### A.7 HTTP client and TLS

| Vendor | Client | TLS default | TLS options |
|---|---|---|---|
| Sentry | reqwest 0.13 (also ureq 3, curl) | `native-tls` via `transport` feature | `rustls`, `rustls-no-provider` (`sentry-rust/sentry/Cargo.toml:57-69`) |
| OTLP | `reqwest::blocking` (default), async reqwest, hyper, tonic | **None**: the default build cannot speak HTTPS (measured: no TLS crate in `cargo tree`) | `reqwest-rustls`, tonic `tls-ring`/`tls-aws-lc`/`tls-provider-agnostic` (`opentelemetry-rust/opentelemetry-otlp/Cargo.toml:77-107`) |
| Datadog | hyper 1 + hyper-util legacy client, pool 20 per host | Plain HTTP to local agent; `https` opt-in | rustls + **ring**; `fips` = aws-lc-rs (`libdatadog/libdd-common/Cargo.toml:36-50`) |
| PostHog | reqwest 0.13 blocking + async | rustls (aws-lc-rs) | none |
| LaunchDarkly | Own `HttpTransport` trait, hyper 1 + hyper-rustls 0.27 | rustls + **ring**, native roots | webpki roots, native-tls (`rust-server-sdk/launchdarkly-server-sdk/Cargo.toml:60-89`) |
| Statsig | reqwest 0.12 + pluggable `NetworkProvider` | rustls + ring, native roots | — |
| Logfire | async reqwest 0.13 via OTLP | `tls-aws-lc` | `rustls-no-provider`; returns `ConfigureError::CryptoProviderRequired` instead of letting reqwest panic (`logfire-rust/logfire/src/exporters.rs:460-479`) |
| axiom-rs | reqwest 0.12 | `default-tls` | TLS features are mutually exclusive via `compile_error!` (`axiom-rs/src/lib.rs:68-75`), which breaks feature additivity |

TLS lessons:

- **Do not install a process-wide rustls `CryptoProvider`.** libdatadog does it with a `Once`
  (`libdatadog/libdd-common/src/connector/mod.rs:121-132`), which can fight the customer's choice.
- **Do not let feature unification change the customer's HTTP stack.** Sentry restored
  `native-tls-no-alpn` and dropped `http2` because its features silently enabled HTTP/2 in user
  apps (`sentry-rust/CHANGELOG.md:79`, #1258).
- **Keep reqwest types out of the public API.** Sentry's `with_client(reqwest::Client)` made
  the reqwest 0.13 upgrade a breaking change (`sentry-rust/CHANGELOG.md:213`).
- **Check the crypto provider in CI.** Logfire runs `cargo tree` to prove `aws-lc-rs` and `ring`
  are absent without `tls-aws-lc` (`logfire-rust/.github/workflows/main.yml:86-91`).
  libdatadog has a `check-crypto-providers.yml` workflow (`libdatadog/.github/workflows/check-crypto-providers.yml:13-24`).
- **`native-tls` does not build on a stock Linux image.** Measured: `agentcat-api` with default
  features and `sentry` with default features both fail in `openssl-sys` because
  `libssl-dev` is missing. The rustls + aws-lc-rs build succeeds with only a C compiler.

### A.8 Compression and wire format

| Vendor | Format | Compression |
|---|---|---|
| Sentry | Envelope (JSON lines) via `serde_json::to_writer` (`sentry-rust/sentry-types/src/protocol/envelope.rs:585-662`) | None |
| OTLP | Protobuf by default, JSON optional | Optional `gzip-http`, `zstd-http` features |
| Datadog agent | msgpack v0.4, hand-written `rmp` encoder (`libdatadog/libdd-trace-utils/src/msgpack_encoder/v04/mod.rs:8,190`) | None to agent; zstd for agentless |
| PostHog | JSON | Off by default; gzip/deflate/br/zstd options (`posthog-rs/src/client/mod.rs:23-47`) |
| LaunchDarkly | JSON | gzip on by default (`event-compression`) |
| Statsig | JSON | gzip at `Compression::best()`, or zstd 12 |
| axiom-rs | NDJSON | gzip, in `spawn_blocking` because it "blocks the async executor" (https://github.com/axiomhq/axiom-rs/pull/9) |

Do CPU-heavy work on the worker thread, not on the caller.

### A.9 Timeouts

| Vendor | Connect | Request | Shutdown default |
|---|---|---|---|
| Sentry | **None** | **None** (`sentry-rust/sentry/src/transports/reqwest.rs:93-121`) | 2 s (`sentry-rust/sentry-core/src/clientoptions.rs:913`) |
| OTLP | — | 10 s (`opentelemetry-rust/opentelemetry-otlp/src/exporter/mod.rs:38,49`) | 5 s per provider (`opentelemetry-rust/opentelemetry-sdk/src/trace/provider.rs:273-275`) |
| Datadog | — | 3 s per attempt (`libdatadog/libdd-common/src/endpoint.rs:157`) | 5 s flush |
| PostHog | — | 30 s; capped at the remaining shutdown deadline (`posthog-rs/src/client/transport.rs:393-406`) | 30 s drain |
| LaunchDarkly | **None** | **None** (`launchdarkly-sdk-transport-0.1.4/src/transport_hyper.rs`, https://docs.rs/launchdarkly-sdk-transport/0.1.4) | `close()` unbounded |
| Statsig | — | 10 s | 3 s |
| axiom-rs | — | 10 s fixed (`axiom-rs/src/http.rs:52-56`) | n/a |
| tracing-appender | — | — | 100 ms + 1000 ms (`tracing/tracing-appender/src/non_blocking.rs:282-302`) |

Worst-case send time is a design input. Datadog's single trace worker can spend about
6 × 3 s + 3.1 s ≈ 21 s on one payload (inference from `libdatadog/libdd-trace-utils/src/send_with_retry/retry_strategy.rs:40-48` and
`libdatadog/libdd-common/src/endpoint.rs:157`). One slow request blocks one in-flight slot for that long.

---

## B. Lifecycle

### B.1 Init API shape

| Vendor | Init | Returns | Can fail? |
|---|---|---|---|
| Sentry | `pub fn init<C: Into<ClientOptions>>(opts: C) -> ClientInitGuard` (`sentry-rust/sentry/src/init.rs:91-94`) | `#[must_use] ClientInitGuard(Arc<Client>)`; Drop flushes (`sentry-rust/sentry/src/init.rs:13-43`) | Panics on an invalid DSN (`sentry-rust/sentry/src/init.rs:86-90`) |
| OTel | `SdkTracerProvider::builder().with_batch_exporter(e).build()` (`opentelemetry-rust/opentelemetry-sdk/src/trace/provider.rs:334-337`); OTLP `SpanExporter::builder().with_http().build() -> Result<_, ExporterBuildError>` | `SdkTracerProvider { inner: Arc<..> }`, `Clone` | Exporter build returns `Result` |
| tracing-appender | `non_blocking<T: Write + Send + 'static>(writer: T) -> (NonBlocking, WorkerGuard)` (`tracing/tracing-appender/src/lib.rs:194`) | `#[must_use] WorkerGuard` | Panics if the thread cannot spawn (`tracing/tracing-appender/src/worker.rs:90`) |
| Datadog | `datadog_opentelemetry::tracing().init() -> SdkTracerProvider` (`dd-trace-rs/datadog-opentelemetry/src/lib.rs:373-398,540`) | OTel provider | Never: the body runs under `catch_unwind` and falls back to a no-op provider (`dd-trace-rs/datadog-opentelemetry/src/lib.rs:548-621`) |
| PostHog | `pub async fn client<C: Into<ClientOptions>>(options: C) -> Client` (`posthog-rs/src/client/async_client.rs:131`) | `Client` | Never: a blank key or failed client build gives a disabled client plus a warning (`posthog-rs/src/client/mod.rs:357-366`) |
| LaunchDarkly | `Client::build(Config) -> Result<Client, BuildError>`, then `start_with_default_executor()` (`rust-server-sdk/launchdarkly-server-sdk/src/client.rs:177,251`) | `Client` | `Result`; `start_with_default_executor` needs a runtime |
| Statsig | `Statsig::new(&str, Option<Arc<StatsigOptions>>) -> Self`, then `async initialize() -> Result<(), StatsigErr>` (`statsig-server-core/statsig-rust/src/statsig.rs:162,268`) | `Statsig` | `initialize` returns `Result` |
| Logfire | `logfire::configure()...finish() -> Result<Logfire, ConfigureError>` (`logfire-rust/logfire/src/lib.rs:237`; `logfire-rust/logfire/src/config.rs:198`) | `Logfire` (`Arc`), plus optional `#[must_use] ShutdownGuard` | `Result` with a `#[non_exhaustive]` error |
| axiom-rs | `Client::builder()...build() -> Result<Client>` (`axiom-rs/src/client.rs:451-527`) | `Client` | `Result` |

Two styles exist: infallible init that degrades to a no-op (Datadog, PostHog) and fallible init
that returns `Result` (Logfire, LaunchDarkly, axiom). For instrumentation that must fail open,
infallible init is the safer default. Offer `try_build() -> Result` for customers who want to
see configuration errors at startup. Sentry's panic on a bad DSN is the anti-pattern.

### B.2 flush(timeout) and shutdown(timeout)

| Vendor | Flush | Shutdown | Idempotent | Async? |
|---|---|---|---|---|
| Sentry | `flush(&self, Option<Duration>) -> bool` (`sentry-rust/sentry-core/src/client/mod.rs:495-511`) | `close(&self, Option<Duration>) -> bool`; afterwards every capture is a silent no-op (`sentry-rust/sentry-core/src/client/envelope_sender.rs:185-218`) | Yes | No, blocks |
| OTel | `force_flush() -> OTelSdkResult`; the BSP waits a hard-coded 5 s (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:693`) | `shutdown_with_timeout(Duration) -> OTelSdkResult`; second call returns `Err(AlreadyShutdown)` (`opentelemetry-rust/opentelemetry-sdk/src/trace/provider.rs:246-270`) | Yes, via `compare_exchange` | No, blocks |
| Datadog | `force_flush()` = `flush_and_wait(5s)` (`dd-trace-rs/datadog-opentelemetry/src/span_processor.rs:681-690`) | One deadline across flush, close and stop (`dd-trace-rs/datadog-opentelemetry/src/span_processor.rs:692-739`) | Yes | No |
| PostHog | `async flush(&self)`, no timeout, returns nothing (`posthog-rs/src/client/async_client.rs:333-347`) | `async shutdown(&self)`, bounded by `shutdown_timeout_ms`, then a blocking `join()` (`posthog-rs/src/client/async_client.rs:403-425`) | Yes | Yes, but ends in a blocking join |
| LaunchDarkly | `flush()` fire-and-forget; `async flush_blocking(Duration) -> bool`, where `Duration::ZERO` waits forever (`rust-server-sdk/launchdarkly-server-sdk/src/client.rs:368-424`) | `close()`, sync, no timeout (`rust-server-sdk/launchdarkly-server-sdk/src/client.rs:345-359`) | Guarded | Flush only |
| Statsig | `async flush_events(&self)`, errors swallowed | `async shutdown()` = `shutdown_with_timeout(3s) -> Result<(), StatsigErr>`; aborts tasks on timeout (`statsig-server-core/statsig-rust/src/statsig.rs:335-383`) | — | Yes |
| Logfire | `force_flush() -> Result<(), OTelSdkError>`, blocking (`logfire-rust/logfire/src/logfire.rs:135-149`) | `shutdown() -> Result<(), ShutdownError>`, no timeout (`logfire-rust/logfire/src/logfire.rs:159-175`) | Returns `AlreadyShutdown` | No |
| tracing-appender | None; `NonBlocking::flush()` is a no-op (`tracing/tracing-appender/src/non_blocking.rs:254-256`) | Guard drop only | — | — |

What the best of them get right, and what we should copy:

- **One deadline for the whole shutdown** (Datadog). Do not apply the full timeout to each
  step; OTel's provider applies it to each processor in turn, so the total can be N × timeout
  (`opentelemetry-rust/opentelemetry-sdk/src/trace/provider.rs:115-129`).
- **Typed outcomes.** OTel separates `AlreadyShutdown`, `Timeout(Duration)` and
  `InternalFailure` and documents that a timed-out operation "may still be complete"
  (`opentelemetry-rust/opentelemetry-sdk/src/error.rs:13-37`).
- **An explicit final drain.** Datadog lost queued chunks because cancellation stopped the loop
  before the last hand-off (https://github.com/DataDog/libdatadog/pull/2313). libhoney broke out
  of its loop and dropped held batches (https://github.com/nlopes/libhoney-rust/issues/65).
- **Flush must wait for delivery, not only for a wake-up.** dd-trace-rs `force_flush` returned
  too early until PR #287 (https://github.com/DataDog/dd-trace-rs/pull/287).
- **A worker cannot be cancelled in the middle of a blocking send.** OTel says so in its
  umbrella issue (https://github.com/open-telemetry/opentelemetry-rust/issues/3542). With an
  async client on our own runtime, we can cancel: drop the in-flight futures at the deadline.

Only PostHog and Statsig offer an async shutdown. PostHog's ends in a blocking `join()`.
Statsig's runs on whatever runtime it picked, often the customer's, and uses tokio timers. No
vendor combines an async API with a worker that the customer's runtime does not drive. **Our
proposed `shutdown().await` is a real improvement**, provided it awaits only a oneshot reply and
the deadline lives on the worker (see C.6).

### B.3 Drop behavior

| Vendor | What Drop does | Bounded? |
|---|---|---|
| Sentry `ClientInitGuard` | `end_session()` then `close(None)` = up to 2 s (`sentry-rust/sentry/src/init.rs:31-43`) | Partly |
| Sentry `TransportThread` | Set a flag, blocking `send(Shutdown)`, `handle.join().unwrap()` with no deadline. Queued envelopes are discarded (`sentry-rust/sentry/src/transports/tokio_thread.rs:222-230`; `sentry-rust/sentry/src/transports/thread.rs:129-132`) | **No**; re-panics if the worker panicked |
| OTel providers | Last drop calls `shutdown()` = 5 s per processor (`opentelemetry-rust/opentelemetry-sdk/src/trace/provider.rs:137-149`) | Yes |
| OTel processors | No Drop. If senders vanish, the worker exits **without exporting** (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:672-681`) | — |
| tracing-appender `WorkerGuard` | 100 ms to send shutdown, 1000 ms for the ack, never joins; failure goes to `println!` (`tracing/tracing-appender/src/non_blocking.rs:282-302`) | Yes, 1.1 s |
| Datadog exporter | Moves buffer and runtime to a `datadog-trace-drop` thread. No flush (`dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:110-135`) | Yes |
| PostHog `Client` | `close_blocking()`: bounded drain plus join; blocks the executor thread if dropped in a task (documented, `posthog-rs/src/client/async_client.rs:1432-1446`). On the worker thread it only queues shutdown (`posthog-rs/src/client/transport.rs:253-269`) | Yes |
| LaunchDarkly | No Drop; the dispatcher exits **without flushing** (`rust-server-sdk/launchdarkly-server-sdk/src/events/dispatcher.rs:168-171`) | — |
| Statsig | Signals `force_shutdown`; no flush (`statsig-server-core/statsig-rust/src/statsig.rs:143-158`) | — |
| Logfire `ShutdownGuard` | Calls `shutdown()`, prints errors with `eprintln!` (`logfire-rust/logfire/src/logfire.rs:560-596`) | Inherits OTel |
| libhoney | `self.stop().unwrap()` on every clone; panics if the queue is full (`libhoney-rust/src/transmission.rs:90-94`) | **No** |

Rules that follow:

1. Only the **last** handle flushes. Do the work in `Drop` for the shared `Arc` inner, not for
   each clone (libhoney's anti-pattern).
2. **Bound every wait.** Use `recv_timeout` on a std channel, not `join()`.
3. **Never `block_on` in Drop.** It panics inside an async context.
4. **Do nothing but signal when Drop runs on the worker thread.** PostHog 0.25.1 fixed a
   deadlock when a worker callback dropped the client (`posthog-rs/CHANGELOG.md:45`).
5. **Never panic in Drop.** A panic during unwinding aborts the process. Sentry's
   `join().unwrap()` re-panics if the worker died.

### B.4 Process exit

Nobody installs an `atexit` or signal handler for flushing. All of them require an explicit
shutdown or a guard held in `main`.

- `std::process::exit` skips destructors. Sentry tracks this in
  https://github.com/getsentry/sentry-rust/issues/178. tracing-appender's docs say the guard
  "should be assigned in the `main` function" and that `exit` loses data
  (`tracing/tracing-appender/src/non_blocking.rs:29-32,74-82`).
- A provider in a `static` is never dropped. OTel removed `global::shutdown_tracer_provider()`
  in 0.28: "The tracer_provider must be kept around to call shutdown on it"
  (`opentelemetry-rust/opentelemetry-sdk/CHANGELOG.md:498-506`; https://github.com/open-telemetry/opentelemetry-rust/issues/1961).
  PostHog and Datadog document the same (`posthog-rs/src/lib.rs:155-157`; `dd-trace-rs/datadog-opentelemetry/src/lib.rs:68-70`).
- Panics with `panic = "abort"`: Sentry's panic hook flushes for up to 2 s inside the hook
  (`sentry-rust/sentry-panic/src/lib.rs:33-40`). Logfire and PostHog also install panic hooks.
  AgentCat does not capture panics, so it needs no hook.
- Serverless: Logfire lost data in a frozen Lambda and added `force_flush`
  (https://github.com/pydantic/logfire-rust/issues/113). LaunchDarkly added `flush_blocking`
  for the same reason (`rust-server-sdk/launchdarkly-server-sdk/CHANGELOG.md:105`).

### B.5 Double init

| Vendor | Second init |
|---|---|
| Sentry | Replaces the bound client; the old guard still closes its own client (`sentry-rust/sentry/src/init.rs:104-105`); panic hook installed once via `Once` |
| OTel | `set_tracer_provider` silently replaces the global and logs `TracerProvider.GlobalSet` (`opentelemetry-rust/opentelemetry/src/global/trace.rs:427-440`) |
| Datadog | No guard; each `init()` builds a new exporter and runtime; telemetry is first-wins `OnceLock` with a user count (`dd-trace-rs/datadog-opentelemetry/src/core/telemetry.rs:26-27`) |
| PostHog | Global init returns `Err(AlreadyInitialized)` (`posthog-rs/src/global.rs:45-48`) |
| Statsig | `new_shared` returns `Err(SharedInstanceFailure)` (`statsig-server-core/statsig-rust/src/statsig.rs:431-437`) |
| Logfire | `TracingAlreadySetup` / `AlreadyConfigured` errors (`logfire-rust/logfire/src/logfire.rs:239-251`) |

### B.6 Global singleton or explicit handle

Almost every modern SDK returns an explicit, cheaply cloneable handle (`Arc` inside) and makes
the global optional. OTel, Logfire, PostHog and Datadog offer both. LaunchDarkly and axiom are
handle-only. Sentry is the outlier with a thread-local `Hub` that forks from the main hub
(`sentry-rust/sentry-core/src/hub_impl.rs:10-27`). That design loses events when a runtime
spawns threads before `sentry::init`, so Sentry's docs tell users not to use `#[tokio::main]`
(https://github.com/getsentry/sentry-rust/issues/920; https://docs.sentry.io/platforms/rust/).
**AgentCat should be handle-only in v1.**

### B.7 Fork safety

Only Datadog and Statsig handle fork, because Python and Ruby bindings fork.

- libdatadog's `ForkSafeRuntime` has `before_fork`, `after_fork_parent` and `after_fork_child`.
  `before_fork` pauses every worker and drops the runtime so that no threads exist at fork time
  (`libdatadog/libdd-shared-runtime/src/shared_runtime/fork_safe.rs:30-138`). Workers pause only
  at trigger points, so "run must always use timeout on long-running await calls to avoid
  blocking forks" (`libdatadog/libdd-shared-runtime/src/worker.rs:24-25`). A waker that kept a
  dropped runtime alive crashed forked children on macOS (https://github.com/DataDog/libdatadog/pull/2050).
- dd-trace-rs, the Rust SDK, does not use any of it: "Would we even support forks?"
  (`dd-trace-rs/datadog-opentelemetry/src/core/configuration/configuration.rs:1913`).
- Statsig stores the pid and rebuilds its global state when the pid changes
  (`statsig-server-core/statsig-rust/src/statsig_global.rs:16-25`), with a `libc::fork` test.
  Its Python before-fork reset killed the parent's only runtime and froze flags for 34 hours
  (https://github.com/statsig-io/statsig-server-core/issues/52).
- Sentry, OTel, tracing-appender, PostHog, LaunchDarkly and Logfire: not found.

Rust MCP servers rarely fork after start. Document "create `AgentCat` after any fork or
daemonize step". A cheap pid check on publish can turn silent loss in a forked child into one
warning.

---

## C. Safety

### C.1 Never-panic policy

| Vendor | Stated policy | Lint enforcement | Known panics left |
|---|---|---|---|
| libdatadog | — | `#![cfg_attr(not(test), deny(clippy::panic, unwrap_used, expect_used, unreachable, todo, unimplemented))]` per crate (`libdatadog/libdd-data-pipeline/src/lib.rs:3-8`) | `lock_or_panic` helper (`libdatadog/libdd-common/src/lock_ext.rs:42-52`) |
| Logfire | — | `unwrap_used`, `print_stdout`, `print_stderr`, `dbg_macro`, `pedantic` = deny (`logfire-rust/logfire/Cargo.toml:133-140`) | — |
| axiom-rs | — | `#![deny(warnings, missing_docs, clippy::unwrap_used, clippy::pedantic)]` (`axiom-rs/src/lib.rs:34-42`) | `deny(warnings)` in a library breaks on new compilers |
| OTel | ADR 001: "No panics from SDK APIs" (`opentelemetry-rust/docs/adr/001_error_handling.md:17-19`) | None (`clippy::all` warn only) | `.expect` on thread spawn (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:687`); `join().unwrap()` in shutdown |
| Sentry | — | None | `spawn().unwrap()` in batcher and flusher; open issue "Panic when system is out of resources" (https://github.com/getsentry/sentry-rust/issues/530); `.expect` on envelope serialization (`sentry-rust/sentry/src/transports/reqwest.rs:136`) |
| PostHog | Init never fails | None | Few `expect` calls |
| LaunchDarkly, Statsig | — | None | `.unwrap()` in the event sender (`rust-server-sdk/launchdarkly-server-sdk/src/events/sender.rs:150,191`); `.expect` on runtime creation (`statsig-server-core/statsig-rust/src/statsig_runtime.rs:218,261`) |

Lint-based enforcement (libdatadog, Logfire) is the only approach that holds over time. The
crates that rely on review all still contain runtime panics on thread or runtime creation.

### C.2 `catch_unwind`

- **Customer callbacks:** PostHog wraps `before_send` and `on_error` (`posthog-rs/src/client/common.rs:61-92`).
- **Init:** Datadog wraps the whole init and returns a no-op provider on panic
  (`dd-trace-rs/datadog-opentelemetry/src/lib.rs:548-621`) and wraps the propagator with a
  `catch_panic!` macro (`dd-trace-rs/datadog-opentelemetry/src/core/mod.rs:20-43`).
- **Runtime misconfiguration:** Datadog turns tokio's "timers are disabled" panic into an error
  (`libdatadog/libdd-shared-runtime/src/shared_runtime/mod.rs:147-171`).
- **FFI boundaries:** libdatadog (`libdatadog/libdd-shared-runtime-ffi/src/lib.rs:11-37`).
- **Not used:** Sentry, OTel, LaunchDarkly, Statsig and Logfire use it only in tests. No vendor
  wraps its worker loop, so a panic in the worker kills delivery for good (OTel #3640;
  `libdatadog/libdd-shared-runtime/src/shared_runtime/pausable_worker.rs:180-187`).

`catch_unwind` does nothing under `panic = "abort"`. It is a second line of defense after lints.

### C.3 Mutex poisoning

| Approach | Who |
|---|---|
| `lock().unwrap_or_else(PoisonError::into_inner)` | Sentry hub stack (`sentry-rust/sentry-core/src/hub_impl.rs:103-116`), tracing-appender (`tracing/tracing-appender/src/sync.rs:1-12`), PostHog (`posthog-rs/src/client/mod.rs:93-96`) |
| Map poison to an error or a no-op | OTel global provider and `SimpleSpanProcessor` (`opentelemetry-rust/opentelemetry/src/global/trace.rs:375-384`), Datadog trace buffer (`libdatadog/libdd-data-pipeline/src/trace_buffer/mod.rs:528-533`), Logfire (`if let Ok(..)`) |
| `parking_lot` (no poisoning) | LaunchDarkly, Statsig (with `try_lock_for(5s)` everywhere), libhoney |
| `.lock().unwrap()` | Sentry batcher and client, OTel BSP shutdown, libdatadog `lock_or_panic` |

The cheapest correct choice: no lock on the hot path at all (a channel plus atomics), and
`unwrap_or_else(PoisonError::into_inner)` for the few cold-path locks.

### C.4 Feedback loops

| Mechanism | Who | Evidence |
|---|---|---|
| Thread-local suppression flag entered by every worker thread | OTel 0.30+ | `Context::enter_telemetry_suppressed_scope()` at `opentelemetry-rust/opentelemetry/src/context.rs:433-435`; entered at `opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:574` |
| Suppression in `on_thread_start` of its own runtime, plus a filter on its tracing layer | Logfire | `logfire-rust/logfire/src/logfire.rs:757-776`; `logfire-rust/logfire/src/bridges/tracing.rs:55-68` |
| Self-diagnostics through `eprintln!`, never `log` or `tracing` | Sentry, tracing-appender | `sentry-rust/sentry-core/src/macros.rs:55-63`; `tracing/tracing-appender/src/worker.rs:87` |
| Emit only `tracing` events, never spans; suppress around export-error logging | Datadog | `dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:514-518` |
| Hooks "MUST NOT call back into the SDK"; panic hook skips the worker thread | PostHog | `posthog-rs/src/client/mod.rs:417-425`; `posthog-rs/src/client/transport.rs:170-176` |
| Separate muted HTTP client with 0 retries for SDK error reports | Statsig | `statsig-server-core/statsig-rust/src/observability/sdk_errors_observer.rs:43,55-127` |
| User-side filter `reqwest=off,hyper=off,h2=off,tonic=off` | OTel docs | `opentelemetry-rust/docs/logs.md:27-36` |

Two real loops happened. Sentry deadlocked when a "dropped envelope" warning became a
breadcrumb that needed a write lock while `send_envelope` held the read lock
(https://github.com/getsentry/sentry-rust/pull/633). Logfire exported its own hyper logs (#141).
OTel's suppression has a known gap: hyper and tonic tasks on other threads "do not propagate
OpenTelemetry's `Context`" (`opentelemetry-rust/docs/design/logs.md:391-394`).

AgentCat's exposure is small. It captures MCP tool calls, not logs or spans, so its own
diagnostics cannot become AgentCat events. The risks are second-order: our warnings and the
debug logs of reqwest and hyper on our worker thread flow into the customer's tracing pipeline,
and a customer hook could call back into the SDK from the worker thread.

### C.5 Self-diagnostics

| Vendor | Channel | Switch |
|---|---|---|
| Sentry | `eprint!("[sentry] ")` | `debug: bool` option (`sentry-rust/sentry-core/src/clientoptions.rs:152`) |
| OTel | `otel_debug!`, `otel_warn!` ... expand to `tracing::…!(name: .., target: env!("CARGO_PKG_NAME"))` | `internal-logs` feature, on by default (`opentelemetry-rust/opentelemetry/src/global/internal_logging.rs:24-62`) |
| Datadog | `dd_debug!` ... to the app's tracing subscriber, else `log`, else stdout | `DD_LOG_LEVEL`, default Error (`dd-trace-rs/datadog-opentelemetry/src/core/log.rs:6-58`) |
| PostHog | `tracing` + `on_error` hook | — |
| LaunchDarkly | `log` crate | — |
| Statsig | Own macros; installs `simple_logger` as the **global logger** if none is set | Warn default (`statsig-server-core/statsig-rust/src/output_logger.rs:84-135`) |

Patterns to copy: stable, namespaced event names (`BatchSpanProcessor.SpansDropped`), a target
equal to the crate name, response bodies logged only at debug "because they may contain
sensitive information" (`opentelemetry-rust/opentelemetry-otlp/src/exporter/http/mod.rs:540-552`).
Pattern to avoid: installing a global logger (Statsig).

`tracing` costs AgentCat nothing: rmcp already depends on it (measured in the rmcp baseline).

### C.6 Not blocking the customer's executor

| Vendor | Hot path | Blocking calls the customer can hit |
|---|---|---|
| OTel | Atomic increment + `try_send` (`opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:812-818`) | `force_flush`, `shutdown` (blocking `recv_timeout`) |
| Sentry | Scope, integrations and `before_send` run on the caller, **inside the transport read lock**, then `try_send` (`sentry-rust/sentry-core/src/client/mod.rs:316-391`; `sentry-rust/sentry-core/src/client/envelope_sender.rs:189-195`) | Guard drop, `flush` |
| Datadog | Sharded lock + `Notify::notify_one`; conversion on the worker (`dd-trace-rs/datadog-opentelemetry/src/span_processor.rs:643-679`) | Init waits for the init thread |
| PostHog | Atomic CAS + unbounded send; `before_send` and serialization on the worker | `shutdown().await` ends in `join()`; `Drop` |
| LaunchDarkly | `try_send` | `close()`; `flush_blocking` uses `spawn_blocking` |
| Statsig | One shard lock | Serialization and gzip `best()` on whichever runtime it picked, possibly the customer's |

**The timer trap.** If `shutdown(timeout).await` wraps its wait in `tokio::time::timeout`, it
panics on a customer runtime built without `enable_time`. Datadog had to catch exactly that
panic (`libdatadog/libdd-shared-runtime/src/shared_runtime/mod.rs:104-125`). The safe design:
the caller sends `Shutdown { deadline, reply: oneshot::Sender }` on the control channel and
awaits only the `oneshot::Receiver`. tokio's sync primitives work on any runtime and without
one. The worker, which owns a runtime with timers, enforces the deadline and always replies. If
the worker is gone, the sender drops and the await returns at once.

---

## D. Leanness and performance

### D.1 Measured dependency counts

Method: one probe crate per row, `cargo generate-lockfile`, then count unique `name vX.Y.Z`
lines in `cargo tree -e normal,build --prefix none` for the host target, root excluded. "Over
rmcp stdio" adds the vendor crate to a baseline of `rmcp =3.5.1` (`server`, `macros`,
`transport-io`) plus `tokio` (`rt-multi-thread`, `macros`, `io-std`), which has **58** packages.
"Over rmcp HTTP" uses a baseline that also enables `transport-streamable-http-server` and
`axum 0.8` (**89** packages). Scripts and lockfiles are in the session scratchpad
(`.../scratchpad/depcount/`).

| Probe (default features unless stated) | Alone | Over rmcp stdio | Over rmcp HTTP |
|---|---|---|---|
| `tracing-appender` 0.2.5 | 22 | +13 | — |
| **ureq 3.4 + `json` (rustls + ring)** | — | **+24** | **+20** |
| reqwest 0.13.5, `rustls-no-provider` + `json`, rustls with `ring` | — | +64 | — |
| reqwest 0.13.5, `native-tls` + `json` | — | +64 | — |
| `opentelemetry-otlp` 0.33.1 + sdk (**no TLS**, cannot reach HTTPS) | 93 | +66 | — |
| reqwest 0.13.5, `rustls` (aws-lc-rs) + `json` | 92 | +69 | — |
| `agentcat-api` 0.0.3, default (`native-tls`) | 108 | +74 | — |
| **`agentcat-api` 0.0.3, `rustls`; the proposed design** | 111 | **+79** | **+59** |
| `sentry` 0.49.3, `reqwest` + `rustls` only | 115 | +84 | — |
| `opentelemetry-otlp` 0.33.1, trace + `reqwest-rustls` | 114 | +87 | — |
| `axiom-rs` 0.11.6 | 126 | +90 | — |
| `sentry` 0.49.3, default | 130 | +98 | +77 |
| `launchdarkly-server-sdk` 3.3.1 | 152 | +106 | — |
| `logfire` 0.13.1 | 142 | +106 | — |
| `posthog-rs` 0.27.0 | 147 | +109 | — |
| `datadog-opentelemetry` 0.5.2 | 173 | +125 | — |
| `statsig-rust` 0.23.2 | 252 | +205 | — |

What `agentcat-api` adds over plain reqwest (`rustls`, `json`): `serde_with`,
`serde_with_macros`, `serde_repr`, `serde_urlencoded`, `mime`, `mime_guess`, `unicase`,
`iana-time-zone`, `ryu` and the crate itself. The generator turns on reqwest `multipart`,
`query` and `form` (`agentcat-api-0.0.3/Cargo.toml`), which the events call does not use.

The rmcp baseline already contains `serde`, `serde_json`, `tokio`, `tokio-util`, `futures`,
`tracing`, `thiserror` 2, `chrono`, `uuid` and `schemars`. Those cost AgentCat nothing.

### D.2 Measured clean build times

Clean `cargo build` (debug, `-j 4`, offline, fresh `target/`), wall clock. Run-to-run noise is
about ±3 s (the ureq row builds faster than its own baseline).

| Probe (over rmcp stdio) | Time | Delta | `target/` size |
|---|---|---|---|
| Baseline rmcp stdio | 30.5 s (re-run: 30.3 s) | — | 375 MB |
| + ureq 3 (ring) | 28.3 s | ≈0 | 487 MB |
| + `opentelemetry-otlp` default (no TLS) | 39.8 s | +9 s | 662 MB |
| + reqwest, `rustls-no-provider` + ring | 43.4 s | +13 s | 605 MB |
| + reqwest, `rustls` (aws-lc-rs) + `json` | 44.3 s | +14 s | — |
| + `sentry`, `reqwest` + `rustls` | 47.6 s | +17 s | 743 MB |
| + `launchdarkly-server-sdk` | 51.2 s | +21 s | 914 MB |
| **+ `agentcat-api` `rustls` (proposed)** | **52.4 s** | **+22 s** | 700 MB |
| + `logfire` | 52.9 s | +22 s | 899 MB |
| + `posthog-rs` | 64.2 s | +34 s | 875 MB |
| + `datadog-opentelemetry` | 66.8 s | +36 s | 981 MB |
| + `statsig-rust` | 159.4 s | +129 s | 1,353 MB |
| + `agentcat-api` default (`native-tls`) | **fails**: `openssl-sys` build script, no OpenSSL headers | — | — |
| + `sentry` default (`native-tls`) | **fails**: same | — | — |

Reading: the proposed stack is in line with Sentry, LaunchDarkly and Logfire. reqwest, hyper and
rustls cost about +14 s. The crypto provider barely matters here: `ring` instead of aws-lc-rs
saves about 1 s and 5 crates. The generated `agentcat-api` crate costs about **+8 s and 10
crates** on top of reqwest. It pulls `serde_with` and its proc macros only for one
`double_option` attribute on `session_id`
(`agentcat-api-0.0.3/src/models/publish_event_request.rs:23`). A blocking ureq worker would be
far leaner, which is why Sentry offers a ureq transport (`sentry-rust/sentry/Cargo.toml:57-69`). Recommendation 29 explains
why we still keep reqwest.

### D.3 Cargo feature layouts

| Vendor | Default features | Notes |
|---|---|---|
| Sentry | `backtrace`, `contexts`, `debug-images`, `logs`, `metrics`, `panic`, `transport` (= `reqwest` + `native-tls`), `release-health` (`sentry-rust/sentry/Cargo.toml:23-33`) | Transports: `reqwest`, `curl`, `ureq`, `embedded-svc-http`. TLS: `native-tls`, `rustls`, `rustls-no-provider`. Its reqwest dependency also turns on `blocking` and `json`, which the code does not use (`sentry-rust/sentry/Cargo.toml:84`). Docs mention a `ureq-native-tls` feature that does not exist (`sentry-rust/sentry/src/lib.rs:79,123`) |
| OTLP | `http-proto`, `reqwest-blocking-client`, `trace`, `metrics`, `logs`, `internal-logs` (`opentelemetry-rust/opentelemetry-otlp/Cargo.toml:67`) | Manifest comments say which clients work with which processors (`opentelemetry-rust/opentelemetry-otlp/Cargo.toml:94-103`) |
| Datadog | `default = []` (`dd-trace-rs/datadog-opentelemetry/Cargo.toml:101`) | Every libdatadog dependency uses `default-features = false`; workspace policy declares each external dependency once with default features off (`libdatadog/Cargo.toml:115-120`) |
| PostHog | `async-client`, `error-tracking` | `async-client` changes the **type** of `Client`, so the feature is not additive (`posthog-rs/src/client/mod.rs:49-67`) |
| LaunchDarkly | `hyper-rustls-native-roots`, `crypto-aws-lc-rs`, `event-compression`, `float-roundtrip` | TLS and crypto backends are separate axes |
| Logfire | `data-dir`, `export-http-protobuf`, `tls-aws-lc` | Separates TLS transport from crypto provider after churn (`logfire-rust/logfire/CHANGELOG.md:8-9`) |
| Statsig | `reqwest` | tonic, prost, brotli, zstd, rkyv, memmap2, uaparser and `simple_logger` are always on (`statsig-server-core/statsig-rust/Cargo.toml:48-59`) |
| axiom-rs | `tokio`, `default-tls` | Mutually exclusive features via `compile_error!` |

Rules the good layouts share: features gate dependencies, not behavior; features are additive;
TLS backend and crypto provider are separate choices; default features stay small (Datadog) or
at least build everywhere (not Sentry's `native-tls` default).

### D.4 serde and the wire

- serde + serde_json everywhere except Datadog's trace path (hand-written `rmp`) and OTLP
  (prost).
- `serde_json/preserve_order` changes JSON key order for the **whole** customer build.
  libdatadog bans it in `cargo-deny` (`libdatadog/deny.toml:25-30`); Statsig turns it on
  unconditionally (`statsig-server-core/statsig-rust/Cargo.toml:40-44`). Keep it opt-in, as `03`
  §5.4 proposes.
- Serialize once, outside the retry loop, into `Bytes` so a retry is a cheap clone (OTLP
  `opentelemetry-rust/opentelemetry-otlp/src/exporter/http/mod.rs:441-446`; axiom `axiom-rs/src/http.rs:28,93-96`).

### D.5 Hot-path allocation, sampling and lazy work

- **Static strings without copies.** OTel's `OtelString { Owned(Box<str>), Static(&'static str), RefCounted(Arc<str>) }`
  with a `const fn from_static_str` (`opentelemetry-rust/opentelemetry/src/common.rs:34-36,113-117`).
  Sentry stores option strings as `Cow<'static, str>` (`sentry-rust/sentry-core/src/clientoptions.rs:156-196`).
- **Copy-on-write shared state.** Sentry scopes are `Arc`s mutated with `Arc::make_mut`
  (`sentry-rust/sentry-core/src/hub_impl.rs:226-229`). Logfire put its handle behind an `Arc` so
  calls stop cloning about 2 KiB (`logfire-rust/logfire/src/logfire.rs:67-71`).
- **Arenas and pools.** Datadog converts spans in a bumpalo arena and reuses a 10,000-span pool
  (`dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:333-335,444`).
- **Small channel slots.** PostHog boxes events (`posthog-rs/src/client/transport.rs:35-38`).
  OTel boxes log records (`opentelemetry-rust/opentelemetry-sdk/src/logs/batch_log_processor.rs:84`).
- **Do not allocate for an item you will drop.** tracing-appender calls `buf.to_vec()` before
  `try_send`, even when the line is then dropped (`tracing/tracing-appender/src/non_blocking.rs:242`).
- **Sample first.** Datadog decides at span start, so unsampled spans skip everything
  (`dd-trace-rs/datadog-opentelemetry/src/span_processor.rs:611-613`); libhoney samples before the
  queue (`libhoney-rust/src/event.rs:81-86`). Sentry checks `sample_rate` **last**, after scopes,
  integrations and `before_send` (`sentry-rust/sentry-core/src/client/mod.rs:384-390`).
- **Lazy work.** Sentry breadcrumbs accept closures (`sentry-rust/sentry-core/src/breadcrumbs.rs:39-45`).
  PostHog moved `before_send` and serialization to the worker in 0.14.0.
- **Measure self-cost.** OTel's optional processed-span metric costs "~1.8 ns" with bound
  instruments versus "roughly 50 ns" without (`opentelemetry-rust/docs/design/observability.md:51-57`).

### D.6 MSRV policy and CI enforcement

| Vendor | `rust-version` | Policy | How CI enforces it |
|---|---|---|---|
| rmcp 3.x | 1.88 | MSRV bump is a minor release (`03` §5.3) | — |
| Sentry | 1.88 (`sentry-rust/Cargo.toml:26`) | "tries to support a _6 months_ old Rust version" (`sentry-rust/README.md:108-110`) | `[stable, 1.88.0]` × Linux, macOS, Windows (`sentry-rust/.github/workflows/check.yml:14-15`; `sentry-rust/.github/workflows/test.yml:14-15`); `sentry-rust/clippy.toml` `msrv` |
| OTel | 1.75.0 | Stable plus the three previous minors; a bump is not semver-breaking (`opentelemetry-rust/README.md:144-155`) | `cargo msrv verify` per crate (`opentelemetry-rust/.github/workflows/ci.yml:132-157`) |
| tracing | 1.71.0 (appender says 1.63) | Same three-version rule (`tracing/README.md:262-272`) | Toolchain matrix; the appender's 1.63 is never tested |
| libdatadog | 1.87.0 | Must not exceed what Alpine and RHEL ship (`libdatadog/Cargo.toml:101-107`) | Default toolchain pinned to the MSRV (`libdatadog/rust-toolchain.toml`) |
| dd-trace-rs | 1.87.0 | — | `cargo hack --each-feature clippy` on 1.87.0, stable, nightly (`dd-trace-rs/.github/workflows/lint.yaml:43-68`) |
| PostHog | 1.78.0 | — | `cargo check --locked` on MSRV for both clients (`posthog-rs/.github/workflows/ci.yaml:90-97`) |
| LaunchDarkly | 1.97.0 | Third-newest stable | A daily job reads endoflife.date and opens a bump PR (`rust-server-sdk/.github/workflows/check-rust-versions.yml:17-61`) |
| Logfire | 1.91 | — | CI reads `rust-version` with Python `tomllib` and adds it to the matrix (`logfire-rust/.github/workflows/main.yml:35-60`) |
| Statsig, axiom | none / 1.60 | — | None |

Quality tools found in CI workflows:

| Tool | Used by |
|---|---|
| `cargo msrv verify` | OTel |
| `cargo hack` | OTel (`--each-feature`, `opentelemetry-rust/scripts/lint.sh:18`), tracing (`--feature-powerset`, `tracing/.github/workflows/CI.yml:71-121`), dd-trace-rs (`--each-feature`) |
| `cargo semver-checks` | OTel (on labelled PRs, `opentelemetry-rust/.github/workflows/semver.yml:13-27`), libdatadog |
| `cargo public-api` snapshot | PostHog (`posthog-rs/.github/workflows/ci.yaml:59-73`), libdatadog |
| `cargo check-external-types` | OTel (`opentelemetry-rust/.github/workflows/ci.yml:127-129`) |
| `cargo deny` | OTel, libdatadog, dd-trace-rs (bans block merges, advisories only comment) |
| `cargo minimal-versions check` | tracing (`tracing/.github/workflows/CI.yml:158-166`) |
| `cargo machete` / `cargo shear` | libdatadog / OTel |
| Miri | OTel (smoke), libdatadog |
| Fuzzing | libdatadog (cargo-bolero) |
| RustSec audit | Sentry (weekly), tracing (daily) |
| Docs with `-D warnings` | Sentry, OTel |
| Coverage (`cargo-llvm-cov`) | Sentry, OTel, libdatadog, Logfire |

No vendor runs all of them. OTel comes closest. Sentry runs none of `cargo-hack`,
`semver-checks`, `cargo-deny` or `cargo-msrv`; the phantom `ureq-native-tls` feature in its docs
is the kind of drift a feature-matrix check catches.

---

## E. Public API design

| Practice | Who does it | Evidence |
|---|---|---|
| Options with builder setters and private fields | Sentry since 0.49, OTel, LaunchDarkly, PostHog, Logfire | Sentry: "`ClientOptions` is now `#[non_exhaustive]` ... must now be constructed with the builder-style setters" (`sentry-rust/CHANGELOG.md:38-55`) |
| Public struct fields with `..Default::default()` | Statsig, libhoney, RudderStack, Sentry before 0.49 | RudderStack had to release 2.0.0 to add `retry_config` (`rudder-sdk-rust/CHANGELOG.md:10-15`) |
| `#[must_use]` on builders, guards and options | Sentry (`sentry-rust/sentry-core/src/clientoptions.rs:141`), Logfire (`logfire-rust/logfire/src/config.rs:25`), tracing-appender, Datadog `WorkerHandle` | — |
| `#[non_exhaustive]` on errors and growable enums | Sentry, LaunchDarkly, PostHog, Logfire, axiom | OTel's ADR disagrees for closed error enums: "Don't use `#[non_exhaustive]` by default" (`opentelemetry-rust/docs/adr/001_error_handling.md:174-175`) |
| Sealed traits | Sentry `IntoProtocolMetric` | `sentry-rust/sentry-core/src/metrics.rs:144-145` |
| `impl Into<Cow<'static, str>>` | Sentry setters, OTel `tracer(name)` | `sentry-rust/sentry-core/src/clientoptions.rs:326-328`; `opentelemetry-rust/opentelemetry-sdk/src/trace/provider.rs:282` |
| `impl Into<String>` | Logfire, axiom, LaunchDarkly `track_event` | `rust-server-sdk/launchdarkly-server-sdk/src/client.rs:759` |
| Callback type alias `Arc<dyn Fn(T) -> Option<T> + Send + Sync>` with a generic setter | Sentry `BeforeCallback<T>` | `sentry-rust/sentry-core/src/clientoptions.rs:14,576-585` |
| Callbacks wrapped in `catch_unwind` | PostHog | `posthog-rs/src/client/common.rs:61-92` |
| Errors with `thiserror` | OTel ADR ("Use thiserror by default"), LaunchDarkly, Logfire, axiom | `opentelemetry-rust/docs/adr/001_error_handling.md:170-172` |
| Error strings for logging only | OTel `InternalFailure(String)` | `opentelemetry-rust/opentelemetry-sdk/src/error.rs:30-36` |
| Public API listed as a contract | PostHog: re-export list "any change to this is breaking" | `posthog-rs/src/lib.rs:104-167` |
| No dependency types in public API | Checked by OTel's `check-external-types`; broken by Sentry's `with_client(reqwest::Client)` | `sentry-rust/CHANGELOG.md:213` |
| Typestate builder | OTLP: `.with_http()` or `.with_tonic()` before `.build()` | `opentelemetry-rust/docs/design/otlp.md:52` |
| Own transport trait | LaunchDarkly `HttpTransport`, Statsig `NetworkProvider`, OTel `HttpClient`, Sentry `Transport` | `opentelemetry-rust/opentelemetry-http/src/lib.rs:137-147`; `sentry-rust/sentry-core/src/transport/mod.rs:15-34` |
| Migration guides | OTel `opentelemetry-rust/docs/migration_0.28.md`, `opentelemetry-rust/docs/migration_0.29.md` | — |
| `compile_fail` doctest for a type property | Sentry proves `Hub` guard is `!Send` | `sentry-rust/sentry-core/src/hub_impl.rs:44-49` |

Sentry also turned on the clippy lint `impl_trait_in_params` so public generics stay
turbofish-friendly (`sentry-rust/Cargo.toml:28-31`).

Sentry's `send_envelope` returns nothing, and an open issue asks for a `Result` so offline
caching becomes possible (https://github.com/getsentry/sentry-rust/issues/482). Design the
internal sink trait to return an outcome from the start.

---

## F. Testing

| Vendor | Test sink | HTTP mock | Deterministic time | Other |
|---|---|---|---|---|
| Sentry | `TestTransport`, `with_captured_events` (`sentry-rust/sentry-core/src/test.rs:28-132`) | None | None | 1 criterion bench |
| OTel | `InMemorySpanExporter` behind `testing` (`opentelemetry-rust/opentelemetry-sdk/src/trace/in_memory_exporter.rs:50-52`) | None; in-process fake endpoint for retry | None (real sleeps) | testcontainers OTel Collector; stress tests |
| Datadog | Test agent container with snapshots (`libdatadog/libdd-trace-utils/src/test_utils/datadog_test_agent.rs:16-29`) | httpmock | tokio `test-util`, `SleepCapability` | proptest, system-tests |
| PostHog | `pending_events` under `test-harness` | httpmock 0.8 | **Injected `Clock` + `ManualClock` + `Tick`** (`posthog-rs/src/client/transport.rs:81-99`) | Real-time test that shutdown bounds a stalled send (`posthog-rs/src/client/transport.rs:1546-1575`); cross-SDK compliance harness |
| LaunchDarkly | `InMemoryEventSender` (`rust-server-sdk/launchdarkly-server-sdk/src/events/sender.rs:208-243`) | mockito | `start_paused` | Contract tests against the shared SDK test harness |
| Statsig | — | wiremock, mockito | Env-var time overrides | `rusty-fork`, a `libc::fork` test |
| Logfire | OTel in-memory exporters | **wiremock** as OTLP endpoint (`logfire-rust/logfire/tests/test_http_sink.rs:4-22`) | Deterministic ids and timestamp remap (`logfire-rust/logfire/src/test_utils.rs:31,64-85`) | insta snapshots |
| axiom-rs | — | httpmock, incl. a 429 case | — | Live tests against dev and staging |
| libhoney | Public `TransmissionMock` for downstream users (`libhoney-rust/src/mock.rs:12-21`) | mockito | — | — |
| RudderStack | — | Hand-written `TcpListener` server | Zero delays | — |

The most useful patterns for us: a sink trait with an in-memory implementation (everyone),
an injected clock or a paused tokio clock for retry and backoff tests (PostHog, LaunchDarkly),
a real HTTP mock for status and header handling (wiremock or httpmock), and one real-time test
that proves shutdown is bounded when the server stalls (PostHog).

---

## G. Release practice

| Vendor | Changelog and release tooling | Publish | docs.rs | Docs lint | `unsafe` |
|---|---|---|---|---|---|
| Sentry | CHANGELOG with Breaking/Features/Fixes sections; getsentry/craft (`sentry-rust/.craft.yml:1-12`); lockstep versions | Manual `workflow_dispatch` (`sentry-rust/.github/workflows/release.yml:3-42`) | `all-features`, `--cfg doc_cfg` (`sentry-rust/sentry/Cargo.toml:18-21`) | `missing_docs = "warn"` + CI `-Dwarnings` | `deny(unsafe_code)` in some sub-crates only |
| OTel | Per-crate CHANGELOG with `## vNext`; manual process (`opentelemetry-rust/scripts/RELEASING.md:24-53`); Conventional Commit PR titles | Manual tag + publish | `all-features`, `--cfg docsrs` | `warn(missing_docs, unreachable_pub, ...)` (`opentelemetry-rust/opentelemetry-sdk/src/lib.rs:149-163`) | No `unsafe` blocks; no `forbid` |
| tracing | Keep a Changelog; `bin/publish` runs a cargo-hack powerset before publishing (`tracing/bin/publish:28-59`) | Script | — | `warn(missing_docs)` | — |
| libdatadog | git-cliff per crate, cargo-release, semver-checks in a release-proposal workflow | Workflow | — | — | Uses `unsafe` (FFI) |
| dd-trace-rs | cargo-release + git-cliff release PR (`dd-trace-rs/docs/releasing.md`) | Tag-triggered, protected environment, second approver | `--cfg docsrs` | `#![deny(missing_docs)]` | Uses `unsafe` |
| PostHog | Sampo changesets (`posthog-rs/RELEASING.md:3-19`) | Approved in Slack | — | — | None used |
| LaunchDarkly | release-please | Workflow | `features = []` | `#![deny(missing_docs)]`, `deny(rustdoc::missing_crate_level_docs)` (`rust-server-sdk/launchdarkly-server-sdk/src/lib.rs:9-10`) | — |
| Logfire | CHANGELOG with PR links; `release.sh` checks versions | GitHub Release → approval environment → **crates.io trusted publishing (OIDC)** (`logfire-rust/.github/workflows/main.yml:117-136`) | — | `missing_docs = "warn"` | Needs `unsafe` for a tracing downcast |
| RudderStack | release-please | **OIDC trusted publishing** with a tag-to-version check | — | — | — |
| Statsig, axiom | None | `cargo publish` on release or tag | — | axiom: `deny(missing_docs)` | — |

No vendor uses release-plz. The common shape is: Conventional Commit PR titles, a generated
changelog (git-cliff, release-please or changesets), a release PR, a tag, and a gated publish
job. Trusted publishing over OIDC (Logfire, RudderStack) removes the long-lived crates.io token.

---

## Recommendations for agentcat-sdk

### R.0 Verdict on each part of the proposed design

| Proposed element (`02` §3.5, `03` §0, §5.2) | Verdict | Change |
|---|---|---|
| One dedicated OS thread for delivery | **Right** | Name it `agentcat-publisher`. Handle a spawn failure (item 3). |
| Its own `current_thread` tokio runtime | **Right** | Build it inside the thread. One runtime for AgentCat and all exporters. |
| Async reqwest 0.13 via `agentcat-api` | **Right, with changes** | Build the `reqwest::Client` yourself; do not use `Configuration::default()`; read response headers (item 9). |
| `publish()` = `tokio::sync::mpsc::Sender::try_send` into a bounded queue | **Right** | Add a byte budget and drop counters (items 5-7). |
| `shutdown(timeout).await` to flush | **Right; better than any vendor** if the deadline lives on the worker | Item 15. |
| `Drop` of the last handle does a bounded blocking flush | **Right, with four guards** | Item 17. |
| "Spawn the worker on the current runtime (`Handle::try_current()`)" (`02` §3.5) | **Wrong** | Delete. It is Statsig's design and fails (#52, Logfire #141). |
| Drop-oldest at 10,000 (`02` §3.5, TS parity) | **Wrong for Rust** | Drop newest; document the divergence from TS. |
| 5 in flight, 3 retries at 1/2/4 s, 10 s per attempt (`03` §5.2) | **Mostly right** | Add jitter, a retryable set, `Retry-After`, a shared pause, a connect timeout, and the shutdown deadline cap (items 10-13). |
| Default features include all exporters and diagnostics (`03` §5.4) | **Too heavy** | Exporters opt-in; behavior toggles become runtime options (item 30). |
| Drop fallback default 2 s (`03` §5.2) | **Right** | Same as Sentry's `shutdown_timeout`. |

### R.1 Delivery

1. **Keep the dedicated thread and its own `current_thread` runtime.** 7 of 8 active vendors
   own their sender. The four closest precedents are Sentry, OTel's `TokioCurrentThread`,
   Logfire and Datadog telemetry (section 0). Build the runtime with
   `Builder::new_current_thread().enable_all().build()` **inside** the thread, and drop it there.
2. **Create, use and drop the `reqwest::Client` only on the worker thread.** Use
   `reqwest::Client::builder()...build()` and handle the `Err`. Never `reqwest::Client::new()`,
   which panics on TLS init failure. Never move a runtime or client to a customer thread.
3. **Fail open at startup.** If `thread::Builder::spawn` fails, if the runtime build fails, or if
   the client build fails, log one warning and run as a disabled client: `publish()` counts
   drops and returns. No `.unwrap()`, no `.expect()` (Sentry #530; OTel `opentelemetry-rust/opentelemetry-sdk/src/trace/span_processor.rs:687`).
4. **Spawn only through the worker's own handle.** Every exporter and retry task runs on the
   AgentCat runtime. Never call ambient `tokio::spawn` from SDK code that can run on a customer
   thread (Logfire #141).
5. **Data channel: `tokio::sync::mpsc::channel::<Box<Event>>(10_000)`.** `try_send` works with
   or without a runtime and never blocks. Box the event to keep slots small (PostHog). The
   worker uses `recv().await`, not a blocking std `recv` inside `block_on` (Sentry's pattern
   starves hyper's tasks).
6. **Bound everything in flight with one budget, by count and by bytes.** Count: 10,000 events
   across channel, in-flight sends and waiting retries (PostHog's single gate). Bytes: 32 MiB of
   estimated size (fixed 512 B plus string lengths of arguments and response text), reserved
   with an `AtomicUsize` before `try_send` and released by the worker (Datadog's estimate
   approach, `dd-trace-rs/datadog-opentelemetry/src/span_exporter.rs:43-49`). Without the byte
   cap, 10,000 events of up to 100 KB each can hold about 1 GB.
7. **Drop the newest event when full. Count every drop by reason.** Reasons: `QueueFull`,
   `BytesFull`, `AfterShutdown`, `Disabled`, `ForkedChild`, `HookDropped`, `HookPanicked`,
   `Invalid`, `NonRetryableStatus`, `RetriesExhausted`, `DeadlineExceeded`. Use
   `[AtomicU64; N]` (Sentry's matrix). Warn once per full episode and re-arm after the queue
   drains (PostHog). Log totals at shutdown (OTel). Expose `AgentCat::stats() -> Stats`.
   Count before `try_send` (OTel #3453). A 429 is not a client loss until retries are exhausted.
8. **Control channel separate from data: `tokio::sync::mpsc::channel::<Control>(16)`.**
   `Control::{Flush { deadline, reply }, Shutdown { deadline, reply }}`. A full data queue can
   then never block or swallow a flush (OTel #2478).
9. **Do the HTTP call for publishing yourself; keep `agentcat-api` for the models.** The generated
   `publish_event` drops the response headers (`agentcat-api-0.0.3/src/apis/mod.rs:5-8`), so it
   cannot honor `Retry-After`. Serialize `models::PublishEventRequest` once into `Bytes`, then
   `client.post(url).header(CONTENT_TYPE, "application/json").body(bytes.clone())`. Set
   `User-Agent: agentcat-rust/<version> rmcp/<version>` (`02` §2.9). Alternatively, patch the
   generator template so `ResponseContent` carries `headers`. Then trim `agentcat-api`: drop
   reqwest `multipart`, `form` and `query`, and replace `serde_with::rust::double_option` with a
   10-line local helper. If the generator cannot do that, hand-write `PublishEventRequest`
   (`02` §2.9 already allows it) and keep `agentcat-api` out of the runtime tree. Measured
   saving: 10 crates and about 8 s of clean build.
10. **In-flight concurrency: 5** (TS and Go parity). Use a `JoinSet` plus a `Semaphore(5)` on
    the worker runtime. Release the permit while a retry sleeps; the waiting retry still counts
    in the budget of item 6 (PostHog's retry queue).
11. **Retry policy.**
    - Retry: connect errors, timeouts, other I/O errors, 408, 429, 500, 502, 503, 504.
    - Do not retry: every other 4xx (400, 401, 403, 404, 413, 415, 422) and 501. Count and log at
      debug; warn once per status code.
    - Up to **3 retries** (4 attempts) at **1 s, 2 s, 4 s with ±20% jitter** (TS schedule plus
      the jitter that PostHog, LaunchDarkly, Statsig and Datadog all lack).
    - `Retry-After` (seconds or HTTP date) is a minimum delay, capped at **30 s** (OTLP
      `opentelemetry-rust/opentelemetry-otlp/src/retry.rs:42-45`; RudderStack's uncapped wait is the anti-pattern).
    - Same event `id` on every attempt (`02` §2.9).
12. **On 429 or 503, pause all sends, not just one.** Keep one `paused_until: Instant` on the
    worker, as Sentry's rate limiter does, but use `Instant`, not `SystemTime`
    (`sentry-rust/sentry/src/transports/ratelimit.rs:12-20`), so wall-clock jumps do not matter.
13. **Timeouts: `connect_timeout(5 s)`, `timeout(10 s)` per attempt.** During flush or shutdown,
    cap each request at the time left before the deadline (PostHog `bound_request`,
    `posthog-rs/src/client/transport.rs:393-406`). At the deadline, drop the in-flight futures;
    an async client is what makes this cancellation possible.
14. **No batching and no compression in v1.** The API takes one event per request. File two
    server requests: `POST /v1/public/events/batch` (up to 100 events or 1 MiB, then flush every
    1 s like Datadog and OTel logs) and `Content-Encoding: gzip` support. Gzip on the worker only.

### R.2 Lifecycle

15. **`shutdown(timeout).await` and `flush(timeout).await` await only a oneshot.** The caller
    sends `Control::Shutdown { deadline: Instant::now() + timeout, reply }` and awaits the
    `tokio::sync::oneshot::Receiver`. No `tokio::time` on the customer's runtime, so a runtime
    without timers cannot panic (Datadog had to catch "timers are disabled"). The worker owns
    the deadline and always replies by it. If the worker is gone, the receiver errors at once.
    - Default: **5 s** (TS, OTel).
    - One deadline for the whole sequence: drain queue → wait in-flight → exporters → reply.
    - Idempotent via `AtomicBool::compare_exchange`; a second call returns `AlreadyShutdown`.
    - After shutdown, `publish()` counts `AfterShutdown` and returns.
    - Return a typed outcome:
      `Result<FlushReport, ShutdownError>` with
      `#[non_exhaustive] pub struct FlushReport { pub delivered: u64, pub pending: u64, pub dropped: DropCounts }` and
      `#[non_exhaustive] pub enum ShutdownError { AlreadyShutdown, Timeout { pending: u64 }, WorkerUnavailable }`.
16. **Detect calls from the worker thread.** Store the worker's `ThreadId`. If a customer hook
    calls `flush`, `shutdown` or `Drop` runs on the worker, only signal and return
    (PostHog 0.25.1 deadlock).
17. **`Drop` of the last handle: bounded blocking flush, with four guards.** Default **2 s**,
    configurable through `flush_on_drop(Duration)`, `Duration::ZERO` disables.
    1. Run only in the `Drop` of the shared inner, not per clone.
    2. Wait with a **std** `mpsc::Receiver::recv_timeout`. Do not use `oneshot::blocking_recv`,
       which panics inside an async context, and never `block_on`.
    3. Never `join()` the worker without a deadline; detach it after the timeout.
    4. Never panic; on the worker thread, only signal (item 16).
    Document that the wait blocks the dropping thread, as PostHog does.
18. **Handle-only API.** `AgentCat` is `Clone` over `Arc`. No global in v1. Multiple instances
    are allowed; warn once if two live instances share a project id (TS is first-wins).
19. **Process exit.** Install no signal handlers and no `atexit`. Document: call
    `agentcat.shutdown(Duration::from_secs(5)).await` after `running.waiting().await` (stdio) or
    after `axum::serve(..).with_graceful_shutdown(..)` (HTTP). `process::exit`, SIGKILL and
    `panic = "abort"` lose queued events. The `Drop` fallback covers a forgotten shutdown when
    `main` returns.
20. **Fork.** Record `std::process::id()` at build. In `publish()`, a changed pid counts
    `ForkedChild`, warns once, and drops. Document "build `AgentCat` after any fork or daemonize
    step". Do not copy libdatadog's pause-and-restart machinery; Datadog's own Rust SDK does not.
21. **Serverless.** Document `flush(timeout).await` before returning from a Lambda handler
    (Logfire #113, LaunchDarkly `flush_blocking`). A `workers-rs` feature stays as in `03`.

### R.3 Safety

22. **Crate-level lints in every crate:**
    ```rust
    #![forbid(unsafe_code)]
    #![deny(missing_docs)]
    #![cfg_attr(not(test), deny(
        clippy::unwrap_used, clippy::expect_used, clippy::panic,
        clippy::todo, clippy::unimplemented, clippy::indexing_slicing,
        clippy::print_stdout, clippy::print_stderr,
    ))]
    ```
    This is libdatadog's set, plus Logfire's print lints, plus `indexing_slicing`. No vendor
    forbids `unsafe` at the top crate; we can, because we need none.
23. **`catch_unwind(AssertUnwindSafe(..))` at three boundaries.** (a) Every customer hook
    (`identify`, `redact`, `redact_event`, `event_tags`, `event_properties`,
    `resolve_session_id`), as PostHog does; a panic counts `HookPanicked` and the event is
    dropped or sent unredacted per a documented rule (drop is safer). (b) The SDK's own pre- and
    post-call code in `Tracked<H>`, so a bug there falls back to calling the inner handler
    untracked. (c) Each event's sync pipeline on the worker (redact, sanitize, truncate,
    serialize), so one bad event cannot kill delivery for the process. No vendor does (c); OTel
    #3640 and libdatadog's `TaskAborted` show the cost. Document that `panic = "abort"`
    disables all three.
24. **No locks on the hot path.** `publish()` touches only atomics and the channel. For cold-path
    locks, use std `Mutex` with `lock().unwrap_or_else(PoisonError::into_inner)`; do not add
    `parking_lot`.
25. **Diagnostics through `tracing` events only, target `agentcat`, never spans.** rmcp already
    depends on `tracing`, so it is free. Use stable names such as `agentcat.queue.full`,
    `agentcat.send.failed`, `agentcat.shutdown.dropped`. Warn at most once per episode. Log
    response bodies only at `debug`. Never install a global subscriber or logger (Statsig's
    `simple_logger`). A `debug(true)` option raises our own verbosity only.
26. **Feedback loops.** AgentCat records MCP tool calls, not logs, so our diagnostics cannot
    become AgentCat events. Still:
    - Document the filter `hyper=off,h2=off,reqwest=off` for customers who forward `tracing`
      to Sentry or OTel (OTel's own advice, `opentelemetry-rust/docs/logs.md:27-36`).
    - Hooks run on the worker thread and must not call back into AgentCat's flush or shutdown
      (item 16). `publish_custom_event` from a hook is safe because it is a `try_send`.
    - If an `opentelemetry` integration ever lands, enter
      `Context::enter_telemetry_suppressed_scope()` on the worker thread (OTel, Logfire).
    - Exporters receive each event once, outside the retry loop (fixes the TS defect in `02` §3.2).
27. **Keep the request path free of SDK work that can wait.** On the tool-call path do only what
    control flow needs: resolve handles, run `identify` if configured, move owned data into the
    event, `try_send`. Token estimates, redaction, sanitization, truncation, validation and
    serialization run on the worker. Check `enabled` first and skip all work when tracking is
    off (Sentry samples last; Datadog decides first).
28. **TLS.** Never install a process-wide rustls `CryptoProvider` (libdatadog does). With
    `rustls-no-provider`, check for a provider at build time and disable with a clear warning
    if none exists, instead of letting reqwest panic (Logfire `logfire-rust/logfire/src/exporters.rs:460-479`). When both
    TLS features are on, pick the backend explicitly in code.

### R.4 Leanness and performance

29. **Accept the reqwest cost; do not add more.** The proposed stack measures **+79 crates and
    +22 s** clean debug build over an rmcp stdio server, and **+59 crates** over an rmcp HTTP
    server. That matches Sentry (+84 lean), Logfire and LaunchDarkly (+106). A blocking ureq
    worker would cost only +24 crates and about 0 s, but it gives up `agentcat-api`, async
    cancellation at the shutdown deadline, and the shared async stack for exporters. Keep
    reqwest for v1. Trim `agentcat-api` (item 9) to get to about +69 crates and +14 s. Revisit a
    `ureq` transport feature (Sentry has one) if customers ask.
30. **Proposed cargo features** (facade `agentcat-sdk`; features gate dependencies, runtime
    options gate behavior):
    ```toml
    [features]
    default = ["rmcp", "rustls", "http-capture"]

    rmcp = ["dep:agentcat-rmcp"]                     # rmcp 3.x adapter: Tracked<H>, TrackedService<S>
    http-capture = ["agentcat-rmcp?/http"]           # read http::request::Parts (headers for identify)

    # TLS for the AgentCat API and exporters. Additive; pick one in code when several are on.
    rustls = ["agentcat-core/rustls"]                # reqwest/rustls (aws-lc-rs), the reqwest 0.13 default
    rustls-no-provider = ["agentcat-core/rustls-no-provider"]  # bring your own CryptoProvider (ring, FIPS)
    native-tls = ["agentcat-core/native-tls"]        # OpenSSL on Linux: needs libssl-dev to build

    # Exporters: hand-written JSON over the same client, zero extra crates, off by default.
    otlp = ["agentcat-core/otlp"]
    datadog = ["agentcat-core/datadog"]
    sentry = ["agentcat-core/sentry"]
    posthog = ["agentcat-core/posthog"]
    exporters = ["otlp", "datadog", "sentry", "posthog"]

    preserve-order = ["agentcat-core/preserve-order"]  # serde_json/preserve_order: global side effect, opt-in
    test-util = ["agentcat-core/test-util"]          # in-memory sink and test hooks; not for production
    ```
    - Drop `diagnostics` and `file-log` as features. They add no dependencies; make them builder
      options (`disable_diagnostics(true)`, `log_file(path)`) plus the env vars.
    - No `compile_error!` when no TLS feature is on (axiom's anti-pattern breaks
      `--no-default-features`). Without TLS, an `https` base URL disables the client with one
      warning at build.
    - Depend on `agentcat-api` with `default-features = false` and no TLS feature; select TLS
      through our own `reqwest` dependency of the same major.
    - Declare every external dependency once at workspace level with `default-features = false`
      (libdatadog's rule, `libdatadog/Cargo.toml:115-120`).
31. **Hot path allocations.** One `Box<Event>` per call. Move owned data in; the only required
    copy is the `arguments` map, because the inner handler consumes the request. Constant tag
    keys are `&'static str` or `Cow<'static, str>`. Serialize once into `Bytes` on the worker;
    retries clone the `Bytes`. Keep `serde_json` without `preserve_order` by default.
32. **No sampling in v1.** Analytics customers expect every tool call. If `sample_rate` comes
    later, decide it first, before any event work.

### R.5 Public API

33. **Builder with private fields; `#[must_use]` on the builder and on `AgentCat`.**
    `AgentCat::builder(project_id: impl Into<String>)` → setters → `build() -> AgentCat`
    (infallible, degrades to disabled with one warning) and `try_build() -> Result<AgentCat, BuildError>`.
    No public option structs with public fields (RudderStack 2.0.0, Sentry 0.49.0).
34. **`#[non_exhaustive]` on every public enum and on every output struct** (`BuildError`,
    `ShutdownError`, `FlushReport`, `DropCounts`, `Stats`, `ExporterConfig`). Errors use
    `thiserror` 2, which rmcp already pulls in.
35. **Callbacks:** store `Arc<dyn Fn(..) + Send + Sync + 'static>`; setters take a generic
    `F: Fn(..) + Send + Sync + 'static` (Sentry's `BeforeCallback`). Document that hooks run on
    the worker thread (except `identify`, which runs on the request path), must be fast, must not
    block, and must not call `flush` or `shutdown`.
36. **Strings:** `impl Into<String>` for configuration values (cold path). Use
    `Cow<'static, str>` internally for per-event constants.
37. **No reqwest, hyper or rustls types in the public API.** rmcp types are intended
    (`Tracked<H: rmcp::ServerHandler>`) and go on the allowlist of item 41. Re-export nothing
    from dependencies.
38. **Docs:** crate docs with stdio and streamable HTTP examples as `no_run` doctests;
    `examples/` for stdio, session HTTP, stateless HTTP, `Router<S>`, `identify`, custom events,
    and shutdown; docs.rs metadata `all-features = true` and `rustdoc-args = ["--cfg", "docsrs"]`.

### R.6 Testing

39. **Split the worker into a runtime-agnostic core and a thin thread wrapper.**
    `async fn run(rx, ctrl, sink: impl Sink, clock)` holds all logic; `spawn_publisher()` only
    builds the thread and runtime. Tests drive `run` under `#[tokio::test(start_paused = true)]`
    so backoff and deadlines are deterministic (LaunchDarkly, PostHog's `ManualClock`). The sink
    trait returns an outcome per event (Sentry #482).
40. **Test matrix.** Use `wiremock` (Logfire, Statsig) for the HTTP cases:
    - 202; 400 not retried; 429 with `Retry-After` in seconds and as an HTTP date; 429 without it;
      503 then 202; a connect timeout; a stalled server with shutdown that returns by its deadline
      (PostHog's real-time test).
    - Queue full and byte budget full: drops counted, one warning per episode.
    - `shutdown` from a `current_thread` runtime, a multi-thread runtime, a runtime built
      **without** `enable_time`, and from no runtime (`futures::executor::block_on`).
    - `Drop` inside an async task, at the end of `#[tokio::main]`, and on the worker thread.
    - A panicking hook and a panicking pipeline stage; delivery continues.
    - A forked child (`rusty-fork` or `libc::fork`, as Statsig does).
    - Ship an in-memory sink under `test-util` so customers can assert on events (Sentry
      `TestTransport`, OTel in-memory exporters, libhoney `TransmissionMock`).

### R.7 CI and release

41. **CI jobs:**
    - MSRV **1.88** (= rmcp 3.x = Sentry). `rust-version` in the workspace; a job reads it and
      runs `cargo check --locked --all-features` on it (Logfire's pattern) on Linux, macOS and
      Windows (Sentry's matrix). Policy text: "MSRV follows rmcp 3.x and is at least 6 months
      old; raising it is a minor release."
    - `cargo hack --each-feature --no-dev-deps check` and
      `cargo hack --feature-powerset --depth 2 clippy -- -D warnings` (OTel, tracing, dd-trace-rs).
    - `cargo clippy --all-targets --all-features -- -D warnings` on stable and MSRV.
    - `cargo doc --no-deps --all-features` with `RUSTDOCFLAGS="-D warnings --cfg docsrs"`.
    - `cargo test` with and without default features; doctests.
    - `cargo deny check` (advisories, licenses, bans, sources).
    - `cargo semver-checks` on every PR, not only labelled ones.
    - `cargo public-api` diff, or `cargo check-external-types` with an allowlist of `rmcp`,
      `http`, `serde`, `serde_json`, `chrono`.
    - `cargo minimal-versions check` (we use ranges like `rmcp = ">=3.0, <4"`).
    - `cargo shear` or `cargo machete` for unused dependencies.
    - Crypto check with `cargo tree -e features`: default pulls `aws-lc-rs` and no `openssl-sys`;
      `rustls-no-provider` pulls neither `ring` nor `aws-lc-rs`; `native-tls` pulls no `rustls`
      (Logfire, libdatadog).
    - Weekly: newest rmcp plus the trait tripwire (`03` §5.7), `rustsec/audit-check`, beta toolchain.
42. **Release:** Conventional Commit PR titles, a generated Keep a Changelog file, and
    release-plz (changelog, version bump PR, semver-checks, publish in one Rust-native tool). No
    vendor uses it yet; release-please (LaunchDarkly, RudderStack) is the fallback. Publish from a
    protected GitHub environment with a required approval, using crates.io trusted publishing
    over OIDC (Logfire, RudderStack). Version `agentcat-sdk` and its internal crates in lockstep
    (Sentry, dd-trace-rs).

### R.8 Numbers at a glance

| Setting | Value | Source of the choice |
|---|---|---|
| Worker | 1 OS thread `agentcat-publisher`, `current_thread` runtime | Sentry, Logfire, OTel adapter |
| Data channel | `tokio::sync::mpsc`, 10,000 `Box<Event>` | TS, PostHog |
| Byte budget | 32 MiB estimated | Datadog uses 5 MB for spans; our events are up to 100 KB |
| Control channel | `tokio::sync::mpsc`, 16 | OTel uses 64 |
| Full policy | Drop newest, count by reason, warn once per episode | OTel, Sentry, PostHog |
| In flight | 5 | TS, Go |
| Retries | 3, at 1/2/4 s ±20% jitter | TS + jitter |
| Retryable | network, timeout, 408, 429, 500, 502, 503, 504 | OTLP, PostHog |
| `Retry-After` cap | 30 s, also pauses all sends | OTLP, Sentry |
| Connect / request timeout | 5 s / 10 s; capped by the shutdown deadline | OTLP 10 s, PostHog cap |
| `shutdown` / `flush` default | 5 s | TS, OTel |
| `Drop` fallback | 2 s, `Duration::ZERO` disables | Sentry |
| Batch (when the API allows) | 100 events or 1 MiB, every 1 s | PostHog 100, Datadog 1 s |
| MSRV | 1.88 | rmcp 3.x, Sentry |

---

## Appendix: measurement commands

```sh
# one probe per row of D.1; Cargo.toml has only the [dependencies] under test
cargo generate-lockfile
cargo tree -e normal,build --prefix none \
  | sed -E 's/ \(\*\)$//; s/ \(proc-macro\)//' | sort -u | grep -v '^probe-' | wc -l

# D.2
cargo fetch && rm -rf target && time cargo build --offline -j 4
```

Probe crates, lockfiles, `count.sh`, `buildtimes.sh`, `buildtimes.txt` and the failed
`openssl-sys` build logs are in
`/tmp/claude-1000/-home-ubuntu-agentcat-rust-sdk/78031ef1-7fed-4ae1-94f2-11a86f1fe961/scratchpad/depcount/`.
