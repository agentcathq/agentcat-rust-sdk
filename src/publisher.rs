//! The event publisher: a bounded queue drained by a dedicated thread that
//! runs its own single-threaded tokio runtime.
//!
//! The customer's request path only does a non-blocking `try_send`. Hook
//! results, redaction, sanitization, truncation, delivery and exports all run
//! on the publisher thread, so they never compete with the MCP server for its
//! runtime, and they keep working whatever runtime flavor the server uses.
//! Because the thread is independent, blocking flushes cannot deadlock.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use agentcat_api::apis::Error as ApiError;
use agentcat_api::apis::configuration::Configuration;
use agentcat_api::apis::events_api::{PublishEventParams, publish_event};
use agentcat_api::models::PublishEventRequest;
use agentcat_api::models::publish_event_request::EventType;
use serde_json::Value;
use tokio::sync::{Notify, Semaphore, mpsc};

use crate::constants::{AGENTCAT_TAG_SESSION_SOURCE, DEFAULT_API_BASE_URL, SDK_VERSION};
use crate::event::Event;
use crate::exporters::Exporter;
use crate::handles::session_from_hook_value;
use crate::hooks::{BoxFuture, EventRedactor, StringRedactor, UserIdentity};
use crate::logging::write_to_log;

/// Events waiting for the publisher thread.
const QUEUE_CAPACITY: usize = 10_000;
/// Events processed at once (hook waits included).
const MAX_PROCESSING: usize = 256;
/// Concurrent requests to the AgentCat API.
const MAX_IN_FLIGHT_SENDS: usize = 5;
/// Concurrent exporter requests.
const MAX_IN_FLIGHT_EXPORTS: usize = 64;
/// Retries after the first attempt, at 1 s, 2 s and 4 s.
const MAX_RETRIES: u32 = 3;
/// Budget for a customer hook to settle once its event is processed.
pub(crate) const PENDING_HOOK_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Memory budget of the queue (approximate payload bytes).
const MAX_QUEUED_BYTES: usize = 32 * 1024 * 1024;
const DIAGNOSTICS_INTERVAL: Duration = Duration::from_secs(2);

/// Validated customer tags, in order.
pub(crate) type Tags = Vec<(String, String)>;

/// Customer hook results, started at request time and awaited only here.
#[derive(Default)]
pub(crate) struct Pending {
    pub(crate) session_hook: Option<BoxFuture<Option<String>>>,
    pub(crate) identity: Option<BoxFuture<Option<UserIdentity>>>,
    pub(crate) tags: Option<BoxFuture<Option<Tags>>>,
    pub(crate) properties: Option<BoxFuture<Option<Value>>>,
}

impl Pending {
    pub(crate) fn is_empty(&self) -> bool {
        self.session_hook.is_none()
            && self.identity.is_none()
            && self.tags.is_none()
            && self.properties.is_none()
    }
}

/// An event plus the work still to do on it.
pub(crate) struct QueuedEvent {
    pub(crate) event: Event,
    pub(crate) pending: Pending,
    pub(crate) event_redactor: Option<EventRedactor>,
    pub(crate) string_redactor: Option<StringRedactor>,
    /// Set by the queue for its memory budget.
    pub(crate) approx_bytes: usize,
}

/// Process-wide settings. Each is set once: the first configuration that
/// provides it wins.
#[derive(Default)]
pub(crate) struct GlobalConfig {
    pub(crate) api_base_url: OnceLock<String>,
    pub(crate) exporters: OnceLock<Vec<(String, Arc<dyn Exporter>)>>,
}

struct Counters {
    /// Events and exports accepted but not finished.
    outstanding: AtomicUsize,
    idle_async: Notify,
    idle_sync: (Mutex<()>, Condvar),
}

impl Counters {
    fn begin(&self) {
        self.outstanding.fetch_add(1, Ordering::SeqCst);
    }

    fn end(&self) {
        if self.outstanding.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.idle_async.notify_waiters();
            let _guard = self.idle_sync.0.lock();
            self.idle_sync.1.notify_all();
        }
    }

    fn is_idle(&self) -> bool {
        self.outstanding.load(Ordering::SeqCst) == 0
    }
}

