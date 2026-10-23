//! The error of the public calls of the SDK.

/// Why a call of the SDK failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A custom event was published on a handle that has no project ID.
    #[error("projectId is required for publishCustomEvent")]
    ProjectIdRequired,
}

const _: () = {
    const fn assert_send_sync_static<T: Send + Sync + 'static>() {}
    assert_send_sync_static::<Error>();
};
