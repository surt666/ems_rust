# Resource Insights ECharts drill-down + dashboard consumption restructure — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Nivo `AggregationChart` with an ECharts bar drill-down on Resource Insights, reached by clicking measure/year-aware per-resource cards on the node dashboard.

**Architecture:** Pure data helpers (measure→endpoint map, client-side daily→week/month/year rollup) live in `lib/agg.ts` and are unit-tested. Two new `client:only="react"` ECharts islands consume them: `ConsumptionCard` (dashboard, per resource, measure+year dropdowns, this-vs-last-year monthly bars, clickable) and `AggregationBarChart` (Resource Insights drill-down, URL-param driven, resolution + resource-type controls). Both render through the existing presentational `EChartsChart` leaf (which already supports `type:"bar"`).

**Tech Stack:** Astro 6 (static), React 18 islands, Apache ECharts 6, TypeScript, `node:test` for pure helpers, Playwright for flow verification. Aggregations HTTP API via `PUBLIC_AGG_API_BASE_URL`.

## Global Constraints

- Work on `main`; do NOT `git push` (user pushes). Commit locally with trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- Reactivity stack is polyglot and stays: Alpine / htmx / hyperscript / invokers. Charts are the sanctioned React-island exception.
- CSS grid only, never flexbox. HTML-over-the-wire elsewhere; chart controls may live inside the React island (no app JS outside the island).
- Aggregations API resolution is `hourly | daily` ONLY. `Uge/Måned/År` are rolled up client-side from daily; `15 min / 30 min` are NOT offered.
- Danish UI copy. Resource wire tokens (lowercase): `electricity, district_heating, district_cooling, gas, water, heat` (match `RESOURCE_LABELS` in `lib/agg.ts`).
- Build check: `cd frontend && npm run build` must succeed (29 pages). No production deploy in this plan.
- `@nivo/*` stays installed — `StandbyChart` still uses it (out of scope here).

---

### Task 1: Pure data helpers in `lib/agg.ts` (+ unit tests + test script)

**Files:**
- Modify: `frontend/src/lib/agg.ts` (append helpers)
- Create: `frontend/src/lib/agg.test.js`
- Modify: `frontend/package.json` (add `test` script)

**Interfaces:**
- Consumes: existing `RESOURCE_LABELS`, `aggBase`, `resolveLevelId` in `lib/agg.ts`.
- Produces:
  - `type Granularity = "hourly" | "daily" | "weekly" | "monthly" | "yearly"`
  - `type Measure = "consumption" | "cost" | "co2e"`
  - `MEASURES: Record<Measure, { action: string; unit: string; label: string; scale: number }>`
  - `apiResolution(g: Granularity): "hourly" | "daily"`
  - `RESOURCE_COLORS: Record<string, string>`
  - `bucketKey(ts: string, g: Granularity): string`
  - `rollup(rows: {purpose:string;unit:string;timestamp:string;value:number}[], g: Granularity): { categories: string[]; byResource: Map<string, number[]>; unitByResource: Map<string,string> }`

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/lib/agg.test.js`:

```js
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

test("rollup sums per-resource values into sorted buckets", () => {
  const rows = [
    { purpose: "electricity", unit: "kWh", timestamp: "2026-03-01T00:00:00", value: 10 },
    { purpose: "electricity", unit: "kWh", timestamp: "2026-03-15T00:00:00", value: 5 },
    { purpose: "water", unit: "m3", timestamp: "2026-03-02T00:00:00", value: 3 },
  ];
  const { categories, byResource, unitByResource } = rollup(rows, "monthly");
  assert.deepEqual(categories, ["2026-03"]);
  assert.deepEqual(byResource.get("electricity"), [15]);
  assert.deepEqual(byResource.get("water"), [3]);
  assert.equal(unitByResource.get("electricity"), "kWh");
});

