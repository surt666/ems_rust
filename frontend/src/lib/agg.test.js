import test from "node:test";
import assert from "node:assert/strict";
import { bucketKey, rollup, apiResolution, MEASURES } from "./agg.ts";

test("apiResolution maps hourly to hourly, everything else to daily", () => {
  assert.equal(apiResolution("hourly"), "hourly");
  for (const g of ["daily", "weekly", "monthly", "yearly"]) assert.equal(apiResolution(g), "daily");
});

test("MEASURES maps each measure to its endpoint action", () => {
  assert.equal(MEASURES.consumption.action, "get_aggregations");
  assert.equal(MEASURES.cost.action, "get_cost");
  assert.equal(MEASURES.co2e.action, "get_emissions");
});

test("bucketKey truncates by granularity", () => {
  const ts = "2026-03-14T09:30:00";
  assert.equal(bucketKey(ts, "hourly"), "2026-03-14T09");
  assert.equal(bucketKey(ts, "daily"), "2026-03-14");
  assert.equal(bucketKey(ts, "monthly"), "2026-03");
  assert.equal(bucketKey(ts, "yearly"), "2026");
});

test("bucketKey weekly returns an ISO week key", () => {
  // 2026-01-01 is a Thursday -> ISO week 2026-W01
  assert.equal(bucketKey("2026-01-01T00:00:00", "weekly"), "2026-W01");
});

test("bucketKey weekly is UTC-stable for a Monday-midnight timestamp", () => {
  // 2026-01-05 is Monday of ISO week 02 (Jan 1 2026 is Thursday = W01)
  assert.equal(bucketKey("2026-01-05T00:00:00", "weekly"), "2026-W02");
});

test("rollup sums per-resource values into sorted buckets", () => {
  const rows = [
    { energy_type: "electricity", unit: "kWh", timestamp: "2026-03-01T00:00:00", value: 10 },
    { energy_type: "electricity", unit: "kWh", timestamp: "2026-03-15T00:00:00", value: 5 },
    { energy_type: "water", unit: "m3", timestamp: "2026-03-02T00:00:00", value: 3 },
  ];
  const { categories, byResource, unitByResource } = rollup(rows, "monthly");
  assert.deepEqual(categories, ["2026-03"]);
  assert.deepEqual(byResource.get("electricity"), [15]);
  assert.deepEqual(byResource.get("water"), [3]);
  assert.equal(unitByResource.get("electricity"), "kWh");
});

test("rollup aligns multiple buckets across resources on a shared axis", () => {
  const rows = [
    { energy_type: "electricity", unit: "kWh", timestamp: "2026-01-10T00:00:00", value: 2 },
    { energy_type: "electricity", unit: "kWh", timestamp: "2026-02-10T00:00:00", value: 4 },
    { energy_type: "water", unit: "m3", timestamp: "2026-02-10T00:00:00", value: 7 },
  ];
  const { categories, byResource } = rollup(rows, "monthly");
  assert.deepEqual(categories, ["2026-01", "2026-02"]);
  assert.deepEqual(byResource.get("electricity"), [2, 4]);
  assert.deepEqual(byResource.get("water"), [0, 7]); // padded on the missing bucket
});
