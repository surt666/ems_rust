import test from "node:test";
import assert from "node:assert/strict";
import { bucketKey, rollup, apiResolution, MEASURES, resolveLevelId } from "./agg.ts";

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

test("resolveLevelId falls back to the URL when session state is empty", () => {
  // A deep link or a reload that lost sessionStorage used to yield "", which the
  // API answers with 200 and zero rows — a blank dashboard that looks like
  // missing data rather than a missing selection.
  const store = {};
  globalThis.sessionStorage = { getItem: (k) => store[k] ?? null };
  globalThis.location = { search: "?id=HN2%2310003" };
  assert.equal(resolveLevelId(), "HN2#10003");

  // A property or building needs its path too: the backend locates a node by its
  // HN2 segment, so a bare id below company level resolves to nothing.
  globalThis.location = { search: "?id=HN4%2310001&path=HN0%23root%7CHN1%2310001%7CHN2%2310003" };
  assert.equal(resolveLevelId(), "HN0#root|HN1#10001|HN2#10003#HN4#10001");

  store.selectedNodeId = "HN4#10001";
  store.selectedNodePath = "HN0#root|HN1#10001|HN2#10003";
  assert.equal(resolveLevelId(), "HN0#root|HN1#10001|HN2#10003#HN4#10001",
    "an explicit selection still wins over the URL");
});

test("a stale company selection never shadows the node's own company", () => {
  // Picking one company in the dropdown then clicking another in the tree used to
  // yield "HN2#10001#HN2#10003"; the backend reads the FIRST HN2 as the partition
  // and finds nothing there, so the whole dashboard renders blank.
  const store = {
    selectedCompanyId: "HN2#10001",
    selectedNodeId: "HN2#10003",
    selectedNodePath: "",
  };
  globalThis.sessionStorage = { getItem: (k) => store[k] ?? null };
  globalThis.location = { search: "" };
  assert.equal(resolveLevelId(), "HN2#10003");

  // Below company level the path already carries the company; leave it alone.
  store.selectedNodeId = "HN4#10001";
  store.selectedNodePath = "HN0#root|HN1#10001|HN2#10003";
  assert.equal(resolveLevelId(), "HN0#root|HN1#10001|HN2#10003#HN4#10001");

  // But a bare node with no company anywhere still gets the dropdown's company.
  store.selectedNodeId = "HN4#10001";
  store.selectedNodePath = "";
  assert.equal(resolveLevelId(), "HN2#10001#HN4#10001");
});
