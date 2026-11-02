//! The clean environment of an example run, which reaches only a loopback API, and its temporary
//! home directory.
use std::ffi::OsString;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::process::Command;

use super::PROJECT_ID;

/// What the example's environment changes in the inherited one.
#[derive(Debug, PartialEq)]
pub(super) struct ExampleEnv {
    /// The variables to remove.
    pub(super) remove: Vec<OsString>,
    /// The variables to set, after the removal.
    pub(super) set: Vec<(&'static str, OsString)>,
}

impl ExampleEnv {
    pub(super) fn apply(&self, command: &mut Command) {
        for name in &self.remove {
            command.env_remove(name);
        }
        for (name, value) in &self.set {
            command.env(name, value);
        }
    }
}

/// The environment of an example run, from the names of the `inherited` variables: every
/// `AGENTCAT_*` and `MCPCAT_*` variable removed (any case: a developer's value must not turn the
/// SDK off or point it elsewhere), then `AGENTCAT_API_URL` (`api_url`), `AGENTCAT_PROJECT_ID`,
/// `HOME` and `USERPROFILE` (`home`, so the SDK does not write the real `~/agentcat.log`) set, and
/// `PORT` set for an HTTP example (`port`) or removed.
///
/// # Errors
///
/// `api_url` is not a loopback URL: the example must not start.
pub(super) fn example_env(
    inherited: impl IntoIterator<Item = OsString>,
    api_url: &str,
    home: &Path,
    port: Option<u16>,
) -> Result<ExampleEnv, String> {
    if !is_loopback_url(api_url) {
        return Err(format!(
            "refusing to start the example: AGENTCAT_API_URL {api_url:?} is not a loopback URL"
        ));
    }
    let mut remove: Vec<OsString> = inherited
        .into_iter()
        .filter(|name| {
            let name = name.to_string_lossy().to_ascii_uppercase();
            name.starts_with("AGENTCAT_") || name.starts_with("MCPCAT_")
        })
        .collect();
    let mut set = vec![
        ("HOME", home.as_os_str().to_owned()),
        ("USERPROFILE", home.as_os_str().to_owned()),
        ("AGENTCAT_API_URL", OsString::from(api_url)),
        ("AGENTCAT_PROJECT_ID", OsString::from(PROJECT_ID)),
    ];
    match port {
        Some(port) => set.push(("PORT", OsString::from(port.to_string()))),
        None => remove.push(OsString::from("PORT")),
    }
    Ok(ExampleEnv { remove, set })
}

/// Whether `url` is an `http://` URL whose host is a loopback address: `localhost`, an address in
/// 127.0.0.0/8, or `[::1]`. A URL with user information (`user@host`) does not count.
pub(super) fn is_loopback_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        match bracketed.split_once(']') {
            Some((host, port)) if port.is_empty() || port.starts_with(':') => host,
            _ => return false,
        }
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// A temporary home directory, removed when dropped.
#[derive(Debug)]
pub(super) struct TempHome(pub(super) PathBuf);

impl TempHome {
    pub(super) fn new() -> Result<Self, String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let path =
            std::env::temp_dir().join(format!("examples-check-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        Ok(Self(path))
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
