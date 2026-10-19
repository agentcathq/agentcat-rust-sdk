# 04 — How Rust MCP servers use the SDK crates

This report extends `docs/ts-sdk-audit/03-rust-mcp-ecosystem-and-design.md`. Report 03
describes rmcp internals and the proposed `Tracked<H: ServerHandler>` design. This report adds
usage evidence: download and dependency data from crates.io, a feature census of 2,440
dependent manifests, and a code survey of 15 real Rust MCP servers.

Research date: 2026-10-19. Read only. Nothing was built or run.

- Clones (shallow, sparse `*.rs` / `*.toml` / `Cargo.lock`): `/home/ubuntu/misc/rust-mcp-survey/`.
- rmcp source: `modelcontextprotocol/rust-sdk` `main` at `ad2a66a` (2026-10-19). Its
  `crates/rmcp/src/handler/server.rs` is identical to tag `rmcp-v3.5.1`.
- Citations: `owner/repo/path:line` for cloned code. For rmcp, paths start at
  `modelcontextprotocol/rust-sdk/crates/`. Short form `rmcp/src/...` means
  `modelcontextprotocol/rust-sdk/crates/rmcp/src/...`.
- Raw data (JSON from crates.io and GitHub, parsing scripts):
  `/tmp/claude-1000/-home-ubuntu-agentcat-rust-sdk/78031ef1-7fed-4ae1-94f2-11a86f1fe961/scratchpad/usage/`.

---

## 0. Summary

1. **rmcp is the only Rust MCP SDK with real usage.** It has 20.9 M downloads in 90 days and
   2,255 reverse dependencies. The next server SDK, `rust-mcp-sdk`, has 117 k and 52. GitHub
   code search finds `use rmcp` in about 39,000 Rust files and `use rust_mcp_sdk` in about
   1,200 (§1).
2. **rmcp 3.x is the main line today, but not the only one.** In the last 7 days 3.x had 73 %
   of rmcp downloads, 1.x 12 %, 2.x 10 %, 0.x 5 %. Of the 1,254 dependents that published in
   the last 90 days, 815 (65 %) declare 3.x (§2).
3. **Popular servers lag behind.** Of the 15 surveyed servers, 9 run 3.x. Chroma (1.8),
   SurrealDB (1.7), vibe-kanban (1.3), devenv (2.0), readyset (2.2) and rust-docs-mcp-server
   (0.1.5) cannot use a 3.x-only SDK until they upgrade (§4).
4. **Dependents use the default features.** 95 % enable `server`, 90 % `macros`, 76 %
   `transport-io` (stdio), 23 % `transport-streamable-http-server`. `auth` is 3 %, `local`
   is one crate. AgentCat needs only `rmcp/server` (§3).
5. **Stdio and HTTP matter equally.** 11 of 15 servers serve stdio and 11 serve streamable
   HTTP. 10 of the 11 HTTP servers use axum. 7 use sessions, 2 are stateless, 2 let the user
   choose (§4.3).
6. **Manual `ServerHandler` impls are as common as macros among large projects.** Codex,
   Wassette, Apollo, Windmill, devenv and bevy_brp write their own `call_tool` and
   `list_tools`. No surveyed server implements `Service<RoleServer>` directly. Only Apollo
   overrides `get_tool` (§4.3).
7. **A value wrapper fits every surveyed 3.x server.** Every server hands rmcp one handler
   value or one factory closure. `agentcat.track(h)` and a `Clone` `Tracked<H>` cover all of
   them. Five factories clone one prebuilt handler (`move || Ok(h.clone())`) (§4.3).
8. **Identity lives in `http::request::Parts`, often as typed values.** Apollo, Chroma,
   SurrealDB and Windmill read auth values that their axum middleware put into
   `parts.extensions`. The `identify` hook must expose the whole `Parts`, not only headers
   (§4.3).
9. **Tool lists change at runtime** in Wassette, Apollo and Windmill. They call
   `notify_tool_list_changed` and override `listen`. AgentCat must forward these methods and
   must not cache injected tool lists across changes (§4.3).
10. **rmcp ships fast and changes behavior inside a major.** It had 68 releases in 19 months
    and three majors in 2026. rmcp-macros 1.4.0 changed the default router of
    `#[tool_handler]`. 3.3.0 added a trait method. 3.4.0 added the `ServerConfig` alias and
    deprecated `ServerInfo`. The unreleased 3.6.0 calls the HTTP factory for each tool-schema
    lookup. No 4.0 work is in progress (§5).
11. **Recommendation:** depend on `rmcp = { version = "3", default-features = false,
    features = ["server"] }`. Test against 3.0.0 and the newest 3.x. Do not build pre-3
    adapters for v1. Add a process-global handle, a blocking flush, a whole-`Parts` hook view,
    a per-server switch for `get_more_tools`, and documentation for git-pinned rmcp (§6).

---

## 1. Which Rust MCP crates have real usage

Sources: `https://crates.io/api/v1/crates?q=mcp&sort=recent-downloads` (pages 1-3),
`/api/v1/crates/{name}`, `/api/v1/crates/{name}/reverse_dependencies` (`meta.total`),
`/api/v1/crates/{name}/owners`, and the GitHub REST API for stars. All data is from
2026-10-19. "Official" means the repository is in the `modelcontextprotocol` GitHub org.

| Crate | Latest stable (date) | Total downloads | 90-day downloads | Reverse deps | GitHub stars | Maintainer |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| **rmcp** | 3.5.1 (2026-10-05) | 37,643,505 | 20,922,462 | 2,255 | 4,029 | official (`modelcontextprotocol/rust-sdk`), crates.io owners jamadeo, alexhancock |
| rmcp-macros | 3.5.1 (2026-10-05) | 32,056,912 | 15,965,444 | 36 | (same repo) | official |
| rust-mcp-schema | 2.0.0 (2026-08-22) | 1,020,785 | 406,526 | 22 | 77 | community (`rust-mcp-stack`, hashemix) |
| rust-mcp-sdk | 2.0.0 (2026-08-27) | 316,213 | 116,784 | 52 | 198 | community (`rust-mcp-stack`, hashemix) |
| rmcp-actix-web | 0.13.1 (2026-09-28) | 259,183 | 67,619 | 5 | 6 (GitLab) | community add-on **for rmcp** (`rmcp ^3.0.0`) |
| pmcp | 2.22.7 (2026-10-04) | 135,006 | 59,506 | 23 | 52 (`paiml/rust-mcp-sdk`) | community (Pragmatic AI Labs) |
| tower-mcp | 0.25.0 (2026-10-19) | 35,109 | 22,356 | 15 | 11 | community |
| rmcp-openapi | 0.32.0 (2026-09-28) | 137,299 | 12,788 | 3 | (GitLab) | community add-on for rmcp |
| turbomcp | 3.6.0 (2026-10-01) | 36,622 | 10,869 | 9 | 98 | community (Epistates) |
| mcp-core | 0.1.50 (2025-05-01) | 104,158 | 6,989 | 10 | 40 | community, no release since 2025-05 |
| mcp-spec | 0.1.0 (2025-02-27) | 99,299 | 4,703 | 4 | (rust-sdk repo) | official org, **abandoned** (owners dsp-ant, baxen) |
| ultrafast-mcp | 202506018.1.0 (2025-07-13) | 17,187 | 2,922 | 3 | 15 | community, stale |
| mcp-protocol-sdk | 0.5.1 (2025-08-03) | 9,498 | 1,581 | 2 | 16 (archived) | community, archived |
| mcpr | 0.2.3 (2025-03-16) | 12,418 | 1,516 | 0 | 347 (archived) | community, archived |
| poem-mcpserver | 0.3.1 (2025-10-13) | 54,793 | 1,209 | 1 | 4,454 (all of `poem`) | Poem web framework |
| async-mcp | 0.1.3 (2025-08-08) | 13,681 | 1,099 | 1 | 22 | community, stale |
| mcp-sdk | 0.0.3 (2025-01-20) | 13,327 | 955 | 1 | 68 | community (AntigmaLabs), stale |
| mcp-attr | 0.0.7 (2025-05-29) | 8,227 | 934 | 3 | 28 | community, stale |
| mcp-server | 0.1.0 (2025-02-27) | 60,951 | 900 | 0 | (rust-sdk repo) | official org, **abandoned** (owners dsp-ant, baxen) |

Notes:

- `mcp-server` and `mcp-spec` are the first official crates from the rust-sdk repo
  (February 2025). rmcp replaced them. They still get a few downloads.
- Report 03 gave turbomcp's latest version as 4.0.0-alpha.5. The latest stable release is
  3.6.0 (2026-10-01). 4.0.0-alpha.5 (2026-09-14) is a pre-release
  (`https://crates.io/api/v1/crates/turbomcp`).
- `rust-mcp-schema` downloads are higher than `rust-mcp-sdk` downloads because other crates use
  the schema types without the SDK.
- The search also returns `smg-mcp` (1.8 M total downloads). It is an MCP **client** that
  depends on `rmcp ^1.7`. It inflates rmcp 1.x downloads.
- GitHub code search counts (approximate `total_count` from `GET /search/code`, Rust files,
  2026-10-19): `"use rmcp"` 39,040, `"use rust_mcp_sdk"` 1,156. That is a 34:1 ratio.

**Answer:** rmcp has about 180 times the 90-day downloads and 43 times the reverse
dependencies of the next server SDK. Two of the five next-largest crates (`rmcp-actix-web`,
`rmcp-openapi`) are rmcp add-ons. A `ServerHandler` wrapper covers them. The other SDKs do not
justify an adapter now. Report 03 §2 reached the same conclusion with older data.

---

## 2. rmcp versions in active use

### 2.1 Download share by major line

Source: `https://crates.io/api/v1/crates/rmcp/{version}/downloads` for each of the 68
versions. Each response gives daily downloads from 2026-07-22 to 2026-10-19. The crate-level
`/api/v1/crates/rmcp/downloads` endpoint lists only the top five versions and puts the rest in
`meta.extra_downloads`, so this report sums the per-version data.

