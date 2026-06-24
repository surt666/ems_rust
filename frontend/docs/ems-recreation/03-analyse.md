# Module 3 — Analyse (Analysis)

The deep-analysis module. Pages present for company 997: **Bygningsbenchmark**, **Forbrug**,
**Standbyanalyse**, **Afkøling**, **Fjernkøling**, **Tjek** *(Udfases)*. Two further menu entries
(**Raven Residential**, **Netværksdiagnostik**) are **role/feature-gated** — not visible for this
company; capture them from an enabled context if we ever need them.

All pages share a left/overlay **filter panel** (`Filtre`, `Nulstil filtre`) + a result area, and an
`Eksport(ér)`. Period pickers + measure toggles (Omkostning/Energi/CO₂e) recur.

---

## 3.1 Bygningsbenchmark — `analysis/insights`
Screenshot: `screenshots/c997-an-bygningsbenchmark.png`

Full building-benchmark page (the dashboard's benchmark card, expanded).
- **Heading:** "Bygningsbenchmark total for Bolag AB Janus — Periode: 01.06.2025–31.05.2026 baseret på
  opvarmet areal (m²)" + a "udspecificeret" (itemised) section.
- **Gauge metrics:** `Performance (omk. i forhold til gnm.)`, `Afvigelse for alle bygninger`,
  `Afvigelse for bygninger dårligere end gnm.` / `Potentielle besparelser`,
  `Afvigelse for bygninger bedre end gnm.` / `Opnåede besparelser`. A `range-graph` slider element.
- **Detail grid:** an **ag-Grid** ("Drag here to set row groups", quick search, column chooser) of
  buildings.
- **Filters:** `Periode` (5 options), `Vælg energiform` (3), `Nøgle` (basis: opvarmet areal m², 2),
  `Vælg referencebygningers land` (reference-building country, 59), `Vælg bygningslande` (1),
  `Vælg bygningsanvendelser` (4), `Vælg bygninger` (12); `Filtre`, `Visning`, `Nulstil`, `Eksportér`, `Luk`.
- **APIs:** `POST /api/insights/building_benchmark`, `GET /api/common/countries`,
  `POST /api/filter/getbuildingsbyfilter/`, `GET/POST /api/common/custom-filter`.

## 3.2 Forbrug — `analysis/consumption/`
Screenshot: `screenshots/c997-an-forbrug.png`

The flagship **consumption analysis** view (per-meter graphs + budget comparison). Note: heavy —
shows "Henter data – kan tage flere minutter" (fetching, may take minutes).
- **Regions:** `Filtre` panel · `Målerliste` (meter list, "69 målere valgt") · `Eksporter forbrugsdata` ·
  graph area; period label e.g. "Januar 2026 - December 2026".
- **Controls:** period length (`1 år`…`7 år`), start month, end month, meter scope (`Alle`/groups),
  `Vælg tags`, meter-type multi-select, `Summering (energiform)` (sum by energy form / m²),
  comparison basis (`Valgte periode` / `Budget` / `Samme periode` sidste år),
  `Grad- og dato-korrigeret budget` (degree+date-corrected budget) / `Akkumuleret`,
  chart type `Søjler` (bars); `Nulstil filtre`, `Eksporter alle grafer`.
- **APIs:** `GET /api/meters/count` (+ a long-running consumption fetch).

## 3.3 Standbyanalyse — `analysis/standby`
Screenshot: `screenshots/c997-an-standbyanalyse.png`

Per-building **standby vs operating** consumption.
- **Tabs:** `EL` / `Varme` / `Vand` (energy type).
- **Table (12 rows, 1/building):** `Bygning`, `Standby`, `Drift` (operating), `Total`,
  `Standby pr. time`, `Drift pr. time`, `Standby andel` (share), `Standby andel vægtet` (weighted).
- **Controls:** building-use filter, energy-class filter, building multi-select; measure toggle
  `Omkostning / Energi / CO₂e`; `Analyser`, `Eksporter`, `Nulstil filtre`, `Annullér`, `Luk`.
- **APIs:** `GET /api/standby/buildings`, `POST /api/filter/getbuildingsbyfilter/`.

## 3.4 Afkøling — `analysis/consumption_cooling`
Screenshot: `screenshots/c997-an-afkoling.png`

District-heating **return-temperature / delta-T (afkøling)** efficiency, one panel per heat meter.
- **Per-meter card:** meter header (name + id + building + `Fjernvarme`), **1 Highcharts** chart
  (8 charts total — one per Hovedmåler), tabs `Nøgletal` / `Metadata`, and a KPI table:
  `Afk.` (cooling), `Energi`, `Flow`, `Omkostning`, `Estimeret meromk.` (est. extra cost), `Afk. Indeks`.
- **Controls:** period (`januar 2026 -> december 2026`), sort (`Afkøling Åtd, dårligst øverst` = worst
  first), threshold input, building/use/tags filters, `Hovedmåler (8)` meter scope; `Analyser`, `Nulstil filtre`.
- **APIs:** `GET /api/cooling/query`, `POST /api/filter/getmetersbyfilter/`, `GET /api/standby/buildings`.

## 3.5 Fjernkøling — `analysis/consumption_heating`
Screenshot: `screenshots/c997-an-fjernkoling.png`

Same delta-T analysis surface as Afkøling, but for **cooling (`Køling`) meters** (sort label
`Opvarmning` / `Opv.`). 1 Highcharts + KPI table (`Opv.`, `Energi`, `Flow`); tabs `Nøgletal` / `Metadata`.
- **API:** `GET /api/cooling/query` (shared with Afkøling).

## 3.6 Tjek — `check`  *(Udfases / phasing out)*
Screenshot: `screenshots/c997-an-tjek.png`

**Traffic-light consumption-vs-budget check** matrix ("Forbrugs-/budgetafvigelse — Lyskurve").
- **Controls:** deviation type (`Afvigelser alle` / `merforbrug`), counter (`Tæller 1` / `Tæller 2`),
  budget basis (`Teknisk budget` / `Tidligere forbrug`), year (2021–2027), month (Januar…), period
  (`Valgt måned` / `Hele året`), summation inclusion. Inputs: buttons, text, checkboxes (a grid/matrix).
- Being phased out → low priority to recreate (superseded by Forbrug + Overvågning).

---

### Recreation notes
- Reuse the **FilterBar** + saved-filters pattern from Resource Insights.
- Afkøling & Fjernkøling are the **same component** parameterised by meter category (heat vs cooling)
  → build once, drive `/api/cooling/query`.
- Bygningsbenchmark needs an **ag-Grid-class** data grid (grouping, column chooser, quick search) — pick
  our grid component here; it recurs in Energimodel, Målere, Brugeradministration.
- Standbyanalyse = simple tabbed table → straightforward HTML fragment.
- Forbrug is the heaviest page (async multi-minute data) — design for progressive/streamed loading.