pub(crate) struct Publisher {
    sender: Mutex<Option<mpsc::Sender<QueuedEvent>>>,
    counters: Arc<Counters>,
    closed: AtomicBool,
    dropped: AtomicUsize,
    /// Approximate bytes of the events waiting in the queue.
    queued_bytes: Arc<AtomicUsize>,
    /// The publisher thread's runtime: its timers serve flush deadlines and
    /// timeouts, so the SDK never needs timers on the customer's runtime.
    runtime: Option<tokio::runtime::Handle>,
    pub(crate) config: Arc<GlobalConfig>,
}

static PUBLISHER: OnceLock<Publisher> = OnceLock::new();

/// The process-wide publisher, started on first use.
pub(crate) fn global() -> &'static Publisher {
    PUBLISHER.get_or_init(Publisher::start)
}

/// The publisher, if anything started it.
pub(crate) fn try_global() -> Option<&'static Publisher> {
    PUBLISHER.get()
}

/// Guard that ends one outstanding unit of work, even on panic.
struct Outstanding(Arc<Counters>);

impl Drop for Outstanding {
    fn drop(&mut self) {
        self.0.end();
    }
}

/// Approximate in-memory size of an event's payloads, for the queue budget.
fn approx_bytes(event: &Event) -> usize {
    fn walk(v: &Value) -> usize {
        match v {
            Value::String(s) => s.len() + 2,
            Value::Array(items) => 2 + items.iter().map(walk).sum::<usize>(),
            Value::Object(map) => 2 + map.iter().map(|(k, v)| k.len() + 3 + walk(v)).sum::<usize>(),
            _ => 8,
        }
    }
    let values = [
        &event.parameters,
        &event.response,
        &event.error,
        &event.identify_actor_data,
    ];
    512 + values.iter().filter_map(|v| v.as_ref()).map(walk).sum::<usize>()
        + event
            .properties
            .as_ref()
            .map_or(0, |p| p.iter().map(|(k, v)| k.len() + walk(v)).sum())
        + event.user_intent.as_ref().map_or(0, String::len)
}

impl Publisher {
    fn start() -> Self {
        let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
        let counters = Arc::new(Counters {
            outstanding: AtomicUsize::new(0),
            idle_async: Notify::new(),
            idle_sync: (Mutex::new(()), Condvar::new()),
        });
        let config = Arc::new(GlobalConfig::default());
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let (handle_tx, handle_rx) = std::sync::mpsc::channel();
        let worker_counters = counters.clone();
        let worker_config = config.clone();
        let worker_bytes = queued_bytes.clone();
        let spawned = std::thread::Builder::new()
            .name("agentcat-publisher".into())
            .spawn(move || {
                // Built on its own thread: a failure here never drops a
                // runtime inside the customer's async context.
                match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => {
                        let _ = handle_tx.send(Some(rt.handle().clone()));
                        rt.block_on(run_worker(rx, worker_counters, worker_config, worker_bytes));
                    }
                    Err(e) => {
                        write_to_log(&format!(
                            "Warning: AgentCat could not start its publisher runtime; events will not be sent - {e}"
                        ));
                        let _ = handle_tx.send(None);
                    }
                }
            });
        let runtime = match spawned {
            Ok(_) => handle_rx.recv().ok().flatten(),
            Err(e) => {
                write_to_log(&format!(
                    "Warning: AgentCat could not start its publisher thread; events will not be sent - {e}"
                ));
                None
            }
        };
        let sender = runtime.is_some().then_some(tx);
        Self {
            sender: Mutex::new(sender),
            counters,
            closed: AtomicBool::new(false),
            dropped: AtomicUsize::new(0),
            queued_bytes,
            runtime,
            config,
        }
    }

    fn record_drop(&self, reason: &str) {
        let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
        // Log the first drop and then every 1000th, not every event.
        if n == 1 || n.is_multiple_of(1000) {
            write_to_log(&format!("{reason}, dropping event ({n} dropped so far)"));
        }
    }

    /// Queues an event. Never blocks; drops the event when the queue is full
    /// (10,000 events or 32 MiB) or the publisher is shut down.
    pub(crate) fn enqueue(&self, mut queued: QueuedEvent) {
        if self.closed.load(Ordering::SeqCst) {
            write_to_log("Queue is shutting down, event dropped");
            return;
        }
        let sender = match self.sender.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => None,
        };
        let Some(sender) = sender else {
            return;
        };
        // Accept while the queue is under budget, so one large event still
        // goes through (truncation reduces it to about 100 KB).
        let bytes = approx_bytes(&queued.event);
        if self.queued_bytes.fetch_add(bytes, Ordering::SeqCst) >= MAX_QUEUED_BYTES {
            self.queued_bytes.fetch_sub(bytes, Ordering::SeqCst);
            self.record_drop("Event queue over its memory budget");
            return;
        }
        queued.approx_bytes = bytes;
        self.counters.begin();
        if let Err(err) = sender.try_send(queued) {
            self.counters.end();
            self.queued_bytes.fetch_sub(bytes, Ordering::SeqCst);
            self.record_drop(match err {
                mpsc::error::TrySendError::Full(_) => "Event queue full",
                mpsc::error::TrySendError::Closed(_) => "Event queue closed",
            });
        }
    }

    /// Waits until every queued event is delivered (or given up on) and every
    /// export finished, or until `timeout`. Returns true when idle. The
    /// publisher thread owns the deadline.
    pub(crate) async fn flush(&self, timeout: Duration) -> bool {
        if self.counters.is_idle() {
            return true;
        }
        let Some(handle) = self.runtime.as_ref() else {
            return self.counters.is_idle();
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let counters = self.counters.clone();
        handle.spawn(async move {
            let _ = tx.send(wait_idle(&counters, timeout).await);
        });
        rx.await.unwrap_or_else(|_| self.counters.is_idle())
    }

    /// Blocking variant of [`Publisher::flush`], usable without a runtime.
    pub(crate) fn flush_blocking(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (lock, cvar) = &self.counters.idle_sync;
        let Ok(mut guard) = lock.lock() else {
            return self.counters.is_idle();
        };
        while !self.counters.is_idle() {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let wait = (deadline - now).min(Duration::from_millis(50));
            guard = match cvar.wait_timeout(guard, wait) {
                Ok((g, _)) => g,
                Err(_) => return self.counters.is_idle(),
            };
        }
        true
    }

    /// Stops accepting events. Already queued events still drain.
    pub(crate) fn close(&self) {
        if !self.closed.swap(true, Ordering::SeqCst)
            && let Ok(mut guard) = self.sender.lock()
        {
            // Dropping the sender lets the worker finish once drained. The
            // worker's last diagnostics flush counts as outstanding work,
            // so a flush after close also waits for it.
            if let Some(sender) = guard.take() {
                self.counters.begin();
                drop(sender);
            }
        }
    }
}