| Line | 90 days | 30 days | 7 days |
| --- | ---: | ---: | ---: |
| 3.x | 9,959,947 (47.6 %) | 6,697,815 (67.4 %) | 1,863,352 (73.4 %) |
| 2.x | 3,029,089 (14.5 %) | 959,998 (9.7 %) | 243,747 (9.6 %) |
| 1.x | 5,639,208 (27.0 %) | 1,580,573 (15.9 %) | 296,813 (11.7 %) |
| 0.x | 2,294,245 (11.0 %) | 701,635 (7.1 %) | 133,537 (5.3 %) |
| Total | 20,922,489 | 9,940,021 | 2,537,449 |

The 90-day column understates 3.x because 3.0.0 was released on day 7 of that window
(2026-07-28).

### 2.2 Download share by minor line (top 14, last 7 days)

| Minor | 7-day downloads | Share | Minor | 7-day downloads | Share |
| --- | ---: | ---: | --- | ---: | ---: |
| 3.1 | 720,492 | 28.4 % | 3.2 | 66,412 | 2.6 % |
| 3.5 | 664,394 | 26.2 % | 0.8 | 40,788 | 1.6 % |
| 3.4 | 343,773 | 13.5 % | 3.3 | 35,701 | 1.4 % |
| 2.2 | 209,961 | 8.3 % | 3.0 | 32,580 | 1.3 % |
| 1.8 | 174,923 | 6.9 % | 2.1 | 31,026 | 1.2 % |
| 1.7 | 74,562 | 2.9 % | 0.16 | 26,365 | 1.0 % |

Across all 90 days the largest single versions are 1.8.0 (2.80 M), 2.2.0 (2.45 M), 3.1.3
(2.09 M) and 3.2.0 (1.82 M). The 3.1 line is high because some heavy consumers pin it exactly
(see §2.3). The last patch of each older line carries most of that line: 0.8.5, 1.8.0, 2.2.0.
Downloads include CI runs and transitive pulls, so §2.3 also counts dependents.

### 2.3 Version requirements of reverse dependencies

Source: `https://crates.io/api/v1/crates/rmcp/reverse_dependencies?per_page=100&page=1..23`,
2,255 records, one per dependent crate (crates.io reports the dependency of each crate's
latest version). "Active" means the dependent published a version on or after 2026-07-22.

| Line declared | All dependents | Active dependents | Total downloads of those dependents |
| --- | ---: | ---: | ---: |
| 3.x | 815 | 815 (65 %) | 4.39 M |
| 2.x | 188 | 151 (12 %) | 0.34 M |
| 1.x | 609 | 188 (15 %) | 5.28 M |
| 0.x | 643 | 100 (8 %) | 0.83 M |
| Total | 2,255 | 1,254 | |

Requirement floors inside each line (all dependents, with active dependents in brackets):

- 3.x: `^3` 88, 3.0 54, **3.1 293**, 3.2 55, 3.3 27, 3.4 109, 3.5 189.
- 2.x: `^2` 49 [40], 2.0 25 [18], 2.1 27 [21], 2.2 87 [72].
- 1.x: `^1` 83 [28], 1.1 48, 1.2 61, 1.3 59, 1.4 36, 1.5 38, 1.6 46, **1.7 191 [63]**, 1.8 44 [34].
- 0.x: spread over 0.1 to 0.17. The largest groups are 0.12 (105), 0.16 (79), 0.14 (75),
  0.1 (67) and 0.8 (60).

Requirement style: 2,117 caret, 120 exact (`=`), 13 tilde, 5 ranges (`>=`). Exact pins
matter because a library that requires a higher floor cannot resolve with them. The most
common exact pins are `=3.5.1` (16), `=2.2.0` (15), `=3.1.4` (15), `=3.2.0` (9). 76 of the
815 3.x dependents pin exactly. 36 pin a version below 3.2 (for example `=3.1.4`, `=3.0.1`).
Windmill pins `=3.1.0` (`windmill-labs/windmill/backend/Cargo.toml:667`). Codex pins a git
revision with `version = "=3.5.1"` (`openai/codex/codex-rs/Cargo.toml:455`).

### 2.4 Unpublished projects on GitHub

Most MCP servers are binaries that are not on crates.io. A second sample covers them:
`gh search code` for Cargo.toml files that mention `rmcp`, `transport-streamable-http-server`
or `rmcp transport-io` (198 files, 185 repositories, 99 of them with at least 100 stars).
The manifests were parsed with `tomllib`.

| Line declared | Repos | Repos with ≥ 100 stars |
| --- | ---: | ---: |
| 3.x | 35 | 26 |
| 2.x | 12 | 6 |
| 1.x | 49 | 25 |
| 0.x | 61 | 23 |
| unknown (workspace inheritance, root manifest not sampled) | 28 | 19 |

The GitHub sample is older than the crates.io sample: only about 22 % of repositories with a
known version are on 3.x. Eleven dependency entries in ten repositories take rmcp from git,
not crates.io (for example `pydantic/mcpdb`, `tidewave-ai/mcp_proxy_rust`). Codex does the
same (§4.2).

### 2.5 Answer

- **3.x is the active line.** It has 73 % of downloads this week and 65 % of recently active
  published dependents. Inside 3.x, every minor from 3.0 to 3.5 is in use, and 3.1 is the most
  common floor (293 dependents).
- **1.7/1.8 and 2.2 are the living legacy lines.** Together they have about 18 % of downloads
  this week, and they include large projects (Chroma 1.8, SurrealDB 1.7, readyset 2.2,
  devenv 2.0).
- **0.x is a long tail.** It has 5 % of downloads and 643 mostly inactive dependents. The
  0.8.5 release still has 41 k downloads per week.

---

## 3. Which rmcp features dependents enable

### 3.1 crates.io dependents

Source: the `features` and `default_features` fields of all 2,255 reverse-dependency records
(§2.3). The table counts the 2,213 normal (non-dev) dependencies. A dependency with
`default_features = true` also gets rmcp's defaults, which are `base64`, `macros` and
`server` in every major (`/api/v1/crates/rmcp` `versions[].features.default` for 0.1.5,
0.8.5, 0.16.0, 1.0.0, 1.8.0, 2.0.0, 2.2.0, 3.0.0, 3.5.1).

| Feature (effective) | All (n=2,213) | 3.x (n=795) | 2.x (n=183) | 1.x (n=598) | 0.x (n=637) |
| --- | ---: | ---: | ---: | ---: | ---: |
| `server` | 2,107 (95 %) | 727 (91 %) | 171 (93 %) | 574 (96 %) | 635 (100 %) |
| `macros` | 1,998 (90 %) | 659 (83 %) | 161 (88 %) | 548 (92 %) | 630 (99 %) |
| `transport-io` (stdio) | 1,673 (76 %) | 609 (77 %) | 139 (76 %) | 456 (76 %) | 469 (74 %) |
| `transport-streamable-http-server` | 507 (23 %) | 190 (24 %) | 52 (28 %) | 143 (24 %) | 122 (19 %) |
| `client` | 503 (23 %) | 164 (21 %) | 39 (21 %) | 152 (25 %) | 148 (23 %) |
| `schemars` (explicit) | 391 (18 %) | 129 (16 %) | 35 (19 %) | 142 (24 %) | 85 (13 %) |
| `transport-child-process` | 312 (14 %) | 100 (13 %) | 26 (14 %) | 77 (13 %) | 109 (17 %) |
| `elicitation` | 105 (5 %) | 46 (6 %) | 3 (2 %) | 48 (8 %) | 8 (1 %) |
| `auth` | 76 (3 %) | 20 (3 %) | 4 (2 %) | 16 (3 %) | 36 (6 %) |
| `transport-async-rw` | 57 (3 %) | 30 (4 %) | 2 (1 %) | 14 (2 %) | 11 (2 %) |
| `transport-streamable-http-server-session` | 39 (2 %) | 11 (1 %) | 4 (2 %) | 12 (2 %) | 12 (2 %) |
| `transport-sse-server` | 31 (1 %) | 0 | 0 | 0 | 31 (5 %) |
| `transport-worker` | 22 (1 %) | 7 (1 %) | 0 | 4 (1 %) | 11 (2 %) |
| `tower` | 17 (1 %) | 1 | 0 | 8 (1 %) | 8 (1 %) |
| `server-side-http` | 16 (1 %) | 9 (1 %) | 1 | 6 (1 %) | 0 |
| `request-state` | 11 (0.5 %) | 11 (1 %) | 0 | 0 | 0 |
| `local` | 1 | 0 | 1 | 0 | 0 |

Other facts from the same data:

- 407 dependents set `default-features = false`. Of these, 301 add `server` back and 192 add
  `macros` back.
- 362 dependents enable both stdio and the HTTP server. 145 enable the HTTP server without
  stdio.
- 89 dependents are client-only (no defaults and no `server`).
- The `transport-sse-server` feature exists up to rmcp 0.10.0 and is gone from 0.11.0
  (2025-12-08) onward (`/api/v1/crates/rmcp` `versions[].features`).

### 3.2 GitHub manifests

The 185 GitHub repositories (§2.4) show the same order: `server` 98 %, `macros` 94 %,
`transport-io` 58 %, `transport-streamable-http-server` 52 %, `client` 32 %, `auth` 11 %,
`transport-sse-server` 6 %. HTTP is over-represented because one of the three searches looked
for `transport-streamable-http-server`.

### 3.3 Answer

- Almost every dependent uses rmcp's default `server` and `macros` features. Three in four
  serve stdio. One in four serves streamable HTTP.
- `auth`, `tower`, `local` and `request-state` are rare. AgentCat must not require them.
- The `local` feature (non-`Send` handlers) has one published dependent. Goose enables it
  only for `wasm32` (`aaif-goose/goose/crates/goose-agent/Cargo.toml:30-31`). It is safe to
  leave `local` unsupported in v1.
- Feature unification means AgentCat cannot turn features off for the customer. It can only
  add them. A dependency with `default-features = false, features = ["server"]` adds nothing
  to a server build, because 91 % of 3.x dependents already enable `server`.

---

## 4. Survey of 15 Rust MCP servers

