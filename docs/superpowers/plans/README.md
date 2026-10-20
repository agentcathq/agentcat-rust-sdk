# Implementation plans

The design ([`../specs/2026-10-19-rust-sdk-design.md`](../specs/2026-10-19-rust-sdk-design.md), §12)
has ten milestones, built through seven plans in this order. Each plan is written when the previous
one is done, so it can use the real names and types the earlier code produced. Every plan runs
through `superpowers:subagent-driven-development`.

| Plan | Milestones | Delivers | Status |
|---|---|---|---|
| [1 Foundation](2026-10-19-plan-1-foundation.md) | M1 gates and CI; M2 primitives; M3 injection and handles | Lint, dependency, MSRV, feature, public-API, coverage and mutation gates; parity manifest and tripwire; parity for constants, KSUID, UUIDv7, tags, tokens, handles, injection, stripping | done ([PR #1](https://github.com/agentcathq/agentcat-rust-sdk/pull/1), [PR #2](https://github.com/agentcathq/agentcat-rust-sdk/pull/2)) |
| 2 Event hygiene | M4 | Event model, redaction (D08), sanitization, truncation (TS cuts), error capture (D10, D09) | next |
| 3 Delivery | M5 | TS queue semantics, retries, exports per attempt, base URL last-wins, 10 s timeout, flush and shutdown | |
| 4 Engine and public API | M6–M7 | `Tracked` parity, D34, D35, D36, D40, hooks; the 10 cases plan 1 defers | |
| 5 Operations | M8 | Logging (D60/D61), diagnostics (D50), kill switch (D44), fake (D41) | |
| 6 Exporters | M9 | D45 types and `from_json`, D18, TS Sentry behavior | |
| 7 Release | M10 | Packaging probe, release workflow, `cargo semver-checks`, README, CHANGELOG, `0.1.0` | |

## Gates that tighten over the plans

- **Coverage:** `scripts/coverage-floor` starts at 81 % of lines and stands at 82 % after plan 1. Each plan
  raises it to its measured value; plan 7 must reach the spec's 95 %.
- **Mutation testing:** changed code in every pull request must score 95 % from plan 1 on. The
  nightly whole-crate floors in `scripts/mutants-floor.env` start at 0 and rise with the measured
  scores; plan 7 must reach 90 % (core) and 80 % (rmcp adapter).
- **Parity tripwire:** `tests/parity/completeness.rs` `ENFORCED` grows by the TS files each plan
  ports; `DEFERRED` must be empty after plan 7, and `ENFORCED` must then hold every TS file.
- **Semver checks:** start in plan 7, after `0.1.0` gives `cargo semver-checks` a baseline.

## D15 reverts still owed

D15 says that the crate copies TS behavior, known defects included. The rows below still differ
from TS, and neither `docs/DECISIONS.md` nor the design spec records them as deliberate. Each row
names the plan that owns the revert to the TS behavior. That plan removes the row in the commit
that reverts it. TS paths are in `agentcat-typescript-sdk` at `9b56e9e`.

| Rust behavior still in the code | TS behavior | Where | Owner |
|---|---|---|---|
| The SDK's HTTP client has a 5 s connect timeout. | No HTTP timeouts. R8 and spec §8.5 keep only the 10 s timeout per request. | `src/publisher.rs:42`, `:358` | plan 3 |
| A tool-call duration of 0 ms is sent as 0. | `publishEvent` replaces a falsy `duration` with now minus `timestamp` (`src/modules/eventQueue.ts:379-382`). | `src/tracked.rs:676`, and `src/agentcat.rs:231` for custom events; `src/publisher.rs:743` sends the value as given | plan 3 |
| The registry rebuild (a call on an instance that never listed) reads every page, up to `REGISTRY_REBUILD_MAX_PAGES` = 20. | The rebuild reads page 1 only: `params: {}` (`src/engine/callWrap.ts:96-97`, spec §9.4). | `src/tracked.rs:60`, `:456`. `tests/pagination.rs:95` `a_call_before_any_listing_rebuilds_the_registry_from_every_page` asserts the Rust behavior and flips with the revert. | plan 4 |
| At most 64 exports run at once. An export past the cap is dropped, with a counted log line. | Every export starts at once, fire-and-forget, with no cap (`src/modules/eventQueue.ts:214-221`, `src/modules/telemetry.ts:46-57`). | `src/publisher.rs:37`, `:619` | plan 3 |
| An identity with an empty user ID is ignored: the event gets no actor fields. | Any identity sets the actor fields, empty values included (`src/modules/pendingEvent.ts:86-90`). | `src/publisher.rs:480` | plan 4 |
| A non-string `context` argument is recorded as its JSON text. | `userIntent` keeps the raw value (`src/engine/callWrap.ts:262`). Truncation throws on a number, boolean or object, and the queue drops the event (`src/modules/truncation.ts:189-191`, `src/modules/eventQueue.ts:148-152`). | `src/tracked.rs:334` (`intent_of`) | plans 2 and 4: the plan 2 event model and truncation accept a non-string intent; plan 4 passes the raw value |
| A custom event from `AgentCat::publish_custom_event` takes the server name from `server_info()` or from the last tool call, and has no client name. | The server name and the client come from the tracked server (`src/modules/eventQueue.ts:372-376`). | `src/agentcat.rs:134-150` | plan 4 |
| The queue also drops the newest event when the untruncated events that it holds add up to 32 MiB. | No memory bound. At 10,000 waiting events, the oldest event is dropped (spec §8.1). | `src/publisher.rs:45`, `:248` | plan 3, with the TS queue semantics |
| `validate_tags` reads the `event_tags` result in the order given. | `validateTags` reads `Object.entries`, which puts integer-like keys first, in ascending order (`src/modules/validation.ts:16`). The order sets the log order and the tags that the 50-entry cap drops. | `src/validation.rs:28` | plan 4, which decides the map type of the hook |
| `mirror_structured_mint_back` changes its argument in place, so the "without mutating" check of `tests/parity/handles.rs:525` (row 57 of `docs/research/06b-ts-test-index.md`) always passes. | `mirrorStructuredMintBack` returns a copy and does not change its input (`src/modules/handles.ts:191-202`). | `src/handles.rs:163`, `src/tracked.rs:752` | plan 4: a test that `Tracked` mirrors into a copy, and that the event keeps the raw result of the customer |

### Open questions for later plans

The final review of plan 1 left these points to later plans. Each plan decides its points, then
reverts to TS, records a difference in `docs/DECISIONS.md`, or adds the test.

- `InjectMemory` (declared `session_id`, reported conflicts) is per `AgentCat` (spec §9.2); TS keys it
  per `track()` (`AgentCatData`). One `AgentCat` that tracks two servers can share "declares
  `session_id`" across tools with the same name. Owner: plan 4.
- The engine merges each listing into the registry; TS replaces the registry on each listing. The
  rebuild error logs `e.message`; TS logs `${error}`. With tracing off, a tool that no listing names
  is not stripped. Owner: plan 4.
- `AgentCat::publish_custom_event(&self, data)` differs from the signature in spec §7.3. Owner: plan 4.
- rmcp serializes schema numbers as `1.0` where JS writes `1` (the host library's serializer). Owner:
  plan 4.
- JS puts integer-like keys first in `{...sc}` (`mirrorStructuredMintBack`) and `{...args}`
  (stripping), as in the tags row above. Owner: plan 4.
- The default panic hook prints to stderr when AgentCat code panics inside `catch_unwind`. Owner:
  plans 4 and 5.
- Recursive JSON walks other than token estimation (sanitization, truncation, serialization, drop)
  on values 5,000 levels deep: spec §10 row 10 covers only token estimates. Owner: plan 2.
- The exporter `BODY` sections of `tests/fixtures/php-audit/exporters/golden-output.txt` have no test
  yet (spec §12 names golden exporter payloads in the plan 6 exit check). Owner: plan 6.
