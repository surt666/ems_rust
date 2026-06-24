# Frontend — EMS view parity (HTML/CSS first, HTMX-ready)

Date: 2026-06-24 · Branch: `feat/ems-frontend-views`

## Goal

Bring the Astro/HTMX frontend (`frontend/`) closer to the live Enity EMS app, building a chosen set
of missing/incorrect views **in HTML/CSS now**, wired for **HTMX** data later, with **Alpine/hyperscript**
for UI reactivity and **no bespoke JavaScript**. Charts: keep **Nivo** for simple lines, add **Apache
ECharts (with `dataZoom`)** as a hydrated island for zoomable/complex graphs (libraries-as-islands, the
existing Nivo pattern). Keep the current visual shape (dark theme, CSS grid, `global.css` tokens).

Reference capture of the real app: `frontend/docs/ems-recreation/` (+ screenshots) and BDD in
`features/frontend/`.

## Constraints / conventions (from the existing codebase)

- Astro 6 file-based pages → `Layout.astro` shell (`grid-areas: nav/breadcrumb/sidebar/main`).
- All styling via `src/styles/global.css` tokens (`--bg-panel`, `--accent`, `--text-*`, `--radius-*`,
  `--level-*`, `.energy-*`). **CSS grid only, no flexbox.** Append new classes; do not restructure.
- Reactivity: HTMX (server swaps) · hyperscript `_="…"` (DOM directives) · Alpine `x-data` (UI state).
  **No `.js`/`.ts` app logic.** React islands allowed *only* for chart libraries.
- Data is HTML-over-the-wire; relative HTMX URLs (`/query/*`, `/command`, `/hierarchy/*`) are
  CloudFront-proxied to the Rust API. `PUBLIC_AGG_API_BASE_URL` (absolute) for the aggregations fn.
- i18n via `data-i18n` keys, applied on load / HTMX swap / view transition.
- Out of scope (already handled elsewhere): **user creation**, **hierarchy-node creation** (moved into
  node page views, not menu points).

## In scope — build now

### Shared components (`src/components/`)
1. `FilterOverlay.astro` — "Filtre og visning" slide-over. Alpine open/close, slotted filter groups,
   `Nulstil` + `Vis/Anvend` actions. Reused by Målere, Forbrug, Afkøling/Fjernkøling, CSRD.
2. `FilterBar.astro` — inline analysis bar: date-range, resolution toggle (`År/Måned/Uge/Dag/Time/30
   min/15 min`), measure toggle (`Energi/Nøgletal`/cost), `Eksport`, `Nulstil`. Reused by Forbrug +
   Afkøling/Fjernkøling.
3. `DataGrid.astro` — `.data-table` + toolbar: quick-search (hyperscript row filter), header-checkbox
   select (Alpine), pagination shell, and an **HTMX body target with static fallback rows inside**.
   Reused by Målere, CSRD.
4. `EChartsChart.tsx` + `EChartsChartWrapper.tsx` — ECharts island with `dataZoom`; props for
   `option`/series/type; static/mock data now (mirrors `StandbyChart.tsx`) with a clear data seam.
   Nivo retained for simple lines.