The survey covers large companies (OpenAI, Block/AAIF, Microsoft, Apollo, PostHog, Chroma,
SurrealDB, Windmill), popular open-source tools (Nushell, Apache Iggy, devenv, Readyset,
vibe-kanban) and small projects (bevy_brp, rust-docs-mcp-server). Stars are from the GitHub
API on 2026-10-19. Elastic's `mcp-server-elasticsearch` (719 stars, rmcp 0.2.1) was also
checked but is left out of the table: its README says it is deprecated and replaced by the
Elastic Agent Builder MCP endpoint (`elastic/mcp-server-elasticsearch/README.md:3-5`).

### 4.1 Overview

| # | Project (stars) | rmcp req → lock | Transports | Handler style | Prompts / resources | Context use | Runtime |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | openai/codex (129,924) | git rev `=3.5.1` → 3.5.1 | HTTP, session (loopback) | manual | no | `context.meta` custom keys | manual multi-thread |
| 2 | aaif-goose/goose (55,441) | 3.4.1 → 3.4.1 | stdio, in-process duplex | macros, stored router | resources (1 server) | `context.meta` | manual multi-thread on 8 MB thread |
| 3 | nushell/nushell (40,690) | 3.4.0 → 3.4.0 | stdio, HTTP session (hyper) | macros, `Self::tool_router()` | no | `ctx.ct` | `Runtime::new` + `block_on` |
| 4 | PostHog/posthog (40,346) | `3` → 3.1.4 | HTTP session | macros, `Self::tool_router()` | no | none (auth in middleware) | `#[tokio::main]` |
| 5 | surrealdb/surrealdb (33,206) | 1.4.0 → 1.7.0 | stdio, HTTP session | macros, `Self::tool_router()` | both, plus `complete` | `Parts` → auth session | manual multi-thread |
| 6 | chroma-core/chroma (29,533) | 1.8.0 → 1.8.0 | HTTP stateless, JSON | macros, stored router | no | `Parts` header, typed scope | manual multi-thread |
| 7 | BloopAI/vibe-kanban (28,400) | 1.2.0 → 1.3.0 | stdio | macros, stored router (1.3 default) | no | none | manual multi-thread |
| 8 | windmill-labs/windmill (18,223) | `=3.1.0` → 3.1.0 | HTTP stateless | manual, generic `Runner<B>` | empty lists | `Parts` typed auth, `listen` | manual multi-thread |
| 9 | cachix/devenv (7,756) | 2.0 → 2.0.0 | stdio (custom tuple), HTTP session | manual, delegates to stored router | no | none | custom runtime (GC threads) |
| 10 | readysettech/readyset (5,292) | 2.2 → 2.2.0 | stdio, HTTP session | `#[tool_router(server_handler)]` | no | none (auth middleware) | `#[tokio::main]` / adapter runtime |
| 11 | apache/iggy (5,115) | 3.5.1 → 3.5.1 | stdio, HTTP session | macros, `Self::tool_router()` | no | none | manual multi-thread |
| 12 | microsoft/wassette (967) | 3.1.3 → 3.5.1 | stdio, HTTP (session or stateless) | manual (boxed futures) | lists | `Peer`, `Parts`, `mcp-session-id`, `listen` | `#[tokio::main]` |
| 13 | apollographql/apollo-mcp-server (315) | 3.5.1 → 3.5.1 | stdio, HTTP (session or stateless) | manual, 15 overrides | both | `Parts`, typed token, forwards `mcp-session-id`, `listen` | `#[tokio::main]` |
| 14 | natepiano/bevy_brp (77) | 3.4.1 → 3.5.0 | stdio | manual, own proc macros | no | none | `#[tokio::main]` |
| 15 | Govcraft/rust-docs-mcp-server (298) | 0.1.5 → 0.1.5 | stdio | 0.1 `#[tool(tool_box)]` | resource (not advertised) | unused `Peer` slot | `#[tokio::main]` |

### 4.2 Per-project notes

Each entry gives the serve code, then the details that matter for `Tracked<H>`.

#### 1. openai/codex (commit 982695c)

Codex has no `codex mcp-server` subcommand at this commit. `codex-rs` holds only `codex-mcp`
and `rmcp-client`. The only production rmcp server is a loopback HTTP server inside the TUI
that exposes "dynamic tools" to the codex app server
(`openai/codex/codex-rs/tui/src/dynamic_tools_mcp.rs:1`). Codex is mainly an rmcp client.

```rust
let service = StreamableHttpService::new(
    move || Ok(handler.clone()),
    Arc::new(LocalSessionManager::default()),
    StreamableHttpServerConfig::default(),
);
let router =
    Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(
            authorization,
            require_authorization,
        ));
```
(`openai/codex/codex-rs/tui/src/dynamic_tools_mcp.rs:190-202`)

- rmcp from git: `rmcp = { git = "…/rust-sdk", rev = "98a5478…", version = "=3.5.1", default-features = false }`
  (`openai/codex/codex-rs/Cargo.toml:455`). There is no `[patch.crates-io]` entry for rmcp
  (`Cargo.toml:656-667`).
- Manual `ServerHandler`: `get_info`, `list_tools`, `call_tool`, no `get_tool`
  (`dynamic_tools_mcp.rs:248-403`). The tool list is rebuilt on every `list_tools` call
  (`:253-299`).
- `call_tool` reads custom `_meta` keys `x-codex-turn-metadata`, `threadId`, `turnId`,
  `callId` (`:306-326`). Auth is a bearer-token axum middleware (`:222-236`).
- Logs go to the `codex-tui.log` file (`openai/codex/codex-rs/tui/src/lib.rs:97, 293`).
  `Drop` aborts the axum task (`dynamic_tools_mcp.rs:216-219`).
- The TUI enables `serde_json/preserve_order` (`openai/codex/codex-rs/tui/Cargo.toml:104`).
- **For AgentCat:** `move || Ok(agentcat.track(handler.clone()))`. AgentCat must not touch
  `_meta`. A git-pinned rmcp needs a `[patch.crates-io]` entry in the customer workspace,
  or AgentCat's crates.io rmcp becomes a second, incompatible copy.

#### 2. aaif-goose/goose (commit fb9e3c8)

Goose ships four built-in servers in `goose-mcp` (`aaif-goose/goose/crates/goose-mcp/src/lib.rs:40-47`).
It runs them in-process over `tokio::io::duplex`, or as `goose mcp <name>` over stdio.

```rust
pub async fn serve<S>(server: S) -> Result<()>
where
    S: rmcp::ServerHandler,
{
    let service = server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {:?}", e);
    })?;
    service.waiting().await?;
    Ok(())
}
```
(`aaif-goose/goose/crates/goose-mcp/src/mcp_server_runner.rs:39-50`; the duplex variant is at
`crates/goose-mcp/src/lib.rs:57-72`)

- Macros with a stored router: `#[tool_handler(router = self.tool_router)]`
  (`crates/goose-mcp/src/memory/mod.rs:116, 138, 510`). ComputerController merges routers
  with `+=` (`computercontroller/mod.rs:366-370`).
- AutoVisualiser overrides `list_resources` and `read_resource` for MCP Apps `ui://`
  templates (`autovisualiser/mod.rs:628-685`).
- Memory tools read a working directory from `context.meta` (`memory/mod.rs:397-400`).
- Servers are registered as bare `fn` pointers: `SpawnServerFn = fn(DuplexStream, DuplexStream)`
  (`crates/goose-mcp/src/lib.rs:49, 79-86`). A closure cannot capture an `AgentCat` handle
  there.
- JSON file logs only (`crates/goose-cli/src/logging.rs:8-22`). Manual runtime on a thread
  with an 8 MB stack (`crates/goose-cli/src/main.rs:41-50`).
- **For AgentCat:** wrap at the two generic choke points, `serve<S>` and `spawn_and_serve<S>`.
  The `fn`-pointer registry needs a process-global AgentCat handle.

#### 3. nushell/nushell (commit 2deff7f)

`nu --mcp` runs the `nu-mcp` library (`nushell/nushell/src/main.rs:615-634`).

```rust
let service = TowerToHyperService::new(StreamableHttpService::new(
    {
        let engine_state = engine_state.clone();
        move || Ok(NushellMcpServer::new((*engine_state).clone()))
    },
    session_manager,
    http_config.clone(),
));
```
(`nushell/nushell/crates/nu-mcp/src/lib.rs:202-209`; stdio at `:155-162`)

- HTTP runs on hyper-util, not axum (`lib.rs:217-234`).
- Bare `#[tool_handler]`, so the router is `Self::tool_router()`. The struct also stores a
  `tool_router` field marked `#[allow(dead_code)]` that nothing reads
  (`crates/nu-mcp/src/server.rs:14-15, 28, 83-84`).
- Overrides only `get_info` (`server.rs:85-93`). The `evaluate` tool passes `ctx.ct` to the
  evaluator (`server.rs:61`).
- A sync function builds `tokio::runtime::Runtime::new()` and calls `block_on`
  (`lib.rs:121-150`). Logs go to stderr (`lib.rs:89-100`).
- The HTTP factory clones the whole `EngineState` per session (`lib.rs:205`).
- **For AgentCat:** `track` for stdio, `track_factory` for HTTP. The sync entry point needs a
  blocking flush.

#### 4. PostHog/posthog (commit 6f3b99c)

`pgapi` is an internal API and MCP server over a Postgres telemetry database
(`PostHog/posthog/rust/pgapi/Cargo.toml:2-6`).

```rust
StreamableHttpService::new(
    move || Ok(PgMcp::new(state.clone())),
    LocalSessionManager::default().into(),
    config,
)
```
(`PostHog/posthog/rust/pgapi/src/mcp.rs:423-427`)

- Requirement is a bare `"3"` (`rust/pgapi/Cargo.toml:22`), resolved to 3.1.4
  (`rust/Cargo.lock:11063-11064`).
- Bare `#[tool_handler]` with about 20 tools. A stored `tool_router` field carries the stale
  comment "referenced by the #[tool_handler] macro expansion" (`mcp.rs:24-25, 135, 407`).
- An axum middleware authenticates from `Tailscale-User-Login`, `x-amzn-oidc-data` or
  `X-Auth-Request-Email` and inserts the principal into the request extensions
  (`rust/pgapi/src/auth.rs:4-7, 296`). The handler never reads it.
