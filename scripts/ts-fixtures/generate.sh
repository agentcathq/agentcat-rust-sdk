#!/bin/bash
# Rebuilds tests/fixtures/ts-vectors.json from the TypeScript SDK source.
# Usage: scripts/ts-fixtures/generate.sh [path-to-agentcat-typescript-sdk]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$(dirname "$HERE")")"
TS_SDK="${1:-$ROOT/../agentcat-typescript-sdk}"
OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT
"$TS_SDK/node_modules/.bin/esbuild" "$HERE/entry.ts" \
  --bundle --platform=node --format=esm --log-level=warning \
  --alias:@ts="$TS_SDK/src" --outfile="$OUT_DIR/vectors.mjs" \
  --banner:js="import { createRequire as __agentcatCreateRequire } from 'module'; const require = __agentcatCreateRequire(import.meta.url);"
( cd "$TS_SDK" && node "$OUT_DIR/vectors.mjs" ) > "$ROOT/tests/fixtures/ts-vectors.json"
echo "TS commit: $(git -C "$TS_SDK" rev-parse --short HEAD)" >&2
echo "Wrote $ROOT/tests/fixtures/ts-vectors.json" >&2
