//! The [`Publisher`] that posts to the `AgentCat` API through `agentcat-api` (C44).
//!
//! One attempt is one `publish_event` call on a `reqwest` client with a 5 s connect timeout, a
//! per-attempt timeout (10 s) and the User-Agent `agentcat-rust/<crate version>`, which is also
//! `Configuration::user_agent`. There is no API key. Any `Err` is a failed attempt whose text is
//! the error's `Display`; the worker retries it with TS's delays. A 2xx answer needs a JSON body
//! under `application/json` (RD-37), or `agentcat-api` fails it. No `reqwest` or `agentcat-api`
//! type leaves this module.
use std::fmt;
use std::future::ready;
use std::sync::Arc;
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

/// What an attempt reports when the client could not be built.
const NO_CLIENT: &str = "the HTTP client could not be built";

/// Posts events to `<base URL>/v1/public/events/`.
pub(crate) struct ApiPublisher {
    /// `None` when the client could not be built: every attempt then fails.
    configuration: Option<Arc<Configuration>>,
}

impl fmt::Debug for ApiPublisher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiPublisher")
            .field("ready", &self.configuration.is_some())
            .finish_non_exhaustive()
    }
}

impl ApiPublisher {
    /// A publisher for `base_url` with the default attempt timeout. Lines for the SDK's log (a
    /// client that could not be built, an `https:` URL in a build without a TLS feature) go to
    /// `log`.
    pub(crate) fn new(base_url: String, log: &mut Vec<String>) -> Self {
        Self::with_attempt_timeout(base_url, ATTEMPT_TIMEOUT, log)
    }

    /// [`ApiPublisher::new`] with `attempt_timeout` for each attempt.
    pub(crate) fn with_attempt_timeout(
        base_url: String,
        attempt_timeout: Duration,
        log: &mut Vec<String>,
    ) -> Self {
        #[cfg(not(any(feature = "rustls", feature = "native-tls")))]
        if base_url
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("https:")
        {
            log.push(format!(
                "Warning: agentcat-sdk was built without a TLS feature (rustls or native-tls), so events cannot be sent to {base_url}"
            ));
        }
        let user_agent = concat!("agentcat-rust/", env!("CARGO_PKG_VERSION")).to_owned();
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(attempt_timeout)
            .user_agent(user_agent.clone())
            .build()
            .map_err(|error| error.to_string());
        Self::from_client(base_url, user_agent, client, log)
    }

    /// The publisher over `client`, or, when it is an `Err`, one that cannot send (one line to
    /// `log`).
    fn from_client(
        base_url: String,
        user_agent: String,
        client: Result<reqwest::Client, String>,
        log: &mut Vec<String>,
    ) -> Self {
        let configuration = match client {
            Ok(client) => Some(Arc::new(Configuration {
                base_path: base_url,
                user_agent: Some(user_agent),
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
        };
        Self { configuration }
    }
}

impl Publisher for ApiPublisher {
    fn unusable(&self) -> Option<String> {
        self.configuration.is_none().then(|| NO_CLIENT.to_owned())
    }

    fn publish(&self, request: PublishEventRequest) -> BoxFuture<'static, Result<(), String>> {
        let Some(configuration) = self.configuration.clone() else {
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