- `#[tokio::main]`, logs to stdout, `axum::serve` without graceful shutdown
  (`rust/pgapi/src/main.rs:82, 88-92, 177`).
- **For AgentCat:** `track_factory` around the closure. `identify` needs `&Parts` to reach the
  principal in `parts.extensions`.

#### 5. surrealdb/surrealdb (commit 18971ff)

`surrealdb-mcp` is a published library, served at `/mcp` and by `surreal mcp` over stdio
(`surrealdb/surrealdb/surrealdb/mcp/Cargo.toml:2-8`).

```rust
let mut config = StreamableHttpServerConfig::default();
config.stateful_mode = true;
StreamableHttpService::new(
    move || {
        let svc = McpService::new(Arc::clone(&ds), None, None, Session::default());
        let svc = if let Some(rec) = metrics_recorder.clone() {
            svc.with_metrics_recorder(rec).with_transport_label("http")
        } else {
            svc.with_transport_label("http")
        };
        Ok(svc)
    },
    Arc::new(LocalSessionManager::default()),
    config,
)
```
(`surrealdb/surrealdb/surrealdb/mcp/src/service.rs:970-984`; stdio uses
`rmcp::ServiceExt::serve(service, (stdin, stdout))` at `:919-928`)

- The axum handlers call `tower_service::Service::call` on the rmcp service directly
  (`surrealdb/surrealdb/surrealdb/server/src/ntw/mcp.rs:34-48, 63-74`).
- Overrides `get_info`, `initialize`, `list_resources`, `list_resource_templates`,
  `read_resource`, `list_prompts`, `get_prompt`, `complete` (`mcp/src/service.rs:782-913`).
  `initialize` returns `Ok(self.get_info())` and reads the auth `Session` from `Parts`
  (`:796-827`).
- Every request checks that the caller's subject matches the subject bound at `initialize`,
  as a defense against `mcp-session-id` hijack (`service.rs:395-441`).
- **Likely latent bug:** the constructor edits a stored router
  (`let mut tool_router = Self::tool_router(); …attach(&mut tool_router)`, `service.rs:268-269`),
  but the bare `#[tool_handler]` (`:780`) on rmcp ≥ 1.4 calls `Self::tool_router()`, so the
  edited output schemas are not advertised.
- SurrealDB already records per-tool audit logs and OpenTelemetry metrics
  (`service.rs:467-509`; `surrealdb/surrealdb/surrealdb/server/src/observe/instruments.rs:191-193`).
- **For AgentCat (after an rmcp 3 upgrade):** `Ok(agentcat.track(svc))` inside the factory.
  `Tracked` must forward `initialize` and patch capabilities on its result.

#### 6. chroma-core/chroma (commit ddc17cd)

The MCP endpoint of Chroma's Foundation API (`chroma-core/chroma/rust/foundation-api/Cargo.toml:1-8`).

```rust
let mcp_service = StreamableHttpService::new(
    {
        let server = server.clone();
        move || Ok(FoundationMcpServer::new(server.clone()))
    },
    Arc::new(NeverSessionManager::default()),
    StreamableHttpServerConfig::default()
        .disable_allowed_hosts()
        .with_stateful_mode(false)
        .with_json_response(true),
);
```
(`chroma-core/chroma/rust/foundation-api/src/routes/mcp/mod.rs:124-134`)

- Stateless with JSON responses, so the factory runs for every HTTP request.
- Macros with a stored router (`#[tool_handler(router = self.tool_router)]`,
  `rust/foundation-api/src/routes/mcp/server.rs:431`).
- Tools read the `CHROMA_TOKEN_HEADER` header and a typed `McpScope` from `Parts`
  (`server.rs:463-496`). Middleware rewrites `Authorization: Bearer` into that header and
  inserts the scope (`mod.rs:185-248`).
- Mounted with `route_service`, not `nest_service`, because rmcp rejects requests without a
  Host or authority after `nest` rewrites the URI (`mod.rs:119-122`).
- **For AgentCat (after an rmcp 3 upgrade):** `track_factory`. Wrapper construction must be
  cheap because it runs per request.

#### 7. BloopAI/vibe-kanban (commit d5cbb53)

A standalone stdio binary, `vibe-kanban-mcp` (`BloopAI/vibe-kanban/crates/mcp/Cargo.toml:7-9`).

```rust
let service = server.init().await?.serve(stdio()).await.map_err(|error| {
    tracing::error!("serving error: {:?}", error);
    error
})?;

service.waiting().await?;
```
(`BloopAI/vibe-kanban/crates/mcp/src/bin/vibe_kanban_mcp.rs:42-47`)

- `rmcp = "1.2.0"` resolves to 1.3.0. The same lockfile also has rmcp 0.15.0 through a git
  dependency on Codex (`Cargo.lock:1259-1270, 7833-7834`).
- Eleven named routers are merged with `+` and stored in a field
  (`crates/mcp/src/task_server/tools/mod.rs:52-73`, `task_server/mod.rs:53`). The bare
  `#[tool_handler]` reads that field only because rmcp-macros 1.3 defaults to
  `self.tool_router`. Since 1.4.0 the default is `Self::tool_router()`, which this type does
  not define. A `cargo update` inside `^1.2.0` breaks the build (§5.4).
- `init()` removes a tool from the router's public `map` before serving
  (`task_server/mod.rs:87-99`).
- **For AgentCat:** blocked on rmcp 1.x. After an upgrade, one line.

#### 8. windmill-labs/windmill (commit a3a4fa5)

The `windmill-mcp` library serves workspace scripts and flows as tools
(`windmill-labs/windmill/backend/windmill-mcp/Cargo.toml:1-16`).

```rust
let service_config = StreamableHttpServerConfig::default()
    .with_sse_keep_alive(Some(Duration::from_secs(15)))
    .with_sse_retry(Some(Duration::from_secs(15)))
    .with_cancellation_token(cancellation_token.clone())
    // (comments removed)
    .with_legacy_session_mode(false)
    .disable_allowed_hosts()
    .with_max_request_body_bytes(*crate::REQUEST_SIZE_LIMIT.read().await);

let service =
    StreamableHttpService::new(move || Ok(runner.clone()), session_manager, service_config);

let router = Router::new().route_service("/", service);
```
(`windmill-labs/windmill/backend/windmill-api/src/mcp/core.rs:616-638`)

- Manual, generic `impl<B: McpBackend> ServerHandler for Runner<B>`
  (`backend/windmill-mcp/src/server/runner.rs:449-659`).
- Overrides `get_info`, `supported_protocol_versions`, `initialize` (calls
  `context.peer.set_peer_info` itself), `accepted_subscription_filter`, `listen`,
  `list_tools`, `call_tool`, `list_resources`, `list_prompts`, `list_resource_templates`.
  No `get_tool`.
- The tool list depends on the caller. It comes from database queries filtered by token
  scopes read from `Parts.extensions` (`runner.rs:130-160`).
- A 30-second poller feeds `listen`, which sends `notify_tool_list_changed`
  (`runner.rs:510-553`; `src/server/list_changes.rs:1-14`).
- `initialize` removes `list_changed` from the advertised capabilities.
- **For AgentCat:** `track_factory`. Stateless mode builds a new `Tracked` per request, and
  without `get_tool` a registry rebuild costs a database-backed `list_tools`. The capability
  patch must not re-add `list_changed`.

#### 9. cachix/devenv (commit ff0d7e7)

The `devenv mcp [--http]` subcommand (`cachix/devenv/devenv/src/main.rs:1736-1742`).

```rust
let service = StreamableHttpService::new(
    move || Ok(server.clone()),
    LocalSessionManager::default().into(),
    Default::default(),
);
let router = axum::Router::new().fallback_service(service);
```
(`cachix/devenv/devenv/src/mcp.rs:608-614`; stdio uses
`server.serve((stdin, tokio::io::stdout()))` with a custom `ThreadedStdin`, `:650-652`)

- Manual `ServerHandler` (`get_info`, `list_tools`, `call_tool`) that delegates to a stored
  `ToolRouter` (`mcp.rs:307-336`).
- rmcp 2.0 API: `call_tool` returns `Result<CallToolResult, McpError>` and the code builds a
  `ListToolsResult { meta, tools, next_cursor }` literal (`mcp.rs:318-329`). Neither compiles
  on 3.x.
- Runs on the devenv CLI runtime, whose worker threads register with the Boehm GC
  (`devenv/src/main.rs:1856-1867`). Shutdown is a `tokio::select!` on a CLI-wide token
  (`mcp.rs:660-665`).
- **For AgentCat:** blocked on rmcp 2.x.

#### 10. readysettech/readyset (commit c994867)

One handler, served by a stdio binary and by an HTTP endpoint inside the Readyset adapter.

```rust
let service: StreamableHttpService<ReadysetMcpServer, LocalSessionManager> =
    StreamableHttpService::new(
        move || Ok(mcp_server.clone()),
        Arc::new(session_manager),
        http_config,
    );
// ...
let router = axum::Router::new().nest_service("/mcp", service).layer(
    axum::middleware::from_fn_with_state(auth_state, bearer_auth),
);
```
(`readysettech/readyset/readyset-adapter/src/mcp_http.rs:98-113`; stdio at
`readyset-mcp/src/main.rs:32-38`)

- `#[tool_router(server_handler)]` generates the whole `ServerHandler`
  (`readyset-mcp/src/server.rs:17-18`). There is no `get_info`. On 3.x the server identity
  becomes the `"rmcp"` placeholder.
- The `bearer_auth` middleware parses the JSON-RPC body. It allows only `initialize`,
  `notifications/initialized`, `notifications/cancelled`, `ping`, `tools/list` and
  `tools/call` (`mcp_http.rs:378-388`). It checks `params.name` of each `tools/call` against
  the token scope, and an unknown tool needs the `Full` scope (`:311-373, 408-420`).
- rmcp 2.2 API: `.with_stateful_mode(true)` (`mcp_http.rs:88`), renamed in 3.0 (§5.7).
- **For AgentCat (after an rmcp 3 upgrade):** `move || Ok(agentcat.track(mcp_server.clone()))`.
  The middleware answers 403 to `get_more_tools` for scoped tokens and blocks
  `server/discover`. AgentCat needs a per-server switch for `get_more_tools`.

