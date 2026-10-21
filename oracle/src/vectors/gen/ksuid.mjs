// Vectors for the prefixed KSUIDs of the TS SDK (src/thirdparty/ksuid): the ID that
// KSUID.withPrefix(prefix).fromParts(timeMs, payload) gives, or the error it throws.
//
// Case name prefixes, each with its fields:
//   from-parts:<label>  input: {prefix, time_ms, payload_hex};  output: the ID, or error: the TS error text
//   min-max:<label>     output: KSUID.MIN_STRING_ENCODED or KSUID.MAX_STRING_ENCODED (27 characters)
import { loadCore } from "oracle/bundle";
import { int, seeded, tsKey } from "../support.mjs";

const k = (title) => tsKey("ksuid", title);

const EPOCH_MS = 14e11;
const MAX_TIME_MS = 1e3 * (2 ** 32 - 1) + EPOCH_MS;
const ZEROS = "00".repeat(16);
const FFS = "ff".repeat(16);

// The inputs of the pure fromParts tests of ksuid.test.ts. The TS tests read Date.now(); the
// vector fixes the time, which changes nothing the tests assert about the encoding.
const FIXED_NOW_MS = Date.parse("2025-06-15T12:34:56.789Z");

export async function generate() {
  const { KSUID } = await loadCore();
  const cases = [];
  const add = (name, prefix, timeMs, payloadHex, tsTest) => {
    const input = { prefix, time_ms: timeMs, payload_hex: payloadHex };
    const c = { name: `from-parts:${name}`, input };
    try {
      c.output = KSUID.withPrefix(prefix).fromParts(timeMs, Buffer.from(payloadHex, "hex"));
    } catch (e) {
      c.error = e.message;
    }
    if (tsTest) c.ts_test = tsTest;
    cases.push(c);
  };

  add("epoch-zeros", "ses", EPOCH_MS, ZEROS, [
    k("KSUID > Edge cases > should handle KSUID epoch boundary"),
  ]);
  add("2024-bytes-00-0f", "evt", Date.parse("2024-01-01T00:00:00Z"), "000102030405060708090a0b0c0d0e0f");
  add("max-time-ff", "ses", MAX_TIME_MS, FFS);
  add("below-epoch", "ses", EPOCH_MS - 1, ZEROS);
  add("above-max-time", "ses", MAX_TIME_MS + 1, ZEROS);

  // The tests of src/tests/ksuid.test.ts that only encode.
  add("ts-test:timestamp-and-payload", "ses", FIXED_NOW_MS, "2a".repeat(16), [
    k("KSUID > fromParts > should create KSUID from timestamp and payload"),
  ]);
  add("ts-test:task-prefix", "task", FIXED_NOW_MS, "2a".repeat(16), [
    k("KSUID > Prefix functionality > should create prefixed KSUIDs with fromParts"),
  ]);
  add("ts-test:time-100", "ses", 100, ZEROS);
  add("ts-test:time-now-plus-1e15", "ses", FIXED_NOW_MS + 1e15, ZEROS);

  const rand = seeded(3);
  for (let i = 0; i < 40; i++) {
    const timeMs = int(rand, EPOCH_MS, MAX_TIME_MS);
    const bytes = Array.from({ length: 16 }, () => int(rand, 0, 255).toString(16).padStart(2, "0"));
    add(`seeded-${i}`, i % 2 ? "evt" : "ses", timeMs, bytes.join(""));
  }

  // The two invalid-timestamp calls of the TS test are one test; both cases carry its key.
  for (const c of cases) {
    if (c.name === "from-parts:ts-test:time-100" || c.name === "from-parts:ts-test:time-now-plus-1e15") {
      c.ts_test = [k("KSUID > fromParts > should throw error for invalid timestamp")];
    }
  }

  cases.push({ name: "min-max:min", output: KSUID.MIN_STRING_ENCODED });
  cases.push({ name: "min-max:max", output: KSUID.MAX_STRING_ENCODED });
  return { cases };
}
