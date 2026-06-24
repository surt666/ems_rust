# Module 1 — Oversigt (Overview)

Default landing module. Pages: **Dashboard**, **Statistik**, **Logs**.
Scope captured: company **Bolag AB Janus** (`/App/company/997/...`).

---

## 1.1 Dashboard — `overview/dashboard`
Screenshot: `screenshots/company997-01-oversigt-dashboard.png`. Charts: **Highcharts**.

Two-column dashboard of the selected context's energy/cost picture.

### Left column
1. **Bygningsbenchmark** (Building benchmark) card — two gauge widgets side by side:
   - `Performance (omk. i forhold til gnm.)` — cost vs. average.
   - `Performance (CO₂e i forhold til gnm.)` — CO₂e vs. average.
   - Each gauge has a coloured performance band (worse→better) + a needle, plus paired metrics:
     `Afvigelse for alle bygninger` (deviation, all buildings),
     `Afvigelse for bygninger dårligere end gnm.` / `Potentielle besparelser` (potential savings),
     `Afvigelse for bygninger bedre end gnm.` / `Opnåede besparelser` (achieved savings).
     Values shown both in **kr.** and **t CO₂** (e.g. `978,7 t CO₂`, `-8,041 t CO₂`).
   - Controls: dropdown `Opvarmet areal (m²)` (heated-area basis; 2 options),
     dropdown `12 måneder tilbage` (period; 5 options).
2. **Call to action** card — subtitle `5 største energispild` (5 biggest energy wastes);
   empty-state line when none. Source `POST /api/calltoaction/getcalltoaction`.
3. **Alarmer** (Alarms) card — subtitle `5 seneste alarmer` (5 latest); counters
   `Alarmopsætninger (N)` (alarm setups), `Ukvit. alarmer (N)` (unacknowledged); CTA button
   `Gå til alarmopsætning` (go to alarm setup). Source `POST /api/alarm-management/alarmruns`,
   `…/alarmconfigurations`, `…/alarm-feedback/missing-feedback-count`.

### Right column — **Forbrugsoverblik** (Consumption overview)
- A header card + one card **per energy type**: combined total, **Varme** (heat), **Vand** (water),
  **EL** (electricity) (energy types present depend on the context's meters).
- Each card: an energy icon + name + trend arrow, a **monthly bar chart** (current year vs previous),
  a small **gauge** (combined card), and 2–3 totals with up/down trend colouring
  (current period, comparison period, full prior year).
- Controls per card: measure dropdown `Omkostning` (Cost) — toggle cost⇄consumption (2 options);
  year dropdown `2026` (11 options); resolution `month`.
- Sources:
  - `GET /api/dashboard/consumption/all?contextType=&contextId=&resolution=month&graph=<range>&curTotal=<range>&prevTotal=<range>&prevTotalFull=<range>`
  - `GET /api/dashboard/consumption/energyType?...&type=heat|water|electricity&resolution=&curGraph=&prevGraph=&curTotal=&prevTotal=&prevTotalFull=`
  - `GET /api/dashboard/statistics`, `POST /api/insights/building_benchmark`.

### Behaviour
- All cards recompute when the context (hierarchy selection) or year/period dropdowns change.
- Trend arrows/colours encode better/worse vs. comparison period.
- "Gå til alarmopsætning" navigates to Overvågning ▸ Alarm opsætning.

---

## 1.2 Statistik — `overview/statistics`
Screenshots: `screenshots/company997-02-oversigt-statistik.png` (company),
`screenshots/overview-statistics.png` (administrator scope — richer; has extra
`Firmaer` + `Administrator indsigt` cards).

A responsive **grid of stat cards**; each card = a count badge + a small breakdown table. **No charts.**

| Card (da) | Meaning | Content |
|---|---|---|
| **Firmaer N** *(admin scope only)* | Companies | table `Senest oprettede` (name, created date), 5 rows |
| **Bygningselementer N** | Building elements | table: `Bygning` (building), `Ejendom` (property), `Gruppéring` (grouping) |
| **Brugere N** | Users | table by role: Standardbruger, Udvikler, Kiggebruger, Administrationsansvarlig-ED, Firmaansvarlig, Systemansvarlig, Installatør, Administrationsansvarlig-Partner |
| **Alarmer N** | Alarms | list by type: Forbrugsalarm (daglig/time/akk. time/ugentlig), Budgetalarm (månedlig), Afkølingsalarm (månedlig) |
| **Administrator indsigt** *(admin scope only)* | Admin insight | bullet text: active/inactive meters, manual vs remote split, remote total, licensed total |
| **Målere N** (heraf mangler X målere opsætning) | Meters (X missing setup) | several tables ↓ |

Målere sub-tables: `Aflæsningstype` (Manuelt aflæst / Fjernaflæst / Beregningsmålere) ·
`Energi- og ressourcemåler` (Hovedmåler & Bimåler → EL/Varme/Vand/Køling/Bygas/Gas/Luftflow/Frie målertyper) ·
`Andre målertyper` (Hovedmåler, Transport afstand, Transport brændstof, Ressourcer, Bimåler) ·
`Fjernaflæst datatilegnelse (N)` (Datalogger by logger brand "(Antal loggere: n)"; API by source; Filer by sftp/csvfile feed).

Sources: `POST /api/meters/stats`, `POST /api/meters/licensStats`, `GET /api/alarm/statistics?contextType=&contextId=`, `GET /api/dashboard/statistics`.

### Behaviour
- Counts + breakdowns are scoped to the selected context and update on context change.
- Read-only dashboard (no row actions observed).

---

## 1.3 Logs — `overview/logs`
- Present in the Oversigt menu, but **inert in company scope**: a real click does nothing and the
  direct slug `/App/company/997/overview/logs` **redirects to** `setup/company_data?quickLink=true`.
- Interpretation: an **audit/event log** page that is meaningful only in **administrator (department)
  scope**. To be re-captured in `administrator/<id>` scope if we need to recreate it.
  *(Not recreating for company users; flag as admin-only.)*

---

### Recreation notes
- Dashboard = grid of independent, individually-refreshing widgets; each widget owns its own
  query params (period/measure/year). Good HTMX-fragment boundaries (project rule: HTML-over-the-wire).
- Statistik = pure server-rendered count/breakdown tables — trivial as HTML fragments, no JS charts.
- Use CSS grid for the card layouts (project rule: grid, not flex).
