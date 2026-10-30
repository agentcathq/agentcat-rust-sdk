//! The rmcp versions that CI tests: the highest patch of each rmcp 3.x minor, each with the
//! rmcp-macros version to test it with.
//!
//! `cargo run -q -p xtask -- rmcp-versions` reads the versions of `rmcp` and `rmcp-macros` from
//! the crates.io API and prints one line of JSON, `[{"version":"3.0.1","macros":"3.0.1"},...]`,
//! which the CI matrix takes as its legs. [`latest_patches`] and [`with_macros`] hold the rules;
//! their tests need no network.
use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The crates.io API root of the crates.
const API: &str = "https://crates.io/api/v1/crates";

/// The `User-Agent` of every request: crates.io asks for a name and a way to reach the owner.
const USER_AGENT: &str = concat!(
    "agentcat-rust-sdk rmcp-versions (",
    env!("CARGO_PKG_REPOSITORY"),
    ")"
);

/// How many times one page is requested before the command gives up.
const ATTEMPTS: usize = 3;

/// How long one request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// One version of a crate, as the crates.io API lists it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Version {
    /// The version number, such as `3.4.1`.
    pub num: String,
    /// Whether the owner yanked the version.
    #[serde(default)]
    pub yanked: bool,
}

/// One CI leg: an rmcp version and the rmcp-macros version to test it with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Leg {
    /// The rmcp version.
    pub version: String,
    /// The rmcp-macros version.
    pub macros: String,
}

/// The minor and the patch of a `3.<minor>.<patch>` version; `None` for another major, a
/// pre-release or build suffix, or any other form.
fn minor_and_patch(num: &str) -> Option<(u64, u64)> {
    let (minor, patch) = num.strip_prefix("3.")?.split_once('.')?;
    let number = |text: &str| {
        if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
            text.parse::<u64>().ok()
        } else {
            None
        }
    };
    Some((number(minor)?, number(patch)?))
}

/// The highest patch of each 3.x minor, with no pre-release and no yanked version, sorted by the
/// minor as a number (3.10.x follows 3.9.x). Patches compare as numbers too (3.1.10 is above
/// 3.1.9).
#[must_use]
pub fn latest_patches(versions: &[Version]) -> Vec<String> {
    let mut best: BTreeMap<u64, (u64, &str)> = BTreeMap::new();
    for version in versions.iter().filter(|v| !v.yanked) {
        let Some((minor, patch)) = minor_and_patch(&version.num) else {
            continue;
        };
        let entry = best.entry(minor).or_insert((patch, &version.num));
        if entry.0 < patch {
            *entry = (patch, &version.num);
        }
    }
    best.into_values().map(|(_, num)| num.to_owned()).collect()
}

/// The `3.<minor>` part of a version.
fn minor_of(num: &str) -> &str {
    match num.match_indices('.').nth(1) {
        Some((end, _)) => &num[..end],
        None => num,
    }
}

/// Pairs each rmcp version with the highest rmcp-macros of the same minor, by the rules of
/// [`latest_patches`]. rmcp and rmcp-macros release patches apart, so an rmcp patch may have no
/// rmcp-macros of the same number; rmcp asks for `^<its own version>` of rmcp-macros, which the
/// highest patch of the minor meets.
///
/// # Errors
///
/// A minor of `rmcp` has no rmcp-macros release; the message names the rmcp version.
pub fn with_macros(rmcp: &[String], macros: &[Version]) -> Result<Vec<Leg>, String> {
    let by_minor: BTreeMap<String, String> = latest_patches(macros)
        .into_iter()
        .map(|num| (minor_of(&num).to_owned(), num))
        .collect();
    rmcp.iter()
        .map(|version| {
            let minor = minor_of(version);
            by_minor
                .get(minor)
                .map(|macros| Leg {
                    version: version.clone(),
                    macros: macros.clone(),
                })
                .ok_or_else(|| {
                    format!("rmcp-versions: no rmcp-macros {minor}.x release for rmcp {version}")
                })
        })
        .collect()
}

/// The URL of the next page: `next` (the `meta.next_page` of a page, a query such as
/// `?per_page=100&seek=...`, a path, or a whole URL) resolved against the versions URL of `krate`.
fn next_page_url(krate: &str, next: &str) -> String {
    if next.starts_with("https://") || next.starts_with("http://") {
        next.to_owned()
    } else if next.starts_with('/') {
        format!("https://crates.io{next}")
    } else {
        format!("{API}/{krate}/versions{next}")
    }
}

/// The versions of one page and the `meta.next_page` of the page, when there is one.
///
/// # Errors
///
/// The page has no `versions` list, or a version has no `num`.
fn parse_page(page: &Value) -> Result<(Vec<Version>, Option<String>), String> {
    let versions = page
        .get("versions")
        .cloned()
        .ok_or("the page has no `versions` list")?;
    let versions: Vec<Version> =
        serde_json::from_value(versions).map_err(|e| format!("the `versions` list: {e}"))?;
    let next = page
        .pointer("/meta/next_page")
        .and_then(Value::as_str)
        .filter(|next| !next.is_empty())
        .map(str::to_owned);
    Ok((versions, next))
}

