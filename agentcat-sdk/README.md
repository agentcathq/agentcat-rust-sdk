# agentcat-sdk

[![crates.io](https://img.shields.io/crates/v/agentcat-sdk.svg)](https://crates.io/crates/agentcat-sdk)
[![docs.rs](https://docs.rs/agentcat-sdk/badge.svg)](https://docs.rs/agentcat-sdk)
[![CI](https://github.com/agentcathq/agentcat-rust-sdk/actions/workflows/ci.yml/badge.svg)](https://github.com/agentcathq/agentcat-rust-sdk/actions/workflows/ci.yml)
[![MSRV 1.88](https://img.shields.io/badge/MSRV-1.88-blue.svg)](https://docs.rs/agentcat-sdk/latest/agentcat_sdk/#minimum-supported-rust-version)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)

[Docs](https://docs.agentcat.com) · [Website](https://agentcat.com)

AgentCat analytics for MCP servers built on [rmcp](https://crates.io/crates/rmcp), the official
Rust SDK of the Model Context Protocol (MCP).

AgentCat shows the maintainers of an MCP server what agents use the server for and where the
agents get stuck. This crate records each tool call of your server with the intent of the agent,
the session, the actor, errors and token estimates. It sends the events to your AgentCat project
from a background thread. It acts like the AgentCat TypeScript SDK.

## Install

Add the crate, `rmcp` 3 with its stdio transport, and tokio. The `#[tool]` macros of `rmcp` also
need `schemars` and `serde`:

```sh
cargo add agentcat-sdk
cargo add rmcp@3 --features transport-io
cargo add tokio --features macros,rt-multi-thread
cargo add schemars
cargo add serde --features derive
```

The crate needs Rust 1.88 or later, and `rmcp` 3.0.0 or a later 3.x release.

## Quickstart

Wrap your server with `agentcat_sdk::track`, and serve the wrapper where you served your server:

```rust,no_run
use agentcat_sdk::Options;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{InitializeResult, ServerCapabilities};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
struct AddArgs {
    a: f64,
    b: f64,
}

#[derive(Clone)]
struct Calculator;

#[tool_router]
impl Calculator {
    #[tool(description = "Adds two numbers")]
    async fn add(&self, Parameters(args): Parameters<AddArgs>) -> String {
        (args.a + args.b).to_string()
    }
}

#[tool_handler]
impl ServerHandler for Calculator {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // One line: wrap the server with AgentCat.
    let server = agentcat_sdk::track(Calculator, "proj_...", Options::default());
    let running = server.serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}
```

Replace `proj_...` with the ID of your project.

A stdio server talks JSON-RPC on stdout. The SDK writes nothing to stdout or stderr: its own log
lines go to `~/agentcat.log`. Send your own logs to stderr.

By default, AgentCat adds the parameters `context` and `session_id` to each tool, and the tool
`get_more_tools` to the list. The first call of a task gets a text block that begins
`[session_id issued` before the result of the tool. The agent sends the issued ID in its later
calls. The [crate guide](https://docs.rs/agentcat-sdk) explains this and how to turn it off.

## Learn more

- The [crate guide](https://docs.rs/agentcat-sdk): streamable HTTP and stateless servers, the
  options, the environment variables, shutdown, logging, testing and the known limits.
- The [examples](https://github.com/agentcathq/agentcat-rust-sdk/tree/main/agentcat-sdk/examples):
  stdio, streamable HTTP, stateless HTTP, and a server that uses every hook except
  `resolve_session_id`.
- The [CHANGELOG](https://github.com/agentcathq/agentcat-rust-sdk/blob/main/agentcat-sdk/CHANGELOG.md).

## License

MIT. See [LICENSE](https://github.com/agentcathq/agentcat-rust-sdk/blob/main/agentcat-sdk/LICENSE).
