//! The mock `AgentCat` API: a wiremock server on loopback that answers each publish request
//! (`POST /v1/public/events/`) with `202 {"status": "accepted"}` and records it. Any other
//! request gets wiremock's 404, and fails the run when the publishes are read.
use std::time::Duration;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// The path of a publish request.
const PUBLISH_PATH: &str = "/v1/public/events/";

/// How long the SDK may take to make the publishes a scenario expects.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);

/// How often [`MockApi::wait_for_publishes`] looks at the recorded requests.
const POLL: Duration = Duration::from_millis(10);

/// The running mock.
pub(crate) struct MockApi {
    server: MockServer,
}

/// A recorded publish request.
pub(crate) struct Publish {
    /// The `content-type` header.
    pub(crate) content_type: Option<String>,
    /// The query string, with its `?` (empty without one).
    pub(crate) query: String,
    /// The body, parsed as JSON.
    pub(crate) body: Value,
}

impl MockApi {
    /// Starts the mock on a free loopback port.
    pub(crate) async fn start() -> Self {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(PUBLISH_PATH))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({"status": "accepted"})))
            .mount(&server)
            .await;
        Self { server }
    }

    /// The base URL of the mock, for [`agentcat_sdk::Options::api_base_url`].
    pub(crate) fn url(&self) -> String {
        self.server.uri()
    }

    /// Waits until the mock received `count` requests.
    ///
    /// # Panics
    ///
    /// When fewer arrive within [`PUBLISH_TIMEOUT`].
    pub(crate) async fn wait_for_publishes(&self, count: usize) {
        let waited = tokio::time::timeout(PUBLISH_TIMEOUT, async {
            while self.requests().await.len() < count {
                tokio::time::sleep(POLL).await;
            }
        })
        .await;
        assert!(
            waited.is_ok(),
            "the SDK made {} of {count} publishes within {PUBLISH_TIMEOUT:?}",
            self.requests().await.len()
        );
    }

    /// The publish requests, in arrival order.
    ///
    /// # Panics
    ///
    /// When the mock received another request, or a publish body that is not JSON.
    pub(crate) async fn publishes(&self) -> Vec<Publish> {
        self.requests().await.iter().map(publish).collect()
    }

    /// Every request received so far.
    async fn requests(&self) -> Vec<Request> {
        self.server.received_requests().await.unwrap_or_default()
    }
}

/// The publish of `request`.
///
/// # Panics
///
/// When `request` is not a publish, or its body is not JSON.
fn publish(request: &Request) -> Publish {
    let is_publish = request.method.as_str() == "POST" && request.url.path() == PUBLISH_PATH;
    assert!(
        is_publish,
        "the mock API received a request other than a publish: {} {}",
        request.method, request.url
    );
    let content_type = request
        .headers
        .get("content-type")
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned());
    let query = request
        .url
        .query()
        .map_or_else(String::new, |query| format!("?{query}"));
    let body = serde_json::from_slice(&request.body).unwrap_or_else(|error| {
        panic!(
            "a publish body is not JSON ({error}): {}",
            String::from_utf8_lossy(&request.body)
        )
    });
    Publish {
        content_type,
        query,
        body,
    }
}
