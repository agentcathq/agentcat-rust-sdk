//! The memory of the server info heartbeat: when the SDK first checked in this process, and when
//! it last started a heartbeat for each project and server name. It holds times only, it lives in
//! the memory of the process and nowhere else, and it only limits how often the SDK sends: a
//! process that starts again starts with an empty memory and sends again.
//!
//! One memory serves every handle of the process, so a server that is built and tracked again for
//! each request shares it.
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

#[cfg(test)]
mod tests;

/// How long the SDK waits after the start of an attempt before the next one for the same key.
pub(crate) const INTERVAL: Duration = Duration::from_secs(3600);

/// The age from which a memory counts as one that lasts: in a younger one, a tool call starts no
/// heartbeat.
pub(crate) const LASTING_AGE: Duration = Duration::from_secs(60);

/// The most keys the memory holds; a new key onto that many empties it first.
pub(crate) const MAX_KEYS: usize = 1000;

/// How many builds of tool calls in a row may be dropped before the attempt stays recorded.
pub(crate) const MAX_DROPPED: u32 = 5;

/// The kind of request that starts a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// A `tools/list` request.
    Listing,
    /// A `get_more_tools` call that the SDK answers.
    FeedbackNote,
    /// Any other `tools/call` request.
    ToolCall,
}

/// What the memory knows about one project and server name.
#[derive(Debug, Default)]
struct Entry {
    /// When the last attempt started; `None` after a dropped build gave the attempt back.
    attempt: Option<Instant>,
    /// The builds of tool calls dropped in a row since the last build that ended.
    dropped: u32,
}

/// The times of one process (see the module docs).
#[derive(Debug)]
pub(crate) struct Memory {
    /// When the first check ran.
    started: Option<Instant>,
    /// The entries by project ID, then by server name.
    entries: BTreeMap<String, BTreeMap<String, Entry>>,
    /// How many entries `entries` holds.
    keys: usize,
}

impl Memory {
    /// An empty memory, in which no check has run.
    pub(crate) const fn new() -> Self {
        Self {
            started: None,
            entries: BTreeMap::new(),
            keys: 0,
        }
    }

    /// The check of one request at `now`: whether a heartbeat for `project_id` and `server_name`
    /// is due. When it is, the attempt is recorded as started at `now`, so a second request at
    /// the same moment starts none.
    ///
    /// The first check of a memory sets its start. A [`Trigger::ToolCall`] is never due while the
    /// memory is younger than [`LASTING_AGE`]. An attempt is due when the key has none, or when
    /// its last one started [`INTERVAL`] ago or more.
    pub(crate) fn claim(
        &mut self,
        trigger: Trigger,
        project_id: &str,
        server_name: &str,
        now: Instant,
    ) -> bool {
        let started = *self.started.get_or_insert(now);
        if trigger == Trigger::ToolCall && now.saturating_duration_since(started) < LASTING_AGE {
            return false;
        }
        if let Some(entry) = self.entry(project_id, server_name) {
            let waiting = entry
                .attempt
                .is_some_and(|at| now.saturating_duration_since(at) < INTERVAL);
            if waiting {
                return false;
            }
            entry.attempt = Some(now);
            return true;
        }
        if self.keys >= MAX_KEYS {
            self.entries.clear();
            self.keys = 0;
        }
        self.entries
            .entry(project_id.to_owned())
            .or_default()
            .insert(
                server_name.to_owned(),
                Entry {
                    attempt: Some(now),
                    dropped: 0,
                },
            );
        self.keys += 1;
        true
    }

    /// Gives back the attempt of a build that `trigger` started and that was dropped before it
    /// ended, so the next request starts it again. The [`MAX_DROPPED`]th dropped build of tool
    /// calls in a row keeps its attempt instead, and the count starts again.
    pub(crate) fn release(&mut self, trigger: Trigger, project_id: &str, server_name: &str) {
        let Some(entry) = self.entry(project_id, server_name) else {
            return;
        };
        if trigger == Trigger::ToolCall {
            entry.dropped += 1;
            if entry.dropped >= MAX_DROPPED {
                entry.dropped = 0;
                return;
            }
        }
        entry.attempt = None;
    }

    /// Notes that a build for the key ended, whatever its result: its attempt stays, and the
    /// count of dropped builds starts again.
    pub(crate) fn settle(&mut self, project_id: &str, server_name: &str) {
        if let Some(entry) = self.entry(project_id, server_name) {
            entry.dropped = 0;
        }
    }

    /// The entry of the key, when the memory holds it.
    fn entry(&mut self, project_id: &str, server_name: &str) -> Option<&mut Entry> {
        self.entries.get_mut(project_id)?.get_mut(server_name)
    }
}

/// The memory of this process.
static MEMORY: Mutex<Memory> = Mutex::new(Memory::new());

/// The memory of this process; a poisoned lock is taken over. The lock is held for one method
/// call only, never across an `.await`.
pub(crate) fn process() -> MutexGuard<'static, Memory> {
    MEMORY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Empties the memory of this process, as a new process has it.
#[cfg(test)]
pub(crate) fn forget() {
    *process() = Memory::new();
}

/// Makes the memory of this process one that started `age` before now, with no attempt.
#[cfg(test)]
pub(crate) fn start_aged(age: Duration) {
    let mut memory = process();
    *memory = Memory::new();
    memory.started = Instant::now().checked_sub(age);
}
