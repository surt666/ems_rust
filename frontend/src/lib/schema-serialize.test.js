import { test } from "node:test";
import assert from "node:assert/strict";
import { defaultPreset, blankState, serialize } from "./schema-serialize.js";

test("preset serializes to the canonical v2 schema", () => {
  const json = serialize(defaultPreset());
  assert.deepEqual(json, {
    version: 2,
    edges: {
      company: { group: {}, property: {}, building: {} },
      group: { building: {} },
      property: { building: {} },
      building: { area: {} },
    },
    metadata: {
      building: {
        lat: { type: "number", required: true, min: -90, max: 90 },
        lng: { type: "number", required: true, min: -180, max: 180 },
      },
    },
    sensors: ["building", "area"],
  });
});

test("edges omit parents with no children; metadata omits empty types", () => {
  const json = serialize(blankState());
  assert.deepEqual(json.edges, {});
  assert.deepEqual(json.metadata, {});
  assert.deepEqual(json.sensors, []);
  assert.equal(json.version, 2);
});

test("each constraint type serializes correctly, blanks omitted", () => {
  const state = {
    types: ["company", "x"],
    edges: { company: ["x"], x: [] },
    metadata: {
      x: [
        { name: "s", type: "string", required: false, min_len: 1 },
        { name: "n", type: "number", required: true, min: 0, max: 5 },
        { name: "i", type: "integer", required: false },
        { name: "b", type: "boolean", required: false },
        { name: "t", type: "timestamp", required: false },
        { name: "e", type: "enum", required: true, one_of: ["a", "b"] },
      ],
    },
    sensors: [],
  };
  const md = serialize(state).metadata.x;
  assert.deepEqual(md.s, { type: "string", required: false, min_len: 1 });
  assert.deepEqual(md.n, { type: "number", required: true, min: 0, max: 5 });
  assert.deepEqual(md.i, { type: "integer", required: false });
  assert.deepEqual(md.b, { type: "boolean", required: false });
  assert.deepEqual(md.t, { type: "timestamp", required: false });
  assert.deepEqual(md.e, { type: "enum", required: true, one_of: ["a", "b"] });
});

test("serialize drops metadata for types not in the types list", () => {
  const state = { types: ["company"], edges: { company: [] }, metadata: { ghost: [{ name: "x", type: "string", required: false }] }, sensors: [] };
  assert.deepEqual(serialize(state).metadata, {});
});

import { reachable, reachableFromRoot, longestDepth, wouldCycle, hasCycle, validate } from "./schema-serialize.js";

test("wouldCycle blocks an edge that closes a loop", () => {
  const edges = { company: ["group"], group: ["building"], building: [], area: [] };
  assert.equal(wouldCycle(edges, "building", "group"), true);
  assert.equal(wouldCycle(edges, "building", "area"), false);
  assert.equal(wouldCycle(edges, "building", "building"), true);
});

test("reachableFromRoot finds all connected types", () => {
  const edges = { company: ["group"], group: ["building"], building: [], orphan: [] };
  const r = reachableFromRoot(edges);
  assert.ok(r.has("building"));
  assert.ok(!r.has("orphan"));
});

test("longestDepth counts edges from company; boundary 7 ok / 8 too deep", () => {
  const mk = (n) => {
    const edges = {}; let p = "company";
    for (let i = 1; i <= n; i++) { const c = "t" + i; edges[p] = [c]; p = c; }
    edges[p] = [];
    return edges;
  };
  assert.equal(longestDepth(mk(7)), 7);
  const ok = validate({ types: ["company", ...Array.from({ length: 7 }, (_, i) => "t" + (i + 1))], edges: mk(7), metadata: {}, sensors: [] });
  assert.equal(ok.errors.some((e) => e.includes("too deep")), false);
  const bad = validate({ types: ["company", ...Array.from({ length: 8 }, (_, i) => "t" + (i + 1))], edges: mk(8), metadata: {}, sensors: [] });
  assert.equal(bad.errors.some((e) => e.includes("too deep")), true);
});

test("validate flags unreachable, partner, dup field, min>max, empty enum", () => {
  const state = {
    types: ["company", "building", "orphan", "partner"],
    edges: { company: ["building"], building: [], orphan: [], partner: [] },
    metadata: {
      building: [
        { name: "a", type: "number", required: true, min: 5, max: 1 },
        { name: "a", type: "string", required: false },
        { name: "e", type: "enum", required: true, one_of: [] },
      ],
    },
    sensors: [],
  };
  const { ok, errors } = validate(state);
  assert.equal(ok, false);
  assert.ok(errors.some((e) => e.includes("orphan") && e.includes("reachable")));
  assert.ok(errors.some((e) => e.includes("partner") && e.includes("reserved")));
  assert.ok(errors.some((e) => e.includes('duplicate field "a"')));
  assert.ok(errors.some((e) => e.includes("min > max")));
  assert.ok(errors.some((e) => e.includes("enum") && e.includes("value")));
});

test("validate catches a cycle even without the UI guard", () => {
  const state = { types: ["company", "a", "b"], edges: { company: ["a"], a: ["b"], b: ["a"] }, metadata: {}, sensors: [] };
  assert.ok(validate(state).errors.some((e) => e.includes("cycle")));
});

test("preset validates clean", () => {
  assert.equal(validate(defaultPreset()).ok, true);
});
