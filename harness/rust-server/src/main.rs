//! The Rust reference server of the differential harness: the twin of
//! `harness/src/servers/ts-v2.mjs`. It serves the tools of the tool contract (`tools`) wrapped with
//! `agentcat-sdk`, and follows the server protocol of `harness/README.md` ("Server protocol").
//!
//! ```text
//! harness-ref-server stdio                  MCP over stdio, one tracked server for the connection;
//!                                           exits when stdin closes
//! harness-ref-server http <port>            stateful streamable HTTP on 127.0.0.1:<port>/mcp, one
//!                                           tracked server per MCP session
//! harness-ref-server http-sse <port>        as `http`, with SSE responses (rmcp's default,
//!                                           `with_json_response(false)`)
//! harness-ref-server http-stateless <port>  streamable HTTP without sessions, one tracked server
//!                                           per request
//! ```
//!
//! rmcp serves 2026-07-28 requests and `server/discover` itself, in every mode; over HTTP each
//! 2026-07-28 request gets a fresh tracked server. Every tracked server comes from one shared
//! [`AgentCat`] handle. HTTP modes answer in JSON response mode (`http-sse`: SSE), print one ready line on stdout
//! (`{"ready":true,"url":"http://127.0.0.1:<port>/mcp"}`) and exit on SIGTERM.
//!
//! Environment: `AGENTCAT_API_URL` (required, `http://` on `127.0.0.1`, `localhost` or `[::1]`),
//! `HARNESS_PROJECT_ID` (required unless untracked), `HARNESS_OPTIONS` (the options profile, see
//! `profile`), `HARNESS_NO_TRACK=1` (serve the tools untracked), `HARNESS_PAGE_SIZE=<n>` (serve
//! `tools/list` in pages of `n`, see `tools::page`), `HARNESS_CANARY` (the perturbation of an L3
//! canary run: `1` the context description, `call` the mint-back text of results, `event` the
//! resource name of events; see `profile::Canary`). Exit 2: bad arguments, a non-local
//! `AGENTCAT_API_URL`, a bad page size or canary, or an invalid profile. Logs go to stderr only.
#![forbid(unsafe_code)]

mod canary;
mod profile;
mod tools;

use std::ffi::OsString;
use std::future::{Future, IntoFuture};
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use agentcat_sdk::AgentCat;
use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::post_service;
use rmcp::service::ServerInitializeError;
use rmcp::transport::stdio;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServerHandler, ServiceExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::canary::CallCanary;
use crate::profile::{Canary, Profile};
use crate::tools::{CustomTarget, RefServer, Tools};

/// The prefix of log lines.
const NAME: &str = "harness-ref-server";
/// The exit code for bad arguments, a non-local `AGENTCAT_API_URL` or an invalid profile.
const EXIT_REFUSED: u8 = 2;
/// How long the drain before exit may take.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
/// The path of the streamable HTTP endpoint.
const MCP_PATH: &str = "/mcp";
/// How long the runtime waits for its tasks once serving ends. A stdio read that blocks on an
/// open stdin (after SIGTERM) would otherwise keep the process alive.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);
/// The http-stateless answer to every method but POST, as `methodNotAllowed` in `common.mjs`.
const METHOD_NOT_ALLOWED: &str =
    r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Method not allowed."},"id":null}"#;

/// How the server speaks MCP (argv).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// MCP over stdio.
    Stdio,
    /// Streamable HTTP on `127.0.0.1:<port>` (0: any free port), with MCP sessions unless
    /// `stateless`; POSTs answer in JSON unless `sse`.
    Http {
        port: u16,
        stateless: bool,
        sse: bool,
    },
}

/// What the command line and the environment configure.
#[derive(Debug)]
struct Startup {
    mode: Mode,
    /// The handle every tracked server shares; `None` untracked (`HARNESS_NO_TRACK=1`).
    agentcat: Option<AgentCat>,
    /// The project every tracked server publishes to, also the target of `emit_custom` and
    /// `emit_custom_server`.
    project_id: String,
    /// The tools of one `tools/list` page; `None`: every tool on one page.
    page_size: Option<usize>,
    /// The perturbation of an L3 canary run.
    canary: Canary,
}

/// Why the server does not start: the line for stderr. The exit code is [`EXIT_REFUSED`].
#[derive(Debug)]
struct Refusal(String);

