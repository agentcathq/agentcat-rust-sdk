# harness/ingest

The local ingest replay (layer L4, owner decision R14). It feeds the publish bodies of one harness run
through the real agentcat-server ingest function `process_event`, in the server's own pytest setup,
against an ephemeral Postgres testcontainer. Each body must return `ProcessResult.SUCCESS` and be
stored. The schema check (`src/contract/validate.mjs`) runs after every driver run; this replay adds
the backend's own checks (project ID format, timestamps, session upsert, tag and property
sanitizing, token counts, issue matching). Run it before the final PR.

agentcat-server is private, so this runs locally only, never in CI.

## Safety

- The replay never touches your agentcat-server checkout: it reads it only to make a throwaway
  clone at the commit in `harness/contract/SOURCE`, removes the clone's remote, and works there.
- Do not copy your agentcat-server `.env` into the clone: it can hold production credentials. The
  only setting the server requires is `RESEND_API_KEY`; the steps pass a dummy value. No `.env` file
  is needed.
- The test runs under `env -i`: no AWS keys or other credentials of your shell reach it.
- `test_ingest_capture.py` installs an egress guard: every Python socket connection must be a Unix
  socket (the Docker daemon) or a loopback address (the testcontainer), or it fails.
- `process_event` is called directly: no SQS, no HTTP endpoint, no AgentCat service.
- The server counts tokens with tiktoken (`cl100k_base`) for a body without `input_tokens` or
  `output_tokens` (custom events). tiktoken downloads that file from
  `openaipublic.blob.core.windows.net` unless it is in `TIKTOKEN_CACHE_DIR`. Step 4 seeds the cache
  from an earlier download when one exists. Without it, the egress guard blocks the download, the
  server logs `tiktoken encoding cl100k_base unavailable` and falls back to its character estimate,
  and the replay still runs.
- Docker pulls `postgres:16-alpine` and `testcontainers/ryuk` from Docker Hub when they are not
  cached, and `poetry install` downloads packages from PyPI. Nothing else leaves the machine.

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
(cd "$WORK/agentcat-server" && env -i HOME="$HOME" PATH="$PATH" POETRY_VIRTUALENVS_IN_PROJECT=true \
  PYTHON_KEYRING_BACKEND=keyring.backends.null.Keyring poetry install --no-interaction)

# 4. Optional: seed tiktoken's cache from an earlier download (see Safety).
mkdir -p "$WORK/tiktoken-cache"
cp /tmp/data-gym-cache/9b5ad71b2ce5302211f9c61530b329a4922fc6a4 "$WORK/tiktoken-cache/" 2>/dev/null || true

# 5. The replay. Add DOCKER_HOST="$DOCKER_HOST" if Docker is not at its default socket.
(cd "$WORK/agentcat-server" && env -i HOME="$HOME" PATH="$PATH" POETRY_VIRTUALENVS_IN_PROJECT=true \
  PYTHON_KEYRING_BACKEND=keyring.backends.null.Keyring \
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

To see every connection the replay makes, run step 5 under strace and list the IP connections:

```sh
(cd "$WORK/agentcat-server" && env -i ... strace -f -qq -e trace=connect -o "$WORK/connect.log" \
  poetry run pytest test_ingest_capture.py -s -p no:cacheprovider)
grep -E 'AF_INET6?' "$WORK/connect.log" | grep -o -E 'inet_(addr|pton)\([^)]*\)' | sort | uniq -c
```

Expected: only `127.0.0.1` and `::1` (plus Unix sockets such as `/var/run/docker.sock`).
