# Dashboard demo, 2026-10-28

The demo (R12, C58) ran one fixed scenario through the Rust SDK and through the TypeScript SDK, published both to
the AgentCat project "Testing" (`proj_2yAR5QHJ3CE3icTpJlKJqu1ZiXG`) on the production API, and read the events
back with the AgentCat MCP tools (`search_sessions`, `get_session`, `get_events`). The two servers published the
same events, with one known difference (RD-39) and the payload shapes that the deviation register lists.

## Run

- Commit: `0f61f2e` (branch `rewrite`).
- Command: `AGENTCAT_DEMO_PROD=yes node harness/demo/run.mjs --prod --project proj_2yAR5QHJ3CE3icTpJlKJqu1ZiXG`,
  exit 0, at 2026-10-28T15:58:13Z.
- Servers: the `hooks` example (Rust, `agentcat-sdk/examples/hooks.rs`) and `harness/demo/ts-server.mjs` (the
  same tools and options on the TS SDK at `9b56e9e`, on `@modelcontextprotocol/server` 2.0.0), each over stdio
  with the TS client `agentcat-demo` 1.0.0.
- Scenario (`harness/demo/scenario.json`): session 1 has `tools/list`, `add`, `echo`, `echo` with `secret-token`
  in its text, and `fail`; session 2 has one `echo`. The `echo` tool publishes the custom event `echo-called` on
  every call.

## Sessions

| Server | Session | SDK language | Actions | Errors |
|---|---|---|---|---|
| Rust | [`ses_3LKWA9fEbWVODLkcMkg5la0tdbE`](https://agentcat.com/app/sessions/replay/?session-id=ses_3LKWA9fEbWVODLkcMkg5la0tdbE) | Rust | 6 | 1 |
| Rust | [`ses_3LKWA9KTSJfZhkV49HbS96pv5n9`](https://agentcat.com/app/sessions/replay/?session-id=ses_3LKWA9KTSJfZhkV49HbS96pv5n9) | Rust | 2 | 0 |
| TS | [`ses_3LKWAA7ArICbwHue0FqE9gDo7wU`](https://agentcat.com/app/sessions/replay/?session-id=ses_3LKWAA7ArICbwHue0FqE9gDo7wU) | TypeScript | 6 | 1 |
| TS | [`ses_3LKWA8Arr3xGRcmnPqdUEiGOAMV`](https://agentcat.com/app/sessions/replay/?session-id=ses_3LKWA8Arr3xGRcmnPqdUEiGOAMV) | TypeScript | 1 | 0 |
| TS | [`ses_3LKWAAqF67mP02XNeALXzdvD38P`](https://agentcat.com/app/sessions/replay/?session-id=ses_3LKWAAqF67mP02XNeALXzdvD38P) | TypeScript | 1 | 0 |

All sessions show client `agentcat-demo` 1.0.0 and server `agentcat-example-hooks` 1.0.0. Every session except
the last one belongs to one actor, `demo-user` ("Demo User", `actor_3LKWA9Jb78m1zd74XpECqJHbgpp`): the backend
merged the two SDKs' `identify` results into one actor. The last session holds only the TS custom event of
session 2, with an anonymous actor (RD-39, below). Both `fail` events link to the same error issue, `TEST-42`
([dashboard](https://agentcat.com/app/issues/breakdown/?issue_id=issue_3LKWAEod9cL1jprCXktEWLY3h0O)), so the
backend gives the two SDKs' errors the same fingerprint.

## Events

Each row is one event pair, in scenario order. "Same" means equal in the stored events of both servers.

| Step | Type | Name | Intent | Tokens in / out | Tags | Properties | Result |
|---|---|---|---|---|---|---|---|
| `add` | `mcp:tools/call` | `add` | Adding two numbers to check the total of a demo order | 23 / 1 | Same | Same | Same |
| `echo` | `mcp:tools/call` | `echo` | Sending a greeting back | 33 / 6 | Same | Same | Same |
| custom | `mcpcat:custom` | `echo-called` | the echo tool ran | none | Same (none) | Same | Same |
| `echo` (secret) | `mcp:tools/call` | `echo` | Sending text that holds a secret | 36 / 7 | Same | Same | Same: `my [REDACTED] is here` |
| custom | `mcpcat:custom` | `echo-called` | the echo tool ran | none | Same (none) | Same | Same |
| `fail` | `mcp:tools/call` | `fail` | Looking up a record that does not exist | 29 / 8 | Same | Same | Same error message and type |
| session 2 `echo` | `mcp:tools/call` | `echo` | Sending a greeting in a new session | 25 / 8 | Same | Same | Same |
| session 2 custom | `mcpcat:custom` | `echo-called` | the echo tool ran | none | Same (none) | Same | Rust: in session 2; TS: in a separate session (RD-39) |

The checked fields:

- Tags of every tool call: `example: hooks`, `agentcat:token_source: sdk`, and `agentcat_session_id_source`:
  `minted` on the first call of each session and `supplied` on the later calls, on both servers.
- Properties: `source: hooks-example` and `redacted_by: example` on every tool call; `redacted_by: example` on
  every custom event (the `redact_event` hook ran on each event).
- Redaction: no stored event holds `secret-token` (a `get_events` scan found 0 matches on all 16 events), and
  `[REDACTED]` appears once in `parameters` and once in `response` of the secret `echo` of each server.
- Error of `fail`: `isError: true`, the message `the record does not exist` and the same type on both servers.
- Token counts: equal on every tool call, with `tokenSource: sdk`; custom events have none on both servers.
- Duration: present on every event of both servers (0 to 5 ms).

## Differences

All differences are known; none is new.

- RD-39 (owner ruling R29: Rust keeps the session): the TS custom event of session 2's first call has no session,
  because the handler cannot see the session ID that the SDK issues in that response. The backend put it into a
  session of its own, `ses_3LKWAAqF67mP02XNeALXzdvD38P`, with an anonymous actor. The Rust custom event carries the
  issued session, so it appears in session 2 with the actor `demo-user`.
- RD-07: `parameters.extra` is the host's handler context. Rust records `{"requestId": <id>}`; TS on its v2 host
  records `{"mcpReq": {"id", "method", "signal"}}`.
- RD-13: the TS v2 host leaves `isError` out of a successful result, so the TS `response` of a successful call
  has no `isError`; the Rust `response` has `"isError": false`. Failed results have `"isError": true` on both.
- The error's own fields: Rust records `platform: "rust"`; TS records `platform: "javascript"`, a `stack` and
  `frames`. The platform, stack and frames come from each SDK. The type is the same on both servers only because
  `ts-server.mjs` sets `error.name` to the Rust type text (`core::io::error::Error`).
- Durations differ by a few milliseconds, as timing does.

## Finding outside the SDK

The Testing project redacts sensitive values at ingest, and that redaction stores the error type
`core::io::error::Error` as `core<ip>io<ip>rror<ip>rror`: its pattern for compressed IPv6 addresses (a `::` with
optional hexadecimal groups) matches `::`, `::e` and `::E`. Every Rust error type is a `::` path, so on a project
with ingest redaction every Rust error type is changed in the same way. The SDK sends the type unchanged: the gate's
mock run of the same scenario passes only when the Rust type equals `core::io::error::Error`, the text that
`ts-server.mjs` sets. The fix belongs to agentcat-server's redaction rules, not to this SDK:
agentcathq/agentcat-server#623 (owner ruling R29).