test("rollup aligns multiple buckets across resources on a shared axis", () => {
  const rows = [
    { purpose: "electricity", unit: "kWh", timestamp: "2026-01-10T00:00:00", value: 2 },
    { purpose: "electricity", unit: "kWh", timestamp: "2026-02-10T00:00:00", value: 4 },
    { purpose: "water", unit: "m3", timestamp: "2026-02-10T00:00:00", value: 7 },
  ];
  const { categories, byResource } = rollup(rows, "monthly");
  assert.deepEqual(categories, ["2026-01", "2026-02"]);
  assert.deepEqual(byResource.get("electricity"), [2, 4]);
  assert.deepEqual(byResource.get("water"), [0, 7]); // padded on the missing bucket
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd frontend && node --test src/lib/agg.test.js`
Expected: FAIL — `bucketKey`/`rollup`/`apiResolution`/`MEASURES` are not exported yet.

- [ ] **Step 3: Append the helpers to `lib/agg.ts`**

Append to `frontend/src/lib/agg.ts`:

```ts
export type Granularity = "hourly" | "daily" | "weekly" | "monthly" | "yearly";
export type Measure = "consumption" | "cost" | "co2e";

/** Measure → aggregations query action + display metadata. `scale` multiplies the
 *  summed value (CO₂e is returned in kg → tonnes). Consumption unit is per-resource. */
export const MEASURES: Record<Measure, { action: string; unit: string; label: string; scale: number }> = {
  consumption: { action: "get_aggregations", unit: "", label: "Forbrug", scale: 1 },
  cost:        { action: "get_cost",         unit: "kr.", label: "Omkostning", scale: 1 },
  co2e:        { action: "get_emissions",    unit: "t CO₂e", label: "CO₂e", scale: 0.001 },
};

/** The API only rolls up hourly or daily; week/month/year are summed client-side from daily. */
export const apiResolution = (g: Granularity): "hourly" | "daily" => (g === "hourly" ? "hourly" : "daily");

/** Per-resource series colours (aligned with enity-theme tokens). */
export const RESOURCE_COLORS: Record<string, string> = {
  electricity: "#46b97c",
  district_heating: "#f5841f",
  district_cooling: "#1f9e8f",
  gas: "#a855f7",
  water: "#3b82f6",
  heat: "#f5841f",
};

interface AggRow { purpose: string; unit: string; timestamp: string; value: number }

/** Bucket key for a timestamp at a granularity. Timestamps are UTC ISO strings. */
export function bucketKey(ts: string, g: Granularity): string {
  if (g === "hourly") return ts.slice(0, 13); // YYYY-MM-DDTHH
  if (g === "daily") return ts.slice(0, 10); // YYYY-MM-DD
  if (g === "monthly") return ts.slice(0, 7); // YYYY-MM
  if (g === "yearly") return ts.slice(0, 4); // YYYY
  // weekly → ISO-8601 week: YYYY-Www
  const d = new Date(ts);
  const day = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()));
  const dayNum = (day.getUTCDay() + 6) % 7; // Mon=0
  day.setUTCDate(day.getUTCDate() - dayNum + 3); // nearest Thursday
  const firstThursday = new Date(Date.UTC(day.getUTCFullYear(), 0, 4));
  const week = 1 + Math.round((day.getTime() - firstThursday.getTime()) / 86400000 / 7);
  return `${day.getUTCFullYear()}-W${String(week).padStart(2, "0")}`;
}

