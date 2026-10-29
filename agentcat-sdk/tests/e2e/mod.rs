//! The end-to-end suite. A scenario (`scenarios/<name>.json`) runs against a tracked server over
//! one transport and one protocol lane, with the SDK publishing to a mock `AgentCat` API on
//! loopback. The client's view of the run and the published events, normalized, must equal the
//! golden file `golden/<name>/<transport>-<lane>.json`.
//!
//! - `scenario`: the scenario format.
//! - `server`: the tracked server: its tools, its options from the scenario's profile, and how
//!   each transport serves it.
//! - `driver`: the scripted client, raw JSON-RPC over an in-memory pipe or HTTP.
//! - `mock_api`: the mock `AgentCat` API that records the publish requests.
//! - `normalize`: the normalized document, and the one list of the values it replaces.
//! - `golden`: the comparison with the golden file, and `UPDATE_GOLDEN=1`.
mod driver;
mod golden;
mod mock_api;
mod normalize;
mod scenario;
mod server;

use std::time::Duration;

use agentcat_sdk::{AgentCat, LogDestination};
use serde_json::Value;

pub(crate) use golden::{assert_golden, assert_matches};

/// How long the SDK may take to deliver the queued events once the clients are closed.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// How the client reaches the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transport {
    /// Newline-delimited JSON-RPC over an in-memory pipe, one tracked server per connection.
    Stdio,
    /// Streamable HTTP with sessions, JSON responses where rmcp gives them.
    Http,
    /// Streamable HTTP with sessions and SSE responses (rmcp's default).
    HttpSse,
    /// Streamable HTTP without sessions: one tracked server per request.
    HttpStateless,
}

impl Transport {
    /// The name in golden file names.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::HttpSse => "http-sse",
            Self::HttpStateless => "http-stateless",
        }
    }
}

/// The protocol version the client speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    /// 2025-11-25: an `initialize` handshake starts each session.
    V2025_11_25,
    /// 2026-07-28: no handshake; `server/discover`, then every request carries its own `_meta`.
    V2026_07_28,
}

impl Lane {
    /// The protocol version, also the lane's name in golden file names.
    pub(crate) fn version(self) -> &'static str {
        match self {
            Self::V2025_11_25 => "2025-11-25",
            Self::V2026_07_28 => "2026-07-28",
        }
    }
}

/// Runs the scenario `name` over `transport` on `lane` and returns the normalized document of the
/// run (see `normalize`).
///
/// # Panics
///
/// When the environment changes what the SDK publishes (see [`check_environment`]), when the
/// scenario is malformed, when the run fails, when the SDK does not drain within
/// [`DRAIN_TIMEOUT`], when the mock received a request other than a publish, or when the number
/// of publishes is not the scenario's `expectPublishes`.
pub(crate) async fn run_scenario(name: &str, transport: Transport, lane: Lane) -> Value {
    check_environment();
    let scenario = scenario::Scenario::load(name);
    let mock = mock_api::MockApi::start().await;
    let options = server::options(&scenario.options)
        .api_base_url(mock.url())
        .log_destination(LogDestination::Tracing);
    let agentcat = AgentCat::new(scenario.project_id.clone(), options);
    let served = server::serve(transport, &agentcat, &scenario).await;
    let (transcript, clients) = driver::drive(&scenario, &served, lane).await;
    mock.wait_for_publishes(scenario.expect_publishes).await;
    clients.close().await;
    assert!(
        agentcat.shutdown(DRAIN_TIMEOUT).await,
        "the SDK did not drain its queue within {DRAIN_TIMEOUT:?}"
    );
    served.stop();
    let publishes = mock.publishes().await;
    assert_eq!(
        publishes.len(),
        scenario.expect_publishes,
        "the number of publishes of {name} over {} on {}",
        transport.name(),
        lane.version()
    );
    normalize::document(transcript, publishes, scenario.event_order)
}

/// Refuses an environment in which the SDK publishes other events than the goldens hold:
/// `AGENTCAT_ENABLED` turned off (no events), or backtraces turned on (`RUST_LIB_BACKTRACE`, else
/// `RUST_BACKTRACE`, set to anything but `0`: a captured error then carries a `stack`).
///
/// # Panics
///
/// When one of them is set so.
#[expect(
    clippy::disallowed_methods,
    reason = "the suite reads the variables that change what the SDK publishes"
)]
fn check_environment() {
    if let Ok(value) = std::env::var("AGENTCAT_ENABLED") {
        let off = matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "false" | "0" | "no" | "off"
        );
        assert!(
            !off,
            "AGENTCAT_ENABLED={value:?} turns the SDK off: unset it to run the end-to-end suite"
        );
    }
    let backtraces = match std::env::var("RUST_LIB_BACKTRACE") {
        Ok(value) => value != "0",
        Err(_) => std::env::var("RUST_BACKTRACE").is_ok_and(|value| value != "0"),
    };
    assert!(
        !backtraces,
        "backtraces are on (RUST_LIB_BACKTRACE, else RUST_BACKTRACE), so a captured error carries \
         a stack that the golden files do not hold: run the end-to-end suite with \
         RUST_LIB_BACKTRACE=0 (panics keep their backtraces)"
    );
}

/// Runs the end-to-end test `test`: the scenario `scenario` over `transport` on `lane`, compared
/// with its golden file (see [`assert_golden`]).
///
/// # Panics
///
/// When `test` is not `<scenario>_<transport>_<lane>` with `_` for each `-` (for example
/// `basic_http_sse_2026_07_28`), or as [`run_scenario`] and [`assert_golden`] do.
pub(crate) async fn run_case(test: &str, scenario: &str, transport: Transport, lane: Lane) {
    let expected = format!("{scenario}_{}_{}", transport.name(), lane.version()).replace('-', "_");
    assert_eq!(
        test, expected,
        "the test of {scenario} over {transport:?} on {lane:?}"
    );
    let document = run_scenario(scenario, transport, lane).await;
    assert_golden(scenario, transport, lane, &document);
}

/// One end-to-end test: `e2e_case!(<test>, <scenario>, <Transport>, <Lane>)` emits the test
/// `<test>`, which runs `scenarios/<scenario>.json` over the transport on the lane and compares the
/// document with `golden/<scenario>/<transport>-<lane>.json`. The test's name must be
/// `<scenario>_<transport>_<lane>`, for example
/// `e2e_case!(basic_http_sse_2026_07_28, basic, HttpSse, V2026_07_28)`: the test fails otherwise.
macro_rules! e2e_case {
    ($test:ident, $scenario:ident, $transport:ident, $lane:ident) => {
        #[tokio::test(flavor = "multi_thread")]
        async fn $test() {
            $crate::suite::run_case(
                stringify!($test),
                stringify!($scenario),
                $crate::suite::Transport::$transport,
                $crate::suite::Lane::$lane,
            )
            .await;
        }
    };
}

pub(crate) use e2e_case;
