# harness/ingest

The local ingest replay (layer L4, owner decision R14). It feeds the publish bodies of one harness run
through the real agentcat-server ingest function `process_event`, in the server's own pytest setup,
against an ephemeral Postgres testcontainer. Each body must return `ProcessResult.SUCCESS` and be
stored. The schema check (`src/contract/validate.mjs`) runs after every driver run; this replay adds
the backend's own checks (project ID format, timestamps, session upsert, tag and property
sanitizing, token counts, issue matching). Run it before the final PR.

agentcat-server is private, so this runs locally only, never in CI.

## Safety

What the steps guarantee:

- Your agentcat-server checkout does not change. The steps only read it to make a throwaway clone
  at the commit in `harness/contract/SOURCE`, then remove the clone's remote and work in the clone.
- No AgentCat endpoint is configured. The test calls `process_event` directly: no SQS, no HTTP API.
  The clone has no `.env` file. Do not copy yours: it can hold production credentials. The only
  setting the server requires is `RESEND_API_KEY`, and step 5 gives it a dummy value.
- `env -i` removes every variable of your shell environment (AWS keys included). The replay gets
  only the variables that step 5 lists.
- `HOME` is a new empty directory, so the replay cannot read files that it finds through `HOME`
  (`~/.aws`, `~/.docker/config.json`, `~/.netrc`). A program that reads an absolute path can still
  read it; `env -i` and the egress guard cover that case.
- `AWS_SHARED_CREDENTIALS_FILE=/dev/null` and `AWS_CONFIG_FILE=/dev/null` give boto3 no credential
  files. `AWS_EC2_METADATA_DISABLED=true` stops boto3 from asking the EC2 instance metadata service
  for credentials.

What `test_ingest_capture.py` enforces (its egress guard, in Python code only):

- A socket `connect`, `connect_ex` or `sendto` must go to a Unix socket (the Docker daemon) or a
  loopback address (the testcontainer). Any other address fails before a packet leaves.
- A `getaddrinfo`, `gethostbyname` or `gethostbyname_ex` lookup through Python's `socket` module
  must be of `localhost` or a loopback address. The guard covers call-time lookups only: any other
  name fails with `EAI_NONAME` before a DNS query.

What nothing enforces:

- C extensions bypass the guard. libpq (psycopg2) opens the Postgres connection itself, to the
  address that testcontainers gives (`localhost` here).
- Reverse lookups (`gethostbyaddr`, `getnameinfo`) and `sendmsg` are not guarded.
- The Docker daemon pulls `postgres:16-alpine` and `testcontainers/ryuk` from Docker Hub when they are
  not cached. `poetry install` (step 3) downloads packages from PyPI.
- The server counts tokens with tiktoken (`cl100k_base`) for a body without `input_tokens` or
  `output_tokens` (custom events). tiktoken downloads that file from
  `openaipublic.blob.core.windows.net` unless it is in `TIKTOKEN_CACHE_DIR`. Step 4 seeds the cache
  from an earlier download when one exists. Without it, the guard blocks the lookup, the server logs
  `tiktoken encoding cl100k_base unavailable` and uses its character estimate, and the replay still
  runs.

To see what a run did, use the network audit below. It records every network system call of the
whole process tree, including C code.

## Steps

Prerequisites: Docker running, git, Python 3.12 or 3.13, Poetry 2.x, Node 22 and corepack, and an
agentcat-server checkout (here `~/agentcat-server`) that contains the commit in
`harness/contract/SOURCE` (`git -C ~/agentcat-server fetch origin` if it does not). The harness run
needs `corepack pnpm install` (repository root) and the first oracle build (`node oracle/src/fetch-ts.mjs`).

Run from the repository root:

```sh
# 1. A harness run. The full matrix writes harness/out/<scenario>.<label>.<transport>.raw.json.
node harness/src/matrix.mjs

# 2 to 6. Replay the Rust tracked captures.
harness/ingest/replay.sh harness/out/*.rust-*.raw.json
```

`replay.sh <capture.raw.json>...` takes one or more capture files. It resolves them to absolute
paths and passes them to the test in `CAPTURE_FILES` (colon-separated; `CAPTURE_FILE` with one path
still works when the test runs by hand). It then does these steps, and removes the clone on exit,
also after a failed step:

- Step 2 makes a throwaway clone of agentcat-server at the contract commit and copies the test in.
- Step 3 runs `poetry install` inside the clone. It keeps your real `HOME` for Poetry's download
  cache. It installs packages and runs no test. Nothing else in the script has your `HOME`.
- Step 4 seeds tiktoken's cache from an earlier download when one exists (see Safety).
- Step 5 runs the replay with `env -i`, a new empty `HOME`, no AWS credentials and a dummy
  `RESEND_API_KEY`. Set `DOCKER_HOST` in your shell if Docker is not at its default socket.
- Step 6 removes the clone.

Expected: a line `INGESTED <n> bodies OK from <m> files`, a line `EVENT TYPES {...}` and `1 passed`.
The test fails on any body that is not `ProcessResult.SUCCESS` and names the file and the body index.
`RESULTS.md` records the last full run.

To replay one run of one scenario, pass its file: `harness/ingest/replay.sh harness/out/basic.ts.stdio.raw.json`.

## Network audit (optional)

To record every network system call of the replay, edit step 5 of `replay.sh` to put
`strace -f -qq -e trace=%network -o "$WORK/net.log"` in front of `poetry run`, and copy the log out
before the script ends (the trap removes `$WORK`). Then, with `WORK` set to the log's directory, list the IP
addresses and the Unix sockets that appear in the trace:

```sh
grep -o -E 'inet_(addr|pton)\([^)]*\)' "$WORK/net.log" | sort | uniq -c
grep -o -E 'sun_path="[^"]*"' "$WORK/net.log" | sort | uniq -c
```

Expected: only `127.0.0.1` (connections to the Ryuk and Postgres containers) and `::1` (a `bind` to
port 0 that probes for IPv6), plus `/var/run/docker.sock` and the local name-service cache socket
`/var/run/nscd/socket`. No `sendto` line names a destination address.