/// GETs `url` and parses the body as JSON, with up to [`ATTEMPTS`] attempts.
async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let mut last_error = String::new();
    for _ in 0..ATTEMPTS {
        let attempt = async {
            let response = client.get(url).send().await.map_err(network_error)?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("HTTP {status}"));
            }
            let body = response.bytes().await.map_err(network_error)?;
            serde_json::from_slice::<Value>(&body).map_err(|e| format!("the body is not JSON: {e}"))
        };
        match attempt.await {
            Ok(value) => return Ok(value),
            Err(error) => last_error = format!("{url}: {error}"),
        }
    }
    Err(last_error)
}

/// The text of a network error, for a message that names the URL once: the error without its URL,
/// then each cause of its `source()` chain (the DNS, connect or TLS error).
fn network_error(error: reqwest::Error) -> String {
    error_chain(&error.without_url())
}

/// The message of `error` and of each error in its `source()` chain, joined with `: `.
fn error_chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Every version of `krate`, page by page.
async fn fetch_versions(client: &reqwest::Client, krate: &str) -> Result<Vec<Version>, String> {
    let mut versions = Vec::new();
    let mut url = Some(format!("{API}/{krate}/versions?per_page=100"));
    while let Some(current) = url {
        let page = get_json(client, &current).await?;
        let (page_versions, next) = parse_page(&page).map_err(|e| format!("{current}: {e}"))?;
        versions.extend(page_versions);
        url = next.map(|next| next_page_url(krate, &next));
    }
    Ok(versions)
}

/// Reads both crates' versions and builds the legs.
async fn legs() -> Result<Vec<Leg>, String> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("rmcp-versions: cannot build the HTTP client: {e}"))?;
    let rmcp = latest_patches(
        &fetch_versions(&client, "rmcp")
            .await
            .map_err(|e| format!("rmcp-versions: {e}"))?,
    );
    if rmcp.is_empty() {
        return Err("rmcp-versions: no 3.x version found".to_owned());
    }
    let macros = fetch_versions(&client, "rmcp-macros")
        .await
        .map_err(|e| format!("rmcp-versions: {e}"))?;
    with_macros(&rmcp, &macros)
}