/// Runs on the publisher runtime: waits until idle or `timeout`.
async fn wait_idle(counters: &Counters, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let notified = counters.idle_async.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if counters.is_idle() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        // Re-check at least every 50 ms in case a wakeup races the check.
        let wait = deadline.min(tokio::time::Instant::now() + Duration::from_millis(50));
        let _ = tokio::time::timeout_at(wait, notified).await;
    }
}

// ── Worker ──────────────────────────────────────────────────────────────────

struct Worker {
    client: Option<reqwest::Client>,
    /// The queue's memory budget; an event's bytes stay counted until it is
    /// truncated.
    queued_bytes: Arc<AtomicUsize>,
    dropped_exports: AtomicUsize,
    api: OnceLock<Configuration>,
    config: Arc<GlobalConfig>,
    counters: Arc<Counters>,
    sends: Semaphore,
    exports: Arc<Semaphore>,
}

fn build_client() -> Option<reqwest::Client> {
    // `Client::builder().build()` reports TLS setup failures as an error;
    // `Client::new()` would panic on them.
    match reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .user_agent(format!("agentcat-rust/{SDK_VERSION}"))
        .build()
    {
        Ok(client) => Some(client),
        Err(e) => {
            write_to_log(&format!(
                "Warning: AgentCat could not create its HTTP client; events will not be sent - {e}"
            ));
            None
        }
    }
}

