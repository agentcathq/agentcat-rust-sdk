//! The server info heartbeat: about once an hour, the SDK tells `AgentCat` what the server
//! offers to agents: its name, version, title and instructions and its lists of tools,
//! resources, resource templates and prompts.
//!
//! There is no timer. A handled request starts a check ([`for_listing`], [`for_call`]): a
//! `tools/list` request, a `get_more_tools` call that the SDK answers, and any other tool call
//! once the [`memory`] of the process is old enough. The check reads memory only. When a
//! heartbeat is due, the request gets a [`build`](for_call) future, which reads the lists from
//! the handler's own list methods with the context of that request ([`lists`]).
//!
//! rmcp gives a request context only inside a request, and the handler lives in the wrapper, so
//! the read cannot move to another task: it runs inside the future of the request, next to it
//! ([`beside`]). The request never waits for it. The read that ends hands owned data
//! ([`Snapshot`]) to the sink; the delivery worker builds the body ([`body`]), runs the
//! customer's redaction hook on its text and sends it ([`send`]), off the request's task.
//!
//! No failure leaves this module: a panic of a handler is contained, each list has a time
//! limit, a read that the request outlives is dropped, and what goes wrong is one log line. A
//! line names a fixed reason: it never holds the message of an error or a panic of the
//! customer's code, which can hold the customer's data.
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::{Pin, pin};
use std::task::Poll;

use futures::FutureExt;
use futures::future::poll_fn;
use rmcp::model::{Implementation, InitializeResult};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler};
use tokio::runtime::Handle;
use tokio::time::Instant;

use crate::client::Inner;
use crate::inject::InjectConfig;
use crate::tracked::Tracked;

pub(crate) mod body;
pub(crate) mod lists;
pub(crate) mod memory;
pub(crate) mod send;

#[cfg(test)]
mod tests;

pub(crate) use body::Snapshot;
pub(crate) use memory::Trigger;

/// The start of every log line of the heartbeat. The lines are debug lines
/// ([`logging::level`](crate::logging::level)).
pub(crate) const LINE_PREFIX: &str = "Server info ";

/// The reason of a read that its request outlived.
const DROPPED: &str = "the request ended before the lists were read";

/// What a check that found a heartbeat due hands to the sink.
#[derive(Debug)]
pub(crate) enum Report {
    /// The read ended: what it gave, for the delivery worker to send.
    Built(Box<Snapshot>),
    /// Nothing will be sent, for this reason.
    Skipped(String),
}

/// The log line of a heartbeat that went out with `tools` tools in `bytes` bytes, without the
/// lists `left_out`. It holds counts and list keys only, never the text of a list.
pub(crate) fn sent_line(tools: usize, bytes: usize, left_out: &[&str]) -> String {
    let mut line = format!("{LINE_PREFIX}sent: {tools} tools, {bytes} bytes");
    if !left_out.is_empty() {
        line.push_str(", without the lists that did not work: ");
        line.push_str(&left_out.join(", "));
    }
    line
}

/// The log line of a heartbeat that was not sent, with the reason `why`.
pub(crate) fn not_sent_line(why: &str) -> String {
    format!("{LINE_PREFIX}not sent: {why}")
}

/// The read of a heartbeat, boxed, so the future of a request that starts none does not grow by
/// its size.
type Read<F> = Option<Pin<Box<F>>>;

/// The check of a `tools/list` request on `tracked`: the read to run [`beside`] the request when
/// a heartbeat is due, else `None`. The claim uses the server name that the handle keeps for the
/// handler type. Only when no name is kept, or when the claim is due, the check calls the
/// handler's `get_info`, and it keeps the name that `get_info` gives. When that panics, nothing
/// is due. A panic of the check itself is contained, and nothing is due.
pub(crate) fn for_listing<'a, H: ServerHandler>(
    tracked: &'a Tracked<H>,
    context: &RequestContext<RoleServer>,
) -> Read<impl Future<Output = ()> + use<'a, H>> {
    catch_unwind(AssertUnwindSafe(|| check_listing(tracked, context)))
        .ok()
        .flatten()
}

