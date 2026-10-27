//! `ApiPublisher` against a `wiremock` server on loopback (C44, RD-37): the request it sends,
//! the base URL it uses, and which answers are failed attempts that the worker retries. Every
//! test builds its handle through `AgentCat::with_delivery` with shrunk delays and posts to the
//! mock only, never to a production endpoint (P15).
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::{ApiPublisher, NO_CLIENT};
use crate::client::AgentCat;
use crate::config::{MapEnv, Options, resolve_api_base_url};
use crate::delivery::DeliveryConfig;
use crate::delivery::Publisher;
use crate::test_support::rmcp::{Sample, connect};

const PROJECT: &str = "proj_http";
const WAIT: Duration = Duration::from_secs(3);
const EVENTS_PATH: &str = "/v1/public/events/";

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn fast() -> DeliveryConfig {
    DeliveryConfig {
        max_pending: 10_000,
        concurrency: 5,
        retry_delays: [ms(10), ms(20), ms(40)],
        hook_budget: ms(50),
        drop_drain: ms(1_000),
    }
}

/// A handle posting to `base_url` with `attempt_timeout` per attempt.
fn handle(base_url: &str, attempt_timeout: Duration) -> AgentCat {
    let mut log = Vec::new();
    let publisher =
        ApiPublisher::with_attempt_timeout(base_url.to_owned(), attempt_timeout, &mut log);
    assert!(log.is_empty(), "{log:?}");
    AgentCat::with_delivery(PROJECT, Options::default(), Arc::new(publisher), fast())
}

/// Makes one tool call, which queues one event, and waits for the delivery.
async fn send_one(agentcat: &AgentCat) -> bool {
    let client = connect(agentcat.track(Sample), "http-client").await;
    let arguments = json!({"msg": "hi"});
    let Value::Object(arguments) = arguments else {
        return false;
    };
    client
        .call_tool(CallToolRequestParams::new("echo").with_arguments(arguments))
        .await
        .unwrap();
    client.cancel().await.unwrap();
    agentcat.flush(WAIT).await
}

fn accepted() -> ResponseTemplate {
    ResponseTemplate::new(202).set_body_json(json!({"status": "accepted"}))
}

async fn received(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap_or_default()
}

/// C44, RD-01: one POST to `/v1/public/events/` with a JSON body of `snake_case` keys, the
/// `agentcat-rust/<version>` User-Agent and no credentials.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn posts_to_public_events_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(EVENTS_PATH))
        .respond_with(accepted())
        .mount(&server)
        .await;
    let agentcat = handle(&server.uri(), ms(2_000));
    assert!(send_one(&agentcat).await);

    let requests = received(&server).await;
    assert_eq!(requests.len(), 1, "{requests:?}");
    let request = &requests[0];
    let header = |name: &str| {
        request
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    assert_eq!(header("content-type").as_deref(), Some("application/json"));
    assert_eq!(
        header("user-agent"),
        Some(format!("agentcat-rust/{}", env!("CARGO_PKG_VERSION")))
    );
    assert_eq!(header("user-agent").as_deref(), Some("agentcat-rust/1.0.0"));
    assert!(request.headers.get("authorization").is_none());
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["project_id"], PROJECT);
    assert_eq!(body["resource_name"], "echo");
    let text = body.to_string();
    for camel in ["projectId", "resourceName", "eventType", "sessionId"] {
        assert!(!text.contains(camel), "{camel} in {text}");
    }
}

/// B-006: the `api_base_url` option wins over `AGENTCAT_API_URL`, so the event reaches the mock
/// and not the address the environment names (a port nothing listens on).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn base_url_option_wins_over_env() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(accepted())
        .mount(&server)
        .await;
    let options = Options::default().api_base_url(server.uri());
    let env = MapEnv::new(&[("AGENTCAT_API_URL", "http://127.0.0.1:1")]);
    let url = resolve_api_base_url(&options, &env);
    assert_eq!(url, server.uri());
    let agentcat = handle(&url, ms(2_000));
    assert!(send_one(&agentcat).await);
    assert_eq!(received(&server).await.len(), 1);
}

/// B-102: nothing listens on the port, so each of the four attempts fails; the delivery settles
/// after the last one, within the shrunk delays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_connection_is_retried() {
    // Bind and drop a listener to get a port that refuses connections.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut log = Vec::new();
    let publisher = ApiPublisher::new(format!("http://127.0.0.1:{port}"), &mut log);
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counting = CountingPublisher {
        inner: publisher,
        attempts: Arc::clone(&attempts),
    };
    let agentcat = AgentCat::with_delivery(PROJECT, Options::default(), Arc::new(counting), fast());
    assert!(send_one(&agentcat).await);
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 4);
}