async fn run_worker(
    mut rx: mpsc::Receiver<QueuedEvent>,
    counters: Arc<Counters>,
    config: Arc<GlobalConfig>,
    queued_bytes: Arc<AtomicUsize>,
) {
    let worker = Arc::new(Worker {
        client: build_client(),
        queued_bytes,
        dropped_exports: AtomicUsize::new(0),
        api: OnceLock::new(),
        config,
        counters,
        sends: Semaphore::new(MAX_IN_FLIGHT_SENDS),
        exports: Arc::new(Semaphore::new(MAX_IN_FLIGHT_EXPORTS)),
    });
    let processing = Arc::new(Semaphore::new(MAX_PROCESSING));
    let mut diagnostics_tick = tokio::time::interval(DIAGNOSTICS_INTERVAL);
    diagnostics_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            queued = rx.recv() => {
                let Some(queued) = queued else { break };
                let guard = Outstanding(worker.counters.clone());
                let Ok(permit) = processing.clone().acquire_owned().await else { break };
                let worker = worker.clone();
                tokio::spawn(async move {
                    let _guard = guard;
                    let _permit = permit;
                    let label = queued.event.resource_name.clone().unwrap_or_default();
                    let outcome = std::panic::AssertUnwindSafe(worker.process(queued));
                    if let Err(p) = futures::FutureExt::catch_unwind(outcome).await {
                        write_to_log(&format!(
                            "Warning: AgentCat event processing failed for {label} - {}",
                            crate::util::panic_message(&*p)
                        ));
                    }
                });
            }
            _ = diagnostics_tick.tick() => {
                if let Some(client) = worker.client.clone() {
                    tokio::spawn(async move { crate::diagnostics::flush(&client).await });
                }
            }
        }
    }
    // The queue is closed: let in-flight work finish, then send the last
    // diagnostics.
    let _ = processing
        .acquire_many(u32::try_from(MAX_PROCESSING).unwrap_or(u32::MAX))
        .await;
    if let Some(client) = &worker.client {
        crate::diagnostics::flush(client).await;
    }
    // Ends the unit of work `close()` started.
    worker.counters.end();
}

async fn settle<T>(fut: Option<BoxFuture<Option<T>>>, label: &str) -> Option<T> {
    let fut = fut?;
    match tokio::time::timeout(
        PENDING_HOOK_TIMEOUT,
        futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(fut)),
    )
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(p)) => {
            write_to_log(&format!(
                "Warning: {label} hook failed in background - {}",
                crate::util::panic_message(&*p)
            ));
            None
        }
        Err(_) => {
            write_to_log(&format!(
                "Warning: {label} hook did not settle within {}ms; publishing event without it",
                PENDING_HOOK_TIMEOUT.as_millis()
            ));
            None
        }
    }
}

/// Stage 0: applies the deferred hook results, before any redaction, so the
/// redaction hooks see them and sanitization and truncation cover them.
async fn apply_pending(event: &mut Event, pending: Pending) {
    let has_session_hook = pending.session_hook.is_some();
    let (hook_value, identity, customer_tags, properties) = tokio::join!(
        settle(pending.session_hook, "resolveSessionId"),
        settle(pending.identity, "identify"),
        settle(pending.tags, "eventTags"),
        settle(pending.properties, "eventProperties"),
    );

    if has_session_hook {
        let (session_id, source) =
            session_from_hook_value(hook_value.as_deref(), event.project_id.as_deref());
        event.session_id = session_id;
        event
            .tags
            .get_or_insert_with(BTreeMap::new)
            .insert(AGENTCAT_TAG_SESSION_SOURCE.to_owned(), source.as_str().to_owned());
    }

    // An identity without a user ID names no actor.
    if let Some(identity) = identity.filter(|i| !i.user_id.is_empty()) {
        event.identify_actor_given_id = Some(identity.user_id);
        event.identify_actor_name = identity.user_name;
        event.identify_actor_data = Some(Value::Object(identity.user_data.unwrap_or_default()));
    }

    // SDK tags stay last-writer: on-path tags win name collisions.
    if let Some(mut customer) = customer_tags {
        // The API keeps the first 50 tags. The request map has no order, so
        // leave room for the SDK tags here: customer tags lose, as in TS.
        let sdk_count = event.tags.as_ref().map_or(0, std::collections::BTreeMap::len);
        let room = 50usize.saturating_sub(sdk_count);
        if customer.len() > room {
            write_to_log(&format!(
                "Dropping {} tag(s) — exceeds maximum of 50 entries per event",
                customer.len() - room
            ));
            customer.truncate(room);
        }
        let mut merged: BTreeMap<String, String> = customer.into_iter().collect();
        if let Some(sdk) = event.tags.take() {
            merged.extend(sdk);
        }
        event.tags = Some(merged);
    }

    if let Some(properties) = properties {
        match properties {
            Value::Object(map) => event.properties = Some(map),
            Value::Null => {}
            _ => {
                write_to_log("Warning: eventProperties hook returned a non-object value; properties omitted");
            }
        }
    }
}

/// Releases an event's bytes from the queue budget when dropped.
struct BudgetRelease(Arc<AtomicUsize>, usize);

impl Drop for BudgetRelease {
    fn drop(&mut self) {
        self.0.fetch_sub(self.1, Ordering::SeqCst);
    }
}