/// The check of [`for_listing`].
fn check_listing<'a, H: ServerHandler>(
    tracked: &'a Tracked<H>,
    context: &RequestContext<RoleServer>,
) -> Read<impl Future<Output = ()> + use<'a, H>> {
    let shared = tracked.agentcat().inner();
    if !can_send(shared) {
        return None;
    }
    let kept = shared.with_registry::<H, _>(|registry| registry.server_name().map(str::to_owned));
    let (claim, info) = if let Some(name) = kept {
        let claim = Claim::take(shared, Trigger::Listing, &name)?;
        let Ok(info) = catch_unwind(AssertUnwindSafe(|| tracked.inner().get_info())) else {
            claim.end(Report::Skipped("get_info panicked".to_owned()));
            return None;
        };
        keep_server_name::<H>(shared, &info.server_info.name);
        (claim, info)
    } else {
        let info = catch_unwind(AssertUnwindSafe(|| tracked.inner().get_info())).ok()?;
        keep_server_name::<H>(shared, &info.server_info.name);
        let claim = Claim::take(shared, Trigger::Listing, &info.server_info.name)?;
        (claim, info)
    };
    Some(Box::pin(contained(
        claim,
        build(tracked, info, context.clone()),
    )))
}

/// The check of a tool call on `tracked`, whose record names `server`: the read to run
/// [`beside`] the call when a heartbeat is due, else `None`. `feedback_note` says that the call
/// is a `get_more_tools` call that the SDK answers. The heartbeat carries the name and the
/// version of `server`, so they equal those of the call's event. Without `server` nothing is
/// due. A panic of the check itself is contained, and nothing is due.
pub(crate) fn for_call<'a, H: ServerHandler>(
    tracked: &'a Tracked<H>,
    feedback_note: bool,
    server: Option<&Implementation>,
    context: &RequestContext<RoleServer>,
) -> Read<impl Future<Output = ()> + use<'a, H>> {
    catch_unwind(AssertUnwindSafe(|| {
        check_call(tracked, feedback_note, server, context)
    }))
    .ok()
    .flatten()
}

/// The check of [`for_call`].
fn check_call<'a, H: ServerHandler>(
    tracked: &'a Tracked<H>,
    feedback_note: bool,
    server: Option<&Implementation>,
    context: &RequestContext<RoleServer>,
) -> Read<impl Future<Output = ()> + use<'a, H>> {
    let shared = tracked.agentcat().inner();
    if !can_send(shared) {
        return None;
    }
    let server = server?;
    keep_server_name::<H>(shared, &server.name);
    let trigger = if feedback_note {
        Trigger::FeedbackNote
    } else {
        Trigger::ToolCall
    };
    let claim = Claim::take(shared, trigger, &server.name)?;
    let Ok(mut info) = catch_unwind(AssertUnwindSafe(|| tracked.inner().get_info())) else {
        claim.end(Report::Skipped("get_info panicked".to_owned()));
        return None;
    };
    info.server_info = server.clone();
    Some(Box::pin(contained(
        claim,
        build(tracked, info, context.clone()),
    )))
}

/// Keeps `name` as the server name of the handler type `H` on the handle `shared`, for the next
/// check of a listing.
fn keep_server_name<H: ServerHandler>(shared: &Inner, name: &str) {
    shared.with_registry::<H, _>(|registry| registry.keep_server_name(name));
}

/// Whether the handle `shared` sends heartbeats at all: the SDK is on, the handle is not one of
/// the test fake, tracing is on and the project ID is not empty.
fn can_send(shared: &Inner) -> bool {
    shared.enabled
        && shared.server_info
        && shared.options.tracing()
        && !shared.project_id.is_empty()
}

