//! `ApiPublisher` against a `wiremock` server on loopback: the request it sends, the base URL it
//! uses, and which answers are failed attempts that the worker retries. Every test but
//! `real_transport_process_publishes` builds its handle through `AgentCat::with_delivery` with
//! shrunk delays; that one builds the transport of `AgentCat::new`. Every test posts to the mock
//! only, never to a production endpoint.
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::{ApiPublisher, NO_CLIENT};
use crate::client::AgentCat;
use crate::config::Options;
use crate::config::env::{AGENTCAT_API_URL, MCPCAT_API_URL, MapEnv, resolve_api_base_url};
use crate::delivery::DeliveryConfig;
use crate::delivery::Publisher;
use crate::logging;
use crate::test_support::TempHome;
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

/// One POST to `/v1/public/events/` with a JSON body of `snake_case` keys, the
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
    assert!(request.headers.get("authorization").is_none());
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["project_id"], PROJECT);
    assert_eq!(body["resource_name"], "echo");
    let text = body.to_string();
    for camel in ["projectId", "resourceName", "eventType", "sessionId"] {
        assert!(!text.contains(camel), "{camel} in {text}");
    }
}

/// The transport of `AgentCat::new` outside unit tests (`Transport::api`) sets the process logger
/// to the home directory of its environment and posts to the API URL it resolves: the event reaches
/// the mock, and the worker's success line, with the version prefix, reaches `<home>/agentcat.log`.
/// The option and both environment variables name the mock, so no single fault in the URL
/// resolution sends the event to the production default. The first `logging::configure` of a
/// process wins, so this test relies on nextest's process per test.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_transport_process_publishes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(EVENTS_PATH))
        .respond_with(accepted())
        .mount(&server)
        .await;
    let uri = server.uri();
    let home = TempHome::new("http-transport");
    let env = MapEnv::new(&[(AGENTCAT_API_URL, &uri), (MCPCAT_API_URL, &uri)])
        .with_home(home.path().to_path_buf());
    let options = Options::default().api_base_url(uri.clone());
    let agentcat = AgentCat::with_api_transport(PROJECT, options, &env);
    assert!(send_one(&agentcat).await);

    let requests = received(&server).await;
    assert_eq!(requests.len(), 1, "{requests:?}");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["project_id"], PROJECT);
    assert_eq!(body["resource_name"], "echo");
    let id = body["id"].as_str().unwrap();
    let text = std::fs::read_to_string(home.path().join("agentcat.log")).expect("the log file");
    let sent = format!(
        "] {} Successfully sent event {id} | mcp:tools/call | session ",
        logging::prefix()
    );
    let line = format!(" | {PROJECT} | ");
    assert!(
        text.lines().any(|entry| entry.contains(&sent)
            && entry.contains(&line)
            && entry.ends_with(" ms | anonymous")),
        "{text}"
    );
}

/// The `api_base_url` option wins over `AGENTCAT_API_URL`, so the event reaches the mock and not
/// the address the environment names (a port nothing listens on).
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

/// Nothing listens on the port, so each of the four attempts fails; the delivery settles after the
/// last one, within the shrunk delays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_connection_is_retried() {
    // A socket that is bound but never listens refuses connections, and it holds the port until
    // the test ends. A port that is freed again can go to the mock server of a test that runs at
    // the same time, which then receives these attempts.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let port = socket.local_addr().unwrap().port();
    let mut log = Vec::new();
    // Linux and macOS refuse the connection at once. Windows sends the SYN again first and reports
    // the refusal after about 2 s, so the attempt timeout ends each attempt there, and the four
    // attempts settle within `WAIT`.
    let publisher =
        ApiPublisher::with_attempt_timeout(format!("http://127.0.0.1:{port}"), ms(300), &mut log);
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counting = CountingPublisher {
        inner: publisher,
        attempts: Arc::clone(&attempts),
    };
    let agentcat = AgentCat::with_delivery(PROJECT, Options::default(), Arc::new(counting), fast());
    assert!(send_one(&agentcat).await);
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 4);
    drop(socket);
}

/// Counts the attempts that reach an inner publisher.
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

/// A 500 answer fails the attempt; there are 4 attempts in all.
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

/// An attempt that hangs is cut off by the attempt timeout (200 ms here, 10 s by default), retried,
/// and the whole delivery still settles inside `WAIT`.
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

/// A 202 with a `text/plain` body is a failed attempt, so it is retried.
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

/// A build without a TLS feature logs one warning for an `https:` base URL, and none for an `http:`
/// one. Runs only in that build: `scripts/check.sh` and the CI test job run `cargo nextest run -p
/// agentcat-sdk --no-default-features`.
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

/// A build with a TLS feature can reach an `https:` base URL, so it logs no warning for one, as for
/// an `http:` one. Runs in the builds with a TLS feature, the default included.
#[cfg(any(feature = "rustls", feature = "native-tls"))]
#[test]
fn https_with_tls_feature_does_not_warn() {
    for base_url in ["https://api.example.test", "http://127.0.0.1:9"] {
        let mut log = Vec::new();
        let _ = ApiPublisher::new(base_url.to_owned(), &mut log);
        assert!(log.is_empty(), "{base_url}: {log:?}");
    }
}

/// `new` builds no client. The first `prepare`, which the worker calls before an event's first
/// attempt, builds it, and a client that builds logs nothing.
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

