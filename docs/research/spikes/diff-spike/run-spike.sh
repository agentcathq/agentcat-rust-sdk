#!/usr/bin/env bash
# Reproduces the differential-harness spike. Throwaway; not a product.
# Prereqs: ts-sdk-src/ built from `git archive origin/main` of agentcat-typescript-sdk
# (pnpm install --frozen-lockfile && pnpm build), node_modules symlinks in place,
# rust-ref/ built with `cargo build`.
set -euo pipefail
cd "$(dirname "$0")"
TS_SERVER=(node --import ./egress-guard.mjs ts-server.mjs)

# 1. TS reference server + real TS SDK, both transports.
node driver.mjs scenarios/basic.json ts stdio -- "${TS_SERVER[@]}"
node driver.mjs scenarios/basic.json ts http  -- "${TS_SERVER[@]}"

# 2. A/A: a second stdio run must normalize to the identical document.
cp out/basic.ts.stdio.normalized.json out/basic.ts.stdio.run1.normalized.json
node driver.mjs scenarios/basic.json ts stdio -- "${TS_SERVER[@]}" >/dev/null
node diff.mjs out/basic.ts.stdio.run1.normalized.json out/basic.ts.stdio.normalized.json

# 3. Transport diff (expected: only HTTP header capture differs).
node diff.mjs out/basic.ts.stdio.normalized.json out/basic.ts.http.normalized.json || true

# 4. Host baseline: TS MCP SDK vs rmcp, no AgentCat on either side.
SPIKE_NO_TRACK=1 node driver.mjs scenarios/host-baseline.json ts-untracked stdio -- "${TS_SERVER[@]}"
TOOL_CONTRACT="$PWD/tool-contract.json" node driver.mjs scenarios/host-baseline.json rust-untracked stdio -- rust-ref/target/debug/rust-ref-server
node diff.mjs out/host-baseline.ts-untracked.stdio.normalized.json out/host-baseline.rust-untracked.stdio.normalized.json || true

# 5. TS-derived unit vectors (real TS modules, bundled with esbuild).
(cd vectors && TS_SHA=$(git -C /home/ubuntu/agentcat-typescript-sdk rev-parse origin/main) node gen-vectors.bundle.mjs)
