//! Telemetry exporters: forward each event to OTLP, Datadog, Sentry or
//! `PostHog`, alongside (or instead of) `AgentCat`.
//!
//! Exporters build their vendor's JSON by hand over the shared HTTP client,
//! so they add no SDK dependencies and install no global state (no tracer
//! provider, no Sentry hub, no panic hook).

use std::sync::Arc;

use crate::event::Event;
use crate::hooks::BoxFuture;

#[cfg(feature = "datadog")]
mod datadog;
#[cfg(feature = "otlp")]
mod otlp;
#[cfg(feature = "posthog")]
mod posthog;
#[cfg(feature = "sentry")]
mod sentry;
pub(crate) mod trace_context;

#[cfg(feature = "datadog")]
pub use datadog::DatadogExporterConfig;
#[cfg(feature = "otlp")]
pub use otlp::OtlpExporterConfig;
#[cfg(feature = "posthog")]
pub use posthog::PostHogExporterConfig;
#[cfg(feature = "posthog")]
pub(crate) use posthog::to_uuid_v7 as posthog_uuid_v7;
#[cfg(feature = "sentry")]
pub use sentry::SentryExporterConfig;

/// One configured exporter.
pub(crate) trait Exporter: Send + Sync {
    fn export(&self, client: reqwest::Client, event: Arc<Event>) -> BoxFuture<Result<(), String>>;
}

/// Configuration of one telemetry exporter.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ExporterConfig {
    /// OpenTelemetry (OTLP/HTTP JSON traces).
    #[cfg(feature = "otlp")]
    Otlp(OtlpExporterConfig),
    /// Datadog logs and metrics.
    #[cfg(feature = "datadog")]
    Datadog(DatadogExporterConfig),
    /// Sentry logs, errors and (optionally) transactions.
    #[cfg(feature = "sentry")]
    Sentry(SentryExporterConfig),
    /// `PostHog` capture events.
    #[cfg(feature = "posthog")]
    PostHog(PostHogExporterConfig),
}

#[cfg(feature = "otlp")]
impl From<OtlpExporterConfig> for ExporterConfig {
    fn from(c: OtlpExporterConfig) -> Self {
        Self::Otlp(c)
    }
}
#[cfg(feature = "datadog")]
impl From<DatadogExporterConfig> for ExporterConfig {
    fn from(c: DatadogExporterConfig) -> Self {
        Self::Datadog(c)
    }
}
#[cfg(feature = "sentry")]
impl From<SentryExporterConfig> for ExporterConfig {
    fn from(c: SentryExporterConfig) -> Self {
        Self::Sentry(c)
    }
}
#[cfg(feature = "posthog")]
impl From<PostHogExporterConfig> for ExporterConfig {
    fn from(c: PostHogExporterConfig) -> Self {
        Self::PostHog(c)
    }
}

/// Builds an exporter; `Err` carries the reason it could not be created.
#[allow(
    unreachable_code,
    unused_variables,
    reason = "builds without some exporter features leave out their arms"
)]
pub(crate) fn create(config: &ExporterConfig) -> Result<Arc<dyn Exporter>, String> {
    match config {
        #[cfg(feature = "otlp")]
        ExporterConfig::Otlp(c) => Ok(Arc::new(otlp::OtlpExporter::new(c))),
        #[cfg(feature = "datadog")]
        ExporterConfig::Datadog(c) => Ok(Arc::new(datadog::DatadogExporter::new(c))),
        #[cfg(feature = "sentry")]
        ExporterConfig::Sentry(c) => Ok(Arc::new(sentry::SentryExporter::new(c)?)),
        #[cfg(feature = "posthog")]
        ExporterConfig::PostHog(c) => Ok(Arc::new(posthog::PostHogExporter::new(c))),
        #[allow(
            unreachable_patterns,
            reason = "unreachable when every exporter is compiled in"
        )]
        _ => Err("exporter type not compiled in".to_owned()),
    }
}

/// Seconds since the Unix epoch, with milliseconds, as JavaScript computes
/// `date.getTime() / 1000`.
#[allow(clippy::cast_precision_loss, reason = "Unix milliseconds stay below 2^53")]
pub(crate) fn epoch_seconds(event: &Event) -> f64 {
    event.timestamp.timestamp_millis() as f64 / 1000.0
}

/// A duration in milliseconds as seconds, as JavaScript computes `ms / 1000`.
#[allow(
    clippy::cast_precision_loss,
    reason = "durations stay below 2^53 milliseconds"
)]
pub(crate) fn millis_to_seconds(millis: u64) -> f64 {
    millis as f64 / 1000.0
}

/// A response check shared by the exporters: non-2xx becomes an error with
/// the status and body.
#[allow(dead_code, reason = "unused when no exporter is compiled in")]
pub(crate) async fn check(
    response: Result<reqwest::Response, reqwest::Error>,
    what: &str,
) -> Result<(), String> {
    match response {
        Ok(r) if r.status().is_success() => Ok(()),
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            Err(format!("{what} failed - Status: {status}, Body: {body}"))
        }
        Err(e) => Err(format!("{what} network error: {e}")),
    }
}
