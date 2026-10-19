//! The README's code snippets, compiled. Keep them in sync with README.md.
#![allow(dead_code, unused_variables, clippy::redundant_closure)]

mod common;

use std::time::Duration;

use agentcat::{AgentCat, UserIdentity};
use common::Todo as MyServer;

mod myapi {
    pub struct User {
        pub id: String,
        pub name: String,
    }
    pub async fn get_user(_token: String) -> Result<User, ()> {
        Ok(User {
            id: "u".into(),
            name: "n".into(),
        })
    }
}

fn redact(text: &str) -> String {
    text.to_owned()
}

async fn redact_async(text: String) -> String {
    text
}

async fn getting_started() -> anyhow::Result<()> {
    use rmcp::{ServiceExt, transport::stdio};

    // Track the server with AgentCat
    let agentcat = agentcat::AgentCat::new("proj_0000000");
    let service = agentcat.track(MyServer::new()).serve(stdio()).await?;
    service.waiting().await?;

    // Flush queued events before exit
    agentcat::shutdown(Duration::from_secs(5)).await;
    Ok(())
}

fn streamable_http() {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let agentcat = agentcat::AgentCat::new("proj_0000000");
    let service = StreamableHttpService::new(
        move || Ok(agentcat.track(MyServer::new())), // track every instance
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );
    let app: axum::Router = axum::Router::new().nest_service("/mcp", service);
}

fn identifying_users() {
    let agentcat = AgentCat::builder("proj_0000000")
        .identify(|call| {
            let token = call.header("authorization").map(str::to_owned); // HTTP header
            let org_id = call.header("x-org-id").map(str::to_owned);
            async move {
                let user = myapi::get_user(token?).await.ok()?;
                Some(
                    UserIdentity::new(user.id)
                        .name(user.name)
                        .data(serde_json::json!({ "orgId": org_id })),
                )
            }
        })
        .build();
}

fn redacting() {
    let agentcat = AgentCat::builder("proj_0000000")
        .redact_sensitive_information(|text| redact(text))
        .build();
    let agentcat = AgentCat::builder("proj_0000000")
        .redact_sensitive_information_async(|text| async move { redact_async(text).await })
        .build();

    let agentcat = AgentCat::builder("proj_0000000")
        .redact_event(|mut event| {
            // Drop events from tools that handle secrets entirely
            if event.resource_name.as_deref() == Some("get_credentials") {
                return None;
            }
            // Strip response payloads from a specific tool
            if event.resource_name.as_deref() == Some("export_report") {
                event.response = None;
            }
            Some(event)
        })
        .build();
}

fn vendors() -> anyhow::Result<()> {
    use agentcat::exporters::{
        DatadogExporterConfig, OtlpExporterConfig, PostHogExporterConfig, SentryExporterConfig,
    };

    // Use AgentCat::telemetry_only() instead if you just want to forward telemetry
    let agentcat = AgentCat::builder("proj_0000")
        // /v1/traces is appended automatically
        .exporter("otlp", OtlpExporterConfig::new("http://localhost:4318"))
        .exporter(
            "datadog",
            DatadogExporterConfig::new(std::env::var("DD_API_KEY")?, "datadoghq.com", "my-mcp-server"),
        )
        .exporter(
            "sentry",
            SentryExporterConfig::new(std::env::var("SENTRY_DSN")?).environment("production"),
        )
        .exporter(
            "posthog",
            // Optional: defaults to US region
            PostHogExporterConfig::new(std::env::var("POSTHOG_API_KEY")?).host("https://us.i.posthog.com"),
        )
        .build();
    Ok(())
}

fn diagnostics_and_custom_events() {
    let agentcat = AgentCat::builder("proj_0000000")
        .disable_diagnostics(true)
        .build();
    agentcat.publish_custom_event(agentcat::CustomEvent::new().resource_name("checkout"));
}

#[test]
fn readme_snippets_compile() {}
