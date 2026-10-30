//! [`ToolRegistry`]: what the SDK learned from the tool listings of one handler type. A listing
//! page does not replace the registry: the registry merges per tool name across pages and
//! instances, so a tool on page 2 keeps its entry when page 1 is listed again.

use std::collections::HashMap;

use crate::inject::{InjectState, InjectedList, ToolEntry};

#[cfg(test)]
mod tests;

/// The entries by tool name, the pipeline state and whether any listing was recorded.
#[derive(Debug, Default)]
pub(crate) struct ToolRegistry {
    state: InjectState,
    entries: HashMap<String, ToolEntry>,
    listed: bool,
}

impl ToolRegistry {
    /// Merges one injected page: the entry of each listed tool name replaces the one recorded
    /// before, and the entries of tools the page does not list stay. A tool that declares its own
    /// `session_id` joins the declared set, which only grows.
    pub(crate) fn record(&mut self, list: &InjectedList) {
        for (name, entry) in &list.entries {
            if entry.declares_session {
                self.state.declared_session_params.insert(name.clone());
            }
            self.entries.insert(name.clone(), entry.clone());
        }
        self.listed = true;
    }

    /// The entry recorded for the tool `name`.
    pub(crate) fn entry(&self, name: &str) -> Option<&ToolEntry> {
        self.entries.get(name)
    }

    /// Whether any listing page was recorded.
    pub(crate) fn listed(&self) -> bool {
        self.listed
    }

    /// Whether the tool `name` has ever declared its own `session_id`.
    pub(crate) fn declares_session(&self, name: &str) -> bool {
        self.state.declared_session_params.contains(name)
    }

    /// The pipeline state, for [`build_injected_list`](crate::inject::build_injected_list) and
    /// [`inject_tool`](crate::inject::inject_tool).
    pub(crate) fn state_mut(&mut self) -> &mut InjectState {
        &mut self.state
    }
}
