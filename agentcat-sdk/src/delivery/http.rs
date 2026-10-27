//! The [`Publisher`] that posts to the `AgentCat` API through `agentcat-api` (C44).
//!
//! One attempt is one `publish_event` call on a `reqwest` client with a 5 s connect timeout, a
//! per-attempt timeout (10 s) and the User-Agent `agentcat-rust/<crate version>`, which is also
//! `Configuration::user_agent`. There is no API key. The client is built lazily (design §6.7): the
//! worker's first [`prepare`](Publisher::prepare) builds it, on the delivery thread, so building a
//! handle loads no TLS roots on the caller's thread. Any `Err` is a failed attempt whose text is
//! the error's `Display`; the worker retries it with TS's delays. A 2xx answer needs a JSON body
//! under `application/json` (RD-37), or `agentcat-api` fails it. No `reqwest` or `agentcat-api`
//! type leaves this module.
use std::fmt;
use std::future::ready;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agentcat_api::apis::configuration::Configuration;
use agentcat_api::apis::events_api::{PublishEventParams, publish_event};
use agentcat_api::models::PublishEventRequest;

use super::Publisher;
use crate::sink::BoxFuture;

/// The connect timeout of every attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The timeout of one attempt.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

/// The User-Agent of every attempt, on the client and in the `Configuration`.
const USER_AGENT: &str = concat!("agentcat-rust/", env!("CARGO_PKG_VERSION"));

/// What an attempt reports when the client could not be built.
const NO_CLIENT: &str = "the HTTP client could not be built";

/// Builds the HTTP client with the given per-attempt timeout; `Err` holds the error's text.
type BuildClient = fn(Duration) -> Result<reqwest::Client, String>;

/// Posts events to `<base URL>/v1/public/events/`.
pub(crate) struct ApiPublisher {
    base_url: String,
    attempt_timeout: Duration,
    /// [`build_client`], or a build that fails in a test.
    build: BuildClient,
    /// Set by the first use: `None` inside when the client could not be built, and every attempt
    /// then fails.
    configuration: OnceLock<Option<Arc<Configuration>>>,
}

impl fmt::Debug for ApiPublisher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let client = match self.configuration.get() {
            None => "not built",
            Some(Some(_)) => "built",
            Some(None) => "failed",
        };
        f.debug_struct("ApiPublisher")
            .field("client", &client)
            .finish_non_exhaustive()
    }
}

impl ApiPublisher {
    /// A publisher for `base_url` with the default attempt timeout. It builds no client yet. The
    /// line for an `https:` URL in a build without a TLS feature goes to `log`.
    pub(crate) fn new(base_url: String, log: &mut Vec<String>) -> Self {
        Self::with_attempt_timeout(base_url, ATTEMPT_TIMEOUT, log)
    }

    /// [`ApiPublisher::new`] with `attempt_timeout` for each attempt.
    pub(crate) fn with_attempt_timeout(
        base_url: String,
        attempt_timeout: Duration,
        log: &mut Vec<String>,
    ) -> Self {
        log.extend(tls_warning(&base_url));
        Self {
            base_url,
            attempt_timeout,
            build: build_client,
            configuration: OnceLock::new(),
        }
    }

    /// [`ApiPublisher::new`] whose client comes from `build`.
    #[cfg(test)]
    pub(super) fn with_build(base_url: String, build: BuildClient) -> Self {
        Self {
            build,
            ..Self::new(base_url, &mut Vec::new())
        }
    }

    /// The configuration of every attempt, built on the first call; `None` when the client could
    /// not be built, which the first call logs to `log`.
    fn configuration(&self, log: &mut Vec<String>) -> Option<&Arc<Configuration>> {
        self.configuration
            .get_or_init(|| match (self.build)(self.attempt_timeout) {
                Ok(client) => Some(Arc::new(Configuration {
                    base_path: self.base_url.clone(),
                    user_agent: Some(USER_AGENT.to_owned()),
                    client,
                    basic_auth: None,
                    oauth_access_token: None,
                    bearer_access_token: None,
                    api_key: None,
                })),
                Err(error) => {
                    log.push(format!(
                        "Failed to build the HTTP client, events will not be sent: {error}"
                    ));
                    None
                }
            })
            .as_ref()
    }
}

/// The warning for an `https:` `base_url` in a build without a TLS feature, which cannot reach
/// it (C44); `None` in a build with one.
fn tls_warning(base_url: &str) -> Option<String> {
    let https = base_url
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("https:");
    (https && cfg!(not(any(feature = "rustls", feature = "native-tls")))).then(|| {
        format!(
            "Warning: agentcat-sdk was built without a TLS feature (rustls or native-tls), so \
             events cannot be sent to {base_url}"
        )
    })
}

/// The client of every attempt (C44).
fn build_client(attempt_timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(attempt_timeout)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| error.to_string())
}

impl Publisher for ApiPublisher {
    /// Builds the client on the first call (C44): a failed build gives one line to `log` and,
    /// on this call and every later one, [`NO_CLIENT`].
    fn prepare(&self, log: &mut Vec<String>) -> Result<(), String> {
        match self.configuration(log) {
            Some(_) => Ok(()),
            None => Err(NO_CLIENT.to_owned()),
        }
    }

    fn publish(&self, request: PublishEventRequest) -> BoxFuture<'static, Result<(), String>> {
        // The worker calls `prepare` first, which logged a failed build.
        let Some(configuration) = self.configuration(&mut Vec::new()).cloned() else {
            return Box::pin(ready(Err(NO_CLIENT.to_owned())));
        };
        Box::pin(async move {
            let params = PublishEventParams {
                publish_event_request: Some(request),
            };
            publish_event(&configuration, params)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }
}

#[cfg(test)]
mod tests;
