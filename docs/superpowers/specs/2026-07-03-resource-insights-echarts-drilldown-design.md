# Resource Insights ECharts drill-down + dashboard consumption restructure — Design Spec

**Date:** 2026-07-03
**Status:** Draft (design) — for review; no implementation yet
**Surface:** Frontend (`frontend/`) — node dashboard + Resource Insights page + shared chart islands
**Supersedes:** the Nivo `AggregationChart` on `rimain.astro` (removed here). Partially advances the
parked "Charts → ECharts, drop React" memory (Nivo goes; React stays — the ECharts charts are React islands).

## Context & goal

The live Enity EMS flow (verified 2026-07-03 on `ems.enity.io`):

1. The **node dashboard** ("Forbrugsoverblik" column) has a **combined** card plus one card **per
   resource** (Varme / Vand / EL …). Each card has a **measure** dropdown (cost / consumption / CO₂e), a
   **year** dropdown, and **monthly bars** (this year vs last year).
2. **Clicking a card's graph drills into Resource Insights → Analyse**, which shows a **column (bar)
   chart** for that resource + measure, with a **resolution** selector (`År / Måned / Uge / Dag / Time`)
   and a **resource-type** selector at the top. The y-axis unit follows the measure (kr. / kWh·m³ / t CO₂e).

Our recreation is behind on both ends: the dashboard cards are fixed 30-day ECharts with no
measure/year picker and aren't clickable, and Resource Insights (`rimain.astro`) is a Nivo cumulative
line chart with its own ad-hoc controls. This spec rebuilds the **consumption side of the dashboard**
and the **Resource Insights detail chart** to match the live drill-down, on ECharts, removing the Nivo
`AggregationChart`.

Motivating win beyond parity: one coherent measure model (consumption / cost / CO₂e) shared by the
dashboard cards and the drill-down, driven by the aggregations API we already have.

## Approved approaches

- **Resolution beyond hourly/daily → client-side rollup (fork 1A).** The aggregations API grain is
  `hourly | daily` only. `Time`→hourly and `Dag`→daily map directly; `Uge / Måned / År` are summed
  client-side from **daily** rows. `15 min / 30 min` are **not offered** (no sub-hourly rollup exists).
- **Dashboard → full restructure to match live (fork 2A).** The right column becomes a combined card +
  per-resource cards, each with measure + year dropdowns, this-vs-last-year monthly bars, clickable to
  drill. This consolidates today's `ResourceChart` / `CostCard` / `EmissionsCard` / `ResourceCards` into
  one reusable per-resource card island.

## Backend contract (already deployed — no backend work in this spec)

`crates/services/aggregations` HTTP API, base `PUBLIC_AGG_API_BASE_URL`:

| Measure | Route | Params | Returns |
|---|---|---|---|
| Consumption | `GET /meterdata/query/get_aggregations` | `level_id`, `resolution=hourly\|daily`, `resource?` (filter to one), `start`, `end` | rows `{purpose(resource), unit, timestamp, value, …}` |
| Cost (kr.) | `GET /meterdata/query/get_cost` | `level_id`, `resolution=hourly\|daily`, `start`, `end` | rows per (resource, bucket), value in DKK (all resources) |
| CO₂e | `GET /meterdata/query/get_emissions` | `level_id`, `resolution=hourly\|daily`, `start`, `end` | rows per (resource, bucket); client sums ÷1000 → tonnes |

- `get_cost` / `get_emissions` return **all** resources per bucket — filter to one resource **client-side**.
- `level_id` = the node's full hierarchy path (existing `resolveLevelId()` contract in `lib/agg.ts`).

## Architecture

### Components

- **`AggregationBarChart.tsx`** (NEW, `client:only="react"`) — the Resource Insights drill-down island.
  Owns its filter bar (date-range, resolution buttons, resource-type multiselect) and the bar chart.
  Reads initial state from the **URL query params**, updates the URL (`history.replaceState`) on control
  change, fetches the measure's endpoint, client-side rolls up to the chosen resolution, renders a
  **column** chart via the `EChartsChart` leaf. Includes a faint **previous-year comparison** series.
- **`ConsumptionCard.tsx`** (NEW, `client:only="react"`) — one dashboard consumption card. Props: `resource`
  (or `combined`), initial `measure`, initial `year`. Renders measure + year dropdowns and this-vs-last-year
  **monthly bars** via `EChartsChart`; on chart/card click, navigates to `/rimain?...` (drill-down).
- **`EChartsChart.tsx`** (REUSE, minor extension) — presentational leaf. Already supports `type:'bar'`.
  Extend only as needed for the y-axis unit label and a faint comparison series style.
- **`lib/agg.ts`** (EXTEND) — add: `MEASURES` (measure → {endpoint, unit, label, danishLabel}),
  `RESOURCE_COLORS` palette, and `rollupDaily(rows, granularity)` (sum daily rows into week/month/year buckets).

### Removed / consolidated

- **Delete** `AggregationChart.tsx` + `AggregationChartWrapper.tsx` (Nivo; replaced by `AggregationBarChart`).
- **Rewrite** `rimain.astro` — drop the page `<script>` (toUTC/updateChart/initRiPage); render
  `<AggregationBarChart client:only="react" />`. Control state lives in the island (URL-param driven), so
  no page-level raw JS remains.
