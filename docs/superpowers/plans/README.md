# Implementation plans

The design ([`../specs/2026-10-19-rust-sdk-design.md`](../specs/2026-10-19-rust-sdk-design.md), §12)
has ten milestones, built through seven plans in this order. Each plan is written when the previous
one is done, so it can use the real names and types the earlier code produced. Every plan runs
through `superpowers:subagent-driven-development`.

| Plan | Milestones | Delivers | Status |
|---|---|---|---|
| [1 Foundation](2026-10-19-plan-1-foundation.md) | M1 gates and CI; M2 primitives; M3 injection and handles | Lint, dependency, MSRV, feature, public-API, coverage and mutation gates; parity manifest and tripwire; parity for constants, KSUID, UUIDv7, tags, tokens, handles, injection, stripping | written |
| 2 Event hygiene | M4 | Event model, redaction (D08), sanitization, truncation (TS cuts), error capture (D10, D09) | next |
| 3 Delivery | M5 | TS queue semantics, retries, exports per attempt, base URL last-wins, 10 s timeout, flush and shutdown | |
| 4 Engine and public API | M6–M7 | `Tracked` parity, D34, D35, D36, D40, hooks; the 10 cases plan 1 defers | |
| 5 Operations | M8 | Logging (D60/D61), diagnostics (D50), kill switch (D44), fake (D41) | |
| 6 Exporters | M9 | D45 types and `from_json`, D18, TS Sentry behavior | |
| 7 Release | M10 | Packaging probe, release workflow, `cargo semver-checks`, README, CHANGELOG, `0.1.0` | |

## Gates that tighten over the plans

- **Coverage:** `scripts/coverage-floor` starts at 81 % of lines (the measured baseline). Each plan
  raises it to its measured value; plan 7 must reach the spec's 95 %.
- **Mutation testing:** changed code in every pull request must score 95 % from plan 1 on. The
  nightly whole-crate floors in `scripts/mutants-floor.env` start at 0 and rise with the measured
  scores; plan 7 must reach 90 % (core) and 80 % (rmcp adapter).
- **Parity tripwire:** `tests/parity/completeness.rs` `ENFORCED` grows by the TS files each plan
  ports; `DEFERRED` must be empty after plan 7, and `ENFORCED` must then hold every TS file.
- **Semver checks:** start in plan 7, after `0.1.0` gives `cargo semver-checks` a baseline.