/// A client that fails to build gives one log line, at its first use only, and a publisher that can
/// never send: every `prepare` gives the reason for the final failure line. reqwest has no natural
/// build failure here, so the test hands the publisher a build that fails.
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

/// The attempt timeout reaches the build, and the default one is 10 s.
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

const SERVER_INFO_PATH: &str = "/v1/public/server-info/";

/// A heartbeat body for [`PROJECT`].
fn heartbeat() -> agentcat_api::models::PublishServerInfoRequest {
    agentcat_api::models::PublishServerInfoRequest {
        project_id: PROJECT.to_owned(),
        server_name: Some("demo-server".to_owned()),
        server_version: Some("1.2.3".to_owned()),
        server_title: Some("Demo Title".to_owned()),
        instructions: Some("Use the demo tools.".to_owned()),
        features: Some(json!({"tools": [{"name": "alpha"}]})),
    }
}

/// A publisher for the mock `server`, which answers a heartbeat with `answer`.
async fn heartbeat_publisher(server: &MockServer, answer: ResponseTemplate) -> ApiPublisher {
    Mock::given(method("POST"))
        .and(path(SERVER_INFO_PATH))
        .respond_with(answer)
        .mount(server)
        .await;
    let publisher = ApiPublisher::with_attempt_timeout(server.uri(), ms(2_000), &mut Vec::new());
    publisher
        .prepare(&mut Vec::new())
        .expect("the client is built");
    publisher
}

/// One POST to `<custom base URL>/v1/public/server-info/`, with the body of the request
/// model, the headers of an event attempt and no credentials.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v14_a_heartbeat_goes_to_the_server_info_path_of_the_custom_base_url() {
    let server = MockServer::start().await;
    let publisher = heartbeat_publisher(&server, accepted()).await;
    let answer = publisher.publish_server_info(heartbeat()).await;
    assert_eq!(answer, Ok(crate::delivery::ServerInfoAnswer::Accepted));

    let requests = received(&server).await;
    assert_eq!(requests.len(), 1, "{requests:?}");
    let request = &requests[0];
    assert_eq!(request.method.as_str(), "POST");
    // The request reached the mock, so it went to the custom base URL.
    assert_eq!(request.url.path(), SERVER_INFO_PATH);
    assert_eq!(request.url.query(), None);
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
    assert!(request.headers.get("authorization").is_none());
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(
        body,
        json!({
            "project_id": PROJECT,
            "server_name": "demo-server",
            "server_version": "1.2.3",
            "server_title": "Demo Title",
            "instructions": "Use the demo tools.",
            "features": {"tools": [{"name": "alpha"}]},
        })
    );
}

/// A `413` is the answer that asks for the smaller body; it is not a failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_413_to_a_heartbeat_is_too_large() {
    let server = MockServer::start().await;
    let too_large = ResponseTemplate::new(413).set_body_json(json!({"error": "too large"}));
    let publisher = heartbeat_publisher(&server, too_large).await;
    let answer = publisher.publish_server_info(heartbeat()).await;
    assert_eq!(answer, Ok(crate::delivery::ServerInfoAnswer::TooLarge));
    assert_eq!(received(&server).await.len(), 1);
}

/// Any other answer is a failure with its text, and the publisher sends one request for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn any_other_answer_to_a_heartbeat_is_a_failure() {
    for status in [400, 412, 414, 500, 503] {
        let server = MockServer::start().await;
        let refused = ResponseTemplate::new(status).set_body_json(json!({"error": "no"}));
        let publisher = heartbeat_publisher(&server, refused).await;
        let answer = publisher.publish_server_info(heartbeat()).await;
        let text = answer.expect_err("a failure");
        assert!(text.contains(&status.to_string()), "{status}: {text}");
        assert_eq!(received(&server).await.len(), 1, "{status}");
    }
}

/// A publisher whose client could not be built sends no heartbeat.
#[tokio::test]
async fn a_heartbeat_without_a_client_fails() {
    let publisher = ApiPublisher::with_build("http://127.0.0.1:9".to_owned(), |_timeout| {
        Err("no client today".to_owned())
    });
    assert!(publisher.prepare(&mut Vec::new()).is_err());
    let answer = publisher.publish_server_info(heartbeat()).await;
    assert_eq!(answer, Err(NO_CLIENT.to_owned()));
}

/// The whole path against the mock: a `tools/list` request on a handle with the real publisher
/// sends the heartbeat to the base URL of the options.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v14_a_listing_sends_the_heartbeat_to_the_base_url_of_the_handle() {
    crate::server_info::memory::forget();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(SERVER_INFO_PATH))
        .respond_with(accepted())
        .mount(&server)
        .await;
    let agentcat = handle(&server.uri(), ms(2_000));
    let client = connect(agentcat.track(Sample), "http-client").await;
    let listed = client.list_all_tools().await.unwrap();
    let started = Instant::now();
    while received(&server).await.is_empty() && started.elapsed() < WAIT {
        tokio::time::sleep(ms(5)).await;
    }
    let requests = received(&server).await;
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].url.path(), SERVER_INFO_PATH);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["project_id"], PROJECT);
    assert_eq!(
        body["features"]["tools"],
        serde_json::to_value(&listed).unwrap()
    );
    assert_eq!(
        body["features"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["tools"]
    );
}