- **Rewrite** `NodeDashboard.astro` right column — replace `ResourceChart` / `CostCard` / `EmissionsCard` /
  `ResourceCards` with a **combined `ConsumptionCard`** + one `ConsumptionCard` **per resource** present under
  the node. `BenchmarkCard` and `AlarmsCard` (left column) are unchanged.
- Consequently `ResourceChart.tsx`, `CostCard.tsx`, `EmissionsCard.tsx`, `ResourceCards.tsx` are absorbed
  into `ConsumptionCard` and deleted (verify no other importers first).

### Data flow

```
Dashboard ConsumptionCard(resource=heat, measure=cost, year=2026)
   │  user clicks the card's chart
   ▼
navigate('/rimain?resource=heat&measure=cost&from=2026-01-01&to=2026-12-31&resolution=monthly')
   │  (node comes from sessionStorage via resolveLevelId())
   ▼
rimain.astro → <AggregationBarChart/>
   reads URL params → fetch MEASURES[measure].endpoint(level_id, resolution*, from, to)
   → filter to resource(s) → rollupDaily(...) if Uge/Måned/År → EChartsChart bars
   top controls (resolution buttons, resource multiselect, date range) rewrite URL + re-fetch
```

`*` resolution sent to the API is `hourly` for `Time`, else `daily`; `Uge/Måned/År` fetch daily then roll up.

### Measure → endpoint / unit

| Measure (URL `measure=`) | Endpoint | Unit (y-axis) | Notes |
|---|---|---|---|
| `consumption` | `get_aggregations` | resource's unit (kWh, m³, …) | supports server-side `resource` filter |
| `cost` | `get_cost` | kr. | filter resource client-side |
| `co2e` | `get_emissions` | t CO₂e | filter client-side; ÷1000 for tonnes |

### Resolution → data source (client-side rollup)

| Button | API resolution | Client rollup |
|---|---|---|
| `Time` (hour) | `hourly` | none |
| `Dag` (day) | `daily` | none |
| `Uge` (week) | `daily` | sum into ISO-week buckets |
| `Måned` (month) | `daily` | sum into month buckets |
| `År` (year) | `daily` | sum into year buckets |

(`15 min` / `30 min` are omitted — the rollup's finest grain is hourly.)

### Resource-type selector

The row of resource chips/icons at the top of the RI chart. Multi-select; default = the drilled resource.
Options = the resources present under the node (from the fetched rows' `purpose`), labelled via
`RESOURCE_LABELS` and coloured via `RESOURCE_COLORS`. For `consumption` a single-resource selection uses the
server `resource` filter; multi-select and `cost`/`co2e` filter client-side. Mixed-unit multi-select (e.g.
kWh + m³) renders one series per resource (no cross-unit sum).

### Comparison series (this vs last year)

Both the dashboard cards and the RI chart show a **previous-period** comparison as a second, faint series
(same window shifted back **one year**). RI: one extra fetch for the shifted window, aligned onto the same
bucket axis. Dashboard monthly card: current-year vs previous-year, 12 month buckets.

### Combined card nuance

The "combined" dashboard card sums across resources. This is only unit-safe for **cost** (kr.) and **CO₂e**
(t). For **consumption** (mixed kWh/m³), the combined card renders **one series per resource** rather than a
single summed bar. Default measure for the combined card = **cost**.

## Error / empty / loading states

Reuse the existing island convention (`loading | ok | empty | error` in `ResourceChart.tsx`): centered
muted "Indlæser…", "Ingen data for perioden.", and a red error line. No node selected → "Ingen node valgt."

## Testing

- **Pure helpers** in `lib/agg.ts` (`rollupDaily`, `MEASURES` mapping, bucket-key functions) get unit tests
  (`node:test`, mirroring `schema-serialize.test.js`). **Wire a `test` script** (`node --test src/lib/*.test.js`)
  so these — and the currently-orphaned `schema-serialize.test.js` — actually run.
- **Flow** verified via the existing Playwright `e2e/` harness (or manual preview): dashboard card → click →
  RI chart renders with the passed resource/measure/window; changing resolution/resource re-renders.

## Out of scope / non-goals

- The live filter bar's **`Energi / Nøgletal`** (KPI/normalised) toggle, **meter-type / tag / building**
  multiselects, and the server-side **`history` token**. We use plain URL params + resource-only filtering.
- **`15 min` / `30 min`** resolutions (no sub-hourly rollup).
- **Standby** (`standbyanalysis.astro`) still imports Nivo, so `@nivo/core` + `@nivo/line` **cannot be
  dropped** by this spec. Porting `StandbyChart` to ECharts (and then removing `@nivo/*`) is an explicit
  **follow-up**.

## Open questions

1. **Comparison period** — previous *year* (proposed) vs previous *equal-length window*? Proposed: prev year.
2. **Combined consumption card** — one-series-per-resource (proposed) vs hide the combined card when measure
   = consumption?
3. **Resource selector default** — single drilled resource (proposed) vs all resources present?