impl Refusal {
    /// A refusal logged with the server's name, as `fail` in `common.mjs`.
    fn named(message: impl std::fmt::Display) -> Self {
        Self(format!("{NAME}: {message}"))
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let startup = match startup(&args) {
        Ok(startup) => startup,
        Err(Refusal(line)) => {
            write_stderr(&line);
            return ExitCode::from(EXIT_REFUSED);
        }
    };
    let tools = match Tools::from_contract(startup.page_size) {
        Ok(tools) => tools,
        Err(error) => {
            log(&error);
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log(&format!("cannot start the tokio runtime: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let code = runtime.block_on(serve(startup, tools));
    runtime.shutdown_timeout(SHUTDOWN_GRACE);
    code
}

/// Reads the command line (`args`, without the program name) and the environment, in the order of
/// `startup` in `common.mjs`.
fn startup(args: &[OsString]) -> Result<Startup, Refusal> {
    let mode = mode(args)?;
    let api_url = env("AGENTCAT_API_URL").unwrap_or_default();
    if !is_local_http_url(&api_url) {
        return Err(Refusal(format!(
            "refusing to start: AGENTCAT_API_URL must point at the local mock (got {})",
            Value::from(api_url)
        )));
    }
    let tracked = env("HARNESS_NO_TRACK").as_deref() != Some("1");
    let project_id = env("HARNESS_PROJECT_ID").unwrap_or_default();
    if tracked && project_id.is_empty() {
        return Err(Refusal::named("HARNESS_PROJECT_ID is required"));
    }
    let page_size = page_size(env("HARNESS_PAGE_SIZE").as_deref())?;
    let canary = Canary::from_env(env("HARNESS_CANARY").as_deref()).map_err(Refusal::named)?;
    let options = env("HARNESS_OPTIONS").unwrap_or_else(|| "{}".to_owned());
    let Profile { options } = profile::parse(&options, canary)
        .map_err(|error| Refusal::named(format_args!("HARNESS_OPTIONS: {error}")))?;
    // The checked local URL goes in as the option, not only through the environment.
    let options = options.api_base_url(api_url);
    Ok(Startup {
        mode,
        agentcat: tracked.then(|| AgentCat::new(project_id.clone(), options)),
        project_id,
        page_size,
        canary,
    })
}

/// The page size of `HARNESS_PAGE_SIZE`: a positive decimal integer without a leading zero, or
/// `None` when it is unset or empty.
fn page_size(text: Option<&str>) -> Result<Option<usize>, Refusal> {
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    text.parse::<usize>()
        .ok()
        .filter(|_| text.bytes().all(|b| b.is_ascii_digit()) && !text.starts_with('0'))
        .map(Some)
        .ok_or_else(|| {
            Refusal::named(format_args!(
                "HARNESS_PAGE_SIZE: must be a positive integer, got {}",
                Value::from(text)
            ))
        })
}

/// The mode of argv: `stdio` (also with no argument), `http [<port>]` or
/// `http-sse [<port>]` or `http-stateless [<port>]`; a missing port is 0.
fn mode(args: &[OsString]) -> Result<Mode, Refusal> {
    let usage = || {
        Refusal::named(format_args!(
            "usage: {NAME} stdio | http <port> | http-sse <port> | http-stateless <port>"
        ))
    };
    let args: Vec<&str> = args
        .iter()
        .map(|arg| arg.to_str().ok_or_else(usage))
        .collect::<Result<_, _>>()?;
    let (stateless, sse, port) = match args.as_slice() {
        [] | ["stdio"] => return Ok(Mode::Stdio),
        ["http", port @ ..] => (false, false, port),
        ["http-sse", port @ ..] => (false, true, port),
        ["http-stateless", port @ ..] => (true, false, port),
        _ => return Err(usage()),
    };
    let port = match port {
        [] => 0,
        [port] => port
            .parse()
            .ok()
            .filter(|_| port.bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| Refusal::named(format_args!("bad port {port}")))?,
        _ => return Err(usage()),
    };
    Ok(Mode::Http {
        port,
        stateless,
        sse,
    })
}

/// Whether `text` is an `http://` URL on `127.0.0.1`, `localhost` or `[::1]`
/// (harness/README.md, `AGENTCAT_API_URL`). Stricter than the WHATWG parse of `common.mjs`, never
/// looser: no user info, no backslash, whitespace or control character, the host spelled as
/// listed (`localhost` in any case), and a port, when present, of 1 to 5 digits below 65536.
fn is_local_http_url(text: &str) -> bool {
    if text
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return false;
    }
    let Some(rest) = text
        .get(..7)
        .filter(|scheme| scheme.eq_ignore_ascii_case("http://"))
        .and_then(|_| text.get(7..))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // `port` is what follows the host: it must be empty or `:<digits>`.
    let (host, port) = match (authority.strip_prefix("[::1]"), authority.find(':')) {
        (Some(port), _) => ("[::1]", port),
        (None, Some(colon)) => authority.split_at(colon),
        (None, None) => (authority, ""),
    };
    let host_ok = host == "[::1]" || host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost");
    let port_ok = port.is_empty()
        || port.strip_prefix(':').is_some_and(|digits| {
            (1..=5).contains(&digits.len())
                && digits.bytes().all(|b| b.is_ascii_digit())
                && digits.parse::<u16>().is_ok()
        });
    host_ok && port_ok
}

/// Serves the mode: `stdio`, or HTTP with one tracked server per session or request. With the
/// call canary, each tracked server is wrapped in [`CallCanary`]. A tracked server then drains
/// the handle (`agentcat.shutdown(5 s)`) and prints `{"drained":<bool>}` on stderr before it
/// exits, on stdin EOF (stdio) and on SIGTERM (every mode). An untracked server prints nothing.
async fn serve(startup: Startup, tools: Tools) -> ExitCode {
    let Startup {
        mode,
        agentcat,
        project_id,
        page_size: _,
        canary,
    } = startup;
    let custom =
        |agentcat: &AgentCat| Some(CustomTarget::new(agentcat.clone(), project_id.clone()));
    let call_canary = canary == Canary::Call;
    let code = match (mode, &agentcat) {
        (Mode::Stdio, Some(agentcat)) => {
            let server = agentcat.track(RefServer::new(tools, custom(agentcat)));
            if call_canary {
                serve_stdio(CallCanary(server)).await
            } else {
                serve_stdio(server).await
            }
        }
        (Mode::Stdio, None) => serve_stdio(RefServer::new(tools, None)).await,
        (
            Mode::Http {
                port,
                stateless,
                sse,
            },
            Some(agentcat),
        ) => {
            let handle = agentcat.clone();
            let target = custom(agentcat);
            let tracked = move || handle.track(RefServer::new(tools.clone(), target.clone()));
            if call_canary {
                serve_http(port, stateless, sse, move || Ok(CallCanary(tracked()))).await
            } else {
                serve_http(port, stateless, sse, move || Ok(tracked())).await
            }
        }
        (
            Mode::Http {
                port,
                stateless,
                sse,
            },
            None,
        ) => {
            serve_http(port, stateless, sse, move || {
                Ok(RefServer::new(tools.clone(), None))
            })
            .await
        }
    };
    if let Some(agentcat) = agentcat {
        let drained = agentcat.shutdown(DRAIN_TIMEOUT).await;
        write_stderr(&json!({ "drained": drained }).to_string());
    }
    code
}

/// Serves `server` over stdio until stdin closes or SIGTERM arrives.
async fn serve_stdio<S: ServerHandler>(server: S) -> ExitCode {
    let stop = termination();
    tokio::pin!(stop);
    let running = tokio::select! {
        started = server.serve(stdio()) => match started {
            Ok(running) => running,
            // stdin closed before a lifecycle began, as after the v2 client's server/discover probe.
            Err(ServerInitializeError::ConnectionClosed(_)) => return ExitCode::SUCCESS,
            Err(error) => {
                log(&format!("stdio: {error}"));
                return ExitCode::FAILURE;
            }
        },
        () = &mut stop => return ExitCode::SUCCESS,
    };
    tokio::select! {
        ended = running.waiting() => match ended {
            Ok(_reason) => ExitCode::SUCCESS,
            Err(error) => {
                log(&format!("stdio: {error}"));
                ExitCode::FAILURE
            }
        },
        () = &mut stop => ExitCode::SUCCESS,
    }
}

/// Serves rmcp's streamable HTTP service on `127.0.0.1:<port>/mcp` until SIGTERM arrives. Each
/// MCP session (stateful, `legacy_session_mode(true)`) or each request (stateless, and every
/// 2026-07-28 request) gets a server from `factory`.
async fn serve_http<S, F>(port: u16, stateless: bool, sse: bool, factory: F) -> ExitCode
where
    S: ServerHandler,
    F: Fn() -> Result<S, std::io::Error> + Send + Sync + 'static,
{
    let stop = termination();
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
        Ok(listener) => listener,
        Err(error) => {
            log(&format!("cannot listen on 127.0.0.1:{port}: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let address = match listener.local_addr() {
        Ok(address) => address,
        Err(error) => {
            log(&format!("the listener has no address: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(!stateless)
        .with_json_response(!sse);
    let service =
        StreamableHttpService::new(factory, Arc::new(LocalSessionManager::default()), config);
    let router = if stateless {
        Router::new().route(
            MCP_PATH,
            post_service(service).fallback(|| async { method_not_allowed() }),
        )
    } else {
        Router::new().route_service(MCP_PATH, service)
    };
    if let Err(error) = ready(address) {
        log(&format!("cannot write the ready line: {error}"));
        return ExitCode::FAILURE;
    }
    tokio::select! {
        served = axum::serve(listener, router).into_future() => match served {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                log(&format!("http: {error}"));
                ExitCode::FAILURE
            }
        },
        () = stop => ExitCode::SUCCESS,
    }
}

/// The http-stateless answer to GET, DELETE and every other method but POST.
fn method_not_allowed() -> impl IntoResponse {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::ALLOW, "POST"),
        ],
        METHOD_NOT_ALLOWED,
    )
}

/// Prints the ready line on stdout.
fn ready(address: SocketAddr) -> std::io::Result<()> {
    let line = json!({ "ready": true, "url": format!("http://{address}{MCP_PATH}") });
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}")?;
    stdout.flush()
}

/// Starts listening for SIGTERM (Ctrl-C where there is no SIGTERM) and returns a future that
/// resolves when it arrives. Must run inside the runtime. When the handler cannot be installed,
/// the future never resolves and the signal keeps its default action, which ends the process.
fn termination() -> impl Future<Output = ()> + Send {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let installed = signal(SignalKind::terminate());
        async move {
            match installed {
                Ok(mut terminate) => {
                    terminate.recv().await;
                }
                Err(error) => {
                    log(&format!("cannot handle SIGTERM: {error}"));
                    std::future::pending::<()>().await;
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        async {
            if tokio::signal::ctrl_c().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }
}

/// The value of the environment variable `name`, or `None` when it is unset or not UTF-8.
#[expect(
    clippy::disallowed_methods,
    reason = "the server protocol configures the server through its environment"
)]
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Logs `message` on stderr, prefixed with the server's name.
fn log(message: &str) {
    write_stderr(&format!("{NAME}: {message}"));
}

/// Writes `line` to stderr. A failed write has nowhere else to go.
fn write_stderr(line: &str) {
    let _ignored = writeln!(std::io::stderr(), "{line}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode_of(args: &[&str]) -> Result<Mode, String> {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        mode(&args).map_err(|Refusal(line)| line)
    }

    #[test]
    fn modes_and_ports_follow_the_server_protocol() {
        assert_eq!(mode_of(&[]), Ok(Mode::Stdio));
        assert_eq!(mode_of(&["stdio"]), Ok(Mode::Stdio));
        assert_eq!(
            mode_of(&["http", "0"]),
            Ok(Mode::Http {
                port: 0,
                stateless: false,
                sse: false
            })
        );
        assert_eq!(
            mode_of(&["http-stateless", "8080"]),
            Ok(Mode::Http {
                port: 8080,
                stateless: true,
                sse: false
            })
        );
        assert_eq!(
            mode_of(&["http-sse", "0"]),
            Ok(Mode::Http {
                port: 0,
                stateless: false,
                sse: true
            })
        );
        assert_eq!(
            mode_of(&["http"]),
            Ok(Mode::Http {
                port: 0,
                stateless: false,
                sse: false
            })
        );
        for bad in [&["stdio", "1"][..], &["sse"], &["http", "1", "2"]] {
            assert!(
                mode_of(bad).is_err_and(|line| line.contains("usage:")),
                "{bad:?}"
            );
        }
        for bad in ["65536", "-1", "+1", "", "0x10"] {
            assert!(
                mode_of(&["http", bad]).is_err_and(|line| line.contains("bad port")),
                "{bad}"
            );
        }
    }

    #[test]
    fn page_sizes_are_positive_integers() {
        let size = |text: Option<&str>| page_size(text).map_err(|Refusal(line)| line);
        assert_eq!(size(None), Ok(None));
        assert_eq!(size(Some("")), Ok(None));
        assert_eq!(size(Some("3")), Ok(Some(3)));
        assert_eq!(size(Some("12")), Ok(Some(12)));
        for bad in [
            "0",
            "-1",
            "1.5",
            "x",
            "03",
            " 3",
            "+3",
            "99999999999999999999999",
        ] {
            assert_eq!(
                size(Some(bad)),
                Err(format!(
                    "{NAME}: HARNESS_PAGE_SIZE: must be a positive integer, got {}",
                    Value::from(bad)
                )),
                "{bad}"
            );
        }
    }

    #[test]
    fn only_local_http_urls_are_accepted() {
        for url in [
            "http://127.0.0.1:4000",
            "http://127.0.0.1",
            "http://localhost:1/",
            "HTTP://LOCALHOST:65535/v1?x#y",
            "http://[::1]:9/path",
            "http://[::1]",
        ] {
            assert!(is_local_http_url(url), "{url}");
        }
        for url in [
            "",
            "https://api.agentcat.com",
            "https://localhost:1",
            "http://localhost.example.com",
            "http://127.0.0.1@api.agentcat.com",
            "http://user@localhost:1",
            "http://127.0.0.1.:1",
            "http://127.0.0.2",
            "http://[::1]:65536",
            "http://localhost:",
            "http://localhost:1x",
            "http://[::1].evil",
            "http://localhost\\@evil",
            " http://localhost:1",
            "http:/localhost:1",
            "ws://localhost:1",
        ] {
            assert!(!is_local_http_url(url), "{url}");
        }
    }
}