enum SendError {
    Retryable(String),
    Fatal(String),
}

impl Worker {
    fn api(&self, client: &reqwest::Client) -> &Configuration {
        self.api.get_or_init(|| {
            let base = self.config.api_base_url.get().map_or_else(
                || DEFAULT_API_BASE_URL.to_owned(),
                |u| u.trim_end_matches('/').to_owned(),
            );
            // Built field by field: `Configuration::default()` would create
            // (and could panic creating) a second HTTP client.
            Configuration {
                base_path: base,
                user_agent: Some(format!("agentcat-rust/{SDK_VERSION}")),
                client: client.clone(),
                basic_auth: None,
                oauth_access_token: None,
                bearer_access_token: None,
                api_key: None,
            }
        })
    }

    async fn process(self: Arc<Self>, queued: QueuedEvent) {
        let QueuedEvent {
            mut event,
            pending,
            event_redactor,
            string_redactor,
            approx_bytes,
        } = queued;
        let budget = BudgetRelease(self.queued_bytes.clone(), approx_bytes);
        let Some(client) = self.client.clone() else {
            return; // No HTTP client: the failure was logged at startup.
        };

        if !pending.is_empty() {
            apply_pending(&mut event, pending).await;
        }

        if let Some(hook) = event_redactor {
            match crate::redaction::apply_event_redaction(event, &hook).await {
                Ok(Some(redacted)) => event = redacted,
                Ok(None) => {
                    write_to_log("Event dropped by redactEvent hook");
                    return;
                }
                Err(e) => {
                    write_to_log(&format!("Failed to redact event (event-level hook): {e}"));
                    return;
                }
            }
        }

        if let Some(redactor) = string_redactor
            && let Err(e) = crate::redaction::redact_strings(&mut event, &redactor).await
        {
            write_to_log(&format!("Failed to redact event: {e}"));
            return;
        }

        crate::sanitization::sanitize_event(&mut event);
        let mut event = crate::truncation::truncate_event(event);
        // The event is small now: release its share of the queue budget.
        drop(budget);
        if event.id.is_empty() {
            event.id = crate::ksuid::new_event_id();
        }

        // Exporters get each event once, not once per delivery attempt.
        self.export(&client, &event);

        if event.project_id.as_deref().is_some_and(|p| !p.is_empty())
            && let Err(e) = self.send_with_retries(&client, &event).await
        {
            write_to_log(&format!("Failed to deliver AgentCat event after retries: {e}"));
        }
    }

    fn export(&self, client: &reqwest::Client, event: &Event) {
        let Some(exporters) = self.config.exporters.get() else {
            return;
        };
        if exporters.is_empty() {
            return;
        }
        let event = Arc::new(event.clone());
        for (name, exporter) in exporters {
            // Bounded: when every export slot is busy (a slow or unreachable
            // vendor), drop this export rather than queue it without limit.
            let Ok(permit) = self.exports.clone().try_acquire_owned() else {
                let n = self.dropped_exports.fetch_add(1, Ordering::Relaxed) + 1;
                if n == 1 || n.is_multiple_of(1000) {
                    write_to_log(&format!(
                        "Telemetry export busy, dropping export for {name} ({n} dropped so far)"
                    ));
                }
                continue;
            };
            let guard = Outstanding(self.counters.clone());
            self.counters.begin();
            let exporter = exporter.clone();
            let client = client.clone();
            let event = event.clone();
            let name = name.clone();
            tokio::spawn(async move {
                let _guard = guard;
                let _permit = permit;
                let fut = std::panic::AssertUnwindSafe(exporter.export(client, event));
                match futures::FutureExt::catch_unwind(fut).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => write_to_log(&format!("Telemetry export failed for {name}: {e}")),
                    Err(p) => write_to_log(&format!(
                        "Telemetry export failed for {name}: {}",
                        crate::util::panic_message(&*p)
                    )),
                }
            });
        }
    }

    async fn send_with_retries(&self, client: &reqwest::Client, event: &Event) -> Result<(), String> {
        let request = to_publish_request(event);
        let _permit = self.sends.acquire().await.map_err(|e| e.to_string())?;
        let mut attempt = 0;
        loop {
            match self.send_once(client, request.clone()).await {
                Ok(()) => {
                    write_to_log(&format!(
                        "Successfully sent event {} | {} | session {} | {} | {} ms | {}",
                        event.id,
                        event.event_type,
                        event.session_id,
                        event.project_id.as_deref().unwrap_or_default(),
                        event
                            .duration
                            .map_or_else(|| "undefined".to_owned(), |d| d.to_string()),
                        event.identify_actor_given_id.as_deref().unwrap_or("anonymous"),
                    ));
                    return Ok(());
                }
                Err(SendError::Fatal(e)) => return Err(e),
                Err(SendError::Retryable(e)) => {
                    if attempt >= MAX_RETRIES {
                        return Err(e);
                    }
                    write_to_log(&format!(
                        "Failed to send event {}, retrying... [Error: {e}]",
                        event.id
                    ));
                    tokio::time::sleep(backoff(attempt)).await;
                    attempt += 1;
                }
            }
        }
    }

    async fn send_once(
        &self,
        client: &reqwest::Client,
        request: PublishEventRequest,
    ) -> Result<(), SendError> {
        let params = PublishEventParams {
            publish_event_request: Some(request),
        };
        match publish_event(self.api(client), params).await {
            // Only a 2xx response reaches body parsing: it was accepted.
            Ok(_) | Err(ApiError::Serde(_)) => Ok(()),
            Err(ApiError::ResponseError(rc)) => {
                let status = rc.status.as_u16();
                let message = format!("status {status}: {}", truncate_for_log(&rc.content));
                if matches!(status, 408 | 429 | 500 | 502 | 503 | 504) {
                    Err(SendError::Retryable(message))
                } else {
                    Err(SendError::Fatal(message))
                }
            }
            Err(ApiError::Reqwest(e)) if e.is_builder() => Err(SendError::Fatal(e.to_string())),
            Err(e) => Err(SendError::Retryable(e.to_string())),
        }
    }
}