/** Sum per-resource values into sorted buckets at granularity g, aligned on a shared axis. */
export function rollup(
  rows: AggRow[],
  g: Granularity,
): { categories: string[]; byResource: Map<string, number[]>; unitByResource: Map<string, string> } {
  const cats = Array.from(new Set(rows.map((r) => bucketKey(r.timestamp, g)))).sort();
  const idx = new Map(cats.map((c, i) => [c, i]));
  const byResource = new Map<string, number[]>();
  const unitByResource = new Map<string, string>();
  for (const r of rows) {
    if (!byResource.has(r.purpose)) byResource.set(r.purpose, new Array(cats.length).fill(0));
    byResource.get(r.purpose)![idx.get(bucketKey(r.timestamp, g))!] += r.value;
    if (r.unit) unitByResource.set(r.purpose, r.unit);
  }
  return { categories: cats, byResource, unitByResource };
}
```

- [ ] **Step 4: Add the `test` script**

In `frontend/package.json`, add to `scripts` (this also revives the orphaned `schema-serialize.test.js`):

```json
    "test": "node --test 'src/lib/*.test.js'",
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd frontend && npm test`
Expected: PASS — all `agg.test.js` tests green (and `schema-serialize.test.js` runs too).

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/agg.ts frontend/src/lib/agg.test.js frontend/package.json
git commit -m "agg.ts: measure map + client-side rollup helpers (+ test script)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Resource Insights drill-down — `AggregationBarChart` island + `rimain.astro`

**Files:**
- Create: `frontend/src/components/AggregationBarChart.tsx`
- Modify: `frontend/src/pages/rimain.astro` (full rewrite — render the island, drop page script)
- Delete: `frontend/src/components/AggregationChart.tsx`, `frontend/src/components/AggregationChartWrapper.tsx`

**Interfaces:**
- Consumes: `MEASURES`, `apiResolution`, `rollup`, `RESOURCE_COLORS`, `RESOURCE_LABELS`, `resolveLevelId`, `aggBase` (Task 1 + existing `lib/agg.ts`); `EChartsChart` + `EChartsSeries` (existing leaf).
- Produces: the `/rimain` drill-down target reading URL params `resource, measure, from, to, resolution`.

- [ ] **Step 1: Create the island**

Create `frontend/src/components/AggregationBarChart.tsx`:

```tsx
import { useEffect, useMemo, useState } from "react";
import EChartsChart, { type EChartsSeries } from "./EChartsChart";
import {
  MEASURES, apiResolution, rollup, RESOURCE_COLORS, RESOURCE_LABELS,
  resolveLevelId, aggBase, type Granularity, type Measure,
} from "../lib/agg";

// Resource Insights drill-down (Apache ECharts, bar). URL-param driven so the
// dashboard cards can deep-link into it; the top controls rewrite the URL and
// re-fetch. All chart JS lives in this island (no page-level script).

const RESOLUTIONS: { key: Granularity; label: string }[] = [
  { key: "yearly", label: "År" }, { key: "monthly", label: "Måned" },
  { key: "weekly", label: "Uge" }, { key: "daily", label: "Dag" }, { key: "hourly", label: "Time" },
];

interface Row { purpose: string; unit: string; timestamp: string; value: number }

function readParams() {
  const p = new URLSearchParams(typeof location !== "undefined" ? location.search : "");
  const now = new Date();
  const y = now.getFullYear();
  return {
    measure: (p.get("measure") as Measure) || "consumption",
    resource: p.get("resource") || "",
    from: p.get("from") || `${y}-01-01`,
    to: p.get("to") || `${y}-12-31`,
    resolution: (p.get("resolution") as Granularity) || "monthly",
  };
}

function writeParams(next: Record<string, string>) {
  const p = new URLSearchParams(location.search);
  for (const [k, v] of Object.entries(next)) v ? p.set(k, v) : p.delete(k);
  history.replaceState(null, "", `${location.pathname}?${p.toString()}`);
}