/// Runs the command: returns the exit code, the line for stdout (the JSON list) and the lines
/// for stderr.
#[must_use]
pub fn run(args: &[String]) -> (u8, Option<String>, Vec<String>) {
    if !args.is_empty() {
        return (
            2,
            None,
            vec!["usage: cargo run -q -p xtask -- rmcp-versions".to_owned()],
        );
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            return (
                1,
                None,
                vec![format!("rmcp-versions: cannot start the runtime: {e}")],
            );
        }
    };
    let result = runtime
        .block_on(legs())
        .and_then(|legs| serde_json::to_string(&legs).map_err(|e| e.to_string()));
    match result {
        Ok(json) => (0, Some(json), Vec::new()),
        Err(message) => (1, None, vec![message]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(num: &str) -> Version {
        Version {
            num: num.to_owned(),
            yanked: false,
        }
    }

    fn yanked(num: &str) -> Version {
        Version {
            num: num.to_owned(),
            yanked: true,
        }
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn leg(version: &str, macros: &str) -> Leg {
        Leg {
            version: version.to_owned(),
            macros: macros.to_owned(),
        }
    }

    #[test]
    fn picks_the_highest_patch_of_a_minor() {
        assert_eq!(latest_patches(&[v("3.4.0"), v("3.4.1")]), ["3.4.1"]);
        assert_eq!(latest_patches(&[v("3.4.1"), v("3.4.0")]), ["3.4.1"]);
    }

    #[test]
    fn skips_yanked_pre_release_and_other_majors() {
        let list = latest_patches(&[
            v("3.5.1"),
            yanked("3.5.2"),
            v("3.6.0-beta.1"),
            v("2.2.0"),
            v("4.0.0"),
        ]);
        assert_eq!(list, ["3.5.1"]);
    }

    #[test]
    fn orders_3_10_0_after_3_9_3() {
        assert_eq!(
            latest_patches(&[v("3.10.0"), v("3.9.3"), v("3.9.1"), v("3.2.0")]),
            ["3.2.0", "3.9.3", "3.10.0"]
        );
    }

    #[test]
    fn compares_patches_as_numbers() {
        assert_eq!(latest_patches(&[v("3.1.9"), v("3.1.10")]), ["3.1.10"]);
    }

    #[test]
    fn skips_forms_that_are_not_major_minor_patch() {
        let list = latest_patches(&[
            v("3.7"),
            v("3.7.0.1"),
            v("3.x.0"),
            v("3..0"),
            v("3.8.0+build"),
            v("13.1.0"),
        ]);
        assert!(list.is_empty(), "{list:?}");
    }

    // rmcp and rmcp-macros release patches apart: an rmcp patch can have no rmcp-macros of the
    // same number, so the leg takes the highest rmcp-macros of the same minor.
    #[test]
    fn pairs_each_rmcp_version_with_the_highest_rmcp_macros_of_its_minor() {
        let macros = [
            v("3.1.2"),
            v("3.1.3"),
            v("3.2.0"),
            v("3.2.1"),
            yanked("3.2.2"),
            v("3.3.0-beta.1"),
            v("3.3.0"),
            v("3.10.0"),
        ];
        let rmcp = strings(&["3.1.4", "3.2.0", "3.3.0", "3.10.1"]);
        assert_eq!(
            with_macros(&rmcp, &macros),
            Ok(vec![
                leg("3.1.4", "3.1.3"),
                leg("3.2.0", "3.2.1"),
                leg("3.3.0", "3.3.0"),
                leg("3.10.1", "3.10.0"),
            ])
        );
    }

    #[test]
    fn a_minor_without_an_rmcp_macros_release_fails() {
        let macros = [v("3.3.0"), v("3.5.0"), yanked("3.4.1")];
        let error = with_macros(&strings(&["3.4.0"]), &macros).unwrap_err();
        assert_eq!(
            error,
            "rmcp-versions: no rmcp-macros 3.4.x release for rmcp 3.4.0"
        );
    }

    #[test]
    fn a_leg_prints_version_then_macros() {
        let json = serde_json::to_string(&[leg("3.0.1", "3.0.1"), leg("3.10.0", "3.10.2")]);
        assert_eq!(
            json.unwrap(),
            r#"[{"version":"3.0.1","macros":"3.0.1"},{"version":"3.10.0","macros":"3.10.2"}]"#
        );
    }

    #[test]
    fn a_page_gives_its_versions_and_the_next_page() {
        let page = serde_json::json!({
            "versions": [
                {"num": "3.1.0", "yanked": false, "id": 1},
                {"num": "3.0.0", "yanked": true},
            ],
            "meta": {"total": 3, "next_page": "?per_page=100&seek=abc"},
        });
        let (versions, next) = parse_page(&page).unwrap();
        assert_eq!(versions, [v("3.1.0"), yanked("3.0.0")]);
        assert_eq!(next.as_deref(), Some("?per_page=100&seek=abc"));
        let last = serde_json::json!({"versions": [], "meta": {"next_page": null}});
        assert_eq!(parse_page(&last).unwrap(), (Vec::new(), None));
        assert!(parse_page(&serde_json::json!({"errors": []})).is_err());
    }

    #[test]
    fn the_next_page_resolves_against_the_versions_url() {
        assert_eq!(
            next_page_url("rmcp", "?per_page=100&seek=abc"),
            "https://crates.io/api/v1/crates/rmcp/versions?per_page=100&seek=abc"
        );
        assert_eq!(
            next_page_url("rmcp", "/api/v1/crates/rmcp/versions?seek=x"),
            "https://crates.io/api/v1/crates/rmcp/versions?seek=x"
        );
        assert_eq!(
            next_page_url(
                "rmcp",
                "https://crates.io/api/v1/crates/rmcp/versions?seek=y"
            ),
            "https://crates.io/api/v1/crates/rmcp/versions?seek=y"
        );
    }

    /// An error with an optional cause.
    #[derive(Debug)]
    struct Layer(&'static str, Option<Box<Layer>>);

    impl std::fmt::Display for Layer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl Error for Layer {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.1
                .as_deref()
                .map(|cause| cause as &(dyn Error + 'static))
        }
    }

    #[test]
    fn an_error_names_each_cause_of_its_chain() {
        let refused = Layer("Connection refused (os error 111)", None);
        let connect = Layer("client error (Connect)", Some(Box::new(refused)));
        let error = Layer("error sending request", Some(Box::new(connect)));
        assert_eq!(
            error_chain(&error),
            "error sending request: client error (Connect): Connection refused (os error 111)"
        );
        assert_eq!(error_chain(&Layer("HTTP 500", None)), "HTTP 500");
    }

    /// A request to a closed loopback port: the text has the cause, and no URL (the caller names
    /// it once).
    #[test]
    fn a_network_error_has_its_cause_and_no_url() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .unwrap()
            .port();
        let url = format!("http://127.0.0.1:{port}/versions");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let error = runtime
            .block_on(client.get(&url).send())
            .expect_err("the port is closed");
        let text = network_error(error);
        assert!(!text.contains("127.0.0.1"), "{text}");
        assert!(text.contains(": "), "no cause: {text}");
    }

    #[test]
    fn an_argument_is_a_usage_error() {
        let (code, stdout, stderr) = run(&strings(&["--all"]));
        assert_eq!(code, 2);
        assert_eq!(stdout, None);
        assert!(stderr[0].starts_with("usage:"), "{stderr:?}");
    }
}
