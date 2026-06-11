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
