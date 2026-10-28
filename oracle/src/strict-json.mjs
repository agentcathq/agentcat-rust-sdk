// The strict JSON encoding of every vector file and every oracle reply. Plain JSON loses some values
// TS produces, and JSON.stringify drops or mangles others without a word. This encoding keeps the
// first kind with a marker object and throws on the second:
//   {"$utf16": [<UTF-16 code units>]}   a string that is not well-formed UTF-16 (TS cut a surrogate pair)
//   {"$f64": "<16 lowercase hex digits>"}  -0, or a number that is not plain (see isPlainNumber): its
//                                        IEEE-754 bits, big-endian
//   {"$repeat": ["<unit>", <count>]}    in case inputs only: `unit` repeated `count` times (see expand)

const MARKER_KEYS = ["$utf16", "$f64", "$repeat"];
const IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/;
const ARRAY_INDEX = /^(?:0|[1-9]\d*)$/;
const NUMBER_TEXT = /^-?(\d+)(?:\.(\d+))?(?:e([+-]\d+))?$/;
const MAX_DIGITS = 2n ** 53n;

/**
 * True when serde_json without `float_roundtrip` parses the JSON text of `n` to `n` itself: a safe
 * integer other than -0, or a finite number whose shortest JS text has a digit string (dot and
 * exponent removed) of at most 2^53 and a decimal exponent `e` (value = digits x 10^e) with |e| <= 22.
 * serde_json then needs one correctly rounded operation: digits times or divided by an exact 10^|e|.
 */
export function isPlainNumber(n) {
  if (typeof n !== "number" || !Number.isFinite(n) || Object.is(n, -0)) return false;
  if (Number.isSafeInteger(n)) return true;
  const [, int, frac = "", exp = "0"] = NUMBER_TEXT.exec(String(n));
  const e = Number(exp) - frac.length;
  return BigInt(int + frac) <= MAX_DIGITS && Math.abs(e) <= 22;
}

/**
 * A JSON-safe copy of `value` with the markers applied. Throws `strict-json: <path>: <reason>` for a
 * value JSON cannot carry: undefined (also as an array hole), a function, symbol or bigint, NaN or
 * +-Infinity, an object whose prototype is not Object.prototype or null, a symbol key or a key that is
 * not well-formed UTF-16, an array with a property that is not an index, a cyclic value (the message
 * names the path that closes the cycle and the path it returns to), and an object with a marker key
 * that is not exactly a well-formed $repeat macro.
 */
export function encodeStrict(value) {
  return encode(value, "$", new Map());
}

/** A copy of `value` with every $repeat macro replaced by its string. */
export function expand(value) {
  if (isRepeat(value)) return value.$repeat[0].repeat(value.$repeat[1]);
  if (Array.isArray(value)) return value.map(expand);
  if (isPlainObject(value)) return copyObject(value, expand);
  return value;
}

/** `open` maps each object on the way down to its path, so that a cycle is found and named. */
function encode(value, path, open) {
  switch (typeof value) {
    case "string":
      return value.isWellFormed() ? value : { $utf16: codeUnits(value) };
    case "number":
      if (!Number.isFinite(value)) fail(path, String(value));
      return isPlainNumber(value) ? value : { $f64: f64Hex(value) };
    case "boolean":
      return value;
    case "object":
      if (value === null) return null;
      if (open.has(value)) fail(path, `cyclic reference to ${open.get(value)}`);
      open.set(value, path);
      try {
        return Array.isArray(value) ? encodeArray(value, path, open) : encodeObject(value, path, open);
      } finally {
        open.delete(value);
      }
    default: // undefined, function, symbol, bigint
      return fail(path, typeof value);
  }
}

function encodeArray(value, path, open) {
  if (Object.getOwnPropertySymbols(value).length > 0) fail(path, "array symbol key");
  const extra = Object.keys(value).find((key) => !ARRAY_INDEX.test(key));
  if (extra !== undefined) fail(path, `array property ${JSON.stringify(extra)}`);
  // Index by index, so that a hole is seen as undefined.
  return Array.from({ length: value.length }, (_, i) => encode(value[i], `${path}[${i}]`, open));
}

function encodeObject(value, path, open) {
  if (!isPlainObject(value)) fail(path, `not a plain object (${className(value)})`);
  if (Object.getOwnPropertySymbols(value).length > 0) fail(path, "symbol key");
  for (const key of Object.keys(value)) {
    if (!key.isWellFormed()) fail(path, `key ${JSON.stringify(key)} is not well-formed UTF-16`);
  }
  if (isRepeat(value)) return { $repeat: [...value.$repeat] };
  if (Object.hasOwn(value, "$repeat")) fail(path, "malformed $repeat macro");
  const marker = MARKER_KEYS.find((k) => Object.hasOwn(value, k));
  if (marker !== undefined) fail(path, `reserved key ${marker}`);
  return copyObject(value, (v, key) => encode(v, keyPath(path, key), open));
}

function keyPath(path, key) {
  return IDENTIFIER.test(key) ? `${path}.${key}` : `${path}[${JSON.stringify(key)}]`;
}

/** Exactly `{"$repeat": [<well-formed string>, <safe non-negative integer>]}`. */
function isRepeat(value) {
  if (!isPlainObject(value)) return false;
  const keys = Object.keys(value);
  if (keys.length !== 1 || keys[0] !== "$repeat") return false;
  const r = value.$repeat;
  return (
    Array.isArray(r) &&
    r.length === 2 &&
    typeof r[0] === "string" &&
    r[0].isWellFormed() &&
    Number.isSafeInteger(r[1]) &&
    r[1] >= 0
  );
}

function isPlainObject(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const proto = Object.getPrototypeOf(value);
  return proto === Object.prototype || proto === null;
}

/** A plain copy of `value`'s own enumerable keys in JS order; a `__proto__` key stays an own key. */
function copyObject(value, map) {
  const out = {};
  for (const key of Object.keys(value)) {
    const field = map(value[key], key);
    Object.defineProperty(out, key, { value: field, enumerable: true, writable: true, configurable: true });
  }
  return out;
}

function className(value) {
  const ctor = Object.getPrototypeOf(value)?.constructor;
  if (typeof ctor === "function" && ctor.name) return ctor.name;
  return Object.prototype.toString.call(value).slice(8, -1);
}

function codeUnits(s) {
  return Array.from({ length: s.length }, (_, i) => s.charCodeAt(i));
}

function f64Hex(n) {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, n); // big-endian
  return view.getBigUint64(0).toString(16).padStart(16, "0");
}

function fail(path, reason) {
  throw new Error(`strict-json: ${path}: ${reason}`);
}
