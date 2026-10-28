# Rust SDK plans

The design ([`../specs/2026-10-20-rust-sdk-design.md`](../specs/2026-10-20-rust-sdk-design.md)) has six milestones
that build on each other. Each milestone gets its own plan, written when the previous one is done, so a plan can use
what the earlier milestones taught and the owner's rulings from the previous batch (R17).

| Plan | Milestones (design §12) | Delivers | Status |
|---|---|---|---|
| 1 | 1 + 2 | Workspace, lints, local gate, CI skeleton; the TS oracle at the pinned SHA; the vector pipeline; the JS property-test oracle; the parity list and checker; the harness TS half (v1 and v2 reference servers, driver, mock API, normalizer, rulings); the L4 contract check; the nightly drift sweep. | [Done](2026-10-20-plan-1-foundation.md) — gate and CI green at `46eb58e` (CI run 39531843352, nightly run 39531843346); owner batch answered as R19–R21 |
| 2 | 3 | Pure core against L1 and L2: constants, `js`, KSUID and handles, tokens, tag validation, normalize, truncation, sanitization, redaction. | [Done](2026-10-21-plan-2-pure-core.md) — gate and CI green at `f59791e` (CI run 39677917276, nightly run 39677916538); owner batch answered as R24 |
| 3 | 4 | Interception: `Tracked<H>`, tool-list and tool-call pipelines, registries, `get_more_tools`, client info, `parameters`, panics, `capture_error`; the Rust reference server joins the harness (MCP-wire delta); the rmcp version matrix in CI (R25). | [Done](2026-10-22-plan-3-interception.md) — local gate green at `4525631` (`scripts/check.sh` exit 0, with the full L3 matrix, A/A and canary); CI did not run on this plan's commits, and since plan 5 Task 1 CI runs on Linux, macOS and Windows on every push, green at `037e731` and later; owner batch answered as R27 |
| 4 | 5 | Delivery and lifecycle: event pipeline, worker thread, queue, retries, flush, shutdown, last-drop drain, logging, environment, custom events, `Fake`; event comparison in the harness. | [Done](2026-10-23-plan-4-delivery.md) — local gate green at `3ea2724` (`scripts/check.sh` exit 0: nextest 434/434 with all features and 357/357 without default features, the full matrix of 78 combinations with event comparison, A/A and four canaries; parity 327 ported, 650 n/a, 46 todo); CI did not run on this plan's commits, and since plan 5 Task 1 CI runs on Linux, macOS and Windows on every push, green at `037e731` and later; owner batch answered as R28 |
| 5 | 6 | Finish: README and rustdoc, examples, CHANGELOG, public API snapshot, coverage and mutation floors, the rmcp `main` nightly leg (the canary and the version matrix are in plan 3), docs.agentcat.com pages (R26), zero `todo` in the manifest, ingest replay, dashboard demo, the PR. | [Done](2026-10-27-plan-5-finish.md) — local gate green at `0f61f2e` (`scripts/check.sh` exit 0; parity 371 ported, 652 n/a, 0 todo; line coverage 97.9 % for the crate, 99.9 % for the pure set), CI run 40720870740 green on Linux, macOS and Windows; nightly run 40703209631 green with 100 % of the viable mutants detected; ingest replay of 856 bodies from 104 captures; dashboard demo ([report](../../demo/2026-10-dashboard-demo.md)); docs.agentcat.com draft PR agentcathq/agentcat-ui#265; owner batch answered as R29 |

Items that the reviews of plans 1 to 4 deferred to later plans are listed in [`carry-forward.md`](carry-forward.md).

Every plan ends with the same two steps:

1. Run the full local gate (`scripts/check.sh`) and confirm CI is green on `rewrite`.
2. Bring the milestone's pending rulings (deviation register rows marked `pending`) to the owner as one batch of
   multiple-choice questions (R17), and record the answers in `docs/DECISIONS.md` and the design's section 9.