export default function AggregationBarChart() {
  const [{ measure, resource, from, to, resolution }, setState] = useState(readParams);
  const [rows, setRows] = useState<Row[]>([]);
  const [selected, setSelected] = useState<string[]>(resource ? [resource] : []);
  const [status, setStatus] = useState<"loading" | "ok" | "empty" | "error">("loading");
  const [msg, setMsg] = useState("");

  const measureDef = MEASURES[measure];

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      const lvl = resolveLevelId();
      if (!lvl) { setStatus("error"); setMsg("Ingen node valgt."); return; }
      const params = new URLSearchParams({
        level_id: lvl, resolution: apiResolution(resolution),
        start: new Date(from).toISOString(), end: new Date(to + "T23:59:59").toISOString(),
      });
      if (measure === "consumption" && resource) params.set("resource", resource);
      try {
        setStatus("loading");
        const res = await fetch(`${aggBase()}/meterdata/query/${measureDef.action}?${params.toString()}`);
        if (!res.ok) { if (!cancelled) { setStatus("error"); setMsg(`Fejl: ${res.statusText}`); } return; }
        const data = (await res.json()) as Row[];
        if (cancelled) return;
        setRows(data);
        setStatus(data.length ? "ok" : "empty");
        // default the resource selector to all present if nothing chosen yet
        if (!selected.length && data.length) {
          const present = Array.from(new Set(data.map((r) => r.purpose)));
          setSelected(resource ? [resource] : present.slice(0, 1));
        }
      } catch (e) {
        if (!cancelled) { setStatus("error"); setMsg(e instanceof Error ? e.message : "Kunne ikke hente data"); }
      }
    };
    run();
    return () => { cancelled = true; };
  }, [measure, resource, from, to, resolution]);

  const present = useMemo(() => Array.from(new Set(rows.map((r) => r.purpose))), [rows]);
  const active = selected.length ? selected : present;

  const { categories, series, unit } = useMemo(() => {
    const filtered = rows.filter((r) => active.includes(r.purpose));
    const { categories, byResource, unitByResource } = rollup(filtered, resolution);
    const series: EChartsSeries[] = Array.from(byResource.entries()).map(([res, data]) => ({
      name: RESOURCE_LABELS[res] ?? res,
      data: data.map((v) => v * measureDef.scale),
      color: RESOURCE_COLORS[res],
      type: "bar",
    }));
    const unit = measureDef.unit || unitByResource.get(active[0]) || "";
    return { categories, series, unit };
  }, [rows, active, resolution, measure]);

  const update = (patch: Partial<ReturnType<typeof readParams>>) => {
    const next = { measure, resource, from, to, resolution, ...patch };
    writeParams(next as any);
    setState(next);
  };

  const toggleResource = (res: string) => {
    setSelected((s) => (s.includes(res) ? s.filter((x) => x !== res) : [...s, res]));
  };

  return (
    <div>
      <div className="ri-filterbar">
        <div className="ri-daterange">
          <input type="date" className="form-input" value={from} onChange={(e) => update({ from: (e.target as HTMLInputElement).value })} />
          <span>–</span>
          <input type="date" className="form-input" value={to} onChange={(e) => update({ to: (e.target as HTMLInputElement).value })} />
        </div>
        <div className="ri-resolutions">
          {RESOLUTIONS.map((r) => (
            <button className={`ri-res-btn${r.key === resolution ? " active" : ""}`} onClick={() => update({ resolution: r.key })}>{r.label}</button>
          ))}
        </div>
        <div className="ri-resources">
          {present.map((res) => (
            <button className={`ri-res-chip${active.includes(res) ? " active" : ""}`}
              style={{ borderColor: RESOURCE_COLORS[res] ?? "#ccc" }} onClick={() => toggleResource(res)}>
              {RESOURCE_LABELS[res] ?? res}
            </button>
          ))}
        </div>
      </div>
      {status !== "ok" ? (
        <div style={{ display: "grid", placeItems: "center", height: "420px", color: status === "error" ? "#dc2626" : "#6b7280" }}>
          {status === "loading" ? "Indlæser…" : status === "empty" ? "Ingen data for perioden." : msg}
        </div>
      ) : (
        <EChartsChart categories={categories} series={series} unit={unit} height={440} zoom={true} />
      )}
    </div>
  );
}
```

- [ ] **Step 2: Rewrite `rimain.astro`**

Replace the entire contents of `frontend/src/pages/rimain.astro` with:

```astro
---
import Layout from '../layouts/Layout.astro';
import AggregationBarChart from '../components/AggregationBarChart.tsx';
---

<Layout title="Resource Insights - Analyse">
	<div class="page-container">
		<h1 class="page-title">Resource Insights</h1>
		<div class="card">
			<AggregationBarChart client:only="react" />
		</div>
	</div>
</Layout>

