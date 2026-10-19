# PHP audit fixtures

The PHP SDK audit generated these files. It ran the real TS modules of
`agentcat-typescript-sdk@9b56e9e` (bundled with esbuild, run on Node 22). The
generator scripts and their notes are in
`agentcat-php-sdk/docs/audit/fixtures/README.md`.

These files are the shared oracle of the PHP, C# and Rust SDKs (`DECISIONS.md`
R10). A Rust result that differs is a bug, unless `DECISIONS.md` or spec §10
lists the difference.

Never edit these files by hand. Copy them again from
`agentcat-php-sdk/docs/audit/fixtures/` when the audit regenerates them.

| File | Content |
|---|---|
| `handles/strings.json` | Every agent-facing string: parameter descriptions, the session pattern, both mint-back blocks, `mcp_session` field descriptions, the `get_more_tools` descriptor and reply text |
| `handles/golden.txt` | sha256 and byte lengths of those strings, plus structured mint-back samples |
| `handles/derive-vectors.json` | Hook-mode session derivation vectors (`id`, `projectId` → `ses_…`) |
| `handles/listing-fixtures.json` | `tools/list` injection output (tools, injected-parameter registry, output registry) for 7 option sets over 5 input tools |
| `exporters/golden-output.txt` | Trace and span IDs, Datadog decimal IDs, UUIDv7 vectors, and the exact request of each exporter for the shared golden event |
