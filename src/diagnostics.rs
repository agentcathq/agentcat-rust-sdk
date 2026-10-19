//! Anonymous SDK diagnostics: every SDK log line is batched and sent as OTLP
//! logs to AgentCat's collector, so setup failures across the fleet are
//! visible. No event payloads or user data are sent.
//!
//! On by default. Off with `disable_diagnostics(true)` or the
//! `DISABLE_DIAGNOSTICS` environment variable; `DISABLE_DIAGNOSTICS=false`
//! (or `0`, `no`, `off`) forces it on, also in test environments.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};

use crate::constants::{
    DEFAULT_DIAGNOSTICS_ENDPOINT, DEFAULT_DIAGNOSTICS_TOKEN, DIAGNOSTICS_SCOPE_NAME, SDK_VERSION,
};

const MAX_BUFFER: usize = 1000;

static ENABLED: AtomicBool = AtomicBool::new(false);
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static STATIC_ATTRIBUTES: OnceLock<Vec<Value>> = OnceLock::new();
static BUFFER: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());

#[derive(Debug, PartialEq, Eq)]
enum EnvFlag {
    Disabled,
    ForceEnabled,
    Unset,
}

fn env_flag() -> EnvFlag {
    match std::env::var("DISABLE_DIAGNOSTICS") {
        Ok(raw) if !raw.trim().is_empty() => {
            match raw.trim().to_ascii_lowercase().as_str() {
                "false" | "0" | "no" | "off" => EnvFlag::ForceEnabled,
                _ => EnvFlag::Disabled,
            }
        }
        _ => EnvFlag::Unset,
    }
}

/// Test runners that identify themselves through the environment.
fn is_test_environment() -> bool {
    cfg!(test)
        || std::env::var_os("NEXTEST").is_some()
        || std::env::var_os("NEXTEST_RUN_ID").is_some()
        || std::env::var("AGENTCAT_ENV").is_ok_and(|v| v == "test")
}

/// Enables diagnostics once per process (first call wins).
pub(crate) fn init(project_id: Option<&str>, disabled: bool) {
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return;
    }
    let flag = env_flag();
    let enabled = !disabled
        && flag != EnvFlag::Disabled
        && (flag == EnvFlag::ForceEnabled || !is_test_environment());
    if enabled {
        let _ = STATIC_ATTRIBUTES.set(static_attributes(project_id));
        ENABLED.store(true, Ordering::SeqCst);
    }
}

pub(crate) fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn attr(key: &str, value: Option<String>) -> Option<Value> {
    value
        .filter(|v| !v.is_empty())
        .map(|v| json!({"key": key, "value": {"stringValue": v}}))
}

fn hostname() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .filter(|h| !h.is_empty())
}

/// A stable, anonymous ID for one binary on one host.
fn install_id() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let seed = format!("{}|{}", hostname().unwrap_or_default(), exe.display());
    Some(crate::util::hex(&crate::util::sha256(seed.as_bytes()))[..16].to_owned())
}

fn static_attributes(project_id: Option<&str>) -> Vec<Value> {
    let mut out = Vec::new();
    let mut push = |key: &str, value: Option<String>| {
        if let Some(a) = attr(key, value) {
            out.push(a);
        }
    };
    match project_id.filter(|p| !p.is_empty()) {
        Some(p) => push("agentcat.project_id", Some(p.to_owned())),
        None => push("agentcat.install_id", install_id()),
    }
    push("agentcat.sdk.language", Some("rust".into()));
    push("agentcat.sdk.version", Some(SDK_VERSION.into()));
    push(
        "agentcat.mcp_sdk.version",
        crate::runtime::mcp_library_version().map(str::to_owned),
    );
    push("process.runtime.name", Some("rust".into()));
    push("process.pid", Some(std::process::id().to_string()));
    push("os.type", Some(std::env::consts::OS.into()));
    push("host.arch", Some(std::env::consts::ARCH.into()));
    push(
        "host.cpu.count",
        std::thread::available_parallelism()
            .ok()
            .map(|n| n.to_string()),
    );
    out
}

fn record_attributes() -> Vec<Value> {
    [
        attr("agentcat.sdk.version", Some(SDK_VERSION.into())),
        attr(
            "agentcat.mcp_sdk.version",
            crate::runtime::mcp_library_version().map(str::to_owned),
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Severity from the log line, as the TS SDK infers it: any `fail`/`error`
/// is ERROR, `Warning:` is WARN, everything else INFO.
fn severity(entry: &str) -> (u8, &'static str) {
    let lower = entry.to_ascii_lowercase();
    if lower.contains("fail") || lower.contains("error") {
        (17, "ERROR")
    } else if entry.contains("Warning:") {
        (13, "WARN")
    } else {
        (9, "INFO")
    }
}

fn build_record(entry: &str) -> Value {
    let (number, text) = severity(entry);
    json!({
        "timeUnixNano": (crate::util::unix_millis() as u128 * 1_000_000).to_string(),
        "severityNumber": number,
        "severityText": text,
        "body": {"stringValue": entry},
        "attributes": record_attributes(),
    })
}

/// Buffers one log line. Called from `write_to_log`; never panics.
pub(crate) fn capture(entry: &str) {
    if !is_enabled() {
        return;
    }
    if let Ok(mut buffer) = BUFFER.lock() {
        if buffer.len() >= MAX_BUFFER {
            buffer.pop_front();
        }
        buffer.push_back(build_record(entry));
    }
}

fn endpoint() -> String {
    let base = std::env::var("DIAGNOSTICS_ENDPOINT")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_DIAGNOSTICS_ENDPOINT.to_owned());
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1/logs") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/v1/logs")
    }
}

fn token() -> String {
    std::env::var("DIAGNOSTICS_TOKEN")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_DIAGNOSTICS_TOKEN.to_owned())
}

/// Takes the buffered records and builds the OTLP request body.
fn take_payload() -> Option<Value> {
    if !is_enabled() {
        return None;
    }
    let records: Vec<Value> = BUFFER.lock().ok()?.drain(..).collect();
    if records.is_empty() {
        return None;
    }
    Some(json!({
        "resourceLogs": [{
            "resource": {"attributes": STATIC_ATTRIBUTES.get().cloned().unwrap_or_default()},
            "scopeLogs": [{
                "scope": {"name": DIAGNOSTICS_SCOPE_NAME, "version": SDK_VERSION},
                "logRecords": records,
            }],
        }],
    }))
}

/// Sends the buffered records. Fire-and-forget: network errors are ignored.
pub(crate) async fn flush(client: &reqwest::Client) {
    let Some(payload) = take_payload() else {
        return;
    };
    let _ = client
        .post(endpoint())
        .header("Authorization", format!("Bearer {}", token()))
        .json(&payload)
        .send()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_matches_ts() {
        assert_eq!(severity("Failed to send").1, "ERROR");
        assert_eq!(severity("x error y").1, "ERROR");
        assert_eq!(severity("Warning: slow").1, "WARN");
        assert_eq!(severity("WARN: Tool has complex schema").1, "INFO");
        assert_eq!(severity("AgentCat setup started").1, "INFO");
    }

    #[test]
    fn record_shape() {
        let r = build_record("Warning: x");
        assert_eq!(r["severityNumber"], 13);
        assert_eq!(r["body"]["stringValue"], "Warning: x");
        assert!(r["timeUnixNano"].as_str().unwrap().ends_with("000000"));
    }
}
