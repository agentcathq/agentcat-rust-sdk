// Vectors for the JavaScript built-ins the TS SDK leans on, computed by the built-ins themselves
// (no TS code, so no loadCore): JSON.stringify of values and of parsed JSON texts, slice at UTF-16
// cut points, trim, length, String(value) and typeof.
//
// Case name prefixes, each with its fields:
//   stringify:<label>       input: a value;         output: JSON.stringify(input)
//   stringify-text:<label>  input_text: JSON text;  output: JSON.stringify(JSON.parse(input_text))
//   cut:<label>             input: [text, units];   output: text.slice(0, units) (a lone surrogate
//                                                   at the end makes it a $utf16 marker)
//   trim:<label>            input: text;            output: text.trim()
//   utf16-len:<label>       input: text;            output: text.length
//   string:<label>          input: {value?};        output: String(value), an absent value is undefined
//   typeof:<label>          input: {value?};        output: typeof value
//
// A stringify-text input holds JSON texts in an order and with numbers that a value in the file
// cannot carry: keys in insertion order (the file writes objects in JS key order), repeated keys,
// whole floats (1.0) and integers JS cannot hold (2^53 + 1). Its numbers are plain (strict-json.mjs:
// at most 2^53 as a digit string, a decimal exponent of at most 22 either way), which serde_json
// without float_roundtrip parses to the same f64 as JSON.parse (R22), or integers that fit in an i64
// or a u64, which serde_json parses exactly.
import { int, pick, seeded, text } from "../support.mjs";

// What String.prototype.trim removes: the WhiteSpace and LineTerminator code points of ECMA-262.
const JS_WHITESPACE = [
  "\u0009", "\u000A", "\u000B", "\u000C", "\u000D", "\u0020", "\u00A0", "\u1680",
  "\u2000", "\u2001", "\u2002", "\u2003", "\u2004", "\u2005", "\u2006", "\u2007", "\u2008", "\u2009", "\u200A",
  "\u2028", "\u2029", "\u202F", "\u205F", "\u3000", "\uFEFF",
];
// Neighbors of that set that trim keeps (U+0085 is Unicode White_Space; U+180E was Zs before Unicode 6.3).
const NOT_JS_WHITESPACE = ["\u0085", "\u180E", "\u200B"];

const CONTROL = Array.from({ length: 0x20 }, (_, i) => String.fromCharCode(i)).join("");

// Characters of the random strings and keys: escapes, whitespace, BMP letters and astral characters.
const ALPHABET = `ab Z09"\\/\u0000\u0001\u0008\u0009\u000A\u000C\u000D\u001F\u007F\u0085\u00A0\u2028\u2029\uFEFFé世😀𝄞`;
const KEYS = ["0", "1", "2", "9", "10", "4294967294", "4294967295", "01", "-1", "1.5", "a", "b", "id", "..."];
const NUMBERS = [0, -0, 1, -1, 1.5, 0.1 + 0.2, 1e21, 1e-7, 1e-6, 123e-20, 5e-324, Number.MAX_VALUE, 2 ** 53, -(2 ** 53), 2 ** 64];

function hex(c) {
  return `U+${c.codePointAt(0).toString(16).toUpperCase().padStart(4, "0")}`;
}

function stringifyCases() {
  const fixed = {
    "float-one": { n: 1.0 },
    "1e21": 1e21,
    "1e-7": 1e-7,
    "1e-6": 1e-6,
    "negative-zero": -0,
    "0.1+0.2": 0.1 + 0.2,
    "min-subnormal": 5e-324,
    "max-value": 1.7976931348623157e308,
    "2^53": 2 ** 53,
    // The doubles nearest to these integers; stringify-text:int-* has them as integers.
    "u64-max-as-double": 18446744073709551615,
    "i64-min-as-double": -9223372036854775808,
    "control-characters": CONTROL,
    "backslash": "\\",
    "quote": '"',
    "U+007F": "\u007F",
    "line-and-paragraph-separators": "\u2028\u2029",
    "html-characters": "<>&",
    "astral": "😀𝄞",
    "nested": { a: [1, [2, { b: null, c: [true, false] }], {}], "": [[]], "0": { x: "y" } },
  };
  const cases = Object.entries(fixed).map(([label, input]) => ({ name: `stringify:${label}`, input }));
  const rand = seeded(1);
  for (let i = 0; i < 40; i++) {
    const input = randomValue(rand, 3, true);
    cases.push({ name: `stringify:seeded-${i}`, input });
  }
  return cases.map((c) => ({ ...c, output: JSON.stringify(c.input) }));
}

// A random value; an array or an object when `container` is true.
function randomValue(rand, depth, container = false) {
  const kind = container ? int(rand, 4, 5) : depth > 0 ? int(rand, 0, 5) : int(rand, 0, 3);
  switch (kind) {
    case 0:
      return pick(rand, [null, true, false]);
    case 1:
      return pick(rand, [pick(rand, NUMBERS), int(rand, -1000, 1000), (rand() - 0.5) * 10 ** int(rand, -30, 30)]);
    case 2:
    case 3:
      return text(rand, ALPHABET, 0, 8);
    case 4:
      return Array.from({ length: int(rand, 0, 4) }, () => randomValue(rand, depth - 1));
    default: {
      const out = {};
      for (let n = int(rand, 0, 4); n > 0; n--) {
        const key = rand() < 0.7 ? pick(rand, KEYS) : text(rand, ALPHABET, 0, 4);
        out[key] = randomValue(rand, depth - 1);
      }
      return out;
    }
  }
}