/// Counts the attempts that reach an inner publisher.
#[derive(Debug)]
struct CountingPublisher {
    inner: ApiPublisher,
    attempts: Arc<std::sync::atomic::AtomicUsize>,
}

impl Publisher for CountingPublisher {
    fn publish(
        &self,
        request: agentcat_api::models::PublishEventRequest,
    ) -> crate::sink::BoxFuture<'static, Result<(), String>> {
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.publish(request)
    }
}

/// B-102: a 500 answer fails the attempt; there are 4 attempts in all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_error_is_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    let agentcat = handle(&server.uri(), ms(2_000));
    assert!(send_one(&agentcat).await);
    assert_eq!(received(&server).await.len(), 4);
}

/// C44: an attempt that hangs is cut off by the attempt timeout (200 ms here, 10 s by default),
/// retried, and the whole delivery still settles inside `WAIT`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hung_attempt_times_out() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(accepted().set_delay(Duration::from_secs(10)))
        .mount(&server)
        .await;
    let agentcat = handle(&server.uri(), ms(200));
    let started = Instant::now();
    assert!(send_one(&agentcat).await);
    assert!(started.elapsed() < WAIT, "{:?}", started.elapsed());
    assert_eq!(received(&server).await.len(), 4);
}

/// RD-37: a 202 with a `text/plain` body is a failed attempt, so it is retried.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_body_is_a_failed_attempt() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(202).set_body_string("accepted"))
        .mount(&server)
        .await;
    let agentcat = handle(&server.uri(), ms(2_000));
    assert!(send_one(&agentcat).await);
    assert_eq!(received(&server).await.len(), 4);
}

/// A 202 with a JSON body is one successful attempt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_json_body_is_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(accepted())
        .mount(&server)
        .await;
    let agentcat = handle(&server.uri(), ms(2_000));
    assert!(send_one(&agentcat).await);
    assert_eq!(received(&server).await.len(), 1);
}

/// C44: a build without a TLS feature logs one warning for an `https:` base URL, and none for an
/// `http:` one. Runs only in that build: `scripts/check.sh` and the CI test job run
/// `cargo nextest run -p agentcat-sdk --no-default-features`.
#[cfg(not(any(feature = "rustls", feature = "native-tls")))]
#[test]
fn https_without_tls_feature_warns_once() {
    let mut log = Vec::new();
    let _ = ApiPublisher::new("https://api.example.test".to_owned(), &mut log);
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(log[0].starts_with("Warning:"), "{log:?}");
    let mut log = Vec::new();
    let _ = ApiPublisher::new("http://127.0.0.1:9".to_owned(), &mut log);
    assert!(log.is_empty(), "{log:?}");
}

/// Design §6.7: `new` builds no client. The first `prepare`, which the worker calls before an
/// event's first attempt, builds it, and a client that builds logs nothing.
#[test]
fn client_is_built_at_first_use() {
    let mut log = Vec::new();
    let publisher = ApiPublisher::new("http://127.0.0.1:9".to_owned(), &mut log);
    assert!(log.is_empty(), "{log:?}");
    assert!(publisher.configuration.get().is_none(), "built by new");
    assert_eq!(publisher.prepare(&mut log), Ok(()));
    assert!(log.is_empty(), "{log:?}");
    assert!(publisher.configuration.get().is_some_and(Option::is_some));
}

/// C44: a client that fails to build gives one log line, at its first use only, and a publisher
/// that can never send: every `prepare` gives the reason for TS's final failure line. reqwest has
/// no natural build failure here, so the test hands the publisher a build that fails.
#[test]
fn failed_client_build_logs_once_at_first_use() {
    let publisher = ApiPublisher::with_build("http://127.0.0.1:9".to_owned(), |_| {
        Err("no TLS backend".into())
    });
    let mut log = Vec::new();
    assert_eq!(publisher.prepare(&mut log), Err(NO_CLIENT.to_owned()));
    assert_eq!(
        log,
        ["Failed to build the HTTP client, events will not be sent: no TLS backend"]
    );
    let mut log = Vec::new();
    assert_eq!(publisher.prepare(&mut log), Err(NO_CLIENT.to_owned()));
    assert!(log.is_empty(), "{log:?}");
}

/// C44: the attempt timeout reaches the build, and the default one is 10 s.
#[test]
fn build_gets_the_attempt_timeout() {
    let publisher = ApiPublisher::with_build("http://127.0.0.1:9".to_owned(), |timeout| {
        Err(format!("{timeout:?}"))
    });
    let mut log = Vec::new();
    let _ = publisher.prepare(&mut log);
    assert_eq!(
        log,
        ["Failed to build the HTTP client, events will not be sent: 10s"]
    );
}
