# Frontend BDD — Enity EMS recreation

Gherkin success criteria for recreating the Enity EMS UI in our own `frontend/` (own visual style).
Derived from a read-only walkthrough of the live app (company scope "Bolag AB Janus" = `company/997`).
Structure & behaviour only — **colours / exact look are intentionally not asserted**.

Full structural reference + screenshots: `frontend/docs/ems-recreation/`.

| Feature file | Covers (module) |
|---|---|
| `global-shell.feature` | App shell: left hierarchy/context panel, header toolbar, module bar, routing |
| `oversigt.feature` | Oversigt: Dashboard, Statistik, Logs |
| `resource-insights.feature` | Resource Insights: Overblik, Analyse, Energimodel |
| `analyse.feature` | Analyse: Bygningsbenchmark, Forbrug, Standbyanalyse, Afkøling, Fjernkøling, Tjek |
| `overvaagning.feature` | Overvågning: Alarmer, Alarm opsætning, AI Alarmcenter (gated) |
| `maalere.feature` | Målere: meter registry |
| `klimaregnskab.feature` | Klimaregnskab: GHG report, CSRD-rapport, Emissionsfaktorer |
| `rapportering.feature` | Aktiv Styring, Rapportering (3 pages), Call to action |
| `opsaetning.feature` | Opsætning: Stamdata, Brugeradministration, Bygningselementer |

Conventions:
- "context" = the selected scope (department/company/building/area) that drives every data query.
- A "results grid" means selectable + searchable + paginated + column-configurable (ag-Grid-class).
- A "filter overlay" = the `Filtre og visning` panel (filter groups + reset + column chooser).
- "boilerplate APIs" (startup, rights, nav tree, news, groups, alarm summary, theme) are assumed on
  every page and not restated per scenario.
