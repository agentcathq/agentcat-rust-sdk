# Decision log for `agentcat-sdk` (Rust)

Owner decisions. Where this file and another document disagree, this file wins. The design that
applies them is [`superpowers/specs/2026-10-19-rust-sdk-design.md`](superpowers/specs/2026-10-19-rust-sdk-design.md).

- Round 1: 2026-10-19, during the first port (key order, rmcp range, headers).
- Round 2: 2026-10-19, design brainstorm: the Rust SDK adopts the PHP/C# decision log, and every
  language-agnostic decision of the PHP log, with the Rust-specific answers below. Round 2 replaces
  round 1's "fix TS defects" choice.

## Carried over from the PHP decision log

Source: `agentcat-php-sdk/docs/audit/DECISIONS.md`. C# applies the same set
(`agentcat-csharp-sdk/docs/superpowers/specs/2026-10-19-csharp-sdk-design.md` §4).

| PHP ID | Decision | How Rust applies it |
|---|---|---|
| D06 | Publish all HTTP headers verbatim, `Authorization` and `Cookie` included | Every header of `http::request::Parts`, lower-case names, repeated values joined with `, ` |
| D07, D33 | Diagnostics carry every log line; no special disclosure | Same; the README describes what is sent and never claims that nothing about users is sent |
| D08 | Redaction scope as TS: `identifyActorData` goes through the string hook; custom events skip it | Same |
| D09 | String limits in UTF-16 code units | Counted with `encode_utf16`; cuts land on a `char` boundary (Rust strings cannot hold a lone surrogate) |
| D10 | `error.type` is the fully qualified type name | Dotted path, see R2 |
| D15 | Copy TS behavior, its known defects included | Same; the only differences are in R1–R11 and the design §10 |
| D16 | Session text block on unknown tools and validation errors as TS | Same where rmcp returns a result; rmcp answers unknown tools with a JSON-RPC error (as TS v2 and C#) |
| D18 | Exporter identity strings as TS (`sentry_client=agentcat/1.0.0`) | Same |
| D31 | Silent in test environments unless a fake is active | Replaced by R3 |
| D32 | Never throw from runtime telemetry; `track()` returns the server it was given; a bad option value logs one line | `build()` and `track()` never fail; AgentCat errors and panics never reach the tool result |
| D34 | `parameters.extra` in the TS v1 shape: `{sessionId?, requestId, _meta?, requestInfo?: {headers, url?}, authInfo?}` | Same keys from the rmcp request context; no `authInfo` (auth claims live in `http_request().extensions`); `url` only when the request URI is absolute (design §9.5) |
| D35 | Hooks receive an immutable tool-call snapshot | `ToolCallRequest` |
| D36 | A missing project ID reads `AGENTCAT_PROJECT_ID`; `""` means telemetry only | `AgentCat::builder(None)` |
| D40 | Custom events take a typed value; TS validation errors carry the TS messages | `CustomEvent`; `Error::MissingProjectId` displays `projectId is required for publishCustomEvent` |
| D41 | A fake records events for assertions | `agentcat::testing::fake()` → `AgentCatFake` |
| D05 | Public `flush()` and `shutdown()` | Same; in Rust they are required before exit (R11) |
| D43 | `flush()` returns `true` when every queued event was sent | `flush`, `shutdown` and their blocking variants return `bool` |
| D44 | `AGENTCAT_ENABLED=false` turns everything off; `true` forces it on | Same (with R3 there is nothing to force on) |
| D45 | One public configuration type per exporter, with TS key names | `OtlpExporter`, `DatadogExporter`, `SentryExporter`, `PostHogExporter`; `ExporterConfig::from_json` takes TS keys |
| D50 | Diagnostics read `AGENTCAT_DISABLE_DIAGNOSTICS`, `AGENTCAT_DIAGNOSTICS_ENDPOINT`, `AGENTCAT_DIAGNOSTICS_TOKEN`; unprefixed `DISABLE_DIAGNOSTICS` only turns them off | Same |
| D60, D61 | The SDK writes its own lines to its own `agentcat.log`, not the app's log | `~/agentcat.log` by default; `.log_to_tracing(true)` routes them to `tracing` |
| B09 | `publishCustomEvent` quirks as TS, misleading log line included | Same |

PHP decisions that do not apply to Rust: D01, D04, D11–D14, D17, D19–D30, D37–D39, D42, D46–D49,
D51–D62. They cover PHP runtimes, PHP tooling, PHP frameworks, or PHP packaging; R1–R11 give the Rust
answers where Rust needs one (identity, versions, lifecycle).

## Rust-specific decisions

| ID | Topic | Decision | Round |
|---|---|---|---|
| R1 | API client (PHP D03) | Keep the full generated `agentcat-api` client (`publish_event` over reqwest 0.13). Unlike PHP, it causes no version conflict: rmcp uses the same reqwest. Consequences: timestamps serialize `+00:00`; `tags` is an unordered map; `Retry-After` is not visible. | 2 |
| R2 | `error.type` form (D10) | Dotted full path, for example `std.num.ParseIntError`: fully qualified like D10, and safe from the server's redaction, which rewrites `::` as an IPv6 address. | 2 |
| R3 | Test environments (D31) | No automatic detection. AgentCat runs in customer test runs unless `AGENTCAT_ENABLED=false`; `testing::fake()` records events for assertions. Diagnostics are on in test runs unless turned off. | 2 |
| R4 | rmcp versions | `>= 3.0, < 4` only; no adapters for rmcp 0.x, 1.x or 2.x. | 1 |
| R5 | JSON key order | `mcp_session` must be the first key of `structuredContent`; `serde_json/preserve_order` is a default feature (opt-out), never opt-in. A one-time log line warns when key order is not preserved. | 1 |
| R6 | SDK identity | `sdk_language` = `Rust`; `error.platform` and the diagnostics `agentcat.sdk.language` = `rust`. | 1 |
| R7 | Process | The C# bar: this log, a design spec, milestone plans executed with subagent-driven development, a parity suite by TS test title, and the CI gates of the design §11. The existing code is reworked in place, test first. | 2 |
| R8 | Kept although TS differs | A 10 s timeout per HTTP attempt (as C#); `context` is stripped with tracing off (as TS high-level servers do through their inner tap); customer tags are capped at 50 minus the SDK tags (the generated client sends tags unordered). | 2 |
| R9 | Repository | `agentcathq/agentcat-rust-sdk`, private. | 1 |
| R10 | Golden fixtures | The PHP audit fixtures are the shared oracle, plus the Rust vectors generated from the TS functions (`scripts/ts-fixtures/`). | 2 |
| R11 | Lifecycle | No signal handlers and no exit hook (Rust has none); customers call `shutdown()` before exit. | 2 |