#### 11. apache/iggy (commit 28bef8f)

`iggy-mcp` exposes about 40 admin and messaging tools of the Iggy streaming platform
(`apache/iggy/core/ai/mcp/src/service/mod.rs:57-730`).

```rust
let service = StreamableHttpService::new(
    move || {
        Ok(IggyService::new(
            iggy_client.clone(),
            iggy_consumer.clone(),
            permissions,
        ))
    },
    LocalSessionManager::default().into(),
    Default::default(),
);
```
(`apache/iggy/core/ai/mcp/src/api.rs:91-101`; stdio at `core/ai/mcp/src/main.rs:151-153, 167`)

- Bare `#[tool_handler]`, no router field (`service/mod.rs:40-47, 734`). `get_info` uses
  `ServerConfig::new(...)` without `server_info`, so the identity is the placeholder
  (`service/mod.rs:735-739`).
- Permission failures return `Err(ErrorData::invalid_request(..))`, a protocol error, not a
  tool error (`main.rs:225-233`).
- Manual multi-thread runtime, then `shutdown_background()` because "Tokio cannot cancel its
  blocking stdin read" (`main.rs:63-69`). The HTTP server runs in a detached task
  (`api.rs:138`). Shutdown is a `select!` on SIGINT/SIGTERM (`main.rs:183-201`).
- Optional OTLP logs and traces (`core/ai/mcp/src/log.rs:74-104`).
- **For AgentCat:** `track` at `main.rs:151`, `track_factory` at `api.rs:92`. Flush must run
  before `shutdown_background()`, or the publisher must survive it.

#### 12. microsoft/wassette (commit a4a5e55)

`wassette` runs WebAssembly components as MCP tools
(`microsoft/wassette/crates/wassette-mcp-server/src/server.rs:44-52`).

```rust
let http_config = http_config
    .with_legacy_session_mode(legacy_sessions)
    .with_json_response(json_response);
let service = StreamableHttpService::new(
    move || Ok(server.clone()),
    LocalSessionManager::default().into(),
    http_config,
);
```
(`microsoft/wassette/crates/wassette-mcp-server/src/main.rs:317-325`, indentation reduced; stdio is
`serve_server(server, transport)` then `ctrl_c` and `running_service.cancel()`, `:168-172`)

- Manual `ServerHandler` with boxed futures under `#[allow(refining_impl_trait_reachable)]`
  (`server.rs:170-197`). Overrides `get_info`, `call_tool`, `list_tools`, `list_prompts`,
  `list_resources`, `list_resource_templates`, `accepted_subscription_filter`, `listen`.
  No `get_tool`.
- Stores a long-lived `Peer` and calls `peer.notify_tool_list_changed()` when components load
  or unload (`server.rs:48, 109-120, 140-152`;
  `microsoft/wassette/crates/mcp-server/src/components.rs:363`).
- Reads `http::request::Parts` and the `mcp-session-id` header to tell legacy-session peers
  from stateless peers (`server.rs:36, 89-100`).
- Sets SEP-2549 cache hints for 2026-07-28 clients (`server.rs:38-42, 253-256`).
- **For AgentCat:** `serve_server(agentcat.track(server), transport)` and
  `move || Ok(agentcat.track(server.clone()))`. `Tracked` must forward `listen` and
  `accepted_subscription_filter`, keep cache hints, and handle a changing tool list.

#### 13. apollographql/apollo-mcp-server (commit a7067e7)

Exposes GraphQL operations as MCP tools
(`apollographql/apollo-mcp-server/crates/apollo-mcp-server/src/server/states/running.rs:693-699`).

```rust
) -> StreamableHttpService<super::running::McpService, LocalSessionManager> {
    let config = host_validation.apply_to(
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(stateful_mode)
            .with_cancellation_token(running.cancellation_token.child_token()),
    );
    StreamableHttpService::new(
        move || Ok(running.for_service()),
        LocalSessionManager::default().into(),
        config,
    )
```
(`crates/apollo-mcp-server/src/server/states/starting.rs:277-287`; stdio at `:241-251`)

- Manual `ServerHandler` with 15 overrides: `initialize` (calls `negotiate_initialize`),
  `on_initialized`, `supported_protocol_versions`, `accepted_subscription_filter`, `listen`,
  `call_tool`, `list_tools`, `get_tool`, `list_resources`, `list_resource_templates`,
  `read_resource`, `list_prompts`, `get_prompt`, `set_level`, `get_info`
  (`running.rs:702-1070`).
- `get_tool` drives SEP-2243 header validation. Its comment says rmcp caches the answer per
  name for the life of the service (`running.rs:918-931`). rmcp 3.6.0 removes that cache
  (§5.5).
- Reads `Parts` for header forwarding (`running.rs:367, 1115-1121`). The auth middleware puts
  a `ValidToken` into the extensions (`src/auth.rs:1028-1030`). The server forwards
  `mcp-session-id` to the GraphQL backend (`src/headers.rs:35-38`).
- Hot reload changes tools at runtime and notifies peers (`running.rs:63-64, 199, 798`;
  `src/server/states/tool_list_changes.rs:66-71`).
- It already records OpenTelemetry spans per tool call, including arguments
  (`running.rs:855-886`).
- The concrete type `StreamableHttpService<McpService, LocalSessionManager>` appears in
  function signatures (`starting.rs:277, 357`).
- **For AgentCat:** `move || Ok(agentcat.track(running.for_service()))`, and the customer
  changes the return type to `StreamableHttpService<Tracked<McpService>, _>`. AgentCat must
  strip injected arguments before Apollo's span records them.

#### 14. natepiano/bevy_brp (commit 9b25623)

`bevy_brp_mcp` drives running Bevy apps over the Bevy Remote Protocol
(`natepiano/bevy_brp/mcp/Cargo.toml:4, 8`).

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize file-based tracing with dynamic level management
    // Uses lazy file creation - file only created on first log write
    TracingLevel::init_file_tracing();

    let mcp_service = McpService::new();

    let server = mcp_service.serve(transport::stdio()).await?;
    server.waiting().await?;
```
(`natepiano/bevy_brp/mcp/src/main.rs:25-34`)

- Manual `ServerHandler` (`get_info`, `list_tools`, `call_tool`) over its own proc-macro
  tool registry (`mcp/src/mcp_service.rs:74-99`).
- `list_tools` sets `ttl_ms` and `CacheScope::Private` because 2026-07-28 clients reject the
  response without them (`mcp_service.rs:63-71`).
- `get_info` uses `ServerConfig::new(...)`, so the identity is the placeholder
  (`mcp_service.rs:75-77`).
- The workspace enables `serde_json/preserve_order` (`natepiano/bevy_brp/Cargo.toml:48`).
- Logs go to a file in the temp directory (`mcp/src/log_tools/tracing.rs:80-96`).
- **For AgentCat:** `agentcat.track(McpService::new()).serve(transport::stdio())`.

#### 15. Govcraft/rust-docs-mcp-server (commit 7ee11e7)

One semantic-search tool over a crate's docs (`Govcraft/rust-docs-mcp-server/src/server.rs:135-142`).

```rust
let server_handle = service.serve(stdio()).await.map_err(|e| {
    eprintln!("Failed to start server: {:?}", e);
    ServerError::McpRuntime(e.to_string()) // Use the new McpRuntime variant
})?;
```
(`Govcraft/rust-docs-mcp-server/src/main.rs:327-330`)

- Still on rmcp 0.1.5 (`Cargo.toml:7`), last push 2025-11-24. It uses the 0.1 macros
  `#[tool(tool_box)]` and `#[tool(aggr)]` (`src/server.rs:130-140, 288-289`).
- The 0.1.5 trait requires `Clone` and has `get_peer`/`set_peer`
  (`rmcp-0.1.5/src/handler/server.rs:104, 219-225` in the crates.io tarball).
- The release profile sets `panic = "abort"` (`Cargo.toml:43`).
- **For AgentCat:** blocked on rmcp 0.x.

### 4.3 Patterns across the 15 servers

| Question | Count | Projects |
| --- | --- | --- |
| rmcp line in lockfile | 3.x: 9, 2.x: 2, 1.x: 3, 0.x: 1 | 2.x devenv, readyset; 1.x chroma, surrealdb, vibe-kanban; 0.x rust-docs |
| Serves stdio | 11 | all except codex, posthog, chroma, windmill |
| Serves streamable HTTP | 11 | all except goose, vibe-kanban, bevy_brp, rust-docs |
| HTTP session mode | session 7, stateless 2, user choice 2 | stateless: chroma, windmill; choice: wassette, apollo |
| HTTP framework | axum 10, hyper-util 1 | nushell uses `TowerToHyperService` |
| Factory clones one prebuilt handler | 5 | codex, wassette, windmill, devenv, readyset |
| Factory builds a new handler | 6 | posthog, chroma, nushell, iggy, apollo, surrealdb |
| Handler: macros, `Self::tool_router()` | 4 | nushell, iggy, posthog, surrealdb |
| Handler: macros, stored router | 3 | goose, chroma, vibe-kanban |
| Handler: `#[tool_router(server_handler)]` | 1 | readyset |
| Handler: manual `ServerHandler` | 6 | codex, wassette, apollo, windmill, devenv, bevy_brp |
| Handler: rmcp 0.1 `#[tool(tool_box)]` | 1 | rust-docs |
| Handler: direct `Service<RoleServer>` | 0 | |
| Overrides `get_tool` by hand | 1 | apollo |
| Overrides `initialize` | 3 | apollo, surrealdb, windmill |
| Overrides `listen` + `accepted_subscription_filter` | 3 | wassette, apollo, windmill |
| Serves real prompts or resources | 3 | goose (resources), apollo, surrealdb |
| Reads `http::request::Parts` | 5 | wassette, apollo, chroma, windmill, surrealdb (and Elastic) |
| Reads typed auth values from `parts.extensions` | 4 | apollo, chroma, windmill, surrealdb (posthog inserts one) |
| Reads or forwards `mcp-session-id` | 2 | wassette, apollo |
| Reads custom `_meta` keys | 2 | codex, goose |
| Tool list changes at runtime | 3 (+2 at startup or per call) | wassette, apollo, windmill (+ vibe-kanban, codex) |
| `#[tokio::main]` | 6 | posthog, wassette, apollo, bevy_brp, rust-docs, readyset-mcp |
| Manual runtime builder or `block_on` | 9 | codex, goose, nushell, iggy, chroma, surrealdb, windmill, vibe-kanban, devenv |
| `current_thread` runtime | 0 | |
| Already has own per-tool telemetry | 2 | apollo (OTel spans), surrealdb (audit + metrics) |
| Server identity is the `"rmcp"` placeholder | 3 | iggy, bevy_brp, readyset (on 3.x) |

