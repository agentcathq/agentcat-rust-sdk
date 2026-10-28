#!/usr/bin/env bash
# Replays harness capture files through agentcat-server's process_event (README steps 2 to 6).
# Usage: harness/ingest/replay.sh <capture.raw.json>...
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: harness/ingest/replay.sh <capture.raw.json>..." >&2
  exit 2
fi

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
SERVER=${AGENTCAT_SERVER:-$HOME/agentcat-server}

# Absolute paths, before any cd; joined with ":" for CAPTURE_FILES.
FILES=""
for f in "$@"; do
  [ -f "$f" ] || { echo "replay.sh: no such file: $f" >&2; exit 2; }
  abs=$(cd "$(dirname "$f")" && pwd)/$(basename "$f")
  case "$abs" in *:*) echo "replay.sh: path holds a colon: $abs" >&2; exit 2 ;; esac
  FILES="${FILES:+$FILES:}$abs"
done

# 2. A throwaway clone of agentcat-server at the contract commit (the checkout is only read).
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
SHA=$(cut -d' ' -f2 "$ROOT/harness/contract/SOURCE")
git clone --no-hardlinks --quiet "$SERVER" "$WORK/agentcat-server"
git -C "$WORK/agentcat-server" checkout --quiet --detach "$SHA"
git -C "$WORK/agentcat-server" remote remove origin
cp "$ROOT/harness/ingest/test_ingest_capture.py" "$WORK/agentcat-server/"

# 3. The server's Python environment; keeps the real HOME for Poetry's download cache.
(cd "$WORK/agentcat-server" && env -i HOME="$HOME" PATH="$PATH" POETRY_VIRTUALENVS_IN_PROJECT=true \
  PYTHON_KEYRING_BACKEND=keyring.backends.null.Keyring poetry install --no-interaction)

# 4. Optional: seed tiktoken's cache from an earlier download.
mkdir -p "$WORK/tiktoken-cache"
cp /tmp/data-gym-cache/9b5ad71b2ce5302211f9c61530b329a4922fc6a4 "$WORK/tiktoken-cache/" 2>/dev/null || true

# 5. The replay, with a new empty HOME and no AWS credentials.
mkdir -p "$WORK/home"
(cd "$WORK/agentcat-server" && env -i HOME="$WORK/home" PATH="$PATH" POETRY_VIRTUALENVS_IN_PROJECT=true \
  ${DOCKER_HOST:+DOCKER_HOST="$DOCKER_HOST"} \
  PYTHON_KEYRING_BACKEND=keyring.backends.null.Keyring \
  AWS_SHARED_CREDENTIALS_FILE=/dev/null AWS_CONFIG_FILE=/dev/null AWS_EC2_METADATA_DISABLED=true \
  CAPTURE_FILES="$FILES" RESEND_API_KEY=local-ingest-replay \
  TIKTOKEN_CACHE_DIR="$WORK/tiktoken-cache" \
  poetry run pytest test_ingest_capture.py -s -p no:cacheprovider)

# 6. The trap removes the clone.
