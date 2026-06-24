# Module 6 — Klimaregnskab (Climate accounting / GHG)

The carbon-accounting module. Pages captured for company 997: **Klimaregnskab** (GHG report),
**CSRD-rapport** (CSRD environment report), **Emissionsfaktorer** (emission factors).

> Duplicate / phasing-out note: the flyout contains **two** `Klimaregnskab` leaves. The first
> (captured here) is the **new** scope/category report at
> `environmental_accounting_root/environmental_accounting_new`. A second `Klimaregnskab` entry is
> the older variant being **Udfases** (phased out) — not separately captured; treat as the same
> climate-report surface, older iteration.

All pages share the standard shell (heading + `Filtre` / period picker where relevant + `Eksport`
+ help/recorder icons). Backed by the `co2value/climate-reporting/*` and `csrd-*` endpoints.

---

## 6.1 Klimaregnskab — `environmental_accounting_root/environmental_accounting_new`
Screenshot: `screenshots/c997-kl-klimaregnskab.png`. Charts: **Highcharts**.

The **GHG report** ("Klimaregnskab total for Bolag AB Janus — Periode: 2026-01-01–2026-06-24").
Emissions broken down by **Scope 1/2/3** and by category, location- vs market-based.

- **Filter / period bar:** **`Lokationsbaseret`** dropdown (Location- vs Market-based emissions),
  a date-range picker (`1.1.2026 - 24.6.2026`), **`Rapporter`** (generate/report button),
  building filters in the overlay (`Bygningsanvendelse` 6, `Bygninger` 16, `Nulstil filtre`,
  `Hent klimaregnskab` = fetch climate accounts, `Luk`), `Eksport`.
- **Top cards / charts:**
  1. **Total emission per scope** — a **donut** (Highcharts) with centre total `45 t CO₂`
     ("Total emission") + legend Scope 1 / Scope 2 / Scope 3 with values (`22 / 15 / 8 t CO₂`).
  2. **Total emission per kategori** — a **stacked bar chart** (Highcharts) per category
     (e.g. Naturgas, El) split by energy form (`Varme`/`EL`); segmented control `Scope 1` /
     `Scope 2` / `Scope 3` to switch the scope shown.
- **Scope cards (3):** `Scope 1` / `Scope 2` / `Scope 3`, each: value `t CO₂`, `Aktuel værdi`
  (current value), and a small **percentage gauge** (Highcharts, `100%`). (5 Highcharts total =
  donut + bar + 3 gauges.)
- **`Emissionsopgørelse`** (emission statement) section below the cards — detailed breakdown
  (sourced from `emission-statement`).
- **Headings observed:** an in-page Introduktion/help block ("Sådan bruger du Klimaregnskabet" with
  Navigation / Forside / Rapport / Lokation og markedsbaseret emissioner / Periodevælger / Graferne /
  Tabellen).
- **APIs (page-specific):**
  `POST /api/co2value/climate-reporting/emission-statement`,
  `POST /api/co2value/climate-reporting/emission-statement-charts`,
  `GET /api/co2value/climate-reporting/company-preference/{companyId}`,
  `GET /api/co2value/climate-reporting/saved-export`,
  `POST /api/filter/getbuildingsbyfilter/`.

## 6.2 CSRD-rapport — `environmental_accounting_root/csrd`
Screenshot: `screenshots/c997-kl-csrd.png`

**CSRD environment report** ("Corporate Sustainability Reporting (Environment)") — a tabular
sustainability report.

- **Region:** an in-page Introduktion/help block ("Introduktion til CSRD rapport" /
  "Sådan bruger du CSRD rapporten") + the report grid.
- **Controls:** date-range picker (`1.1.2026 - 31.12.2026`), `Eksport`, `Luk`.
- **Grid (ag-Grid):** `Søg` / `Hurtig søgning...` quick search; row-grouping affordance
  ("Drag here to set row groups") → a groupable CSRD line-item grid (columns not enumerated; data
  is the CSRD environment disclosure rows).
- **Charts:** none.
- **APIs (page-specific):** `POST /api/csrd-overview`.

## 6.3 Emissionsfaktorer — `environmental_accounting_root/emission_factors`
Screenshot: `screenshots/c997-kl-emissionsfaktorer.png`

**Emission-factor browser** — a navigator/tree of emission factors by emission type and region.

- **Layout:** a single left card ("Navigator") with `Søg` (search) + a **`Vis alle`** toggle
  (show all) over an **expandable emission-type list** (each row = coloured icon + name + chevron):
  `Varme` (heat), `Vand` (water), `EL` (electricity), `Transport afstand` (transport distance),
  `Ressourcer` (resources), `Transport brændstof` (transport fuel). Expanding drills into a
  **Regionshierarki** (region hierarchy) to reach the actual factors.
- **Headings:** Introduktion / Emissionstype / Navigator / Regionshierarki / Emissionsfaktorer.
- **Controls/buttons:** `Søg`, `Vis alle` (toggle), **`Eksporter`**, `Luk`.
- **Charts/tables:** none (tree/list UI; factors shown on drill-down).
- **APIs (page-specific):** `GET /api/co2value/climate-reporting/emission-types/hierarchy`.

---

### Recreation notes
- Klimaregnskab = a **dashboard of independent widgets** (scope donut, per-category stacked bar with
  a scope switch, 3 scope gauge-cards, emission-statement table) each refreshing on the
  period/location-basis/building filters → clean HTMX fragment boundaries (post filter → swap
  fragment), same pattern as Module 1's dashboard.
- The **location-vs-market** toggle and the **Scope 1/2/3** switch are server-side report parameters,
  not client recompute — pass them in the POST body to `emission-statement` / `…-charts`.
- CSRD-rapport reuses the **groupable ag-Grid** (row-group + quick search) seen in Bygningsbenchmark
  / Bygningsrapporten / Call to action — one grid component.
- Emissionsfaktorer is a **tree/navigator** (emission type → region hierarchy → factors); driven by
  `emission-types/hierarchy`. Distinct from the grid pages — a dedicated expandable-tree component.
- Two `Klimaregnskab` nav entries (new + Udfases): recreate the `_new` one; the phased-out variant is
  low priority.