GitHub-wide pattern counts support the survey (approximate `total_count` from
`GET /search/code`, Rust files, 2026-10-19):

| Search | Files |
| --- | ---: |
| `"StreamableHttpService::new"` | 3,864 |
| `"serve(stdio())"` | 2,464 |
| `"#[tool_handler]"` | 2,820 |
| `"#[tool_router(server_handler)]"` | 277 |
| `"notify_tool_list_changed"` | 229 |
| `"Extension<Parts>"` | 164 |
| `"Service<RoleServer> for"` | 86 |

GitHub search ignores punctuation, so these counts are rough. They show that direct
`Service<RoleServer>` impls are rare, which supports report 03's decision to make
`TrackedService<S>` secondary.

### 4.4 What the survey means for `Tracked<H>`

1. **Every surveyed 3.x server can adopt AgentCat with one changed expression.** The handler
   always reaches rmcp as one value (`serve`, `serve_server`, a duplex `serve`) or one factory
   closure.
2. **`Tracked<H>` must implement `Clone` when `H: Clone`.** Five factories clone a prebuilt
   handler. Customers will write `let h = agentcat.track(handler); move || Ok(h.clone())`.
3. **Construction must be cheap and side-effect free.** Stateless servers (chroma, windmill)
   build one handler per request.
4. **Forward every method exactly.** Real servers override `initialize`,
   `supported_protocol_versions`, `listen`, `accepted_subscription_filter`, `get_tool`,
   `set_level`, `complete` and `on_initialized`.
5. **Do not cache injected tool lists across `list_changed`.** Three servers change tools at
   runtime, and one builds the list per caller.
6. **Handle both error forms.** Iggy reports user errors as `Err(ErrorData)`, not as
   `CallToolResult::error`.
7. **Keep `_meta`, cache hints and capability edits.** Codex reads custom `_meta`. Wassette and
   bevy_brp set `ttl_ms`/`cache_scope`. Windmill removes `list_changed`.
8. **Expect strict middleware in front of the handler.** Readyset blocks unknown tool names and
   unknown methods.

---

## 5. rmcp today

### 5.1 Version, date, MSRV

| Fact | Value | Source |
| --- | --- | --- |
| Latest release | `rmcp` and `rmcp-macros` **3.5.1**, 2026-10-05 | `https://crates.io/api/v1/crates/rmcp`; GitHub release `rmcp-v3.5.1` |
| Next release | **3.6.0**, open release PR #1327, changelog dated 2026-10-19, not on crates.io yet | `https://github.com/modelcontextprotocol/rust-sdk/pull/1327` |
| MSRV | **1.88**, edition 2024 | `modelcontextprotocol/rust-sdk/Cargo.toml:12-13`; crates.io `rust_version` is `1.88` for 3.x; versions before 3.0.0-beta.2 declare none |
| Default branch | `main`, HEAD `ad2a66a` (2026-10-19) | |
| SemVer policy | SemVer 2.0; `local` exempt; MSRV bumps only in minor or major; release-plz plus `cargo-semver-checks` | `modelcontextprotocol/rust-sdk/VERSIONING.md:3-59` |

### 5.2 `ServerHandler` method list (3.5.1 and `main`)

The trait body is a `macro_rules!` block shared by the `local` and non-`local` variants
(`rmcp/src/handler/server.rs:308-638`). The trait is
`pub trait ServerHandler: Sized + Send + Sync + 'static` without `local`
(`server.rs:640-644`) and `Sized + 'static` with it (`:646-650`). Every method has a default.

Async methods return `impl Future<Output = R> + MaybeSendFuture + '_`; the table shows only
`R`. `ctx` is `RequestContext<RoleServer>`; `nctx` is `NotificationContext<RoleServer>`. Every
method takes `&self`.

| # | Method | Parameters | Returns (`R`) | Default | Line |
| ---: | --- | --- | --- | --- | ---: |
| 1 | `ping` | `ctx` | `Result<(), McpError>` | `Ok(())` | 310 |
| 2 | `initialize` | `InitializeRequestParams, ctx` | `Result<InitializeResult, McpError>` | store peer info, then `negotiate_initialize` | 317 |
| 3 | `negotiate_initialize` (sync, 3.3+) | `&InitializeRequestParams` | `Result<InitializeResult, McpError>` | negotiate version against `supported_protocol_versions` | 365 |
| 4 | `supported_protocol_versions` (sync) | — | `Cow<'static, [ProtocolVersion]>` | `KNOWN_VERSIONS` | 388 |
| 5 | `discover` | `ctx` | `Result<DiscoverResult, McpError>` | from `get_info` | 392 |
| 6 | `complete` | `CompleteRequestParams, ctx` | `Result<CompleteResult, McpError>` | empty | 401 |
| 7 | `set_level` | `SetLevelRequestParams, ctx` | `Result<(), McpError>` | method not found | 408 |
| 8 | `get_prompt` | `GetPromptRequestParams, ctx` | `Result<GetPromptResponse, McpError>` | method not found | 415 |
| 9 | `list_prompts` | `Option<PaginatedRequestParams>, ctx` | `Result<ListPromptsResult, McpError>` | empty | 422 |
| 10 | `list_resources` | `Option<PaginatedRequestParams>, ctx` | `Result<ListResourcesResult, McpError>` | empty | 429 |
| 11 | `list_resource_templates` | `Option<PaginatedRequestParams>, ctx` | `Result<ListResourceTemplatesResult, McpError>` | empty | 436 |
| 12 | `read_resource` | `ReadResourceRequestParams, ctx` | `Result<ReadResourceResponse, McpError>` | method not found | 445 |
| 13 | `accepted_subscription_filter` (sync) | `&SubscriptionFilter` | `Option<SubscriptionFilter>` | `None` | 460 |
| 14 | `listen` | `SubscriptionContext` | `Result<(), McpError>` | wait for cancel | 472 |
| 15 | `subscribe` (deprecated) | `SubscribeRequestParams, ctx` | `Result<(), McpError>` | method not found | 484 |
| 16 | `unsubscribe` (deprecated) | `UnsubscribeRequestParams, ctx` | `Result<(), McpError>` | method not found | 494 |
| 17 | `call_tool` | `CallToolRequestParams, ctx` | `Result<CallToolResponse, McpError>` | method not found | 531 |
| 18 | `list_tools` | `Option<PaginatedRequestParams>, ctx` | `Result<ListToolsResult, McpError>` | empty | 538 |
| 19 | `get_tool` (sync) | `&str` | `Option<Tool>` | `None` | 549 |
| 20 | `on_custom_request` | `CustomRequest, ctx` | `Result<CustomResult, McpError>` | method not found | 552 |
| 21 | `on_cancelled` | `CancelledNotificationParam, nctx` | `()` | no-op | 566 |
| 22 | `on_progress` | `ProgressNotificationParam, nctx` | `()` | no-op | 573 |
| 23 | `on_initialized` | `nctx` | `()` | logs "client initialized" | 580 |
| 24 | `on_roots_list_changed` | `nctx` | `()` | no-op | 587 |
| 25 | `on_custom_notification` | `CustomNotification, nctx` | `()` | no-op | 593 |
| 26 | `get_info` (sync) | — | `ServerConfig` (= `InitializeResult`) | `ServerConfig::default()` | 602 |
| 27 | `get_task` | `GetTaskParams, ctx` | `Result<GetTaskResult, McpError>` | method not found | 607 |
| 28 | `update_task` | `UpdateTaskParams, ctx` | `Result<(), McpError>` | method not found | 618 |
| 29 | `cancel_task` | `CancelTaskParams, ctx` | `Result<(), McpError>` | method not found | 629 |

rmcp delegates all 29 methods for `Box<T>` and `Arc<T>` with the private macro
`impl_server_handler_for_wrapper!` (`server.rs:652-873`). The macro is not exported.

**Compatibility detail.** The `ServerConfig` alias exists only from 3.4.0. The same release
deprecated the `ServerInfo` alias (#1156, `rmcp/CHANGELOG.md:64`; checked with
`git show rmcp-v3.3.0:crates/rmcp/src/model.rs`). AgentCat code must name the type
`InitializeResult`, which exists in every 3.x release, to compile from 3.0 to 3.6 without
warnings.

### 5.3 Default dispatch of `call_tool` and `list_tools`

1. Every `ServerHandler` is a `Service<RoleServer>` through
   `impl<H: ServerHandler> Service<RoleServer> for H` (`rmcp/src/handler/server.rs:50`).
2. `handle_request` first validates the per-request protocol version and the 2026-07-28
   `_meta` (`server.rs:56-100`).
3. It then matches on `ClientRequest`. `CallToolRequest` calls
   `self.call_tool(request.params, context)` and rejects a `CallToolResponse::Task` if the
   client did not declare tasks (`server.rs:203-218`). `ListToolsRequest` calls
   `self.list_tools(request.params, context)` (`:219-222`).
4. After dispatch it fills or strips cache hints and `resultType` by protocol version, and maps
   `RESOURCE_NOT_FOUND` to `INVALID_PARAMS` for 2026-07-28 peers (`server.rs:246-271`).
5. Without overrides, `call_tool` returns method-not-found (`server.rs:536`) and `list_tools`
   returns an empty list (`:543`).