<style>
	.ri-filterbar {
		display: grid;
		grid-auto-flow: column;
		justify-content: start;
		align-items: center;
		gap: 1.25rem;
		margin-bottom: 1rem;
		flex-wrap: wrap;
	}
	.ri-daterange { display: grid; grid-auto-flow: column; align-items: center; gap: 6px; }
	.ri-daterange .form-input { color-scheme: light; }
	.ri-resolutions, .ri-resources { display: grid; grid-auto-flow: column; gap: 4px; }
	.ri-res-btn, .ri-res-chip {
		background: var(--bg-elevated); border: 1px solid var(--border-medium);
		color: var(--text-secondary); border-radius: 6px; padding: 6px 12px;
		font-size: var(--text-sm); font-weight: 600; cursor: pointer;
	}
	.ri-res-btn.active { background: var(--accent); color: #fff; border-color: var(--accent); }
	.ri-res-chip { border-width: 2px; }
	.ri-res-chip.active { color: var(--text-primary); background: #fff; }
</style>
```

- [ ] **Step 3: Delete the Nivo files**

```bash
git rm frontend/src/components/AggregationChart.tsx frontend/src/components/AggregationChartWrapper.tsx
```

- [ ] **Step 4: Build to verify it compiles**

Run: `cd frontend && npm run build 2>&1 | tail -4`
Expected: `[build] Complete!`, 29 pages. No reference errors to the deleted files (rimain no longer imports them).

- [ ] **Step 5: Verify the drill-down renders in preview**

Run (background): `cd frontend && npx astro preview --port 4340`
Then load `http://localhost:4340/rimain?measure=cost&resource=district_heating&from=2026-01-01&to=2026-12-31&resolution=monthly` in Playwright and confirm: the filter bar (date range, `År/Måned/Uge/Dag/Time` buttons, resource chips) renders, and the chart area shows either bars or the "Ingen data for perioden." empty state (no console errors, no crash). Click `Dag` and a resource chip and confirm the URL updates and the island re-renders. Stop the preview server.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/components/AggregationBarChart.tsx frontend/src/pages/rimain.astro
git commit -m "Resource Insights: ECharts bar drill-down island (replaces Nivo AggregationChart)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Dashboard consumption cards — `ConsumptionCard` + `NodeDashboard.astro`

**Files:**
- Create: `frontend/src/components/ConsumptionCard.tsx`
- Modify: `frontend/src/components/NodeDashboard.astro` (right column rewrite)
- Delete (after confirming no other importers): `frontend/src/components/ResourceChart.tsx`, `CostCard.tsx`, `EmissionsCard.tsx`, `ResourceCards.tsx`

**Interfaces:**
- Consumes: `MEASURES`, `apiResolution`, `rollup`, `RESOURCE_COLORS`, `RESOURCE_LABELS`, `resolveLevelId`, `aggBase` (`lib/agg.ts`); `EChartsChart` (leaf).
- Produces: `ConsumptionCard` island used per-resource + combined on the dashboard; navigates to `/rimain?...` on click.

- [ ] **Step 1: Confirm no stray importers of the cards being deleted**

Run: `cd frontend && grep -rnE "ResourceChart|CostCard|EmissionsCard|ResourceCards" src --include=*.astro --include=*.tsx | grep -v "NodeDashboard.astro"`
Expected: no matches (only `NodeDashboard.astro` uses them). If any other page imports one, note it and keep that file until its usage is migrated.

- [ ] **Step 2: Create the `ConsumptionCard` island**

Create `frontend/src/components/ConsumptionCard.tsx`:

```tsx
import { useEffect, useMemo, useState } from "react";
import EChartsChart, { type EChartsSeries } from "./EChartsChart";
import {
  MEASURES, apiResolution, rollup, RESOURCE_LABELS, resolveLevelId, aggBase,
  type Measure,
} from "../lib/agg";

// One dashboard consumption card (Apache ECharts, monthly bars, this year vs last
// year). Measure + year dropdowns; clicking the chart drills into Resource
// Insights (/rimain) with this card's resource + measure + year.

interface Row { purpose: string; unit: string; timestamp: string; value: number }
interface Props {
  /** A resource token (electricity/water/…) or "combined" (all resources). */
  resource: string;
  title: string;
  height?: number;
}

const yearRange = (): number[] => {
  const y = new Date().getFullYear();
  return [y, y - 1, y - 2, y - 3, y - 4];
};

async function fetchYear(action: string, lvl: string, year: number, resource: string, filterResource: boolean) {
  const params = new URLSearchParams({
    level_id: lvl, resolution: "daily",
    start: `${year}-01-01T00:00:00.000Z`, end: `${year}-12-31T23:59:59.000Z`,
  });
  if (filterResource) params.set("resource", resource);
  const res = await fetch(`${aggBase()}/meterdata/query/${action}?${params.toString()}`);
  if (!res.ok) throw new Error(res.statusText);
  return (await res.json()) as Row[];
}

export default function ConsumptionCard({ resource, title, height = 220 }: Props) {
  const [measure, setMeasure] = useState<Measure>(resource === "combined" ? "cost" : "consumption");
  const [year, setYear] = useState<number>(new Date().getFullYear());
  const [cur, setCur] = useState<Row[]>([]);
  const [prev, setPrev] = useState<Row[]>([]);
  const [status, setStatus] = useState<"loading" | "ok" | "empty" | "error">("loading");

  const md = MEASURES[measure];
  const isCombined = resource === "combined";
  const filterResource = measure === "consumption" && !isCombined;

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      const lvl = resolveLevelId();
      if (!lvl) { setStatus("error"); return; }
      try {
        setStatus("loading");
        const [c, p] = await Promise.all([
          fetchYear(md.action, lvl, year, resource, filterResource),
          fetchYear(md.action, lvl, year - 1, resource, filterResource),
        ]);
        if (cancelled) return;
        const keep = (r: Row) => isCombined || r.purpose === resource || filterResource;
        setCur(c.filter(keep)); setPrev(p.filter(keep));
        setStatus(c.length ? "ok" : "empty");
      } catch { if (!cancelled) setStatus("error"); }
    };
    run();
    return () => { cancelled = true; };
  }, [measure, year, resource]);

  const { categories, series, unit } = useMemo(() => {
    const months = ["jan", "feb", "mar", "apr", "maj", "jun", "jul", "aug", "sep", "okt", "nov", "dec"];
    const sum = (rows: Row[]) => {
      const out = new Array(12).fill(0);
      for (const r of rows) out[new Date(r.timestamp).getUTCMonth()] += r.value * md.scale;
      return out;
    };
    const series: EChartsSeries[] = [
      { name: String(year), data: sum(cur), color: "#f5841f", type: "bar" },
      { name: String(year - 1), data: sum(prev), color: "#aab2ba", type: "bar" },
    ];
    const unit = md.unit || cur[0]?.unit || "";
    return { categories: months, series, unit };
  }, [cur, prev, measure, year]);

  const drill = () => {
    const p = new URLSearchParams({
      measure, from: `${year}-01-01`, to: `${year}-12-31`, resolution: "monthly",
    });
    if (!isCombined) p.set("resource", resource);
    const url = `/rimain?${p.toString()}`;
    if (window.emsNavigate) window.emsNavigate(url); else location.href = url;
  };

  return (
    <div>
      <div className="cc-controls">
        <select className="form-select cc-select" value={measure} onChange={(e) => setMeasure((e.target as HTMLSelectElement).value as Measure)}>
          <option value="consumption">Forbrug</option>
          <option value="cost">Omkostning</option>
          <option value="co2e">CO₂e</option>
        </select>
        <select className="form-select cc-select" value={year} onChange={(e) => setYear(Number((e.target as HTMLSelectElement).value))}>
          {yearRange().map((y) => <option value={y}>{y}</option>)}
        </select>
      </div>
      {status !== "ok" ? (
        <div style={{ display: "grid", placeItems: "center", height: `${height}px`, color: status === "error" ? "#dc2626" : "#6b7280" }}>
          {status === "loading" ? "Indlæser…" : status === "empty" ? "Ingen data." : "Fejl ved indlæsning."}
        </div>
      ) : (
        <div className="cc-chart" onClick={drill} title="Klik for at se detaljer" style={{ cursor: "pointer" }}>
          <EChartsChart categories={categories} series={series} unit={unit} height={height} zoom={false} />
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 3: Rewrite the `NodeDashboard.astro` right column**

In `frontend/src/components/NodeDashboard.astro`: replace the imports of the four deleted cards with `ConsumptionCard`, and replace the entire **right column** (`<!-- RIGHT COLUMN: Forbrugsoverblik -->` `<div class="nd-col">…</div>`) with:

```astro
  <!-- RIGHT COLUMN: Forbrugsoverblik -->
  <div class="nd-col">
    <h3 class="section-title">Forbrugsoverblik</h3>

    <div class="card nd-energy">
      <div class="section-header">
        <h2 class="section-title" style="margin-bottom:0">Samlet</h2>
      </div>
      <ConsumptionCard client:only="react" resource="combined" title="Samlet" height={240} />
    </div>

    <div class="card nd-energy">
      <div class="section-header"><h2 class="section-title" style="margin-bottom:0">Varme</h2></div>
      <ConsumptionCard client:only="react" resource="district_heating" title="Varme" height={200} />
    </div>

    <div class="card nd-energy">
      <div class="section-header"><h2 class="section-title" style="margin-bottom:0">Vand</h2></div>
      <ConsumptionCard client:only="react" resource="water" title="Vand" height={200} />
    </div>

    <div class="card nd-energy">
      <div class="section-header"><h2 class="section-title" style="margin-bottom:0">EL</h2></div>
      <ConsumptionCard client:only="react" resource="electricity" title="EL" height={200} />
    </div>
  </div>
```

And update the import block (top frontmatter) — remove the four deleted imports, add:

```astro
import ConsumptionCard from "./ConsumptionCard.tsx";
```

Keep `BenchmarkCard` and `AlarmsCard` imports/usages (left column) unchanged.

- [ ] **Step 4: Delete the consolidated card files**

```bash
git rm frontend/src/components/ResourceChart.tsx frontend/src/components/CostCard.tsx frontend/src/components/EmissionsCard.tsx frontend/src/components/ResourceCards.tsx
```

- [ ] **Step 5: Add the card control styles**

Append to the `<style>` block in `NodeDashboard.astro`:

```css
  .cc-controls { display: grid; grid-auto-flow: column; justify-content: end; gap: 8px; margin-bottom: 8px; }
  .cc-select { width: auto; padding: 6px 10px; font-size: var(--text-sm); }
```

- [ ] **Step 6: Build to verify it compiles**

Run: `cd frontend && npm run build 2>&1 | tail -4`
Expected: `[build] Complete!`, 29 pages, no unresolved imports.

- [ ] **Step 7: Verify the dashboard + drill-down flow in preview**

Run (background): `cd frontend && npx astro preview --port 4341`. In Playwright, load the page that renders `NodeDashboard` (the `/node` route). Confirm: the right column shows Samlet + Varme + Vand + EL cards each with a measure + year dropdown and a bar chart (or "Ingen data."). Change a card's measure dropdown → chart re-renders. Click a card's chart → navigates to `/rimain?...` with the card's resource/measure/year and the RI bar chart loads. Stop the preview server.

- [ ] **Step 8: Run the unit tests + build once more, then commit**

Run: `cd frontend && npm test && npm run build 2>&1 | tail -3`
Expected: tests PASS, build Complete.

```bash
git add frontend/src/components/ConsumptionCard.tsx frontend/src/components/NodeDashboard.astro
git commit -m "Dashboard: per-resource ConsumptionCard islands (measure/year, click-to-drill)

Consolidates ResourceChart/CostCard/EmissionsCard/ResourceCards into one
measure-aware card; clicking drills into Resource Insights.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Dashboard restructure (combined + per-resource cards, measure/year, this-vs-last-year, clickable) → Task 3. ✓
- RI ECharts bar drill-down, URL-param context, resolution buttons, resource multiselect → Task 2. ✓
- Measure→endpoint map, client-side rollup, resolution constraint → Task 1 + used in 2/3. ✓
- Delete Nivo `AggregationChart`(+Wrapper), consolidate old cards → Tasks 2 & 3. ✓
- `test` script wired (revives `schema-serialize.test.js`) → Task 1. ✓
- Out-of-scope respected: no `@nivo` removal (StandbyChart untouched), no 15/30-min, no Nøgletal/meter-type filters, no server-side history token. ✓
- Open questions from the spec are implemented as the proposed defaults: comparison = previous year (Task 3 fetches `year-1`); combined consumption = one-series-per-resource (Task 3 keeps per-resource rows; combined defaults to `cost`); RI resource default = the drilled resource, falling back to first present (Task 2). These are the spec's recommended answers — adjust in review if the user chose differently.

**Placeholder scan:** No TBD/TODO; every code step has complete code; every command has an expected result.

**Type consistency:** `MEASURES`/`apiResolution`/`rollup`/`bucketKey`/`RESOURCE_COLORS` signatures defined in Task 1 are used with matching names/shapes in Tasks 2 & 3. `EChartsSeries` (`{name,data,color?,type?}`) matches the existing leaf. `window.emsNavigate` is the existing global exposed in `Layout.astro`.

**Note for the implementer:** ECharts/React islands aren't unit-tested here (only the pure `lib/agg.ts` helpers are, in Task 1). Islands are verified by `npm run build` + the Playwright preview checks. If a card's `resource` token has no data under the selected node, the "Ingen data." empty state is expected — verify with a node known to have readings (e.g. the seeded klepierre daqs).
