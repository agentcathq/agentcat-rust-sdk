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
- `HOME` is a new empty directory, so the replay cannot read files under your home directory
  (`~/.aws`, `~/.docker/config.json`, `~/.netrc`).
- `AWS_SHARED_CREDENTIALS_FILE=/dev/null` and `AWS_CONFIG_FILE=/dev/null` give boto3 no credential
  files. `AWS_EC2_METADATA_DISABLED=true` stops boto3 from asking the EC2 instance metadata service
  for credentials.

What `test_ingest_capture.py` enforces (its egress guard, in Python code only):

- A socket `connect`, `connect_ex` or `sendto` must go to a Unix socket (the Docker daemon) or a
  loopback address (the testcontainer). Any other address fails before a packet leaves.
- A `getaddrinfo`, `gethostbyname` or `gethostbyname_ex` lookup must be of `localhost` or a loopback
  address. Any other name fails with `EAI_NONAME` before a DNS query.

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
`harness/contract/SOURCE` (`git -C ~/agentcat-server fetch origin` if it does not).

Run from the repository root:

```sh
# 1. A fresh TS run: writes harness/out/basic.ts.stdio.raw.json (11 publish bodies).
node harness/src/driver.mjs harness/scenarios/basic.json ts stdio \
  -- node --import ./harness/src/egress-guard.mjs harness/src/servers/ts-v1.mjs

# 2. A throwaway clone of agentcat-server at the contract commit.
WORK=$(mktemp -d)
SHA=$(cut -d' ' -f2 harness/contract/SOURCE)
git clone --no-hardlinks --quiet ~/agentcat-server "$WORK/agentcat-server"
git -C "$WORK/agentcat-server" checkout --quiet --detach "$SHA"
git -C "$WORK/agentcat-server" remote remove origin
cp harness/ingest/test_ingest_capture.py "$WORK/agentcat-server/"
cp harness/out/basic.ts.stdio.raw.json "$WORK/capture.json"

# 3. The server's Python environment, inside the clone (agentcat-server CLAUDE.md, "Working in Git
#    Worktrees": poetry install). POETRY_VIRTUALENVS_IN_PROJECT keeps the venv in the clone.
#    This step keeps your HOME for Poetry's download cache. It installs packages and runs no test.
(cd "$WORK/agentcat-server" && env -i HOME="$HOME" PATH="$PATH" POETRY_VIRTUALENVS_IN_PROJECT=true \
  PYTHON_KEYRING_BACKEND=keyring.backends.null.Keyring poetry install --no-interaction)

# 4. Optional: seed tiktoken's cache from an earlier download (see Safety).
mkdir -p "$WORK/tiktoken-cache"
cp /tmp/data-gym-cache/9b5ad71b2ce5302211f9c61530b329a4922fc6a4 "$WORK/tiktoken-cache/" 2>/dev/null || true

# 5. The replay, with a new empty HOME and no AWS credentials.
#    Add DOCKER_HOST="$DOCKER_HOST" if Docker is not at its default socket.
mkdir -p "$WORK/home"
(cd "$WORK/agentcat-server" && env -i HOME="$WORK/home" PATH="$PATH" POETRY_VIRTUALENVS_IN_PROJECT=true \
  PYTHON_KEYRING_BACKEND=keyring.backends.null.Keyring \
  AWS_SHARED_CREDENTIALS_FILE=/dev/null AWS_CONFIG_FILE=/dev/null AWS_EC2_METADATA_DISABLED=true \
  CAPTURE_FILE="$WORK/capture.json" RESEND_API_KEY=local-ingest-replay \
  TIKTOKEN_CACHE_DIR="$WORK/tiktoken-cache" \
  poetry run pytest test_ingest_capture.py -s -p no:cacheprovider)

# 6. Clean up.
rm -rf "$WORK"
```

Expected: a line `INGESTED 11 bodies OK; event types: [...]` and `1 passed`.

To replay another run (a Rust run in plan 3), copy its `harness/out/<scenario>.<label>.<transport>.raw.json`
to `$WORK/capture.json` in step 2 instead.

## Network audit (optional)

To record every network system call of the replay, run step 5 with
`strace -f -qq -e trace=%network -o "$WORK/net.log"` in front of `poetry run`. Then list the IP
addresses and the Unix sockets that appear in the trace:

```sh
grep -o -E 'inet_(addr|pton)\([^)]*\)' "$WORK/net.log" | sort | uniq -c
grep -o -E 'sun_path="[^"]*"' "$WORK/net.log" | sort | uniq -c
```

Expected: only `127.0.0.1` (connections to the Ryuk and Postgres containers) and `::1` (a `bind` to
port 0 that probes for IPv6), plus `/var/run/docker.sock` and the local name-service cache socket
`/var/run/nscd/socket`. No `sendto` line names a destination address.