With `#[tool_handler]`, the generated `call_tool` builds `ToolCallContext::new(self, request,
context)` and calls `<router>.call(tcc)` (`rmcp-macros/src/tool_handler.rs:44-56`).
`ToolRouter::call` returns `invalid_params("tool not found")` for a disabled or unknown name,
calls the route's handler, and turns argument errors into tool results with
`into_tool_argument_error` (`rmcp/src/handler/server/router/tool.rs:560-579`). The generated
`list_tools` returns `<router>.list_all()` sorted by name, with no pagination, and with
`ttl_ms = 0` and `CacheScope::Public` for 2026-07-28 peers (`tool_handler.rs:64-86`;
`router/tool.rs:581-590`).

### 5.4 What `#[tool_handler]` generates

The macro adds each of these methods only if the `impl` block does not already define it
(`rmcp-macros/src/tool_handler.rs:33-110`):

| Method | Generated body |
| --- | --- |
| `call_tool` | `ToolCallContext::new(self, request, context)`, then `#router.call(tcc).await` (`:44-56`) |
| `list_tools` | `#router.list_all()`, `next_cursor: None`, cache hints for 2026-07-28 (`:64-86`) |
| `get_tool` | `#router.get(name).cloned()` (`:88-95`) |
| `get_info` | `ServerConfig::new(capabilities)` with tools (and prompts if `#[prompt_handler]` is present), `with_server_info(...)` from the `name`/`version` attributes or `Implementation::from_build_env()` (`:97-107, 124-169`) |

The default router expression is `Self::tool_router()` (`tool_handler.rs:18-31`). It rebuilds
the router on each call. `#[tool_handler(router = self.tool_router)]` uses a stored router.

**History of the default.** In rmcp-macros 0.x and 1.0-1.3 the default was the stored field
`self.tool_router` (tag `rmcp-v1.3.0`, `crates/rmcp-macros/src/tool_handler.rs:8-22`).
rmcp-macros 1.4.0 (2026-04-09) changed it to `Self::tool_router()` in "auto-generate get_info
and default router" (#785, `rmcp-macros/CHANGELOG.md:161-165`). Three surveyed projects show
the effect: PostHog and Nushell keep dead `tool_router` fields with stale comments, SurrealDB
loses router edits, and vibe-kanban stops compiling after a `cargo update`.

### 5.5 `StreamableHttpService::new`, factory and config

```rust
// rmcp/src/transport/streamable_http_server/tower.rs:1408-1430
impl<S, M> StreamableHttpService<S, M>
where
    S: crate::ServerHandler + Send + 'static,
    M: SessionManager,
{
    pub fn new(
        service_factory: impl Fn() -> Result<S, std::io::Error> + Send + Sync + 'static,
        session_manager: Arc<M>,
        config: StreamableHttpServerConfig,
    ) -> Self
```

- The factory is stored as `Arc<dyn Fn() -> Result<S, io::Error> + Send + Sync>`
  (`tower.rs:1335`). The handler type needs `ServerHandler + Send + 'static`. It does **not**
  need `Clone` (the customer's factory decides).
- `StreamableHttpServerConfig` (`tower.rs:78-188`, `#[non_exhaustive]`, `Clone`) has
  `sse_keep_alive`, `sse_retry`, `legacy_session_mode`, `json_response`,
  `cancellation_token`, `allowed_hosts`, `allowed_origins`, `session_store`,
  `max_request_body_bytes`, `stateless_protocol_metadata_required`. Defaults are at
  `:195-210`; builders `with_*` are at `:214-285`.
- **Stateful vs stateless.** `legacy_session_mode = true` (default) creates a session on
  `initialize` and returns `Mcp-Session-Id`. It applies only to protocol versions before
  2026-07-28. Requests at 2026-07-28 are always stateless (`tower.rs:84-90`). Stateless mode
  uses `NeverSessionManager` and `legacy_session_mode = false`.
- **Factory calls.** Once per new session (`tower.rs:2161-2163`), once per session restore
  (`:1740`), once per stateless or per-request-negotiated request (`:2124, 2251`), and once
  per SEP-2243 tool-schema lookup (`tool_schema`, `:1597-1602`).
- **Change in 3.6.0.** Up to 3.5.1, the service cached each tool's input schema by name for
  its whole life (`tool_schemas`, 3.5.1 `tower.rs:1344, 1602-1617`). PR #1349 (merged
  2026-10-19, in the pending 3.6.0) removes the cache. The service now builds a handler and
  calls `get_tool` for every request that needs header validation.