/// The attempt that a check recorded in the [`memory`], until its read ends. A read that ends
/// hands its [`Report`] over with [`end`](Self::end). When the read is dropped before that (its
/// request ended first), the drop gives the attempt back to the memory, except for the last of
/// too many dropped reads in a row ([`Memory::release`](memory::Memory::release)), which keeps
/// the attempt and reports the read as skipped.
struct Claim<'a> {
    shared: &'a Inner,
    trigger: Trigger,
    server_name: String,
    /// Whether the read ended.
    ended: bool,
}

impl<'a> Claim<'a> {
    /// Records an attempt for the project of `shared` and `server_name` when one is due now.
    fn take(shared: &'a Inner, trigger: Trigger, server_name: &str) -> Option<Self> {
        let due = memory::process().claim(trigger, &shared.project_id, server_name, Instant::now());
        due.then(|| Self {
            shared,
            trigger,
            server_name: server_name.to_owned(),
            ended: false,
        })
    }

    /// Ends the read with `report`, which goes to the sink; the attempt stays recorded.
    fn end(mut self, report: Report) {
        self.ended = true;
        memory::process().settle(&self.shared.project_id, &self.server_name);
        self.shared.sink.server_info(report);
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        let recorded =
            memory::process().release(self.trigger, &self.shared.project_id, &self.server_name);
        if recorded {
            self.shared
                .sink
                .server_info(Report::Skipped(DROPPED.to_owned()));
        }
    }
}

/// The read of a heartbeat that is due: the lists of the handler with `context`, and the
/// [`Report`] of what it gave. The future does no other work: the body, the redaction and the
/// POST belong to the delivery worker.
async fn build<H: ServerHandler>(
    tracked: &Tracked<H>,
    info: InitializeResult,
    context: RequestContext<RoleServer>,
) -> Report {
    let started = std::time::Instant::now();
    let lists = match lists::read(tracked.inner(), &context, &info.capabilities).await {
        Ok(lists) => lists,
        Err(why) => return Report::Skipped(why),
    };
    let shared = tracked.agentcat().inner();
    Report::Built(Box::new(Snapshot {
        server: info.server_info,
        instructions: info.instructions,
        lists,
        inject: InjectConfig::from_options(&shared.options),
        state: shared.with_registry::<H, _>(|registry| registry.state_mut().clone()),
        runtime: Handle::try_current().ok(),
        started,
    }))
}

/// Runs the read `build` of the attempt `claim` and ends the claim with its [`Report`]. A panic in
/// any poll of `build` is contained and ends the claim as a read that panicked, without the
/// message of the panic, so the attempt stays recorded. Dropping the future before it ends gives
/// the attempt of `claim` back ([`Claim`]).
async fn contained(claim: Claim<'_>, build: impl Future<Output = Report>) {
    let report = AssertUnwindSafe(build)
        .catch_unwind()
        .await
        .unwrap_or_else(|_payload| Report::Skipped("the read panicked".to_owned()));
    let _ = catch_unwind(AssertUnwindSafe(|| claim.end(report)));
}

/// Runs `request` to its end and gives its output, with the read `build` of a heartbeat next to
/// it. Each time the future is polled, `request` is polled first. `build` is polled after it
/// while `request` waits, and at least one time; so a read whose handlers answer at once ends
/// also next to a request that answers at once. The output of `request` is returned in the poll
/// that finds it ready, and `build` is then dropped, ended or not. Nothing of `build` reaches
/// the output.
pub(crate) async fn beside<T>(
    request: impl Future<Output = T>,
    build: Option<impl Future<Output = ()>>,
) -> T {
    let Some(build) = build else {
        return request.await;
    };
    let mut request = pin!(request);
    let mut build = pin!(build.fuse());
    let mut polled = false;
    poll_fn(|cx| {
        let output = request.as_mut().poll(cx);
        if output.is_pending() || !polled {
            polled = true;
            let _: Poll<()> = build.as_mut().poll(cx);
        }
        output
    })
    .await
}
