//! The environment the SDK reads: the names of its variables, where it reads them (the process,
//! or a map in tests), the API base URL they give and the switch of `AGENTCAT_ENABLED`. Both
//! [`AgentCat::new`](crate::AgentCat::new) and [`AgentCat::from_env`](crate::AgentCat::from_env)
//! read it; `from_env` also takes its project ID from it.
use std::fmt;
use std::path::PathBuf;

use super::Options;
use crate::wire;

/// The environment variable that holds the project ID when none is passed to the SDK.
pub const AGENTCAT_PROJECT_ID: &str = "AGENTCAT_PROJECT_ID";

/// The environment variable that holds the base URL of the `AgentCat` API.
pub const AGENTCAT_API_URL: &str = "AGENTCAT_API_URL";

/// The environment variable under the former name of [`AGENTCAT_API_URL`], read when that one is
/// unset or empty.
pub const MCPCAT_API_URL: &str = "MCPCAT_API_URL";

/// The environment variable that turns the SDK off: `false`, `0`, `no` or `off` (any case,
/// surrounding white space ignored) disables it.
pub const AGENTCAT_ENABLED: &str = "AGENTCAT_ENABLED";

/// The base URL of the `AgentCat` API when neither the options nor the environment name one.
pub(crate) const DEFAULT_API_BASE_URL: &str = "https://api.agentcat.com";

/// Where the SDK reads its environment: the process, or a map in tests (the one place that
/// touches `std::env`).
pub(crate) trait Env: Send + Sync + fmt::Debug {
    /// The value of the variable `name`, `None` when it is unset or not text.
    fn var(&self, name: &str) -> Option<String>;

    /// The user's home directory.
    fn home_dir(&self) -> Option<PathBuf>;
}

/// The environment of the running process.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProcessEnv;

impl Env for ProcessEnv {
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place that reads the process environment"
    )]
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        std::env::home_dir()
    }
}

/// A fixed environment for tests.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct MapEnv {
    vars: std::collections::HashMap<String, String>,
    home: Option<PathBuf>,
}

#[cfg(test)]
impl MapEnv {
    /// An environment with the variables `vars` and no home directory.
    pub(crate) fn new(vars: &[(&str, &str)]) -> Self {
        Self {
            vars: vars
                .iter()
                .map(|&(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
            home: None,
        }
    }

    /// The same environment with the home directory `home`.
    pub(crate) fn with_home(mut self, home: PathBuf) -> Self {
        self.home = Some(home);
        self
    }
}

#[cfg(test)]
impl Env for MapEnv {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

/// The base URL events go to: the first non-empty of the option, `AGENTCAT_API_URL`,
/// `MCPCAT_API_URL`, else the default. The value is used as given, with no trailing slash
/// trimmed.
pub(crate) fn resolve_api_base_url(options: &Options, env: &dyn Env) -> String {
    options
        .api_base_url_option()
        .map(str::to_owned)
        .or_else(|| env.var(AGENTCAT_API_URL).filter(|url| !url.is_empty()))
        .or_else(|| env.var(MCPCAT_API_URL).filter(|url| !url.is_empty()))
        .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_owned())
}

/// Whether the SDK is on: off when `AGENTCAT_ENABLED`, trimmed ([`wire::trim`]) and lower-cased, is
/// `false`, `0`, `no` or `off`; on otherwise, also when unset.
pub(crate) fn sdk_enabled(env: &dyn Env) -> bool {
    let Some(value) = env.var(AGENTCAT_ENABLED) else {
        return true;
    };
    let value = wire::trim(&value).to_lowercase();
    !matches!(value.as_str(), "false" | "0" | "no" | "off")
}
