import assert from "node:assert/strict";
import { test } from "node:test";
import * as rmcpVersions from "./rmcp-versions.mjs";

const { latestPatches } = rmcpVersions;

const v = (num, yanked = false) => ({ num, yanked });

test("picks the highest patch of a minor", () => {
  assert.deepEqual(latestPatches([v("3.4.0"), v("3.4.1")]), ["3.4.1"]);
  assert.deepEqual(latestPatches([v("3.4.1"), v("3.4.0")]), ["3.4.1"]);
});

test("skips yanked, pre-release and other majors", () => {
  const list = latestPatches([
    v("3.5.1"),
    v("3.5.2", true),
    v("3.6.0-beta.1"),
    v("2.2.0"),
    v("4.0.0"),
  ]);
  assert.deepEqual(list, ["3.5.1"]);
});

test("orders 3.10.0 after 3.9.3", () => {
  assert.deepEqual(latestPatches([v("3.10.0"), v("3.9.3"), v("3.9.1"), v("3.2.0")]), [
    "3.2.0",
    "3.9.3",
    "3.10.0",
  ]);
});

test("compares patches as numbers", () => {
  assert.deepEqual(latestPatches([v("3.1.9"), v("3.1.10")]), ["3.1.10"]);
});

// rmcp and rmcp-macros release patches apart: an rmcp patch can have no rmcp-macros of the same
// number, so the leg takes the highest rmcp-macros of the same minor.
test("pairs each rmcp version with the highest rmcp-macros of its minor", () => {
  const macros = [
    v("3.1.2"),
    v("3.1.3"),
    v("3.2.0"),
    v("3.2.1"),
    v("3.2.2", true),
    v("3.3.0-beta.1"),
    v("3.3.0"),
    v("3.10.0"),
  ];
  assert.deepEqual(rmcpVersions.withMacros(["3.1.4", "3.2.0", "3.3.0", "3.10.1"], macros), [
    { version: "3.1.4", macros: "3.1.3" },
    { version: "3.2.0", macros: "3.2.1" },
    { version: "3.3.0", macros: "3.3.0" },
    { version: "3.10.1", macros: "3.10.0" },
  ]);
});

test("a minor without an rmcp-macros release fails", () => {
  assert.throws(() => rmcpVersions.withMacros(["3.4.0"], [v("3.3.0"), v("3.5.0"), v("3.4.1", true)]), /rmcp 3\.4\.0/);
});
