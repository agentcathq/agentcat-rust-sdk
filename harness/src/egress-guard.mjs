// Preloaded into every TS reference server: `node --import <this file> ...`, and imported by
// ts-v1.mjs and ts-v2.mjs as their first module. The guard itself lives in the oracle package
// (oracle/src/egress-guard.mjs), which also preloads it into its `vitest list` run: it blocks
// every non-local fetch, TCP or TLS connection, DNS lookup and UDP send (that file lists what it
// patches and what it does not cover).
import "oracle/egress-guard";