- **Name change in 3.0.** `stateful_mode` / `with_stateful_mode` became `legacy_session_mode` /
  `with_legacy_session_mode` (#999, #1015; `rmcp/CHANGELOG.md:257, 279`). Chroma, readyset and
  SurrealDB still use the old name.

### 5.6 Reading HTTP request parts and the session ID

- The HTTP service consumes the body and inserts the remaining `http::request::Parts` into the
  request's `Extensions` (`tower.rs:1538, 2069, 2072, 2154, 2275`). Handlers read it as
  `context.extensions.get::<http::request::Parts>()`, or in a `#[tool]` with the
  `Extension<Parts>` extractor (`rmcp/src/handler/server/common.rs:180-190`).
- State that axum middleware adds with `req.extensions_mut().insert(..)` is in
  `parts.extensions` (doc example at `tower.rs:1309-1330`).
- There is no typed session-ID accessor. The documented way is
  `parts.headers.get("mcp-session-id")` (`tower.rs:1289-1306`;
  `HEADER_SESSION_ID = "Mcp-Session-Id"`, `rmcp/src/transport/common/http_header.rs:1`).
  Stateless requests and all 2026-07-28 requests have no session ID.
- `RequestContext` (`rmcp/src/service.rs:1194-1202`) has public `ct`, `id`, `meta`,
  `extensions`, `peer`. Accessors: `protocol_version()` (`:1220`), `client_info()` (`:1229`),
  `Peer::peer_info()` (`:1012`). `SubscriptionContext::sink()` sends notifications from
  `listen` (`rmcp/src/service/server.rs:374`).

### 5.7 Release cadence and breaking changes

Dates are crates.io publish dates (`/api/v1/crates/rmcp` `versions[].created_at`). The
changelog dates differ by up to two days (for example 2.0.0 is 2026-06-27 in
`rmcp/CHANGELOG.md:313`).

| Line | First | Last | Releases | `ServerHandler` methods | Changes that matter to a wrapper |
| --- | --- | --- | ---: | ---: | --- |
| 0.x | 0.1.0, 2025-03-16 | 0.17.0, 2026-02-27 | 36 | 18 (0.8.5) | 17 minor lines (each one breaking) in 11 months. 0.1.x: trait requires `Clone`, has `get_peer`/`set_peer`, macros `#[tool(tool_box)]`. `transport-sse-server` gone from 0.11.0 |
| 1.x | 1.0.0, 2026-03-03 | 1.8.0, 2026-06-23 | 11 | 26 (1.8.0) | `#[non_exhaustive]` on many types (#715). Old tasks API (`enqueue_task`, `list_tasks`, `get_task_info`, `get_task_result`, `cancel_task`). Macro default router changed in 1.4.0 |
| 2.x | 2.0.0, 2026-06-29 | 2.2.0, 2026-07-08 | 3 | 27 (2.2.0) | Models aligned with the 2025-11-25 spec (#927). Adds `on_task_status`. `call_tool` still returns `CallToolResult` |
| 3.x | 3.0.0, 2026-07-28 | 3.5.1, 2026-10-05 (3.6.0 pending) | 18 incl. 5 betas | 28 (3.0), 29 (3.3+) | `call_tool` returns `CallToolResponse` (MRTR); `get_prompt`/`read_resource` return `*Response`; adds `discover`, `listen`, `accepted_subscription_filter`, `supported_protocol_versions`; new tasks API (`get_task`, `update_task`, `cancel_task`); `stateful_mode` renamed. 3.3.0 adds `negotiate_initialize` (#1247). 3.4.0 adds `ServerConfig`, deprecates `ServerInfo` (#1156) |

Method lists come from `crates/rmcp/src/handler/server.rs` in the crates.io tarballs for
0.8.5, 1.8.0 and 2.2.0, and from tags `rmcp-v3.0.0` and `rmcp-v3.5.1`. The 3.0.0-beta.1
changelog marks 11 items as breaking (`rmcp/CHANGELOG.md:229-283`).

Cadence: 68 versions in 19 months, about 3.6 per month. 3.x shipped a minor every 1 to 4
weeks: 3.1 (07-31), 3.2 (08-31), 3.3 (09-10), 3.4 (09-15), 3.5 (09-28). Majors came 4 months
(1.0 → 2.0) and 1 month (2.0 → 3.0) apart. The 2.0 → 3.0 major followed the 2026-07-28 spec
release.

### 5.8 Is a next major in progress?

No active work was found.

- No `v4`/`4.0` branch, milestone or issue. The only milestone is "2026-07-28 spec"
  (`gh api repos/modelcontextprotocol/rust-sdk/milestones`).
- The open release PR #1327 is a minor (3.6.0), with no breaking entries.
- 40 open PRs are fixes or additive features. Two touch areas AgentCat uses: #1369 "support
  local streamable HTTP handler futures" and #1375 "answer a known method with invalid params
  with -32602".
- The one breaking proposal is issue #1067 "Rationalize feature flags for v3.0" (opened
  2026-07-28). Branch `feat/rationalize-feature-flags` has one `refactor!` commit from
  2026-07-28. The issue is marked stale after 60 days without activity. If it lands, it needs
  a 4.0 and renames features such as `server` and `transport-*`.

---

## 6. Recommendations

### 6.1 rmcp version range

```toml
[dependencies]
rmcp = { version = "3", default-features = false, features = ["server"] }
http = "1"    # same major as rmcp, so the TypeId of http::request::Parts matches
```

Reasons:

1. **3.x is where usage is today and where it grows.** It has 73 % of downloads this week and
   65 % of recently active dependents, and every 3.x minor is in use (§2).
2. **The floor must be 3.0.0.** 293 published dependents declare `^3.1`, 54 declare `^3.0`,
   and 36 pin exactly below 3.2 (Windmill pins `=3.1.0`). A higher floor makes their builds
   fail to resolve. All APIs the wrapper needs exist in 3.0.0: the 28 methods,
   `CallToolResponse`, `RequestContext::client_info()` and `protocol_version()` (checked at
   tags `rmcp-v3.0.0` and `rmcp-v3.5.1`).
3. **Write code that compiles on every 3.x.** Use `InitializeResult`, not `ServerConfig` or
   `ServerInfo` (§5.2). Do not override `negotiate_initialize`, which exists only from 3.3.
   Forwarding `initialize` is enough (report 03 §1.2).
4. **`server` is the only feature needed.** 95 % of dependents already enable it. The HTTP
   `Parts` type comes from the `http` crate, not from an rmcp feature.
5. **Do not support 0.x, 1.x or 2.x in v1.** The trait differs in each major: 18, 26 and 27
   methods, `CallToolResult` instead of `CallToolResponse`, and different tasks APIs. Each
   major needs its own adapter. If a paying customer needs one, the best second target is a combined
   1.7-2.2 adapter: 1.8 and 2.2 together have 15 % of this week's downloads, and Chroma,
   SurrealDB, readyset and devenv run them.
6. **CI matrix:** rmcp 3.0.0 (`cargo update -p rmcp --precise 3.0.0`), the newest 3.x, and
   rmcp `main` weekly, plus the trait tripwire from report 03 §3.8.

### 6.2 Integration shapes the public API must support

| Shape | Seen in | API requirement |
| --- | --- | --- |
| Wrap a value for `serve(stdio())`, `serve((stdin, stdout))`, `serve_server(h, t)`, duplex | goose, nushell, iggy, wassette, bevy_brp, surrealdb, devenv | `agentcat.track(h) -> Tracked<H>`, `Tracked<H>: ServerHandler` |
| Factory that clones a prebuilt handler | codex, wassette, windmill, devenv, readyset | `impl<H: ServerHandler + Clone> Clone for Tracked<H>`; clones share config and publisher, start with an empty per-instance registry |
| Factory that builds a handler per session or request | posthog, chroma, nushell, iggy, apollo, surrealdb | `agentcat.track_factory(f)`; `Tracked::new` does no I/O and no allocation beyond an `Arc` clone |
| Generic serve helpers (`fn serve<S: ServerHandler>`) | goose, windmill (`Runner<B>`) | no extra bounds on `H` beyond `ServerHandler` (plus `Clone` only for `Clone`) |
| Handlers registered as `fn` pointers | goose | a process-global handle: `agentcat::init(builder)` and `agentcat::global()` |
| Concrete service types in signatures | apollo | document `StreamableHttpService<Tracked<H>, M>`; export `Tracked` at the crate root |
| Identity from typed values in `parts.extensions` | apollo, chroma, windmill, surrealdb, posthog | the hook view exposes `http_parts() -> Option<&http::request::Parts>`, not only `header()` |
| Runtime tool changes, `listen`, `notify_tool_list_changed` | wassette, apollo, windmill | forward all 29 methods; registry per instance; rebuild via `inner.get_tool(name)`, then `inner.list_tools` |
| Strict auth middleware that filters tools | readyset | per-server `disable_report_missing(true)` (no `get_more_tools`) and an option to skip injection |
| Sync `main` with `block_on`, `shutdown_background()` | nushell, iggy, vibe-kanban, chroma | `shutdown_blocking(timeout)` callable outside a runtime; publisher on its own thread |
| stdio servers that must keep stdout clean | all 11 stdio servers | log only through `tracing` (target `agentcat`); never install a subscriber; never print |
| Placeholder server identity | iggy, bevy_brp, readyset | builder option `server_info(name, version)` that overrides `get_info().server_info` in events |
| rmcp from git | codex, 10 GitHub repositories | document `[patch.crates-io] rmcp = { git = …, rev = … }` so only one rmcp is compiled |

### 6.3 Code sketches

The sketches use the API proposed in report 03 §5.6, plus `init`/`global`,
`shutdown_blocking` and `http_parts` from §6.2.

**(a) Macro-based stdio server.** One line changes; the `#[tool_router]`/`#[tool_handler]`
code stays as it is.

```rust
use std::time::Duration;
use agentcat::AgentCat;
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stdio servers must log to stderr or a file, never stdout.
    tracing_subscriber::fmt().with_writer(std::io::stderr).with_ansi(false).init();

    let agentcat = AgentCat::builder("proj_0000000").build();

    // Counter has #[tool_router] on its impl and #[tool_handler] on `impl ServerHandler`.
    let running = agentcat.track(Counter::new()).serve(stdio()).await?;
    running.waiting().await?;

    agentcat.shutdown(Duration::from_secs(5)).await;
    Ok(())
}
```

**(b) Streamable HTTP with axum and auth middleware.**

```rust
use std::{sync::Arc, time::Duration};
use agentcat::{AgentCat, UserIdentity};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;

let agentcat = AgentCat::builder("proj_0000000")
    .identify(|call| {
        // The app's own middleware inserted `AuthedUser` into the request extensions.
        let user = call.http_parts()?.extensions.get::<AuthedUser>()?;
        Some(UserIdentity::new(&user.id).name(&user.email))
    })
    .build();

let state = Arc::new(AppState::load().await?);
let ct = CancellationToken::new();
let mcp: StreamableHttpService<agentcat::Tracked<MyServer>, LocalSessionManager> =
    StreamableHttpService::new(
        agentcat.track_factory({
            let state = state.clone();
            move || Ok(MyServer::new(state.clone())) // runs per session, or per request when stateless
        }),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().with_cancellation_token(ct.child_token()),
    );
// A prebuilt Clone handler works too:
//   let tracked = agentcat.track(handler);  move || Ok(tracked.clone())

let app = axum::Router::new()
    .nest_service("/mcp", mcp)
    .layer(axum::middleware::from_fn_with_state(state.clone(), require_auth));
let listener = tokio::net::TcpListener::bind("127.0.0.1:8000").await?;
axum::serve(listener, app)
    .with_graceful_shutdown(async move {
        tokio::signal::ctrl_c().await.ok();
        ct.cancel();
    })
    .await?;
agentcat.shutdown(Duration::from_secs(5)).await;
```

**(c) Manual `ServerHandler` impl.** The customer's impl does not change. Adding `get_tool` is
optional; it lets AgentCat rebuild one tool without calling `list_tools`.

```rust
use std::sync::Arc;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{CallToolRequestParams, CallToolResponse, InitializeResult, ListToolsResult,
            PaginatedRequestParams, Tool},
    service::RequestContext,
};

#[derive(Clone)]
struct McpService { state: Arc<State> }

impl ServerHandler for McpService {
    fn get_info(&self) -> InitializeResult { self.state.info() }

    async fn list_tools(
        &self, _req: Option<PaginatedRequestParams>, ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> { self.state.list(&ctx).await }

    async fn call_tool(
        &self, req: CallToolRequestParams, ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> { self.state.call(req, ctx).await }

    fn get_tool(&self, name: &str) -> Option<Tool> { self.state.tool(name) } // optional
}

fn main() -> anyhow::Result<()> {
    // Process-global handle, for code that cannot pass a value around (fn-pointer registries).
    agentcat::init(agentcat::AgentCat::builder("proj_0000000").server_info("my-server", "1.2.0"));
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async {
        let svc = agentcat::global().track(McpService::new());
        rmcp::serve_server(svc, rmcp::transport::stdio()).await?.waiting().await?;
        anyhow::Ok(())
    })?;
    // Sync main: flush without a runtime.
    agentcat::global().shutdown_blocking(std::time::Duration::from_secs(5));
    Ok(())
}
```

### 6.4 Risks

1. **Version split.** 27 % of this week's rmcp downloads and 6 of the 15 surveyed servers are
   on 0.x, 1.x or 2.x. A 3.x-only SDK cannot serve Chroma, SurrealDB, vibe-kanban, devenv,
   readyset or rust-docs-mcp-server until they upgrade.
2. **Trait drift.** The method count went 18 → 26 → 27 → 28 → 29 across releases. 3.3.0 added
   a method in a minor. The CI tripwire from report 03 §3.8 is required.
3. **Behavior changes inside a major.** The macro default router changed in 1.4.0. The
   `ServerConfig` alias appeared in 3.4.0. 3.6.0 changes how often the HTTP factory runs.
   Test the floor and the newest release on every AgentCat release.
4. **Dependency resolution.** 120 dependents pin rmcp exactly, and some projects take rmcp
   from git. An AgentCat floor above a customer's pin fails to resolve. A git-pinned rmcp
   without `[patch.crates-io]` produces two rmcp crates and a type error at `track()`.
5. **Factory cost.** Stateless servers and rmcp 3.6.0 build a handler per request. Any work in
   `Tracked::new` multiplies by request rate.
6. **Stale registries.** Three servers change tools at runtime, and one builds the list per
   caller. If AgentCat caches injected lists across `list_changed`, it advertises wrong
   schemas and breaks SEP-2243 header validation through `get_tool`.
7. **Middleware conflicts.** Auth layers that allowlist tool names or methods (readyset) reject
   `get_more_tools` with HTTP 403.
8. **Overlap with in-house telemetry.** Apollo and SurrealDB already record per-tool spans,
   audit logs and metrics. Apollo's spans record arguments, so injected arguments must be
   stripped before the inner call.
9. **Identity is app-specific.** Most HTTP servers keep the user in typed `parts.extensions`
   values. The SDK cannot find it alone; the `identify` hook is the only way. stdio has no
   `Parts` at all.
10. **Process exit.** Sync mains, `shutdown_background()` (iggy), aborted tasks on `Drop`
    (codex) and `panic = "abort"` (rust-docs) all drop queued events unless the publisher has
    its own thread and the docs show where to flush.
11. **Global build settings.** `serde_json/preserve_order` is on in some customer builds
    (codex TUI, bevy_brp) and off in others, so injected schema property order differs by
    customer (report 03 §6 item 8).
12. **Upstream feature rename.** If issue #1067 lands, rmcp 4.0 renames features. AgentCat's
    `features = ["server"]` then needs a new adapter or a renamed dependency.
13. **Download data is noisy.** CI and transitive pulls inflate downloads (for example the
    `smg-mcp` client pulls rmcp 1.x). Reverse dependencies and the survey are the better
    signals.
