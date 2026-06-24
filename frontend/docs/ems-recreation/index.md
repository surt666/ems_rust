# Enity EMS — UI reconstruction reference

Captured from the live app at `https://ems.enity.io` on 2026-06-24 (read-only walkthrough via
Playwright; **no forms submitted, no data mutated**). Purpose: catalogue the structure, components,
data and behaviour of every menu point so we can rebuild equivalents in our own `frontend/` in our
own visual style. **Colours / exact look are deliberately NOT recorded** — structure & behaviour only.

- App: **Enity EMS** (Energy Management System), Danish UI, client-rendered SPA.
- Auth: Auth0 (`login.ems.enity.io`, OAuth2 + PKCE).
- API base: `https://ems.enity.io/api/...` (REST-ish; mix of GET + POST-for-read).
- BDD success criteria live in `features/frontend/*.feature` (Gherkin).
- Screenshots: `./screenshots/` (reference only — not a styling spec).

## URL / routing model

```
/App/<contextType>/<contextId>/<module>/<page>
```

- `contextType` ∈ `administrator` (a "department"/partner scope) | `company` | `building` | `area`.
- Context is chosen in the **left hierarchy panel** (see `global-chrome.md`). Selecting a company
  rewrites the whole URL, e.g. picking **Bolag AB Janus** → `/App/company/997/overview/dashboard`.
- Walkthrough scope for this capture: **company "Bolag AB Janus" = company id 997**
  (chosen by the user). Some pages also seen in `administrator/4` (Enity Personale) scope.
- `module`/`page` are slugs (e.g. `overview/statistics`). Discovered slugs are listed per page doc.

## Master navigation tree (top menu — 9 modules)

Badges seen: **Nyt** (New), **In progress**, **Udfases** (being phased out), **Call to action**.

| # | Module (da / en) | Pages | Doc |
|---|---|---|---|
| 1 | **Oversigt** / Overview | Dashboard · Statistik · Logs | [01-oversigt.md](01-oversigt.md) |
| 2 | **Resource Insights** | Overblik · Analyse · Energimodel · Energimodel *(In progress)* · Call to action | [02-resource-insights.md](02-resource-insights.md) |
| 3 | **Analyse** / Analysis | Bygningsbenchmark · Forbrug · Standbyanalyse · Afkøling · Fjernkøling · Tjek *(Udfases)* · Raven Residential · Netværksdiagnostik | [03-analyse.md](03-analyse.md) |
| 4 | **Overvågning** / Monitoring | Alarmer · Alarm opsætning · AI Alarmcenter *(Nyt)* | [04-overvaagning.md](04-overvaagning.md) |
| 5 | **Målere** / Meters | Målere *(Nyt)* | [05-maalere.md](05-maalere.md) |
| 6 | **Klimaregnskab** / Climate accounting | Klimaregnskab *(Nyt)* · CSRD-rapport *(Nyt)* · Klimaregnskab *(Udfases)* · Emissionsfaktorer *(Nyt)* | [06-klimaregnskab.md](06-klimaregnskab.md) |
| 7 | **Aktiv Styring** / Active control | (single page) | [07-aktiv-styring-rapportering.md](07-aktiv-styring-rapportering.md) |
| 8 | **Rapportering** / Reporting | Brugertilpassede rapporter · Bygningsrapporten · Eksporter forbrugsdata | [07-aktiv-styring-rapportering.md](07-aktiv-styring-rapportering.md) |
| 9 | **Opsætning** / Setup | Stamdata · Brugeradministration *(Nyt)* · Bygningselementer | [08-opsaetning.md](08-opsaetning.md) |

Global shell (header, breadcrumb, toolbar, user menu, left hierarchy panel): [global-chrome.md](global-chrome.md).

## Danish → English glossary (recurring terms)

| da | en |
|---|---|
| Oversigt | Overview |
| Forbrug | Consumption |
| Måler / Målere | Meter / Meters |
| Hovedmåler / Bimåler | Main meter / Sub-meter |
| Fjernaflæst / Manuelt aflæst | Remote-read / Manually read |
| Varme / Vand / Køling / Bygas / Gas / Luftflow | Heat / Water / Cooling / Town gas / Gas / Air flow |
| Alarmer / Alarm opsætning | Alarms / Alarm setup |
| Afkøling / Fjernkøling | Cooling (delta-T) / District cooling |
| Klimaregnskab | Climate / carbon accounting |
| Emissionsfaktorer | Emission factors |
| Bygningselementer | Building elements |
| Ejendom / Bygning / Gruppéring | Property / Building / Grouping |
| Stamdata | Master data |
| Brugeradministration | User administration |
| Rapportering | Reporting |
| Vælg afdeling / Vælg firma | Select department / Select company |
| Senest oprettede | Recently created |
| Udfases / Nyt | Being phased out / New |

## Capture status — COMPLETE

- [x] Master navigation tree · [x] Global chrome
- [x] 01 Oversigt · [x] 02 Resource Insights · [x] 03 Analyse · [x] 04 Overvågning
- [x] 05 Målere · [x] 06 Klimaregnskab · [x] 07 Aktiv Styring + Rapportering + Call to action · [x] 08 Opsætning
- [x] BDD features → `features/frontend/*.feature`
- 27 reference screenshots in `./screenshots/`. The capture helper is `_walker.js` (reusable;
  edit MODULE+LEAVES and run via Playwright `browser_run_code_unsafe`).

### Pages that were gated / not reachable in company scope (recapture from an enabled context if needed)
- **Oversigt ▸ Logs** — admin (department) scope only.
- **Overvågning ▸ AI Alarmcenter** — feature/licence gated (leaf present, does not navigate).
- **Analyse ▸ Raven Residential** and **Analyse ▸ Netværksdiagnostik** — feature/role gated (not in menu for this company).
- Duplicate "Nyt/Udfases" variants of **Målere** and **Klimaregnskab** — same surface, newer/older iteration.