/// 1 s, 2 s, 4 s, each with ±20 % jitter so retries from many servers
/// spread out.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the delay is a positive number of milliseconds below 2^21"
)]
fn backoff(attempt: u32) -> Duration {
    let base = 1000u64 << attempt.min(10);
    let mut byte = [0u8; 1];
    crate::ksuid::fill_random(&mut byte);
    let factor = 0.8 + 0.4 * (f64::from(byte[0]) / 255.0);
    Duration::from_millis((base as f64 * factor) as u64)
}

fn truncate_for_log(s: &str) -> String {
    match crate::util::truncate_js(s, 500) {
        Some(cut) => cut,
        None => s.to_owned(),
    }
}

/// Converts an event to the API request. Sessionless events send an explicit
/// `"session_id": null`, which the API treats as stateless mode.
pub(crate) fn to_publish_request(event: &Event) -> PublishEventRequest {
    let mut req = PublishEventRequest::new(event.project_id.clone().unwrap_or_default());
    req.id = Some(event.id.clone());
    req.session_id = Some((!event.session_id.is_empty()).then(|| event.session_id.clone()));
    req.event_type = serde_json::from_value::<EventType>(Value::String(event.event_type.clone())).ok();
    req.timestamp = Some(event.timestamp.fixed_offset());
    req.duration = event.duration.map(|d| i32::try_from(d).unwrap_or(i32::MAX));
    req.input_tokens = event.input_tokens.map(|t| i32::try_from(t).unwrap_or(i32::MAX));
    req.output_tokens = event.output_tokens.map(|t| i32::try_from(t).unwrap_or(i32::MAX));
    req.resource_name.clone_from(&event.resource_name);
    req.parameters.clone_from(&event.parameters);
    req.response.clone_from(&event.response);
    req.user_intent.clone_from(&event.user_intent);
    req.is_error = event.is_error;
    req.error.clone_from(&event.error);
    req.identify_actor_given_id
        .clone_from(&event.identify_actor_given_id);
    req.identify_actor_name.clone_from(&event.identify_actor_name);
    req.identify_data.clone_from(&event.identify_actor_data);
    req.ip_address.clone_from(&event.ip_address);
    req.sdk_language.clone_from(&event.sdk_language);
    req.agentcat_version.clone_from(&event.agentcat_version);
    req.server_name.clone_from(&event.server_name);
    req.server_version.clone_from(&event.server_version);
    req.client_name.clone_from(&event.client_name);
    req.client_version.clone_from(&event.client_version);
    req.actor_id = event
        .actor_id
        .clone()
        .or_else(|| event.identify_actor_given_id.clone());
    req.event_id.clone_from(&event.event_id);
    req.tags = event.tags.as_ref().map(|t| t.clone().into_iter().collect());
    req.properties = event.properties.clone().map(Value::Object);
    req
}
