//! The scenario format: `scenarios/<name>.json`, read with unknown keys refused.
//!
//! ```json
//! {
//!   "projectId": "proj_...",
//!   "options": { "identify": { "returns": { "userId": "user-1" } } },
//!   "httpHeaders": { "x-test-header": "value" },
//!   "pageSize": 3,
//!   "expectPublishes": 2,
//!   "eventOrder": "multiset",
//!   "sessions": [
//!     {
//!       "client": { "name": "client-a", "version": "1.0.0" },
//!       "steps": [
//!         { "op": "listTools" },
//!         { "op": "callTool", "name": "echo", "args": { "msg": "hi" }, "saveHandleAs": "S1" },
//!         { "op": "callTool", "name": "echo", "args": { "msg": "again", "session_id": "${S1}" } }
//!       ]
//!     }
//!   ]
//! }
//! ```
//!
//! - `options`: the options profile of the tracked server (see `server::options`). Default `{}`.
//! - `httpHeaders`: headers the client adds to every HTTP request. Default none.
//! - `pageSize`: `tools/list` answers in pages of this many tools (see `server::page`).
//! - `expectPublishes`: the number of publish requests the run must make.
//! - `eventOrder`: `"multiset"` orders the events by their normalized body alone, for steps whose
//!   events can share a millisecond (see `normalize`). Without it, the events are in time order.
//! - Each session is a new client connection; its steps run in order.
//! - Steps: `listTools` lists one page (`cursor`, the page to list; `saveCursorAs`, a name for the
//!   result's `nextCursor`); `callTool` calls the tool `name` with `args` (`saveHandleAs`, a name
//!   for the session handle the result issues); `parallel` runs its `callTool` steps at once, each
//!   on the client of `session` (the index of this or an earlier session; default this one).
//! - `${NAME}` in a `callTool` argument is the handle saved under `NAME`, and in a `cursor` the
//!   saved cursor.
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{Map, Value};

/// A scenario file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Scenario {
    /// The project the tracked server publishes to.
    pub(crate) project_id: String,
    /// The options profile.
    #[serde(default)]
    pub(crate) options: Map<String, Value>,
    /// The headers of every HTTP request, in order.
    #[serde(default)]
    pub(crate) http_headers: Map<String, Value>,
    /// The tools of one `tools/list` page; none: every tool on one page.
    pub(crate) page_size: Option<usize>,
    /// The number of publish requests the run makes.
    pub(crate) expect_publishes: usize,
    /// How the normalized document orders the events.
    #[serde(default)]
    pub(crate) event_order: EventOrder,
    /// The client connections, in order.
    pub(crate) sessions: Vec<Session>,
}

/// How the normalized document orders the events.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum EventOrder {
    /// By time, then as `normalize` describes.
    #[default]
    Time,
    /// By the normalized body alone.
    Multiset,
}

/// One client connection.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Session {
    /// The client's name and version (`clientInfo`).
    pub(crate) client: ClientInfo,
    /// The steps, in order.
    pub(crate) steps: Vec<Step>,
}

/// The client's name and version.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientInfo {
    pub(crate) name: String,
    pub(crate) version: String,
}

/// One step of a session.
#[derive(Debug, Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum Step {
    /// Lists one page of tools.
    ListTools {
        /// The page; none: the first.
        cursor: Option<String>,
        /// The name under which the result's `nextCursor` is saved.
        save_cursor_as: Option<String>,
    },
    /// Calls a tool.
    CallTool {
        name: String,
        #[serde(default)]
        args: Map<String, Value>,
        /// The name under which the session handle the result issues is saved.
        save_handle_as: Option<String>,
        /// Inside a `parallel` step only: the index of the session whose client runs the call.
        session: Option<usize>,
    },
    /// Runs its `callTool` steps at once.
    Parallel { steps: Vec<Step> },
}

impl Scenario {
    /// Reads `scenarios/<name>.json`.
    ///
    /// # Panics
    ///
    /// When the file cannot be read or is not a scenario.
    pub(crate) fn load(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/e2e/scenarios")
            .join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{} is not a scenario: {error}", path.display()))
    }

    /// The HTTP headers as name and value.
    ///
    /// # Panics
    ///
    /// When a header value is not a string.
    pub(crate) fn headers(&self) -> Vec<(String, String)> {
        self.http_headers
            .iter()
            .map(|(name, value)| {
                let value = value
                    .as_str()
                    .unwrap_or_else(|| panic!("httpHeaders.{name} must be a string"));
                (name.clone(), value.to_owned())
            })
            .collect()
    }
}
