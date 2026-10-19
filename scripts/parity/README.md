# Parity manifest

`build-manifest.py` writes two files:

- `tests/parity/manifest.tsv`: one row per TS test case, with its
  disposition (`port`, `adapt` or `na`) and the path of its Rust test
  (`rust_test`, relative to `parity::`).
- `tests/parity/NOT_PORTED.md`: every `na` case and the reason that the
  index gives for it.

The tripwire `tests/parity/completeness.rs` reads both files.

## Inputs

1. The `vitest list --json` output of the `v1`, `v2` and `e2e-http`
   projects of `agentcat-typescript-sdk` at commit `9b56e9e`, in
   `scripts/parity/vitest/`. These give the exact test names.
2. The TS test index, `docs/research/06a-ts-test-index.md` to
   `06c-ts-test-index.md`. Each index row gives the port, adapt or N/A
   decision for a test.

vitest does not list the one `it.skip` test of `error-capture.test.ts` or
the three tests of `workers/event-delivery.test.ts`. The script takes their
names from the index.

## List the TS tests

Run these commands in `agentcat-typescript-sdk` at `9b56e9e`, with its
dependencies installed:

```bash
R=/home/ubuntu/agentcat-rust-sdk/scripts/parity/vitest; mkdir -p "$R"
npx vitest list --json="$R/v1.json" --project v1 >/dev/null
npx vitest list --json="$R/v2.json" --project v2 >/dev/null
npx vitest list --json="$R/e2e-http.json" --project e2e-http >/dev/null
python3 -c "import json,sys; print([len(json.load(open(f'$R/{p}.json'))) for p in ('v1','v2','e2e-http')])"
```

The last command prints `[806, 105, 109]`.

## Build the manifest

Run this command in the root of this repository:

```bash
python3 scripts/parity/build-manifest.py tests/parity docs/research scripts/parity/vitest/v1.json scripts/parity/vitest/v2.json scripts/parity/vitest/e2e-http.json
```

It prints `1024 cases: {'port': 672, 'adapt': 201, 'na': 151}`.

## When to regenerate

If the index changes, or if the parity target moves to a new TS commit, run
both steps again. Commit the new `manifest.tsv` and `NOT_PORTED.md` in the
same commit as the index change or the new vitest lists. Never edit the two
generated files by hand.
