import { test } from "node:test";
import assert from "node:assert/strict";
import {
  BASE,
  hashString,
  placeholderCoord,
  resolveCoord,
} from "./sensor-map-coords.js";

const SPREAD = 0.0015; // must match the module

test("hashString is a stable unsigned 32-bit number", () => {
  const h = hashString("daq:abc");
  assert.equal(h, hashString("daq:abc"));
  assert.ok(Number.isInteger(h) && h >= 0 && h <= 0xffffffff);
});

test("placeholderCoord is deterministic for the same daqid", () => {
  assert.deepEqual(placeholderCoord("daq:abc"), placeholderCoord("daq:abc"));
});

test("placeholderCoord differs for different daqids", () => {
  assert.notDeepEqual(placeholderCoord("daq:abc"), placeholderCoord("daq:xyz"));
});

test("placeholderCoord stays within SPREAD of BASE", () => {
  for (const id of ["a", "daq:1", "counter_a", "S#20001", ""]) {
    const c = placeholderCoord(id);
    assert.ok(Math.abs(c.lat - BASE.lat) <= SPREAD, `lat in range for ${id}`);
    assert.ok(Math.abs(c.lng - BASE.lng) <= SPREAD, `lng in range for ${id}`);
  }
});

test("resolveCoord prefers explicit finite lat/lon", () => {
  assert.deepEqual(resolveCoord({ lat: "10.5", lon: "20.25" }, "daq:abc"), {
    lat: 10.5,
    lng: 20.25,
  });
});

test("resolveCoord falls back to placeholder when lat/lon missing or non-finite", () => {
  assert.deepEqual(resolveCoord({}, "daq:abc"), placeholderCoord("daq:abc"));
  assert.deepEqual(
    resolveCoord({ lat: "nope", lon: "20" }, "daq:abc"),
    placeholderCoord("daq:abc"),
  );
});

test("resolveCoord falls back to placeholder for empty/blank string coords", () => {
  assert.deepEqual(resolveCoord({ lat: "", lon: "" }, "daq:abc"), placeholderCoord("daq:abc"));
  assert.deepEqual(resolveCoord({ lat: "  ", lon: "5" }, "daq:abc"), placeholderCoord("daq:abc"));
});