### Pages
| View | Route | Build notes |
|---|---|---|
| Målere **registry** (NEW) | `/meters` | DataGrid columns: Bygning · Målertype (`.energy-*` badge) · Hierarki · Målerbetegnelse · Unikt Id · Tags. FilterOverlay (building + meter filter groups, `og\|eller` meter-type combinator). `Opret måler` CTA (link only). |
| Gateway provisioning (MOVED) | `/gateways` | current `meters.astro` content renamed verbatim; MQTT/LoRaWAN forms unchanged. |
| Statistik (NEW) | `/statistics` | count cards (Bygningselementer, Brugere, Alarmer, Målere) + breakdown tables (Aflæsningstype, Energi-/ressourcemåler, Andre målertyper, Fjernaflæst datatilegnelse). No charts. |
| Forbrug (NEW) | `/consumption` | FilterBar + per-meter **ECharts dataZoom** time-series in `ChartCard`s; budget/degree-day comparison controls; meter list. |
| Afkøling (NEW) | `/cooling` | one reusable cooling component; per-heat-meter delta-T **ECharts** + KPI table (Afk./Energi/Flow/Omkostning/Est. meromk./Afk. Indeks) + `Nøgletal`/`Metadata` tabs (Alpine). |
| Fjernkøling (NEW) | `/districtcooling` | same component, cooling-meter variant. |
| Klimaregnskab (FIX) | `/climate` | restructure to: scope **donut** + per-category **stacked bar** (Scope 1/2/3 switch) + 3 **scope gauge cards** + emission-statement section. Charts via ECharts/Nivo. |
| CSRD-rapport (NEW) | `/csrd` | groupable DataGrid + date range. |
| Emissionsfaktorer (NEW) | `/emissionfactors` | expandable navigator: emission type (Varme/Vand/EL/Transport/Ressourcer) → region tree (Alpine/hyperscript). |

### Navigation (`Navbar.astro`) — minimal
- **Målere** → dropdown: *Målere* (`/meters`) + *Gateways (MQTT/LoRaWAN)* (`/gateways`).
- **Oversigt** dropdown gains *Statistik* (`/statistics`) alongside *Dashboard* (`/main`).
- **Analyse** dropdown gains *Forbrug* (`/consumption`), *Afkøling* (`/cooling`), *Fjernkøling* (`/districtcooling`).
- **Klimaregnskab** dropdown: *Klimaregnskab* (`/climate`), *CSRD-rapport* (`/csrd`), *Emissionsfaktorer* (`/emissionfactors`).
- Nothing else changed.

### CSS (append to `global.css`, reuse tokens)
`.filter-overlay/.filter-panel/.filter-group`, `.filter-bar/.toggle-group` (segmented control),
`.grid-toolbar/.grid-search/.grid-pagination`, `.chart-card/.chart-toolbar`, `.gauge-card/.scope-card`,
`.stat-card`. Energy badges reused for Målertype. No restructuring of existing CSS.

### Data / HTMX-readiness (no backend now)
Each data region: static placeholder content **inside** an HTMX target — `hx-get="/query/…"`,
`hx-trigger="load"`, `hx-target`; filter apply `hx-post` → swap result region. Endpoint contracts noted
as TODO mapped to captured EMS endpoints (`meters/list`, `consumption-analysis/*`, `cooling/query`,
`co2value/climate-reporting/*`, `csrd-overview`, emission-types hierarchy). Renders standalone today.

## Out of scope — backlog (captured, not built now)
- Resource Insights (Overblik/Analyse/Energimodel): replace QuickSight embeds with native HTML/ECharts.
- Stamdata (Opsætning) tabbed master-data editor.
- Tjek (phasing out); Dashboard (`/main`) alignment to EMS Forbrugsoverblik; Brugeradministration list.
- **Entitlement-gated, need an enabled account to capture/build:** AI Alarmcenter, **Netværksdiagnostik**
  (confirmed hidden in company/building/admin scope; route refuses even when forced). Logs = admin scope.
- Excluded by request: user creation, hierarchy-node creation (in node views).

## Success criteria
- `npm run build` succeeds; every new page renders standalone (static fallback content visible) under
  `Layout.astro` with the current theme/shape.
- Menu reflects the new structure; `/gateways` keeps MQTT/LoRaWAN unchanged; `/meters` is the registry.
- New shared components are reused across the stated pages (no per-page duplication of filter/grid).
- Charts: ECharts island renders with `dataZoom`; Nivo untouched.
- No bespoke JS; only HTMX/Alpine/hyperscript (+ chart islands). HTMX hooks present and inert-safe.
- BDD in `features/frontend/` remains the behavioural reference for these views.
