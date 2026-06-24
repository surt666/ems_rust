# Module 2 — Resource Insights

A filter-driven analytics suite over the context's meters/buildings. Pages: **Overblik**, **Analyse**,
**Energimodel** (+ **Energimodel** *In progress* placeholder). Scope: company 997 (Bolag AB Janus).

Shared pattern across all pages: a **filter bar** (date-range picker, multi-select filters, resolution
toggle, Energi/Nøgletal measure toggle, **Eksport**, **Nulstil filtre** = reset, **Luk** = close) over a
**result area** (totals + grid and/or chart). Filters can be saved (`POST /api/common/custom-filter`).

---

## 2.1 Overblik — `resource_insights/resource_insights_overview`
Screenshot: `screenshots/c997-ri-overblik.png`

Building-use **overview with summation rules** ("Anvend summeringsregler" = apply summation rules).

- **Heading:** "Resource Insights for: Bolag AB Janus", "Vis totaler" (show totals), "68 målere".
- **Totals panel** (per unit): kr. (cost), t CO₂, GWh (heat), m³ (water), GWh (electricity), kWh, km,
  Tonne_short, Liter — i.e. one figure per resource/unit the selected buildings expose.
- **Filter bar:** date-range `1.1.2026 - 31.12.2026`; multi-selects `Vælg tags` (3), `Bygningsanvendelse`
  (building use, 4), `Energiklasse` (energy class, 7), `Bygninger` (buildings, 12); resolution buttons
  `År / Måned / Uge / Ingen` (Year/Month/Week/None); measure toggle `Energi / Nøgletal` (energy / KPI);
  `Eksport`, `Nulstil filtre`, `Luk`.
- **Meter grid:** selectable rows (header checkbox "Column with Header Selection"), `Måler` column,
  unit columns (kWh, m³…), pagination (1…30 page size).
- **APIs:** `GET /api/meters/count`, `GET /api/meter-filter/filter`, `GET /api/meters/units/meter-type`,
  `POST /api/consumption-analysis/overview`, `POST /api/consumption-analysis/kpis`,
  `POST/GET /api/common/custom-filter` (saved filters).

## 2.2 Analyse — `resource_insights/resource_insights_analysis`
Screenshot: `screenshots/c997-ri-analyse.png`

Time-series **consumption analysis** chart.

- **Heading + totals:** "Resource Insights for: Bolag AB Janus" with per-resource totals (e.g. 3,603 GWh,
  5.181 m³, 1,876 GWh).
- **Chart:** 1 × Highcharts (time series of consumption).
- **Filter bar:** date-range; resolution buttons `År / Måned / Uge / Dag / Time / 30 min. / 15 min.`;
  measure toggle `Energi / Nøgletal`; multi-selects `Vælg målertyper` (meter types, 11), `Vælg tags` (3),
  `Bygningsanvendelse` (4), `Bygninger` (12); `Eksport`, `Nulstil filtre`, `Luk`.
- **APIs:** `POST /api/consumption-analysis/details`, `POST /api/consumption-analysis/kpis`,
  `GET /api/meters/meter-types`, `GET /api/meter-filter/filter`.

## 2.3 Energimodel — `energy_model`
Screenshot: `screenshots/c997-ri-energimodel.png`

**Energy-model list** per building (degree-day / normalised model registry).

- **Heading:** "Energimodeller for: Bolag AB Janus — 12 energimodeller (heraf 0 valgt)" (12 models, 0
  selected); `Filtre og visning` (filters & view).
- **Grid:** selectable rows + `Hurtig søgning...` (quick search) + column/visning controls.
- **Filters:** `Vælg bygninganvendelser` (4), `Vælg byer` (cities, 10), `Vælg lande` (countries, 1),
  `Vælg kvalitet` (quality, 2); `Filtre`, `Visning` (view/columns), `Nulstil`, `Eksport`, `Luk`.
- **API:** `GET /api/energy-models/buildings-overview`.

## 2.4 Energimodel *(In progress)* & Call-to-action
- A second **Energimodel** menu entry carries an **In progress** badge — a work-in-progress variant
  (not separately recreated; treat as the same energy-model surface, newer iteration).
- **Call to action** is a sibling **top-level** menu module (see `07-...`/dashboard card "5 største
  energispild") — list of the biggest energy-waste opportunities (`POST /api/calltoaction/getcalltoaction`).

---

### Recreation notes
- One reusable **FilterBar** component (date-range + multi-selects + resolution + measure toggle +
  export/reset) drives all three pages; only the result area differs (totals+grid / chart / model grid).
- Server returns aggregates per resource/unit — design the totals panel to be unit-driven (loop the units
  present), not hard-coded to heat/water/electricity.
- Grids are selectable + paginated + column-configurable + searchable → reuse one data-grid component.
- Heavy reliance on `consumption-analysis/*` endpoints with POSTed filter bodies — good HTMX fragment
  boundaries (post filter → swap result fragment).