function stringifyTextCases() {
  const fixed = {
    "index-keys-first": '{"b":1,"1":2,"a":3,"0":4}',
    "index-keys-ascending": '{"10":1,"9":2,"x":3}',
    "index-key-limits": '{"4294967295":1,"4294967294":2,"01":3,"-1":4,"1.5":5}',
    "duplicate-keys": '{"a":1,"b":2,"a":3}',
    "duplicate-index-keys": '{"2":"a","b":1,"2":"c","0":"d"}',
    "nested-index-keys": '{"z":{"3":[{"y":1,"0":2}],"1":{"q":0,"10":1,"2":2}},"5":{"b":1,"a":2,"4":3}}',
    "float-one": '{"n":1.0}',
    "float-texts": "[1.0,-0.0,-0,1e21,1E+21,1e-7,0.000001,123.456e2,-5e-22,1e22,0.5,100]",
    "int-2^53+1": "9007199254740993",
    "int-u64-max": "18446744073709551615",
    "int-i64-min": "-9223372036854775808",
    "ints-past-2^53": "[9007199254740993,-9007199254740993,9007199254740995,18446744073709551615]",
    "whitespace": ' { "b" : [ 1 , 2 ] ,\n\t"0" : null }\r\n',
  };
  const cases = Object.entries(fixed).map(([label, input_text]) => ({ name: `stringify-text:${label}`, input_text }));
  const rand = seeded(2);
  for (let i = 0; i < 20; i++) {
    cases.push({ name: `stringify-text:seeded-${i}`, input_text: randomText(rand, 3, true) });
  }
  return cases.map((c) => ({ ...c, output: JSON.stringify(JSON.parse(c.input_text)) }));
}

// A JSON text with plain numbers, keys in random insertion order and some keys repeated; an array or
// an object when `container` is true.
function randomText(rand, depth, container = false) {
  const kind = container ? int(rand, 4, 5) : depth > 0 ? int(rand, 0, 5) : int(rand, 0, 3);
  switch (kind) {
    case 0:
      return pick(rand, ["null", "true", "false"]);
    case 1:
      return plainNumberText(rand);
    case 2:
    case 3:
      return JSON.stringify(text(rand, ALPHABET, 0, 8));
    case 4:
      return `[${Array.from({ length: int(rand, 0, 4) }, () => randomText(rand, depth - 1)).join(",")}]`;
    default: {
      const entries = [];
      for (let n = int(rand, 0, 5); n > 0; n--) {
        const key = rand() < 0.3 && entries.length > 0 ? pick(rand, entries)[0] : pick(rand, KEYS);
        entries.push([key, randomText(rand, depth - 1)]);
      }
      return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${v}`).join(",")}}`;
    }
  }
}

// A safe integer, or digits (at most 15) times 10^e with |e| <= 22, written with or without a
// decimal point and an exponent.
function plainNumberText(rand) {
  const sign = rand() < 0.3 ? "-" : "";
  if (rand() < 0.4) return `${sign}${int(rand, 0, 2 ** 53 - 1)}`;
  const digits = String(int(rand, 0, 10 ** int(rand, 1, 15) - 1));
  const e = int(rand, -22, 22);
  if (e < 0 && -e < digits.length) {
    const point = digits.length + e;
    return `${sign}${digits.slice(0, point)}.${digits.slice(point)}`;
  }
  if (e < 0) return `${sign}0.${"0".repeat(-e - digits.length)}${digits}`;
  return `${sign}${digits}${pick(rand, ["e", "E", "e+"])}${e}`;
}

function cutCases() {
  const inputs = [
    ...[0, 1, 2, 3, 4].map((n) => ["a😀b", n]),
    ...[1, 3, 5].map((n) => ["😀😀😀", n]),
    ["", 0],
    ["", 3],
  ];
  return inputs.map(([s, n]) => ({
    name: `cut:${s || "empty"}@${n}`,
    input: [s, n],
    output: s.slice(0, n),
  }));
}

function trimCases() {
  const cases = [...JS_WHITESPACE, ...NOT_JS_WHITESPACE].map((c) => ({ name: `trim:${hex(c)}`, input: `${c}x${c}` }));
  cases.push({ name: "trim:all-whitespace", input: JS_WHITESPACE.join("") });
  cases.push({ name: "trim:inner-whitespace-kept", input: `${JS_WHITESPACE.join("")}x y\u3000z${JS_WHITESPACE.join("")}` });
  cases.push({ name: "trim:empty", input: "" });
  return cases.map((c) => ({ ...c, output: c.input.trim() }));
}

function utf16LenCases() {
  const inputs = { empty: "", "e-acute": "é", emoji: "😀", "a-emoji-b": "a😀b" };
  return Object.entries(inputs).map(([label, input]) => ({ name: `utf16-len:${label}`, input, output: input.length }));
}

function valueCases() {
  const values = {
    undefined: {},
    null: { value: null },
    true: { value: true },
    "0": { value: 0 },
    "-0": { value: -0 },
    "1e21": { value: 1e21 },
    "1.5": { value: 1.5 },
    string: { value: "s" },
    "empty-array": { value: [] },
    "mixed-array": { value: [1, [2, 3], null, "a", {}] },
    "empty-object": { value: {} },
  };
  const cases = [];
  for (const [label, input] of Object.entries(values)) {
    cases.push({ name: `string:${label}`, input, output: String(input.value) });
    cases.push({ name: `typeof:${label}`, input, output: typeof input.value });
  }
  return cases;
}

export async function generate() {
  return {
    cases: [
      ...stringifyCases(),
      ...stringifyTextCases(),
      ...cutCases(),
      ...trimCases(),
      ...utf16LenCases(),
      ...valueCases(),
    ],
  };
}
