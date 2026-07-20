import { test } from "node:test";
import assert from "node:assert/strict";
import { markExpanded, markCollapsed, expandedIds } from "./hier-tree-set.js";

// Minimal Storage stand-in (node --test has no sessionStorage / DOM).
function fakeStorage(initial) {
  const m = new Map(initial ? Object.entries(initial) : []);
  return {
    getItem: (k) => (m.has(k) ? m.get(k) : null),
    setItem: (k, v) => m.set(k, String(v)),
  };
}

test("expandedIds is empty when nothing stored", () => {
  assert.deepEqual([...expandedIds(fakeStorage())], []);
});

test("markExpanded adds ids; markCollapsed removes them", () => {
  const s = fakeStorage();
  markExpanded("HN3#200", s);
  markExpanded("HN4#300", s);
  assert.deepEqual([...expandedIds(s)].sort(), ["HN3#200", "HN4#300"]);
  markCollapsed("HN3#200", s);
  assert.deepEqual([...expandedIds(s)], ["HN4#300"]);
});

test("markExpanded is idempotent (a Set, no duplicates)", () => {
  const s = fakeStorage();
  markExpanded("HN3#200", s);
  markExpanded("HN3#200", s);
  assert.equal(expandedIds(s).size, 1);
});

test("empty/falsy ids are ignored", () => {
  const s = fakeStorage();
  markExpanded("", s);
  markExpanded(null, s);
  markExpanded(undefined, s);
  assert.equal(expandedIds(s).size, 0);
});

test("markCollapsed of an unknown id is a no-op", () => {
  const s = fakeStorage({ hierExpandedIds: JSON.stringify(["HN3#200"]) });
  markCollapsed("HN9#999", s);
  assert.deepEqual([...expandedIds(s)], ["HN3#200"]);
});

test("corrupt stored value degrades to an empty set", () => {
  const s = fakeStorage({ hierExpandedIds: "not json" });
  assert.deepEqual([...expandedIds(s)], []);
  markExpanded("HN3#200", s); // recovers, doesn't throw
  assert.deepEqual([...expandedIds(s)], ["HN3#200"]);
});
