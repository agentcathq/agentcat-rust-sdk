//! Timeouts on the customer's request path, timed by a small dedicated
//! thread.
//!
//! The customer's tokio runtime may run without timers, and the publisher's
//! runtime can be busy with a slow redactor or gone after `shutdown()`, so
//! neither can time the request path.

use std::sync::OnceLock;
use std::time::Duration;

use crate::hooks::BoxFuture;

static TIMER: OnceLock<Option<tokio::runtime::Handle>> = OnceLock::new();

fn handle() -> Option<&'static tokio::runtime::Handle> {
    TIMER
        .get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("agentcat-timer".into())
                .spawn(move || {
                    // Built on this thread, so a failure never drops a runtime
                    // inside the customer's async context.
                    match tokio::runtime::Builder::new_current_thread()
                        .enable_time()
                        .build()
                    {
                        Ok(rt) => {
                            let _ = tx.send(Some(rt.handle().clone()));
                            rt.block_on(futures::future::pending::<()>());
                        }
                        Err(_) => {
                            let _ = tx.send(None);
                        }
                    }
                });
            match spawned {
                Ok(_) => rx.recv().ok().flatten(),
                Err(_) => None,
            }
        })
        .as_ref()
}

/// A future that completes after `duration`. It works without a timer on the
/// awaiting runtime.
pub(crate) fn sleep(duration: Duration) -> BoxFuture<()> {
    let (tx, rx) = futures::channel::oneshot::channel::<()>();
    match handle() {
        Some(handle) => {
            handle.spawn(async move {
                tokio::time::sleep(duration).await;
                let _ = tx.send(());
            });
        }
        None => {
            // No timer thread: a plain thread per timeout (rare path).
            let _ = std::thread::Builder::new()
                .name("agentcat-timeout".into())
                .spawn(move || {
                    std::thread::sleep(duration);
                    let _ = tx.send(());
                });
        }
    }
    // A dropped sender also ends the wait: better to time out early than
    // to hang a tool call.
    Box::pin(async move {
        let _ = rx.await;
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn sleeps_without_a_runtime() {
        let started = std::time::Instant::now();
        futures::executor::block_on(super::sleep(std::time::Duration::from_millis(30)));
        assert!(started.elapsed() >= std::time::Duration::from_millis(25));
    }
}
