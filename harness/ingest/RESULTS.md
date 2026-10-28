# Ingest replay results

Last full run: 2026-10-28.

- agentcat-server commit (`harness/contract/SOURCE`): `fba51c0279d52138fb9ddc2e019bcc295362a0af`
- Captures: the Rust tracked runs of one full matrix run (`node harness/src/matrix.mjs`, 13 scenarios,
  transports stdio, http, http-stateless and http-sse, lanes 2025-11-25 and 2026-07-28).
- Files: 104 (13 x 4 x 2). Eight files (the `tracing-off` scenario) hold no publish body, as that
  scenario expects.
- Bodies: 856, every one `ProcessResult.SUCCESS` and stored.
- Event types seen: `mcp:tools/call` 776, `mcpcat:custom` 80.
- Result: `1 passed`.

Command, from the repository root:

```sh
node harness/src/matrix.mjs
harness/ingest/replay.sh harness/out/*.rust-20??-??-??-t.*.raw.json
```

Last output lines:

```
INGESTED 856 bodies OK from 104 files
FILES WITHOUT A PUBLISH BODY 8
EVENT TYPES {'mcp:tools/call': 776, 'mcpcat:custom': 80}
```
